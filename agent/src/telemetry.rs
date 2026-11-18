//! Structured logging, request tracing, minimal metrics and exit taxonomy.
//!
//! The agent intentionally keeps a `std`-only dependency set: `tracing` (and its
//! subscriber machinery) is not available, so structured events are emitted as
//! one JSON object per line on **stderr**. Existing human-readable `eprintln!`
//! messages are left intact; machine readers can select lines that contain an
//! `"event"` key. stdout stays reserved for `--once` JSON.
//!
//! Security rules for everything in this module:
//!
//! - never log `Authorization` values, tokens, or raw query strings,
//! - only the request *path* (no `?...`) is attached to a trace,
//! - `/metrics` exposes counters/gauges only, never machine data or secrets.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// Severity attached to every structured event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// Renders one structured event as a JSON line.
///
/// Kept as a pure function so tests can assert the exact shape without
/// capturing stderr, and so a serialization failure can never abort serving.
pub fn format_event(level: Level, event: &str, fields: Value) -> String {
    let mut value = match fields {
        Value::Object(map) => Value::Object(map),
        other => json!({ "fields": other }),
    };
    if let Value::Object(ref mut map) = value {
        map.insert("ts_ms".to_string(), json!(now_millis()));
        map.insert("level".to_string(), json!(level.as_str()));
        map.insert("event".to_string(), json!(event));
    }
    serde_json::to_string(&value)
        .unwrap_or_else(|_| r#"{"event":"log.serialize.failed"}"#.to_string())
}

/// Emits a structured event on stderr (never stdout).
pub fn emit(level: Level, event: &str, fields: Value) {
    eprintln!("{}", format_event(level, event, fields));
}

/// Stable status bucket used in logs and metrics.
pub fn status_class(status: u16) -> &'static str {
    match status {
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "unknown",
    }
}

/// Minimal in-process metrics: a request counter, status-class counters and
/// two gauges (live connections, uptime). No registry, no new dependency.
pub struct Metrics {
    started: Instant,
    request_ids: AtomicU64,
    requests: AtomicU64,
    responses_2xx: AtomicU64,
    responses_3xx: AtomicU64,
    responses_4xx: AtomicU64,
    responses_5xx: AtomicU64,
    responses_unknown: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            request_ids: AtomicU64::new(1),
            requests: AtomicU64::new(0),
            responses_2xx: AtomicU64::new(0),
            responses_3xx: AtomicU64::new(0),
            responses_4xx: AtomicU64::new(0),
            responses_5xx: AtomicU64::new(0),
            responses_unknown: AtomicU64::new(0),
        }
    }

    /// Monotonic, process-local identifier for a request trace.
    fn next_request_id(&self) -> u64 {
        self.request_ids.fetch_add(1, Ordering::Relaxed)
    }

    fn counter(&self, status: u16) -> Option<&AtomicU64> {
        match status_class(status) {
            "2xx" => Some(&self.responses_2xx),
            "3xx" => Some(&self.responses_3xx),
            "4xx" => Some(&self.responses_4xx),
            "5xx" => Some(&self.responses_5xx),
            "unknown" => Some(&self.responses_unknown),
            _ => None,
        }
    }

    /// Records one completed request and its status bucket.
    pub fn record_response(&self, status: u16) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        if let Some(counter) = self.counter(status) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Renders a Prometheus-compatible text exposition (no dependencies).
    ///
    /// `active` is supplied by the caller because the live-connection gauge
    /// already lives in `SharedState`; duplicating it would risk drift.
    pub fn render_prometheus(&self, active: usize) -> Vec<u8> {
        let uptime = self.started.elapsed().as_secs();
        format!(
            "\
# HELP tm_agent_requests_total Total HTTP requests handled by the agent.
# TYPE tm_agent_requests_total counter
tm_agent_requests_total {requests}
# HELP tm_agent_responses_total HTTP responses by status class.
# TYPE tm_agent_responses_total counter
tm_agent_responses_total{{class=\"2xx\"}} {responses_2xx}
tm_agent_responses_total{{class=\"3xx\"}} {responses_3xx}
tm_agent_responses_total{{class=\"4xx\"}} {responses_4xx}
tm_agent_responses_total{{class=\"5xx\"}} {responses_5xx}
tm_agent_responses_total{{class=\"unknown\"}} {responses_unknown}
# HELP tm_agent_active_connections Currently admitted connections.
# TYPE tm_agent_active_connections gauge
tm_agent_active_connections {active}
# HELP tm_agent_uptime_seconds Seconds since the agent started.
# TYPE tm_agent_uptime_seconds gauge
tm_agent_uptime_seconds {uptime}
",
            requests = self.requests.load(Ordering::Relaxed),
            responses_2xx = self.responses_2xx.load(Ordering::Relaxed),
            responses_3xx = self.responses_3xx.load(Ordering::Relaxed),
            responses_4xx = self.responses_4xx.load(Ordering::Relaxed),
            responses_5xx = self.responses_5xx.load(Ordering::Relaxed),
            responses_unknown = self.responses_unknown.load(Ordering::Relaxed),
        )
        .into_bytes()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// One HTTP request span, mirroring the start/end pair of a tracing span.
///
/// `Drop` is a safety net: if a handler returns early without calling
/// `finish`, the request is still recorded with status `0`/`unknown` instead of
/// silently disappearing from logs and metrics.
pub struct RequestTrace<'a> {
    id: u64,
    method: String,
    path: String,
    started: Instant,
    metrics: &'a Metrics,
    status: Option<u16>,
}

impl<'a> RequestTrace<'a> {
    /// Starts a span. `path` must be the path *only* — callers pass
    /// `request.path`, never the raw target with its query string.
    pub fn begin(metrics: &'a Metrics, method: &str, path: &str) -> Self {
        let id = metrics.next_request_id();
        emit(
            Level::Info,
            "http.request.start",
            json!({ "request_id": id, "method": method, "path": path }),
        );
        Self {
            id,
            method: method.to_string(),
            path: path.to_string(),
            started: Instant::now(),
            metrics,
            status: None,
        }
    }

    /// Used when the request head could not be parsed; no method/path exist.
    pub fn begin_unknown(metrics: &'a Metrics) -> Self {
        Self::begin(metrics, "-", "-")
    }

    /// Records the terminal status. Subsequent calls are no-ops.
    pub fn finish(&mut self, status: u16) {
        if self.status.is_some() {
            return;
        }
        self.status = Some(status);
        self.metrics.record_response(status);
        emit(
            Level::Info,
            "http.request.end",
            json!({
                "request_id": self.id,
                "method": self.method,
                "path": self.path,
                "status": status,
                "outcome": status_class(status),
                "duration_ms": self.started.elapsed().as_millis(),
            }),
        );
    }
}

impl Drop for RequestTrace<'_> {
    fn drop(&mut self) {
        if self.status.is_none() {
            self.finish(0);
        }
    }
}

/// Failure categories mapped to stable process exit codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCategory {
    Success,
    Internal,
    Usage,
    Network,
    Probe,
    Auth,
}

impl ExitCategory {
    pub fn code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::Internal => 1,
            Self::Usage => 2,
            Self::Network => 3,
            Self::Probe => 4,
            Self::Auth => 5,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Internal => "internal",
            Self::Usage => "usage",
            Self::Network => "network",
            Self::Probe => "probe",
            Self::Auth => "auth",
        }
    }
}

/// An error carrying both the operator message and its exit category.
#[derive(Debug)]
pub struct AgentError {
    category: ExitCategory,
    message: String,
}

impl AgentError {
    pub fn new(category: ExitCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    pub fn code(&self) -> i32 {
        self.category.code()
    }

    pub fn category(&self) -> ExitCategory {
        self.category
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Bare `String` errors keep the historical exit code 1, so adding categories
/// cannot silently change the outcome of an existing error path.
impl From<String> for AgentError {
    fn from(message: String) -> Self {
        Self::new(ExitCategory::Internal, message)
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AgentError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads one counter line by its (space-terminated) prefix.
    fn counter_value(rendered: &str, line_prefix: &str) -> u64 {
        rendered
            .lines()
            .find(|line| line.starts_with(line_prefix))
            .and_then(|line| line.rsplit_once(' '))
            .and_then(|(_, value)| value.trim().parse::<u64>().ok())
            .unwrap_or_else(|| panic!("missing counter {line_prefix} in:\n{rendered}"))
    }

    /// Synchronous-sum check: every counted request must land in a class.
    fn total_responses(rendered: &str) -> u64 {
        rendered
            .lines()
            .filter(|line| line.starts_with("tm_agent_responses_total{class="))
            .map(|line| {
                line.rsplit_once(' ')
                    .and_then(|(_, value)| value.trim().parse::<u64>().ok())
                    .expect("response counter line must end with a number")
            })
            .sum()
    }

    #[test]
    fn structured_event_is_json_with_severity_and_timestamp() {
        let line = format_event(
            Level::Warn,
            "test.event",
            json!({ "answer": 42, "path": "/healthz" }),
        );
        let parsed: Value = serde_json::from_str(&line).expect("event must be JSON");
        assert_eq!(parsed["event"], "test.event");
        assert_eq!(parsed["level"], "WARN");
        assert_eq!(parsed["answer"], 42);
        assert_eq!(parsed["path"], "/healthz");
        assert!(parsed["ts_ms"].is_number(), "ts_ms must be numeric");
    }

    #[test]
    fn non_object_fields_are_nested_instead_of_dropped() {
        let line = format_event(Level::Info, "test.scalar", json!(["a", "b"]));
        let parsed: Value = serde_json::from_str(&line).expect("event must be JSON");
        assert_eq!(parsed["fields"][1], "b");
    }

    #[test]
    fn status_classes_are_stable() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(301), "3xx");
        assert_eq!(status_class(401), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(0), "unknown");
    }

    #[test]
    fn trace_finish_updates_metrics_exactly_once() {
        let metrics = Metrics::new();
        let mut trace = RequestTrace::begin(&metrics, "GET", "/healthz");
        trace.finish(200);
        trace.finish(500); // duplicate call must be ignored
        drop(trace);

        let rendered = String::from_utf8(metrics.render_prometheus(1)).expect("utf8");
        assert!(rendered.contains("tm_agent_requests_total 1"), "{rendered}");
        assert!(
            rendered.contains("tm_agent_responses_total{class=\"2xx\"} 1"),
            "{rendered}"
        );
        assert!(
            rendered.contains("tm_agent_responses_total{class=\"5xx\"} 0"),
            "{rendered}"
        );
        assert!(
            rendered.contains("tm_agent_active_connections 1"),
            "{rendered}"
        );
        assert!(rendered.contains("# TYPE tm_agent_uptime_seconds gauge"));
    }

    #[test]
    fn status_zero_record_response_keeps_totals_consistent() {
        let metrics = Metrics::new();
        metrics.record_response(0);
        metrics.record_response(200);
        metrics.record_response(503);

        let rendered = String::from_utf8(metrics.render_prometheus(0)).expect("utf8");
        let requests = counter_value(&rendered, "tm_agent_requests_total ");
        let responses = total_responses(&rendered);
        assert_eq!(requests, 3, "{rendered}");
        assert_eq!(
            responses, requests,
            "every request must land in a class: {rendered}"
        );
        assert_eq!(
            counter_value(&rendered, "tm_agent_responses_total{class=\"unknown\"} "),
            1,
            "{rendered}"
        );
    }

    #[test]
    fn dropped_trace_is_still_counted_as_unknown() {
        let metrics = Metrics::new();
        drop(RequestTrace::begin_unknown(&metrics));
        let rendered = String::from_utf8(metrics.render_prometheus(0)).expect("utf8");
        assert!(rendered.contains("tm_agent_requests_total 1"), "{rendered}");
        assert!(
            rendered.contains("tm_agent_active_connections 0"),
            "{rendered}"
        );
        assert_eq!(
            counter_value(&rendered, "tm_agent_responses_total{class=\"unknown\"} "),
            1,
            "the drop safety net must land in the unknown class: {rendered}"
        );
        assert_eq!(
            total_responses(&rendered),
            counter_value(&rendered, "tm_agent_requests_total "),
            "the drop path must not lose the response from the totals: {rendered}"
        );
    }

    #[test]
    fn exit_categories_have_stable_codes() {
        assert_eq!(ExitCategory::Success.code(), 0);
        assert_eq!(ExitCategory::Internal.code(), 1);
        assert_eq!(ExitCategory::Usage.code(), 2);
        assert_eq!(ExitCategory::Network.code(), 3);
        assert_eq!(ExitCategory::Probe.code(), 4);
        assert_eq!(ExitCategory::Auth.code(), 5);
        let legacy: AgentError = "unknown failure".to_string().into();
        assert_eq!(legacy.code(), 1);
        assert_eq!(legacy.category(), ExitCategory::Internal);
        assert_eq!(legacy.message(), "unknown failure");
    }
}
