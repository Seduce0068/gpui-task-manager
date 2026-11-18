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
//!   response before sleeping, and [`AgentClient`] additionally holds a
//!   non-blocking single-flight permit so a second caller cannot open a
//!   parallel request even if the sampler contract changes (R4-A02),
//! - one fetch is bounded by [`FETCH_TIMEOUT`]; the deadline is driven by
//!   GPUI's executor timer, so the same code path runs on wasm and on the
//!   native test host (no `tokio::time` dependency in the browser),
//! - a response body is read in small chunks and aborted as soon as the
//!   connection settings change mid-read (or the declared size already
//!   exceeds the cap), so a superseded fetch cannot keep pinning up to 8 MiB
//!   of the wasm heap (R4-A02).

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use futures::future::Either;
use futures::{AsyncRead, AsyncReadExt as _};
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

/// Read granularity for the abortable body reader. A superseded fetch drops
/// at most one chunk of work (and heap) before its read is cancelled; small
/// enough to stay comfortably within a wasm stack frame.
const BODY_READ_CHUNK: usize = 16 * 1024;

/// Fixed, redaction-safe error returned to a caller that tries to start a
/// second fetch while one is already in flight. No request is issued at all,
/// so the single-flight contract cannot be broken by a concurrent caller.
const FETCH_BUSY_ERROR: &str = "エージェントへの取得が多重実行中のため、この取得はスキップしました";

/// Fixed error for a body read cancelled because the connection settings
/// changed while it was streaming. Static text: no token or agent data.
const BODY_ABORTED_ERROR: &str = "接続設定が変更されたため応答の読み取りを中止しました";

/// Fixed error strings shared by the abortable reader and its tests.
const BODY_READ_ERROR: &str = "応答の読み取りに失敗しました";
const SIZE_LIMIT_ERROR: &str = "エージェント応答がサイズ上限を超えました";

/// HTTP status codes that mean the loopback agent rejected our credentials.
///
/// The agent answers `401` for a missing/wrong bearer token and `403` for a
/// rejected `Origin`. Both need the same user-facing action: paste the fresh
/// URL (with the new `#token=` fragment) into the address bar (R3-S04/S05).
pub(crate) fn is_auth_failure_status(status: u16) -> bool {
    matches!(status, 401 | 403)
}

/// Fixed, redact-safe guidance for an authentication failure.
///
/// It is a static string with no token, endpoint, hostname or agent-supplied
/// text, so it can flow into the status bar, the `aria-live` mirror and the DOM
/// proof unchanged. The token itself is never echoed — the user re-enters it by
/// pasting the URL the restarted agent printed.
pub const RECONNECT_GUIDANCE: &str = "認証に失敗しました。エージェント再起動後に表示された新しい URL の #token= をアドレスバーへ貼り付けて更新してください";

/// Whether a fetch error must be surfaced as the re-entry flow rather than a
/// generic connectivity error. Kept as a pure predicate for the shell tests.
pub(crate) fn requires_reconnect(error: &str) -> bool {
    error == RECONNECT_GUIDANCE
}

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

/// Connection settings shared with the sampler task.
///
/// The sampler holds an `Arc<AgentClient>`, while `?agent=` / `#token=` may
/// change without a page reload (a restarted agent prints a fresh token). The
/// lock lets the shell re-read the URL on every sampler cycle and hand the new
/// settings to the *same* client, so the single in-flight fetch contract stays
/// intact (R3-S04).
#[derive(Debug)]
struct SharedConfig {
    config: RwLock<AgentConfig>,
    /// Bumped only on a real change. An in-flight body read captures this value
    /// at fetch start and aborts early if it changes mid-read, so a stale
    /// response stops consuming wasm heap as soon as new settings arrive.
    generation: AtomicU64,
}

impl SharedConfig {
    fn new(config: AgentConfig) -> Self {
        Self {
            config: RwLock::new(config),
            generation: AtomicU64::new(0),
        }
    }

    /// Snapshot for one request. Cloned out of the lock so no guard is held
    /// across an `await`.
    fn snapshot(&self) -> AgentConfig {
        self.config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Current settings generation, safe to poll from an async body read
    /// without holding the config lock across an `await`.
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Replaces the settings, returning `true` only when they really changed.
    /// A real change also invalidates any body read already in flight.
    fn replace(&self, fresh: AgentConfig) -> bool {
        let mut current = self.config.write().unwrap_or_else(PoisonError::into_inner);
        if *current == fresh {
            return false;
        }
        *current = fresh;
        self.generation.fetch_add(1, Ordering::SeqCst);
        true
    }
}

/// Non-blocking single-flight permit, enforced at the client boundary.
///
/// The sampler awaits each poll, but that is a caller-side convention: a
/// restarted agent's config reload (or any second caller) must not be able to
/// start a parallel HTTP request, since each in-flight body pins heap on wasm.
/// A losing caller gets [`FETCH_BUSY_ERROR`] immediately and issues no request.
#[derive(Clone, Debug, Default)]
struct FetchGate(Arc<AtomicBool>);

impl FetchGate {
    /// Reserves the only slot, or returns `None` when a fetch is in flight.
    fn try_acquire(&self) -> Option<FetchGuard> {
        self.0
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| FetchGuard(Arc::clone(&self.0)))
    }
}

/// RAII permit for [`FetchGate`]. Dropped on success, error *or* cancellation
/// (a deadline drop unwinds the future holding this guard), releasing the slot.
#[derive(Debug)]
struct FetchGuard(Arc<AtomicBool>);

impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// HTTP client for the loopback agent.
pub struct AgentClient {
    config: SharedConfig,
    http: Arc<dyn HttpClient>,
    /// Supplies [`FETCH_TIMEOUT`] deadlines on both wasm and native hosts.
    executor: BackgroundExecutor,
    /// Enforces at most one in-flight fetch regardless of caller behaviour.
    gate: FetchGate,
}

impl AgentClient {
    /// Creates a client for the given configuration.
    pub fn new(
        config: AgentConfig,
        http: Arc<dyn HttpClient>,
        executor: BackgroundExecutor,
    ) -> Self {
        Self {
            config: SharedConfig::new(config),
            http,
            executor,
            gate: FetchGate::default(),
        }
    }

    /// Re-reads connection settings from the current page URL.
    ///
    /// Returns `true` when the endpoint or token changed, so the caller can
    /// notify the UI and start a fresh connection attempt. No network request is
    /// issued here: the already-running sampler picks the new config up on its
    /// next tick.
    pub fn set_config(&self, config: AgentConfig) -> bool {
        self.config.replace(config)
    }

    /// Endpoint label for the status bar.
    pub fn endpoint_label(&self) -> String {
        self.config.snapshot().endpoint
    }

    /// Requests and parses one snapshot, with a hard [`FETCH_TIMEOUT`] deadline.
    ///
    /// Errors are returned as user-facing Japanese strings with the token
    /// redacted; they never trigger a silent fallback to demo data. The
    /// single-flight permit is held until this method resolves (success, error
    /// or deadline cancellation), so no second request can overlap this one.
    pub async fn fetch_snapshot(&self) -> Result<SnapshotResponse, String> {
        // Losing callers never reach `http.send`: a parallel fetch cannot
        // start, which is the wasm heap bound R4-A02 asks for.
        let _permit = match self.gate.try_acquire() {
            Some(permit) => permit,
            None => return Err(FETCH_BUSY_ERROR.to_string()),
        };
        let deadline = self.executor.timer(FETCH_TIMEOUT);
        with_deadline(self.fetch_snapshot_inner(), deadline).await
    }

    /// The actual request/parse pipeline, kept separate so the deadline can
    /// cover the body read as well as the response headers.
    async fn fetch_snapshot_inner(&self) -> Result<SnapshotResponse, String> {
        // Snapshot once per request, after the shell may have re-read the URL.
        // The guard is dropped here, never held across the awaits below.
        let config = self.config.snapshot();
        // Remember the settings generation so the body read can abort early if
        // a reload supersedes this request while it is streaming (R4-A02).
        let generation = self.config.generation();
        // Never attach the bearer token before checking the destination.
        if !is_loopback_http_endpoint(&config.endpoint) {
            return Err("エージェントの接続先は loopback の http URL のみ許可されます".to_string());
        }
        let url = snapshot_url(&config.endpoint);
        let mut builder = Request::builder().method("GET").uri(&url);
        if let Some(token) = &config.token {
            // クエリではなくヘッダで渡す（URL・Referer・アクセスログに残さない）。
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = builder.body(AsyncBody::empty()).map_err(|error| {
            format!(
                "リクエストの組み立てに失敗しました: {}",
                redact_token(&format!("{error:#}"), config.token.as_deref())
            )
        })?;
        let response = self.http.send(request).await.map_err(|error| {
            // 画面には 1 行の要約だけを出し、詳細（スタックを含む）は
            // トークンを伏せたうえで console に残す。
            log::warn!(
                "エージェントへの fetch に失敗: {}",
                redact_token(&format!("{error:#}"), config.token.as_deref())
            );
            "接続に失敗しました（エージェントが起動しているか、URL とトークンを確認してください）"
                .to_string()
        })?;

        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            if is_auth_failure_status(code) {
                // A restarted agent invalidates its token. Never echo the
                // response body or the token: the fixed guidance is the whole
                // re-entry flow, safe for the DOM and the aria-live mirror.
                // No body is read at all, so the reconnect path stays cheap.
                return Err(RECONNECT_GUIDANCE.to_string());
            }
            return Err(format!("エージェントが HTTP {code} を返しました"));
        }

        // Earliest possible abort: a declared oversized body is rejected from
        // the header alone, before a single byte is pulled into the wasm heap.
        let declared_length = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok());
        if content_length_exceeds_cap(declared_length) {
            return Err(SIZE_LIMIT_ERROR.to_string());
        }
        let body = response.into_body();
        let mut buffer = Vec::new();
        // Chunked and abortable: if the settings change mid-read, drop the
        // stream immediately instead of accumulating up to 8 MiB of heap for a
        // response we will no longer use (R4-A02).
        read_body_abortable(
            body,
            &mut buffer,
            MAX_SNAPSHOT_BYTES,
            config.token.as_deref(),
            || self.config.generation() != generation,
        )
        .await
        .map_err(|error| error.to_string())?;

        let envelope: SnapshotEnvelope = serde_json::from_slice(&buffer).map_err(|error| {
            format!(
                "スナップショット JSON の解析に失敗しました: {}",
                redact_token(&format!("{error:#}"), config.token.as_deref())
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

/// Whether a declared `Content-Length` already proves the body is too large.
///
/// Used before reading anything, so an oversized response never enters the
/// wasm heap. Missing, malformed or small values are left to the streaming
/// reader, which enforces the same cap incrementally.
fn content_length_exceeds_cap(declared: Option<&str>) -> bool {
    declared
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|len| len >= MAX_SNAPSHOT_BYTES)
}

/// Reads `body` into `out`, stopping as soon as `aborted()` turns true or the
/// `limit` would be exceeded.
///
/// `read_to_end` would keep pulling until EOF (or the whole 8 MiB cap) even
/// after the request is no longer wanted, pinning heap on wasm. Chunking makes
/// the read cancellable between chunks: a superseded fetch releases its buffer
/// and drops the underlying stream after at most one extra chunk of work.
/// Returns one of the static error keys above; no token or agent text is ever
/// embedded in the result.
async fn read_body_abortable<R, F>(
    mut body: R,
    out: &mut Vec<u8>,
    limit: u64,
    token: Option<&str>,
    aborted: F,
) -> Result<(), &'static str>
where
    R: AsyncRead + Unpin,
    F: Fn() -> bool,
{
    let mut scratch = [0u8; BODY_READ_CHUNK];
    loop {
        if aborted() {
            return Err(BODY_ABORTED_ERROR);
        }
        let used = out.len() as u64;
        if used >= limit {
            return Err(SIZE_LIMIT_ERROR);
        }
        let want = (limit - used).min(BODY_READ_CHUNK as u64) as usize;
        let read = body.read(&mut scratch[..want]).await.map_err(|error| {
            // Same detail-to-console / summary-to-UI split as the old reader:
            // the token is masked before anything is logged.
            log::warn!(
                "エージェント応答の読み取りに失敗: {}",
                redact_token(&format!("{error:#}"), token)
            );
            BODY_READ_ERROR
        })?;
        if read == 0 {
            return Ok(());
        }
        out.extend_from_slice(&scratch[..read]);
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

/// Accepts wire versions this client understands, including older agents.
///
/// A newer agent may add fields that serde defaults can ignore; only a
/// *future* version (which may change field meaning) is rejected.
fn check_schema_version(version: u32) -> Result<(), String> {
    if version <= WIRE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!(
            "スキーマバージョンが新しすぎます (agent={version}, web={WIRE_SCHEMA_VERSION})"
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
        assert!(check_schema_version(WIRE_SCHEMA_VERSION - 1).is_ok());
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
    fn auth_statuses_are_classified_for_the_reentry_flow() {
        for auth in [401, 403] {
            assert!(is_auth_failure_status(auth), "{auth} must need re-entry");
        }
        for not_auth in [200, 204, 400, 404, 429, 500, 503] {
            assert!(
                !is_auth_failure_status(not_auth),
                "{not_auth} must stay a generic error"
            );
        }
    }

    #[test]
    fn reconnect_guidance_is_actionable_and_redaction_safe() {
        // The guidance tells the user *what* to re-enter and *where*.
        assert!(RECONNECT_GUIDANCE.contains("#token="));
        assert!(RECONNECT_GUIDANCE.contains("アドレスバー"));
        // Fixed text: no `;` / newline that could corrupt the DOM proof, and
        // redacting it with any token leaves it byte-for-byte identical.
        assert!(!RECONNECT_GUIDANCE.contains(';'));
        assert!(!RECONNECT_GUIDANCE.contains('\n'));
        assert_eq!(
            redact_token(RECONNECT_GUIDANCE, Some("super-secret-agent-token")),
            RECONNECT_GUIDANCE
        );
        assert!(requires_reconnect(RECONNECT_GUIDANCE));
        assert!(!requires_reconnect("接続に失敗しました"));
        assert!(!requires_reconnect("エージェントが HTTP 500 を返しました"));
    }

    #[test]
    fn config_reload_replaces_changed_settings_and_ignores_identical_ones() {
        let shared = SharedConfig::new(AgentConfig::new(
            "http://127.0.0.1:18787",
            Some("old-token".into()),
        ));
        // An unchanged URL (the normal poll) must not churn the UI state.
        assert!(!shared.replace(AgentConfig::new(
            "http://127.0.0.1:18787",
            Some("old-token".into()),
        )));
        // A restarted agent prints a fresh token in the same URL fragment.
        assert!(shared.replace(AgentConfig::new(
            "http://127.0.0.1:18787",
            Some("new-token".into()),
        )));
        let snapshot = shared.snapshot();
        assert_eq!(snapshot.endpoint, "http://127.0.0.1:18787");
        assert_eq!(snapshot.token.as_deref(), Some("new-token"));
        // A changed endpoint (new port) is reloaded too.
        assert!(shared.replace(AgentConfig::new(
            "http://127.0.0.1:18788",
            Some("new-token".into()),
        )));
        assert_eq!(shared.snapshot().endpoint, "http://127.0.0.1:18788");
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

    #[test]
    fn single_flight_gate_allows_one_fetch_and_releases_on_drop() {
        let gate = FetchGate::default();
        let permit = gate.try_acquire().expect("the first slot must be free");
        assert!(
            gate.try_acquire().is_none(),
            "a parallel fetch must be rejected before any request is sent"
        );
        drop(permit);
        assert!(
            gate.try_acquire().is_some(),
            "the slot must be reusable after the fetch resolves"
        );
    }

    #[test]
    fn deadline_cancellation_releases_the_single_flight_permit() {
        // A timed-out fetch drops the inner future; the guard lives in the
        // outer future and must be released too, or every later poll is busy.
        let gate = FetchGate::default();
        let observed = gate.clone();
        let result = futures::executor::block_on(with_deadline(
            async move {
                let _permit = observed.try_acquire().expect("free slot");
                futures::future::pending::<Result<u32, String>>().await
            },
            async {},
        ));
        assert_eq!(result, Err(fetch_timeout_error()));
        assert!(
            gate.try_acquire().is_some(),
            "a cancelled fetch must not leave the gate stuck"
        );
    }

    #[test]
    fn content_length_precheck_rejects_only_oversized_values() {
        assert!(content_length_exceeds_cap(Some("9000000")));
        assert!(content_length_exceeds_cap(Some(
            &MAX_SNAPSHOT_BYTES.to_string()
        )));
        for acceptable in [Some("140000"), Some("0"), Some("garbage"), Some("-1"), None] {
            assert!(
                !content_length_exceeds_cap(acceptable),
                "{acceptable:?} must fall through to the streaming cap"
            );
        }
    }

    #[test]
    fn body_reader_aborts_after_one_chunk_when_superseded() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let checks = std::sync::Arc::new(AtomicUsize::new(0));
        let seen = checks.clone();
        let bytes = vec![7u8; BODY_READ_CHUNK * 3];
        let body = bytes.as_slice(); // `&[u8]` implements `futures::AsyncRead`.
        let mut out = Vec::new();
        let error = futures::executor::block_on(read_body_abortable(
            body,
            &mut out,
            MAX_SNAPSHOT_BYTES,
            None,
            || seen.fetch_add(1, Ordering::SeqCst) >= 1,
        ))
        .expect_err("a superseded read must abort early");
        assert_eq!(error, BODY_ABORTED_ERROR);
        assert_eq!(
            out.len(),
            BODY_READ_CHUNK,
            "at most one chunk is consumed after a config change"
        );
        assert!(out.len() < BODY_READ_CHUNK * 3);
        assert!(checks.load(Ordering::SeqCst) >= 2);
        // Static error: safe for the DOM/aria-live mirror and token-free.
        assert_eq!(redact_token(error, Some("secret-token")), error);
    }

    #[test]
    fn body_reader_enforces_the_cap_with_the_original_boundary() {
        let exact = vec![1u8; 10];
        let mut at_cap = Vec::new();
        let error = futures::executor::block_on(read_body_abortable(
            exact.as_slice(),
            &mut at_cap,
            10,
            None,
            || false,
        ))
        .expect_err("a body that reaches the cap must be rejected");
        assert_eq!(error, SIZE_LIMIT_ERROR);
        assert_eq!(at_cap.len(), 10);

        let small_bytes = vec![1u8, 2, 3];
        let mut small = Vec::new();
        futures::executor::block_on(read_body_abortable(
            small_bytes.as_slice(),
            &mut small,
            10,
            None,
            || false,
        ))
        .expect("a body below the cap must stream in full");
        assert_eq!(small, vec![1, 2, 3]);
    }

    #[test]
    fn body_reader_io_failure_stays_redaction_safe() {
        struct Failing;
        impl AsyncRead for Failing {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                _buf: &mut [u8],
            ) -> std::task::Poll<std::io::Result<usize>> {
                std::task::Poll::Ready(Err(std::io::Error::other("token=secret")))
            }
        }

        let mut out = Vec::new();
        let error = futures::executor::block_on(read_body_abortable(
            Failing,
            &mut out,
            10,
            Some("secret"),
            || false,
        ))
        .expect_err("an IO error must be reported");
        assert_eq!(error, BODY_READ_ERROR);
        assert_eq!(redact_token(error, Some("secret")), error);
        assert!(!error.contains("secret"));
    }

    #[test]
    fn config_reload_bumps_generation_only_on_a_real_change() {
        use std::sync::atomic::Ordering;

        let shared = SharedConfig::new(AgentConfig::new("http://127.0.0.1:18787", None));
        let before = shared.generation();
        assert!(!shared.replace(AgentConfig::new("http://127.0.0.1:18787", None)));
        assert_eq!(
            shared.generation(),
            before,
            "an unchanged URL must not abort an in-flight read"
        );
        assert!(shared.replace(AgentConfig::new("http://127.0.0.1:18788", None)));
        assert_eq!(
            shared.generation(),
            before + 1,
            "a real change must invalidate the old read"
        );
    }

    #[test]
    fn new_abort_errors_are_redaction_safe_and_not_auth_failures() {
        for error in [FETCH_BUSY_ERROR, BODY_ABORTED_ERROR] {
            assert_eq!(redact_token(error, Some("secret-agent-token")), error);
            assert!(!error.contains("secret"));
            // The DOM proof's reconnect flag is driven by requires_reconnect;
            // neither new path may flip it (R3-S04/S05 hook stays intact).
            assert!(!requires_reconnect(error));
        }
    }
}
