//! Read-only client for the loopback `tm-agent`.
//!
//! The browser shell cannot read `/proc`, so in "live" mode it polls a local
//! agent over HTTP: `GET /api/v1/snapshot?limit=500` returns
//! `{ schema_version, total_process_count, snapshot }`.
//!
//! Security notes:
//! - the token is read from the URL **fragment** (`#token=...`), which browsers
//!   never send to a server or in the `Referer` header, and is presented to the
//!   agent as `Authorization: Bearer <token>` (never as a query parameter),
//! - the token is never logged or embedded in the HTML; error strings are
//!   redacted before they reach the UI,
//! - the client only ever issues GET requests (the browser shell is read-only),
//! - exactly one fetch is in flight at a time: the sampler loop awaits each
//!   response before sleeping, so a slow agent cannot build a request pile-up,
//! - one fetch is bounded by [`FETCH_TIMEOUT`]; the deadline is driven by
//!   GPUI's executor timer, so the same code path runs on wasm and on the
//!   native test host (no `tokio::time` dependency in the browser).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::AsyncReadExt as _;
use futures::future::Either;
use gpui::BackgroundExecutor;
use gpui::http_client::http::{Request, header};
use gpui::http_client::{AsyncBody, HttpClient};
use serde::Deserialize;
use tm_core::{SystemSnapshot, WIRE_SCHEMA_VERSION};

/// Number of processes requested from the agent.
///
/// The agent clamps this to its own hard upper bound, and it sorts by CPU before
/// truncating, so the table always receives the busiest processes first.
pub const SNAPSHOT_LIMIT: usize = 500;

/// Hard cap on a single snapshot response body (8 MiB), protecting the wasm
/// heap from a broken or malicious loopback agent.
pub const MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;

/// Deadline for one whole snapshot fetch (request headers **and** body read).
///
/// A stalled loopback agent must not park the sampler forever (R2-X01). The
/// browser has no `tokio::time`, so the deadline is produced by the GPUI
/// executor injected into [`AgentClient`]. Once it fires the in-flight fetch
/// future is dropped (which cancels the underlying request) and the sampler
/// is free to retry on its next tick — still one request at a time.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Endpoint and token parsed from the page URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    /// Base URL such as `http://127.0.0.1:18787` (no trailing slash).
    pub endpoint: String,
    /// Optional token, read from the URL fragment and sent as an
    /// `Authorization: Bearer` header (never as a query parameter).
    pub token: Option<String>,
}

impl AgentConfig {
    /// Trims the endpoint and drops empty query values.
    pub fn new(endpoint: impl Into<String>, token: Option<String>) -> Self {
        Self {
            endpoint: endpoint.into().trim().trim_end_matches('/').to_string(),
            token: token
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty()),
        }
    }

    /// Label shown in the status bar (never includes the token).
    pub fn endpoint_label(&self) -> &str {
        &self.endpoint
    }
}

/// Wire envelope returned by `/api/v1/snapshot`.
#[derive(Debug, Deserialize)]
struct SnapshotEnvelope {
    schema_version: u32,
    total_process_count: usize,
    snapshot: SystemSnapshot,
}

/// Returns `true` only for a loopback HTTP endpoint.
///
/// The token comes from the URL fragment and is sent as a Bearer header. If a
/// page can be made to load with `?agent=https://attacker.example/`, the token
/// would be handed to that host. Keep this check textual so it stays testable
/// on the native host (no `web_sys::Url` needed).
pub(crate) fn is_loopback_http_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    // An IPv6 literal contains colons; only split off a port after the closing
    // bracket so `http://[::1]` (default port) is accepted too.
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let Some(end) = inner.find(']') else {
            return false;
        };
        let host = &inner[..end];
        let tail = &inner[end + 1..];
        if !tail.is_empty() && !tail.starts_with(':') {
            return false;
        }
        (format!("[{host}]"), tail.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), Some(port)),
            None => (authority.to_string(), None),
        }
    };
    let host_ok = host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host.eq_ignore_ascii_case("[::1]");
    if !host_ok {
        return false;
    }
    match port {
        None => true,
        Some(port) => !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()),
    }
}

/// One successful snapshot plus the agent's pre-truncation process count.
#[derive(Debug)]
pub struct SnapshotResponse {
    /// Parsed snapshot.
    pub snapshot: SystemSnapshot,
    /// Number of processes on the machine before `?limit=` was applied.
    pub total_process_count: usize,
}

/// HTTP client for the loopback agent.
pub struct AgentClient {
    config: AgentConfig,
    http: Arc<dyn HttpClient>,
    /// Supplies [`FETCH_TIMEOUT`] deadlines on both wasm and native hosts.
    executor: BackgroundExecutor,
}

impl AgentClient {
    /// Creates a client for the given configuration.
    pub fn new(
        config: AgentConfig,
        http: Arc<dyn HttpClient>,
        executor: BackgroundExecutor,
    ) -> Self {
        Self {
            config,
            http,
            executor,
        }
    }

    /// Endpoint label for the status bar.
    pub fn endpoint_label(&self) -> &str {
        self.config.endpoint_label()
    }

    /// Requests and parses one snapshot, with a hard [`FETCH_TIMEOUT`] deadline.
    ///
    /// Errors are returned as user-facing Japanese strings with the token
    /// redacted; they never trigger a silent fallback to demo data. Racing the
    /// inner future against the timer leaves the "one fetch in flight" contract
    /// untouched: this method still resolves (success or error) before the
    /// sampler sleeps and issues the next request.
    pub async fn fetch_snapshot(&self) -> Result<SnapshotResponse, String> {
        let deadline = self.executor.timer(FETCH_TIMEOUT);
        with_deadline(self.fetch_snapshot_inner(), deadline).await
    }

    /// The actual request/parse pipeline, kept separate so the deadline can
    /// cover the body read as well as the response headers.
    async fn fetch_snapshot_inner(&self) -> Result<SnapshotResponse, String> {
        // Never attach the bearer token before checking the destination.
        if !is_loopback_http_endpoint(&self.config.endpoint) {
            return Err("エージェントの接続先は loopback の http URL のみ許可されます".to_string());
        }
        let url = snapshot_url(&self.config.endpoint);
        let mut builder = Request::builder().method("GET").uri(&url);
        if let Some(token) = &self.config.token {
            // クエリではなくヘッダで渡す（URL・Referer・アクセスログに残さない）。
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = builder.body(AsyncBody::empty()).map_err(|error| {
            format!(
                "リクエストの組み立てに失敗しました: {}",
                redact_token(&format!("{error:#}"), self.config.token.as_deref())
            )
        })?;
        let response = self.http.send(request).await.map_err(|error| {
            // 画面には 1 行の要約だけを出し、詳細（スタックを含む）は
            // トークンを伏せたうえで console に残す。
            log::warn!(
                "エージェントへの fetch に失敗: {}",
                redact_token(&format!("{error:#}"), self.config.token.as_deref())
            );
            "接続に失敗しました（エージェントが起動しているか、URL とトークンを確認してください）"
                .to_string()
        })?;

        let status = response.status();
        let body = response.into_body();
        let mut buffer = Vec::new();
        // A broken or hostile loopback agent must not be able to exhaust the
        // wasm heap with an unbounded response. The normal 500-process payload
        // is far below this cap; 8 MiB leaves ample headroom.
        body.take(MAX_SNAPSHOT_BYTES)
            .read_to_end(&mut buffer)
            .await
            .map_err(|error| {
                log::warn!(
                    "エージェント応答の読み取りに失敗: {}",
                    redact_token(&format!("{error:#}"), self.config.token.as_deref())
                );
                "応答の読み取りに失敗しました".to_string()
            })?;
        if buffer.len() as u64 >= MAX_SNAPSHOT_BYTES {
            return Err("エージェント応答がサイズ上限を超えました".to_string());
        }

        if !status.is_success() {
            return Err(format!(
                "エージェントが HTTP {} を返しました",
                status.as_u16()
            ));
        }

        let envelope: SnapshotEnvelope = serde_json::from_slice(&buffer).map_err(|error| {
            format!(
                "スナップショット JSON の解析に失敗しました: {}",
                redact_token(&format!("{error:#}"), self.config.token.as_deref())
            )
        })?;
        check_schema_version(envelope.schema_version)?;
        // Never trust display strings from the wire: a malicious or corrupted
        // agent could inject ANSI/control characters into the canvas (R2X-D1).
        let mut snapshot = envelope.snapshot;
        tm_core::sanitize::sanitize_snapshot(&mut snapshot);
        Ok(SnapshotResponse {
            snapshot,
            total_process_count: envelope.total_process_count,
        })
    }
}

/// Races a fallible operation against a deadline future.
///
/// Both futures are injected by the caller: the operation is the real fetch,
/// the deadline is `BackgroundExecutor::timer` on wasm (or any future in unit
/// tests). When the deadline wins, the still-pending operation is dropped —
/// cancelling the request — and a redaction-free timeout error is returned.
async fn with_deadline<T, F, Deadline>(future: F, deadline: Deadline) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
    Deadline: Future<Output = ()>,
{
    futures::pin_mut!(future, deadline);
    match futures::future::select(future, deadline).await {
        Either::Left((result, _deadline)) => result,
        Either::Right(((), _cancelled_fetch)) => Err(fetch_timeout_error()),
    }
}

/// User-facing timeout message. Intentionally static: no token, endpoint or
/// agent-supplied text is interpolated, so it is safe for the DOM (redact
/// contract) and identical on every failure path.
fn fetch_timeout_error() -> String {
    format!(
        "エージェントからの取得がタイムアウトしました（{}秒）",
        FETCH_TIMEOUT.as_secs()
    )
}

/// Rejects a wire schema version this client does not understand.
fn check_schema_version(version: u32) -> Result<(), String> {
    if version == WIRE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!(
            "スキーマバージョンが一致しません (agent={version}, web={WIRE_SCHEMA_VERSION})"
        ))
    }
}

/// Builds the request URL. The token is **not** part of the URL.
fn snapshot_url(endpoint: &str) -> String {
    format!("{endpoint}/api/v1/snapshot?limit={SNAPSHOT_LIMIT}")
}

/// Removes the token from any diagnostic string.
fn redact_token(message: &str, token: Option<&str>) -> String {
    let Some(token) = token else {
        return message.to_string();
    };
    message
        .replace(token, "***")
        .replace(&encode_query_value(token), "***")
}

/// Percent-encodes one query-string value (RFC 3986 unreserved characters kept).
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_wire_schema_version_is_rejected() {
        assert!(check_schema_version(WIRE_SCHEMA_VERSION).is_ok());
        let error = check_schema_version(WIRE_SCHEMA_VERSION + 1).expect_err("must reject");
        assert!(error.contains("スキーマバージョン"));
        assert!(error.contains(&(WIRE_SCHEMA_VERSION + 1).to_string()));
    }

    #[test]
    fn response_size_cap_leaves_room_for_a_full_snapshot() {
        // A 500-process payload is ~140 KB on the development host; the cap
        // must be comfortably above it while still bounding the wasm heap.
        assert_eq!(MAX_SNAPSHOT_BYTES, 8 * 1024 * 1024);
        // Inline const keeps this a compile-time check without tripping
        // clippy::assertions_on_constants under `-D warnings`.
        const { assert!(MAX_SNAPSHOT_BYTES > 1_000_000) };
    }

    #[test]
    fn only_loopback_http_endpoints_are_allowed() {
        for allowed in [
            "http://127.0.0.1:18787",
            "http://localhost:5173",
            "http://[::1]:18787",
            "http://[::1]",
            "http://127.0.0.1",
        ] {
            assert!(
                is_loopback_http_endpoint(allowed),
                "{allowed} should be allowed"
            );
        }
        for rejected in [
            "https://127.0.0.1:18787",
            "http://192.168.1.10:18787",
            "http://127.0.0.1.evil.example",
            "ftp://127.0.0.1:18787",
            "http://localhost:",
            "http://127.0.0.1:80:80",
            "",
        ] {
            assert!(
                !is_loopback_http_endpoint(rejected),
                "{rejected} should be rejected"
            );
        }
    }

    #[test]
    fn endpoint_is_normalized() {
        let config = AgentConfig::new("  http://127.0.0.1:18787/  ", Some("  ".into()));
        assert_eq!(config.endpoint, "http://127.0.0.1:18787");
        assert_eq!(config.token, None);
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode_query_value("abc-123_~"), "abc-123_~");
        assert_eq!(encode_query_value("a b&c"), "a%20b%26c");
    }

    #[test]
    fn token_is_redacted_from_errors() {
        let redacted = redact_token(
            "fetch failed for ...?limit=500&token=secret%20token",
            Some("secret token"),
        );
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("***"));
    }

    #[test]
    fn snapshot_url_has_no_token() {
        let url = snapshot_url("http://127.0.0.1:18787");
        assert_eq!(url, "http://127.0.0.1:18787/api/v1/snapshot?limit=500");
        assert!(!url.contains("token"));
    }

    #[test]
    fn fetch_timeout_is_bounded() {
        // Long enough for a 500-process loopback payload, short enough that a
        // stalled sampler recovers within a few polling intervals (R2-X01).
        const { assert!(FETCH_TIMEOUT.as_secs() >= 1) };
        const { assert!(FETCH_TIMEOUT.as_secs() <= 30) };
    }

    #[test]
    fn deadline_passes_through_a_fast_success() {
        let result = futures::executor::block_on(with_deadline(
            async { Ok::<u32, String>(42) },
            futures::future::pending::<()>(),
        ));
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn deadline_passes_through_the_inner_error_unchanged() {
        let result = futures::executor::block_on(with_deadline(
            async { Err::<u32, String>("接続に失敗しました".to_string()) },
            futures::future::pending::<()>(),
        ));
        assert_eq!(result, Err("接続に失敗しました".to_string()));
    }

    #[test]
    fn deadline_cancels_a_stalled_fetch_and_returns_a_redacted_error() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct DropFlag(std::sync::Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = std::sync::Arc::new(AtomicBool::new(false));
        let observed = dropped.clone();
        let result = futures::executor::block_on(with_deadline(
            async move {
                let _flag = DropFlag(observed);
                futures::future::pending::<Result<u32, String>>().await
            },
            async {},
        ));

        let error = result.expect_err("a stalled fetch must time out");
        assert_eq!(error, fetch_timeout_error());
        assert!(error.contains("タイムアウト"));
        // No token/endpoint interpolation — the message itself is redaction-safe.
        assert_eq!(redact_token(&error, Some("super-secret-token")), error);
        assert!(!error.contains("secret"));
        // Cancelling the request is what keeps at most one fetch in flight.
        assert!(
            dropped.load(Ordering::SeqCst),
            "timeout must drop the pending fetch"
        );
    }
}
