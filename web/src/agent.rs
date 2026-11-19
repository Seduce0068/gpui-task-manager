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
//! - every response snapshot is sanitised **and** redacted on arrival: the agent's
//!   `--redact` flag defaults to false, so the web side recovers identifiers from
//!   the snapshot itself instead of trusting the agent to mask them,
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

use std::fmt;
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

/// Fixed guidance for a rejected connection origin (`403`).
///
/// Unlike [`RECONNECT_GUIDANCE`] this is **not** a token problem: pasting a new
/// URL would not help, so the DOM proof's `reconnect=` flag stays `0`. Static
/// text with no token, endpoint, hostname or agent-supplied data. The agent's
/// allowed origins are hard-coded to loopback with no user-facing setting, so
/// the text describes the actual constraint instead of asking for a change
/// the user cannot make.
pub const FORBIDDEN_GUIDANCE: &str = "別ホスト／別オリジンからのアクセスはサポートしていません。アプリとブラウザを同一ホストで起動してください";

/// Fixed, redact-safe guidance for an authentication failure (`401`).
///
/// It is a static string with no token, endpoint, hostname or agent-supplied
/// text, so it can flow into the status bar, the `aria-live` mirror and the DOM
/// proof unchanged. The token itself is never echoed. The agent reuses its token
/// file on restart (no new URL is printed), so the user is directed to that
/// file's contents instead of hunting for a "new" token (R2-AUTHMSG).
pub const RECONNECT_GUIDANCE: &str = "認証に失敗しました。トークンファイル（既定は XDG state 配下）の内容を #token= に貼り付けてください。あるいは --token-file を指定して再起動してください。再起動ではトークンは変わりません";

/// Fixed guidance for a transport failure. The underlying error detail is
/// written to `console` after redaction; only this summary reaches the UI.
const CONNECT_ERROR: &str =
    "接続に失敗しました（エージェントが起動しているか、URL とトークンを確認してください）";

/// Fixed guidance for a transient failure (`429` / other `5xx` / timeout).
///
/// The sampler already retries on its next tick, so the only useful next action
/// concerns a *persistent* failure. Static text keeps the DOM proof redaction-
/// safe: no status code, body or agent-supplied string is interpolated (R2-AUTHMSG).
const TRANSIENT_RETRY_GUIDANCE: &str =
    "自動で再試行します。解消しない場合はエージェントを再起動してください";

/// Fixed reason for `503`. The agent returns it when its concurrent-connection
/// limit is reached (`MAX_CONNECTIONS` / `MAX_STREAMS` on the agent side), which
/// is exactly the `body.message` cause. Kept static instead of echoing the body:
/// the error path reads no body, so no request-derived text can reach the UI.
const SERVICE_UNAVAILABLE_GUIDANCE: &str = "エージェントの同時接続数が上限に達しました。自動で再試行します。解消しない場合はエージェントを再起動してください";

/// Fixed guidance for a non-loopback endpoint.
const INVALID_ENDPOINT_ERROR: &str = "エージェントの接続先は loopback の http URL のみ許可されます";

/// Every failure mode of one snapshot fetch.
///
/// The enum carries only typed data (status codes, versions and already
/// redacted detail strings); all user-facing Japanese text is produced by the
/// [`Display`](fmt::Display) impl below. This keeps the redaction boundary in
/// one place and lets the DOM proof decide `reconnect=` by *variant* instead
/// of comparing display strings (R3-S04/S05, C2-WEB).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// `401`: missing/wrong bearer token — the only reconnect trigger.
    Auth,
    /// `403`: rejected `Origin` — actionable, but a new token will not help.
    Forbidden,
    /// Any other non-success HTTP status.
    Http(u16),
    /// The whole fetch exceeded [`FETCH_TIMEOUT`].
    Timeout,
    /// A second fetch was rejected by the single-flight gate.
    Busy,
    /// The body read was cancelled because connection settings changed.
    BodyAborted,
    /// The response body reached [`MAX_SNAPSHOT_BYTES`].
    SizeLimit,
    /// The body stream failed to read (detail stays in `console`).
    BodyRead,
    /// The snapshot JSON could not be parsed (detail already redacted).
    Parse(String),
    /// The HTTP request could not be assembled (detail already redacted).
    Build(String),
    /// The transport failed before a response arrived (detail in `console`).
    Connect,
    /// The configured endpoint is not a loopback `http://` URL.
    InvalidEndpoint,
    /// The agent speaks a wire version this build does not understand.
    SchemaVersion { agent: u32, web: u32 },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth => f.write_str(RECONNECT_GUIDANCE),
            Self::Forbidden => f.write_str(FORBIDDEN_GUIDANCE),
            // 503 has a known, user-actionable cause (too many simultaneous
            // connections). Other 5xx and 429 are transient too; plain 4xx keeps
            // the terse form because retrying cannot fix a bad request.
            Self::Http(503) => f.write_str(SERVICE_UNAVAILABLE_GUIDANCE),
            Self::Http(429) => write!(
                f,
                "エージェントが HTTP 429 を返しました。{TRANSIENT_RETRY_GUIDANCE}"
            ),
            Self::Http(code) if (500..=599).contains(code) => {
                write!(f, "エージェントが HTTP {code} を返しました。{TRANSIENT_RETRY_GUIDANCE}")
            }
            Self::Http(code) => write!(f, "エージェントが HTTP {code} を返しました"),
            Self::Timeout => f.write_str(&fetch_timeout_error()),
            Self::Busy => f.write_str(FETCH_BUSY_ERROR),
            Self::BodyAborted => f.write_str(BODY_ABORTED_ERROR),
            Self::SizeLimit => f.write_str(SIZE_LIMIT_ERROR),
            Self::BodyRead => f.write_str(BODY_READ_ERROR),
            Self::Parse(detail) => {
                write!(f, "スナップショット JSON の解析に失敗しました: {detail}")
            }
            Self::Build(detail) => {
                write!(f, "リクエストの組み立てに失敗しました: {detail}")
            }
            Self::Connect => f.write_str(CONNECT_ERROR),
            Self::InvalidEndpoint => f.write_str(INVALID_ENDPOINT_ERROR),
            // Internal field names and the numeric versions stay in `console`
            // (see `check_schema_version`); this text is version-neutral.
            Self::SchemaVersion { .. } => f.write_str(
                "エージェントとアプリのバージョンが一致しません。両方を同じバージョンに更新してください",
            ),
        }
    }
}

/// Maps a non-success HTTP status to its typed error.
///
/// `401` and `403` are deliberately split: only the former means "paste a new
/// token", while the latter explains that cross-host access is unsupported
/// because the agent's allowed origins are loopback-only (C2-WEB).
fn status_error(code: u16) -> FetchError {
    match code {
        401 => FetchError::Auth,
        403 => FetchError::Forbidden,
        other => FetchError::Http(other),
    }
}

/// Whether a fetch error must be surfaced as the token re-entry flow rather
/// than a generic connectivity error. Kept as a pure predicate on the variant
/// so the DOM proof never depends on a display string (R3-S04/S05).
pub(crate) fn requires_reconnect(error: &FetchError) -> bool {
    matches!(error, FetchError::Auth)
}

/// Endpoint and token parsed from the page URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    /// Base URL such as `http://127.0.0.1:8787` (no trailing slash).
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

/// Wire envelope returned by `/api/v1/snapshot` and by SSE `snapshot` events.
#[derive(Debug, Deserialize)]
pub(crate) struct SnapshotEnvelope {
    schema_version: u32,
    total_process_count: usize,
    snapshot: SystemSnapshot,
}

impl SnapshotEnvelope {
    /// Converts a validated wire frame into the public response the shell
    /// already uses for its polling path, so both sources share one ingest
    /// function (and therefore the same redaction / stale / reconnect logic).
    ///
    /// This is the **single** sanitise-and-redact gate for every wire source:
    /// the polling reader and the SSE callback both funnel through it. Doing the
    /// work here — instead of only in the reader — is what stops an SSE frame
    /// from bypassing identifier masking or Cf/control-character stripping. The
    /// helper re-derives identifiers from the raw snapshot on entry and is
    /// idempotent, so no separate `OwnedIdentifiers` argument is required.
    pub(crate) fn into_response(mut self) -> SnapshotResponse {
        crate::redaction::sanitize_and_redact_wire_snapshot(&mut self.snapshot);
        SnapshotResponse {
            snapshot: self.snapshot,
            total_process_count: self.total_process_count,
        }
    }
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
    /// Bumped only on a real change. An in-flight body read (or SSE stream)
    /// captures this value at start and aborts early if it changes, so a stale
    /// response stops consuming wasm heap as soon as new settings arrive.
    /// The `Arc` lets a long-lived stream own an abort handle without borrowing
    /// the client (GPUI tasks require `'static` futures).
    generation: Arc<AtomicU64>,
}

impl SharedConfig {
    fn new(config: AgentConfig) -> Self {
        Self {
            config: RwLock::new(config),
            generation: Arc::new(AtomicU64::new(0)),
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
    /// Errors are typed [`FetchError`] values whose display text is redacted;
    /// they never trigger a silent fallback to demo data. The single-flight
    /// permit is held until this method resolves (success, error or deadline
    /// cancellation), so no second request can overlap this one.
    pub async fn fetch_snapshot(&self) -> Result<SnapshotResponse, FetchError> {
        // Losing callers never reach `http.send`: a parallel fetch cannot
        // start, which is the wasm heap bound R4-A02 asks for.
        let _permit = match self.gate.try_acquire() {
            Some(permit) => permit,
            None => return Err(FetchError::Busy),
        };
        let deadline = self.executor.timer(FETCH_TIMEOUT);
        with_deadline(self.fetch_snapshot_inner(), deadline).await
    }

    /// The actual request/parse pipeline, kept separate so the deadline can
    /// cover the body read as well as the response headers.
    async fn fetch_snapshot_inner(&self) -> Result<SnapshotResponse, FetchError> {
        // Snapshot once per request, after the shell may have re-read the URL.
        // The guard is dropped here, never held across the awaits below.
        let config = self.config.snapshot();
        // Remember the settings generation so the body read can abort early if
        // a reload supersedes this request while it is streaming (R4-A02).
        let generation = self.config.generation();
        // Never attach the bearer token before checking the destination.
        if !is_loopback_http_endpoint(&config.endpoint) {
            return Err(FetchError::InvalidEndpoint);
        }
        let url = snapshot_url(&config.endpoint);
        let mut builder = Request::builder().method("GET").uri(&url);
        if let Some(token) = &config.token {
            // クエリではなくヘッダで渡す（URL・Referer・アクセスログに残さない）。
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = builder.body(AsyncBody::empty()).map_err(|error| {
            FetchError::Build(redact_token(&format!("{error:#}"), config.token.as_deref()))
        })?;
        let response = self.http.send(request).await.map_err(|error| {
            // 画面には 1 行の要約だけを出し、詳細（スタックを含む）は
            // トークンを伏せたうえで console に残す。
            log::warn!(
                "エージェントへの fetch に失敗: {}",
                redact_token(&format!("{error:#}"), config.token.as_deref())
            );
            FetchError::Connect
        })?;

        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            // `401` invalidates the token and is the only reconnect trigger;
            // `403` is a rejected Origin and gets its own guidance, so the DOM
            // proof's `reconnect=` flag stays false for it (C2-WEB).
            // No body is read at all, so the error path stays cheap.
            return Err(status_error(code));
        }

        // Earliest possible abort: a declared oversized body is rejected from
        // the header alone, before a single byte is pulled into the wasm heap.
        let declared_length = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok());
        if content_length_exceeds_cap(declared_length) {
            return Err(FetchError::SizeLimit);
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
        .await?;

        let envelope: SnapshotEnvelope = serde_json::from_slice(&buffer).map_err(|error| {
            FetchError::Parse(redact_token(&format!("{error:#}"), config.token.as_deref()))
        })?;
        check_schema_version(envelope.schema_version)?;
        // `into_response` is the one sanitise-and-redact gate shared with SSE.
        // Never trust display strings from the wire: a malicious or corrupted
        // agent could inject ANSI/control characters into the canvas (R2X-D1),
        // and its `--redact` flag defaults to false, so host / user / home /
        // tailnet identifiers must be masked here too. Do NOT duplicate the
        // redaction call in this reader — that would split the boundary again.
        Ok(envelope.into_response())
    }

    /// Subscribes to the agent's long-lived `GET /api/v1/stream` endpoint.
    ///
    /// Setup errors (invalid endpoint / malformed URI) are returned eagerly;
    /// transport, status, stream and frame errors are reported by the returned
    /// future. The future owns its request and an `Arc` to the settings
    /// generation, so it is `'static` and can be spawned by GPUI: dropping the
    /// task **or** changing the page URL (which bumps that generation) stops the
    /// body read and cancels the browser-side `ReadableStream`.
    ///
    /// `EventSource` is deliberately not used: it cannot send the
    /// `Authorization: Bearer` header. `FetchHttpClient` exposes the response
    /// as a streaming `AsyncBody`, and the browser adds `Host` itself.
    pub fn subscribe_snapshot_stream<F>(
        &self,
        on_snapshot: F,
    ) -> Result<impl Future<Output = Result<(), FetchError>>, FetchError>
    where
        F: FnMut(SnapshotEnvelope),
    {
        let config = self.config.snapshot();
        let params = prepare_stream(
            &config,
            Arc::clone(&self.http),
            Arc::clone(&self.config.generation),
        )?;
        // Only the *setup* is deadline-bounded: a long-lived stream has no
        // overall timeout. Without this, a server that accepts the connection
        // but never sends headers would keep the shell in `sse=active` while
        // polling is suspended, leaving the user without fresh snapshots.
        let setup_deadline = self.executor.timer(FETCH_TIMEOUT);
        Ok(drive_snapshot_stream(params, on_snapshot, setup_deadline))
    }
}

/// Races a fallible operation against a deadline future.
///
/// Both futures are injected by the caller: the operation is the real fetch,
/// the deadline is `BackgroundExecutor::timer` on wasm (or any future in unit
/// tests). When the deadline wins, the still-pending operation is dropped —
/// cancelling the request — and a redaction-free [`FetchError::Timeout`] is
/// returned.
async fn with_deadline<T, F, Deadline>(future: F, deadline: Deadline) -> Result<T, FetchError>
where
    F: Future<Output = Result<T, FetchError>>,
    Deadline: Future<Output = ()>,
{
    futures::pin_mut!(future, deadline);
    match futures::future::select(future, deadline).await {
        Either::Left((result, _deadline)) => result,
        Either::Right(((), _cancelled_fetch)) => Err(FetchError::Timeout),
    }
}

/// Whether an accumulated body length has reached the hard cap.
///
/// Shared by the declared-length precheck, the abortable polling reader and the
/// SSE accumulator so every wire path treats exactly `MAX_SNAPSHOT_BYTES` as
/// over the limit (inclusive `>=` boundary, fixed by the shared test below).
fn body_exceeds_cap(used: u64, limit: u64) -> bool {
    used >= limit
}

/// Whether a declared `Content-Length` already proves the body is too large.
///
/// Used before reading anything, so an oversized response never enters the
/// wasm heap. Missing, malformed or small values are left to the streaming
/// reader, which enforces the same cap incrementally.
fn content_length_exceeds_cap(declared: Option<&str>) -> bool {
    declared
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|len| body_exceeds_cap(len, MAX_SNAPSHOT_BYTES))
}

/// Reads `body` into `out`, stopping as soon as `aborted()` turns true or the
/// `limit` would be exceeded.
///
/// `read_to_end` would keep pulling until EOF (or the whole 8 MiB cap) even
/// after the request is no longer wanted, pinning heap on wasm. Chunking makes
/// the read cancellable between chunks: a superseded fetch releases its buffer
/// and drops the underlying stream after at most one extra chunk of work.
/// Returns a typed [`FetchError`]; no token or agent text is ever embedded in
/// it (the display strings are fixed constants).
async fn read_body_abortable<R, F>(
    mut body: R,
    out: &mut Vec<u8>,
    limit: u64,
    token: Option<&str>,
    aborted: F,
) -> Result<(), FetchError>
where
    R: AsyncRead + Unpin,
    F: Fn() -> bool,
{
    let mut scratch = [0u8; BODY_READ_CHUNK];
    loop {
        if aborted() {
            return Err(FetchError::BodyAborted);
        }
        let used = out.len() as u64;
        if body_exceeds_cap(used, limit) {
            return Err(FetchError::SizeLimit);
        }
        let want = (limit - used).min(BODY_READ_CHUNK as u64) as usize;
        let read = body.read(&mut scratch[..want]).await.map_err(|error| {
            // Same detail-to-console / summary-to-UI split as the old reader:
            // the token is masked before anything is logged.
            log::warn!(
                "エージェント応答の読み取りに失敗: {}",
                redact_token(&format!("{error:#}"), token)
            );
            FetchError::BodyRead
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
        "エージェントからの取得がタイムアウトしました（{}秒）。{TRANSIENT_RETRY_GUIDANCE}",
        FETCH_TIMEOUT.as_secs()
    )
}

/// Accepts wire versions this client understands, including older agents.
///
/// A newer agent may add fields that serde defaults can ignore; only a
/// *future* version (which may change field meaning) is rejected.
fn check_schema_version(version: u32) -> Result<(), FetchError> {
    if version <= WIRE_SCHEMA_VERSION {
        Ok(())
    } else {
        // The numbers are diagnostics, not user guidance: keep them in `console`
        // so the status bar and the `aria-live` mirror show only the actionable,
        // version-neutral message from `FetchError::SchemaVersion` (R2-AUTHMSG).
        log::warn!(
            "エージェントとアプリのバージョン不一致: agent={version}, web={}",
            WIRE_SCHEMA_VERSION
        );
        Err(FetchError::SchemaVersion {
            agent: version,
            web: WIRE_SCHEMA_VERSION,
        })
    }
}

/// Builds the request URL. The token is **not** part of the URL.
fn snapshot_url(endpoint: &str) -> String {
    format!("{endpoint}/api/v1/snapshot?limit={SNAPSHOT_LIMIT}")
}

/// Path of the agent's long-lived server-sent-events endpoint (R1-I14).
pub const STREAM_PATH: &str = "/api/v1/stream";

/// Fixed, redaction-safe first-run instructions shown by the browser shell
/// (R1-I16). No token, hostname, endpoint or agent-supplied text appears here;
/// the strings also avoid `;` and newlines so they could be mirrored into the
/// `key=value;` DOM proof without changing its grammar.
const ONBOARDING_STEPS: &[&str] = &[
    "1. ローカルで tm-agent を起動します（トークンは --token-file 推奨）。",
    "2. 表示された 127.0.0.1 の URL に、トークンファイル（既定は XDG state 配下）の内容を #token= として付けてください。",
    "3. このページの URL に ?agent=<エンドポイント> と #token=<トークン> を付けて再読込します。",
    "4. 接続後は自動更新されます。行を右クリックすると行メニューが開きます。",
];

/// Returns the fixed onboarding checklist for a first-time browser user.
pub fn onboarding_steps() -> &'static [&'static str] {
    ONBOARDING_STEPS
}

/// Builds the SSE URL for an already-normalised loopback endpoint.
///
/// The bearer token is still sent in the `Authorization` header, never in the
/// URL or the query string; the agent rejects query tokens as well.
pub fn stream_url(endpoint: &str) -> String {
    format!("{endpoint}{STREAM_PATH}")
}

/// Parses one SSE `snapshot` frame from the agent.
///
/// Only compact JSON produced by `serde_json` is accepted. Error strings are
/// fixed (no agent text is echoed), so a malformed or hostile stream cannot
/// inject a token or control characters into the UI or the `aria-live` mirror.
/// A frame carries at most one `event:` field and exactly one `data:` field:
/// SSE allows a single event type per frame, and this probe emits one compact
/// JSON line, so a second occurrence of either field is a parse error.
pub(crate) fn parse_sse_event_frame(frame: &str) -> Result<SnapshotEnvelope, String> {
    const PARSE_ERROR: &str = "SSE データの解析に失敗しました";

    let mut event = None;
    let mut data = None;
    for line in frame.lines() {
        if let Some(name) = line.strip_prefix("event:") {
            if event.is_some() {
                return Err(PARSE_ERROR.to_string());
            }
            event = Some(name.trim_start());
        } else if let Some(value) = line.strip_prefix("data:") {
            if data.is_some() {
                // Decision: keep the single-line data contract. SSE would allow
                // joining several `data:` values with '\n', but the agent emits
                // one compact JSON document per frame; accepting more would hide
                // a second payload and weaken the no-echo/no-injection boundary.
                return Err(PARSE_ERROR.to_string());
            }
            data = Some(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    match event {
        Some("snapshot") => {}
        Some("error") => return Err("エージェントがエラーイベントを返しました".to_string()),
        _ => return Err("不明な SSE イベントを受信しました".to_string()),
    }
    let data = data
        .filter(|value| !value.is_empty())
        .ok_or_else(|| PARSE_ERROR.to_string())?;
    let envelope: SnapshotEnvelope =
        serde_json::from_str(data).map_err(|_| PARSE_ERROR.to_string())?;
    check_schema_version(envelope.schema_version).map_err(|error| error.to_string())?;
    Ok(envelope)
}

/// Owned inputs for one SSE subscription.
///
/// Cloned out of [`AgentClient`] so the driving future is `'static` and never
/// borrows the client (GPUI foreground tasks require `'static` futures).
struct StreamParams {
    http: Arc<dyn HttpClient>,
    request: Request<AsyncBody>,
    token: Option<String>,
    generation: Arc<AtomicU64>,
}

/// Prepares the `GET /api/v1/stream` request.
///
/// Kept synchronous so an invalid endpoint or a malformed URI is rejected
/// before a task is spawned. The token is placed **only** in the
/// `Authorization` header — never in the URL, query string or an error string.
/// `Host` is intentionally not set: `web_sys::Headers` forbids it and the
/// browser always supplies it from the request URL (the agent requires a
/// loopback Host).
fn prepare_stream(
    config: &AgentConfig,
    http: Arc<dyn HttpClient>,
    generation: Arc<AtomicU64>,
) -> Result<StreamParams, FetchError> {
    if !is_loopback_http_endpoint(&config.endpoint) {
        return Err(FetchError::InvalidEndpoint);
    }
    let url = stream_url(&config.endpoint);
    let mut builder = Request::builder().method("GET").uri(&url);
    if let Some(token) = &config.token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = builder.body(AsyncBody::empty()).map_err(|error| {
        FetchError::Build(redact_token(&format!("{error:#}"), config.token.as_deref()))
    })?;
    Ok(StreamParams {
        http,
        request,
        token: config.token.clone(),
        generation,
    })
}

/// Length of the longest SSE frame delimiter (`\r\n\r\n`).
const SSE_DELIMITER_MAX_LEN: usize = 4;

/// Finds the first SSE frame delimiter at or after `from`.
///
/// Returns `(delimiter_start, delimiter_len)`. Both LF (`\n\n`, which the agent
/// emits) and CRLF (`\r\n\r\n`) blank lines are accepted; the earliest start
/// wins, matching the old "scan the whole buffer twice and pick the smaller
/// position" behaviour. A frame split across several reads simply stays in the
/// buffer until its delimiter arrives.
///
/// `from` is a *resume offset*: positions before it were already tested as
/// delimiter starts during an earlier read. Scanning byte-by-byte from there —
/// instead of re-running `windows(2)`/`windows(4)` over the whole growing buffer
/// on every chunk — keeps a long, chunked frame at O(total bytes) rather than
/// O(n^2) comparisons. The caller resumes `SSE_DELIMITER_MAX_LEN - 1` bytes back
/// so a delimiter straddling a read boundary is still found.
fn find_sse_delimiter(buffer: &[u8], from: usize) -> Option<(usize, usize)> {
    for at in from.min(buffer.len())..buffer.len() {
        if buffer[at..].starts_with(b"\n\n") {
            return Some((at, 2));
        }
        if buffer[at..].starts_with(b"\r\n\r\n") {
            return Some((at, SSE_DELIMITER_MAX_LEN));
        }
    }
    None
}

/// Drives one long-lived SSE subscription, calling `on_snapshot` per frame.
///
/// The future ends on a settings-generation change ([`FetchError::BodyAborted`]),
/// a setup timeout ([`FetchError::Timeout`]), an HTTP/read error, a malformed
/// frame, EOF or a size-cap breach. The shell
/// uses that termination to switch back to the ordinary snapshot poll, so a
/// broken or closing stream never leaves the user without monitoring.
async fn drive_snapshot_stream<F, Deadline>(
    params: StreamParams,
    mut on_snapshot: F,
    setup_deadline: Deadline,
) -> Result<(), FetchError>
where
    F: FnMut(SnapshotEnvelope),
    Deadline: Future<Output = ()>,
{
    let StreamParams {
        http,
        request,
        token,
        generation,
    } = params;
    let started_at = generation.load(Ordering::SeqCst);
    let aborted = || generation.load(Ordering::SeqCst) != started_at;
    if aborted() {
        return Err(FetchError::BodyAborted);
    }

    let response = with_deadline(
        async {
            http.send(request).await.map_err(|error| {
                log::warn!(
                    "エージェントのストリーム接続に失敗: {}",
                    redact_token(&format!("{error:#}"), token.as_deref())
                );
                FetchError::Connect
            })
        },
        setup_deadline,
    )
    .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(status.as_u16()));
    }
    // A long-lived stream normally has no Content-Length; if one is declared
    // and already over the cap, reject it before pulling any bytes.
    let declared = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok());
    if content_length_exceeds_cap(declared) {
        return Err(FetchError::SizeLimit);
    }

    let mut body = response.into_body();
    let mut pending = Vec::new();
    // Resume offset into `pending`: positions before it were already tested as
    // delimiter starts. Kept across reads so a frame spanning many chunks is
    // scanned once instead of re-scanning the whole buffer every time.
    let mut scan_from = 0usize;
    let mut scratch = [0u8; BODY_READ_CHUNK];
    loop {
        if aborted() {
            return Err(FetchError::BodyAborted);
        }
        let read = body.read(&mut scratch).await.map_err(|error| {
            log::warn!(
                "エージェントのストリーム読み取りに失敗: {}",
                redact_token(&format!("{error:#}"), token.as_deref())
            );
            FetchError::BodyRead
        })?;
        if read == 0 {
            // SSE dispatches a frame only on its closing blank line; a partial
            // tail is discarded instead of being parsed as a snapshot.
            return Ok(());
        }
        pending.extend_from_slice(&scratch[..read]);
        if body_exceeds_cap(pending.len() as u64, MAX_SNAPSHOT_BYTES) {
            return Err(FetchError::SizeLimit);
        }
        // `pending` always begins with the first unconsumed byte (we drain all
        // finished frames at the end of this read), so `frame_start` is 0 for
        // the first frame and the end of the previous delimiter afterwards.
        let mut frame_start = 0usize;
        let mut cursor = scan_from;
        while let Some((at, delimiter_len)) = find_sse_delimiter(&pending, cursor) {
            if aborted() {
                return Err(FetchError::BodyAborted);
            }
            let frame = &pending[frame_start..at];
            let text = std::str::from_utf8(frame)
                .map_err(|_| FetchError::Parse("SSE データの解析に失敗しました".to_string()))?;
            let envelope = parse_sse_event_frame(text)
                .map_err(|detail| FetchError::Parse(redact_token(&detail, token.as_deref())))?;
            on_snapshot(envelope);
            frame_start = at + delimiter_len;
            cursor = frame_start;
        }
        // One drain per read instead of one per frame: many small frames in a
        // single chunk no longer cause repeated O(remaining) memmoves.
        if frame_start > 0 {
            pending.drain(..frame_start);
        }
        // A delimiter may straddle the next read; recheck the last
        // `SSE_DELIMITER_MAX_LEN - 1` bytes so no boundary is missed.
        scan_from = pending.len().saturating_sub(SSE_DELIMITER_MAX_LEN - 1);
    }
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
        assert_eq!(
            error,
            FetchError::SchemaVersion {
                agent: WIRE_SCHEMA_VERSION + 1,
                web: WIRE_SCHEMA_VERSION,
            }
        );
        let text = error.to_string();
        assert!(text.contains("バージョンが一致しません"));
        assert!(text.contains("同じバージョンに更新してください"));
        // Internal variable names and numbers are console-only, never UI text.
        assert!(!text.contains("agent="));
        assert!(!text.contains(&(WIRE_SCHEMA_VERSION + 1).to_string()));
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
            "http://[::1",
            "http://[::1]evil",
            "http://[::1]:",
            "http://[::1]:80:80",
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
    fn http_status_errors_are_classified_by_variant() {
        assert_eq!(status_error(401), FetchError::Auth);
        assert_eq!(status_error(403), FetchError::Forbidden);
        for other in [200, 204, 400, 404, 429, 500, 503] {
            assert_eq!(
                status_error(other),
                FetchError::Http(other),
                "{other} must stay a generic error"
            );
        }
        // Only 401 drives the token re-entry flow; 403 is an origin problem.
        assert!(requires_reconnect(&status_error(401)));
        assert!(!requires_reconnect(&status_error(403)));
        assert!(!requires_reconnect(&status_error(500)));
    }

    #[test]
    fn transient_errors_explain_retry_and_next_action() {
        // 503 names the real cause (the agent's concurrent-connection cap) and
        // promises the automatic retry the sampler actually performs.
        let busy = status_error(503).to_string();
        assert!(busy.contains("同時接続数が上限に達しました"), "{busy}");
        assert!(busy.contains("自動で再試行します"), "{busy}");
        assert!(busy.contains("再起動してください"), "{busy}");

        // Other transient statuses / the deadline behave the same way.
        for transient in [status_error(429), status_error(500), status_error(502)] {
            let text = transient.to_string();
            assert!(text.contains("自動で再試行します"), "{text}");
            assert!(text.contains("再起動してください"), "{text}");
        }
        let timeout = FetchError::Timeout.to_string();
        assert!(timeout.contains("自動で再試行します"), "{timeout}");
        assert!(timeout.contains("タイムアウト"), "{timeout}");

        // A plain client error stays terse: retrying cannot fix a bad request.
        let bad = status_error(400).to_string();
        assert!(!bad.contains("自動で再試行します"), "{bad}");

        // None of the transient guidance triggers the token re-entry flow.
        for error in [
            status_error(429),
            status_error(503),
            status_error(500),
            FetchError::Timeout,
        ] {
            assert!(!requires_reconnect(&error), "{error}");
        }
    }

    #[test]
    fn forbidden_guidance_is_distinct_and_never_requests_reconnect() {
        assert!(
            FORBIDDEN_GUIDANCE.contains("別ホスト／別オリジンからのアクセスはサポートしていません")
        );
        assert!(FORBIDDEN_GUIDANCE.contains("同一ホストで起動してください"));
        assert_ne!(FORBIDDEN_GUIDANCE, RECONNECT_GUIDANCE);
        // No `#token=` prompt: a new token cannot fix a rejected Origin.
        assert!(!FORBIDDEN_GUIDANCE.contains('#'));
        assert!(!FORBIDDEN_GUIDANCE.contains(';'));
        assert!(!FORBIDDEN_GUIDANCE.contains('\n'));
        assert_eq!(
            redact_token(FORBIDDEN_GUIDANCE, Some("super-secret-agent-token")),
            FORBIDDEN_GUIDANCE
        );
        let error = status_error(403);
        assert_eq!(error, FetchError::Forbidden);
        assert!(!requires_reconnect(&error), "403 must not set reconnect=1");
        assert_eq!(error.to_string(), FORBIDDEN_GUIDANCE);
    }

    #[test]
    fn reconnect_guidance_is_actionable_and_redaction_safe() {
        // The guidance tells the user *what* to re-enter and *where*: the token
        // file is reused on restart, so no "new URL" is promised (R2-AUTHMSG).
        assert!(RECONNECT_GUIDANCE.contains("#token="));
        assert!(RECONNECT_GUIDANCE.contains("トークンファイル"));
        assert!(RECONNECT_GUIDANCE.contains("XDG state"));
        assert!(RECONNECT_GUIDANCE.contains("--token-file"));
        assert!(RECONNECT_GUIDANCE.contains("貼り付けて"));
        assert!(RECONNECT_GUIDANCE.contains("再起動ではトークンは変わりません"));
        assert!(!RECONNECT_GUIDANCE.contains("新しい URL"));
        // Fixed text: no `;` / newline that could corrupt the DOM proof, and
        // redacting it with any token leaves it byte-for-byte identical.
        assert!(!RECONNECT_GUIDANCE.contains(';'));
        assert!(!RECONNECT_GUIDANCE.contains('\n'));
        assert_eq!(
            redact_token(RECONNECT_GUIDANCE, Some("super-secret-agent-token")),
            RECONNECT_GUIDANCE
        );
        let error = FetchError::Auth;
        assert_eq!(error.to_string(), RECONNECT_GUIDANCE);
        assert!(requires_reconnect(&error));
        assert!(!requires_reconnect(&FetchError::Connect));
        assert!(!requires_reconnect(&FetchError::Http(500)));
    }

    #[test]
    fn every_display_string_is_dom_safe_and_token_free() {
        // The DOM proof mirrors status text only through a boolean, but the
        // status bar/aria-live path still requires `;`-free, newline-free,
        // redaction-safe text for every variant (C2-WEB).
        for error in [
            FetchError::Auth,
            FetchError::Forbidden,
            FetchError::Http(500),
            FetchError::Timeout,
            FetchError::Busy,
            FetchError::BodyAborted,
            FetchError::SizeLimit,
            FetchError::BodyRead,
            FetchError::Parse("invalid json".to_string()),
            FetchError::Build("bad uri".to_string()),
            FetchError::Connect,
            FetchError::InvalidEndpoint,
            FetchError::SchemaVersion {
                agent: 99,
                web: WIRE_SCHEMA_VERSION,
            },
        ] {
            let text = error.to_string();
            assert!(!text.contains(';'), "{text}");
            assert!(!text.contains('\n'), "{text}");
            assert_eq!(
                redact_token(&text, Some("super-secret-agent-token")),
                text,
                "{text}"
            );
        }
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
        // A pasted token (read from the reused token file) replaces the cached
        // config; a plain restart would keep the same token (R2-AUTHMSG).
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
            async { Ok::<u32, FetchError>(42) },
            futures::future::pending::<()>(),
        ));
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn deadline_passes_through_the_inner_error_unchanged() {
        let result = futures::executor::block_on(with_deadline(
            async { Err::<u32, FetchError>(FetchError::Connect) },
            futures::future::pending::<()>(),
        ));
        assert_eq!(result, Err(FetchError::Connect));
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
                futures::future::pending::<Result<u32, FetchError>>().await
            },
            async {},
        ));

        let error = result.expect_err("a stalled fetch must time out");
        assert_eq!(error, FetchError::Timeout);
        let text = error.to_string();
        assert_eq!(text, fetch_timeout_error());
        assert!(text.contains("タイムアウト"));
        // No token/endpoint interpolation — the message itself is redaction-safe.
        assert_eq!(redact_token(&text, Some("super-secret-token")), text);
        assert!(!text.contains("secret"));
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
                futures::future::pending::<Result<u32, FetchError>>().await
            },
            async {},
        ));
        assert_eq!(result, Err(FetchError::Timeout));
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
    fn every_wire_path_uses_the_same_inclusive_size_cap() {
        // Single shared predicate: polling precheck, abortable reader and SSE
        // accumulator all reject exactly at MAX_SNAPSHOT_BYTES (u1-sec fix).
        assert!(!body_exceeds_cap(
            MAX_SNAPSHOT_BYTES - 1,
            MAX_SNAPSHOT_BYTES
        ));
        assert!(body_exceeds_cap(MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_BYTES));
        assert!(body_exceeds_cap(MAX_SNAPSHOT_BYTES + 1, MAX_SNAPSHOT_BYTES));
        assert!(content_length_exceeds_cap(Some(
            &MAX_SNAPSHOT_BYTES.to_string()
        )));
        assert!(!content_length_exceeds_cap(Some(
            &(MAX_SNAPSHOT_BYTES - 1).to_string()
        )));
        // The reader's existing boundary test uses the same helper, so an
        // accidental `>` regression in either path changes this contract.
        assert!(body_exceeds_cap(10, 10));
        assert!(!body_exceeds_cap(9, 10));
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
        assert_eq!(error, FetchError::BodyAborted);
        assert_eq!(
            out.len(),
            BODY_READ_CHUNK,
            "at most one chunk is consumed after a config change"
        );
        assert!(out.len() < BODY_READ_CHUNK * 3);
        assert!(checks.load(Ordering::SeqCst) >= 2);
        // Static error: safe for the DOM/aria-live mirror and token-free.
        let text = error.to_string();
        assert_eq!(text, BODY_ABORTED_ERROR);
        assert_eq!(redact_token(&text, Some("secret-token")), text);
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
        assert_eq!(error, FetchError::SizeLimit);
        assert_eq!(error.to_string(), SIZE_LIMIT_ERROR);
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
        assert_eq!(error, FetchError::BodyRead);
        let text = error.to_string();
        assert_eq!(text, BODY_READ_ERROR);
        assert_eq!(redact_token(&text, Some("secret")), text);
        assert!(!text.contains("secret"));
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
    fn non_auth_errors_are_redaction_safe_and_do_not_request_reconnect() {
        for error in [
            FetchError::Busy,
            FetchError::BodyAborted,
            FetchError::SizeLimit,
            FetchError::BodyRead,
            FetchError::Timeout,
            FetchError::Connect,
            FetchError::Forbidden,
            FetchError::InvalidEndpoint,
            FetchError::Http(500),
        ] {
            let text = error.to_string();
            assert_eq!(redact_token(&text, Some("secret-agent-token")), text);
            assert!(!text.contains("secret"));
            // The DOM proof's reconnect flag is driven by requires_reconnect;
            // only `401` may flip it (R3-S04/S05 hook stays intact, C2-WEB).
            assert!(
                !requires_reconnect(&error),
                "{text} must not set reconnect=1"
            );
        }
    }

    // ------------------------------------------------ R1-I14 / R1-I16 support

    #[test]
    fn onboarding_steps_are_fixed_redaction_and_hook_safe() {
        let steps = onboarding_steps();
        assert_eq!(steps.len(), 4);
        for step in steps {
            // No token/hostname is interpolated, and no proof-grammar-breaker.
            assert!(!step.contains(';'), "{step}");
            assert!(!step.contains('\n'), "{step}");
            assert_eq!(
                redact_token(step, Some("super-secret-token")).as_str(),
                *step
            );
        }
        assert!(steps.iter().any(|step| step.contains("?agent=")));
        assert!(steps.iter().any(|step| step.contains("#token=")));
        assert!(steps.iter().any(|step| step.contains("右クリック")));
    }

    #[test]
    fn stream_url_has_no_token_and_uses_the_sse_path() {
        let config = AgentConfig::new(
            "  http://127.0.0.1:18787/ ",
            Some("secret-agent-token".to_string()),
        );
        let url = stream_url(&config.endpoint);
        assert_eq!(url, "http://127.0.0.1:18787/api/v1/stream");
        assert!(!url.contains("secret"));
        assert!(!url.contains("token="));
    }

    #[test]
    fn sse_snapshot_frame_is_parsed_and_schema_checked() {
        let snapshot = tm_core::demo::demo_snapshot(0);
        let json = serde_json::json!({
            "schema_version": WIRE_SCHEMA_VERSION,
            "total_process_count": 7,
            "snapshot": snapshot,
        })
        .to_string();
        let frame = format!("event: snapshot\ndata: {json}\n\n");
        let parsed = parse_sse_event_frame(&frame).expect("parse frame");
        assert_eq!(parsed.schema_version, WIRE_SCHEMA_VERSION);
        assert_eq!(parsed.total_process_count, 7);

        let future = serde_json::json!({
            "schema_version": WIRE_SCHEMA_VERSION + 1,
            "total_process_count": 7,
            "snapshot": tm_core::demo::demo_snapshot(0),
        })
        .to_string();
        let frame = format!("event: snapshot\ndata: {future}\n\n");
        assert!(
            parse_sse_event_frame(&frame)
                .expect_err("future schema must be rejected")
                .contains("バージョンが一致しません")
        );
    }

    #[test]
    fn sse_error_frames_never_echo_agent_text() {
        let error = parse_sse_event_frame("event: error\ndata: {\"error\":\"SECRET-PATH\"}\n\n")
            .expect_err("error event must not parse as a snapshot");
        assert_eq!(error, "エージェントがエラーイベントを返しました");
        assert!(!error.contains("SECRET"));

        let malformed = parse_sse_event_frame("event: snapshot\ndata: {oops\n\n")
            .expect_err("malformed data must be rejected");
        assert_eq!(malformed, "SSE データの解析に失敗しました");
        assert!(!malformed.contains("oops"));
    }

    #[test]
    fn sse_duplicate_event_lines_are_rejected() {
        let json = serde_json::json!({
            "schema_version": WIRE_SCHEMA_VERSION,
            "total_process_count": 1,
            "snapshot": tm_core::demo::demo_snapshot(0),
        })
        .to_string();
        for frame in [
            format!("event: snapshot\nevent: snapshot\ndata: {json}\n\n"),
            format!("event: snapshot\nevent: error\ndata: {json}\n\n"),
            format!("data: {json}\nevent: snapshot\nevent: snapshot\n\n"),
        ] {
            let error =
                parse_sse_event_frame(&frame).expect_err("duplicate event must be rejected");
            assert_eq!(error, "SSE データの解析に失敗しました");
            assert!(!error.contains("SECRET"));
        }
    }

    #[test]
    fn sse_duplicate_data_lines_are_rejected() {
        let json = serde_json::json!({
            "schema_version": WIRE_SCHEMA_VERSION,
            "total_process_count": 1,
            "snapshot": tm_core::demo::demo_snapshot(0),
        })
        .to_string();
        let frame = format!("event: snapshot\ndata: {json}\ndata: {{\"extra\":1}}\n\n");
        let error = parse_sse_event_frame(&frame).expect_err("duplicate data must be rejected");
        assert_eq!(error, "SSE データの解析に失敗しました");
        assert!(!error.contains('{'));
    }

    /// An unredacted wire snapshot: real identifiers, a ZWSP *inside* the host
    /// name (redaction must see it before sanitisation turns it into a space),
    /// plus ANSI/Cf injection in the process text.
    fn dirty_wire_snapshot() -> tm_core::SystemSnapshot {
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        snapshot.hostname = Some("my\u{200b}host".into());
        snapshot.processes = vec![tm_core::ProcessSnapshot {
            key: tm_core::ProcessKey::new(4242, 1_700_000_000),
            name: "alice-worker\u{200b}\u{1b}[2J".into(),
            command: "alice --home /home/alice https://my\u{200b}host.tail1234.ts.net:8443 --id=tail1234abcd \u{1b}[31mRED\u{1b}[0m\u{200b}".into(),
            user: Some("alice".into()),
            status: tm_core::ProcessStatus::Running,
            ..tm_core::ProcessSnapshot::default()
        }];
        snapshot.disks = vec![tm_core::DiskUsageSnapshot {
            mount_point: "/home/alice/work".into(),
            ..tm_core::DiskUsageSnapshot::default()
        }];
        snapshot
    }

    fn assert_wire_snapshot_is_clean(snapshot: &tm_core::SystemSnapshot) {
        assert_eq!(snapshot.hostname.as_deref(), Some("host"));
        assert_eq!(snapshot.disks[0].mount_point, "<home>/work");
        let process = &snapshot.processes[0];
        assert_eq!(process.user.as_deref(), Some("user"));
        assert_eq!(process.name, "user-worker");
        for secret in ["alice", "my-host", "/home/alice", "tail1234", "ts.net"] {
            assert!(
                !process.command.contains(secret),
                "command leaked {secret}: {}",
                process.command
            );
            assert!(
                !process.name.contains(secret),
                "name leaked {secret}: {}",
                process.name
            );
        }
        for value in [process.command.as_str(), process.name.as_str()] {
            assert!(!value.contains('\u{1b}'), "ANSI escape survived: {value:?}");
            assert!(
                !value.contains('\u{200b}'),
                "Cf character survived: {value:?}"
            );
            assert!(
                value.chars().all(|ch| !ch.is_control()),
                "control character survived: {value:?}"
            );
        }
        assert!(
            process.command.contains("RED"),
            "visible text should remain"
        );
    }

    #[test]
    fn sse_wire_envelope_is_redacted_by_the_shared_into_response_gate() {
        // The shell's SSE callback calls `into_response` on every frame. An
        // unredacted agent payload must be masked there, not just on polling.
        let envelope = SnapshotEnvelope {
            schema_version: WIRE_SCHEMA_VERSION,
            total_process_count: 9,
            snapshot: dirty_wire_snapshot(),
        };
        let response = envelope.into_response();
        assert_eq!(response.total_process_count, 9);
        assert_wire_snapshot_is_clean(&response.snapshot);
    }

    #[test]
    fn sse_stream_frame_is_clean_after_the_shared_gate() {
        // End-to-end: a raw SSE frame -> parse -> the same `into_response` gate
        // the shell uses -> no hostname/user/home/tailnet id or control char.
        let json = serde_json::json!({
            "schema_version": WIRE_SCHEMA_VERSION,
            "total_process_count": 3,
            "snapshot": dirty_wire_snapshot(),
        })
        .to_string();
        let body = format!("event: snapshot\ndata: {json}\n\n").into_bytes();
        let (params, _) = stream_params(
            Some("secret-agent-token"),
            body,
            13,
            200,
            Arc::new(AtomicU64::new(0)),
        );
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        futures::executor::block_on(drive_snapshot_stream(
            params,
            move |envelope| {
                // Mirrors shell.rs: the callback converts via the single gate.
                let response = envelope.into_response();
                sink.lock().unwrap().push(response);
            },
            never_timeout(),
        ))
        .expect("a well-formed stream drains to EOF");
        let responses = seen.lock().unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].total_process_count, 3);
        assert_wire_snapshot_is_clean(&responses[0].snapshot);
    }

    // ------------------------------------------------------ SSE stream support

    #[derive(Clone)]
    struct RecordedRequest {
        uri: String,
        authorization: Option<String>,
    }

    /// An `AsyncRead` that returns at most `chunk` bytes per poll, so the frame
    /// splitter is exercised with boundaries inside JSON *and* inside `\n\n`.
    struct ChunkedReader {
        bytes: Vec<u8>,
        position: usize,
        chunk: usize,
    }

    impl AsyncRead for ChunkedReader {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            let remaining = self.bytes.len() - self.position;
            if remaining == 0 {
                return std::task::Poll::Ready(Ok(0));
            }
            let take = remaining.min(buf.len()).min(self.chunk);
            buf[..take].copy_from_slice(&self.bytes[self.position..self.position + take]);
            self.position += take;
            std::task::Poll::Ready(Ok(take))
        }
    }

    struct FakeStreamHttpClient {
        requests: Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
        body: Vec<u8>,
        chunk: usize,
        status: u16,
    }

    impl HttpClient for FakeStreamHttpClient {
        fn user_agent(&self) -> Option<&header::HeaderValue> {
            None
        }

        fn proxy(&self) -> Option<&gpui::http_client::Url> {
            None
        }

        fn send(
            &self,
            request: Request<AsyncBody>,
        ) -> futures::future::BoxFuture<
            'static,
            anyhow::Result<gpui::http_client::http::Response<AsyncBody>>,
        > {
            self.requests.lock().unwrap().push(RecordedRequest {
                uri: request.uri().to_string(),
                authorization: request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
            });
            let body = self.body.clone();
            let chunk = self.chunk;
            let status = self.status;
            Box::pin(async move {
                let reader = ChunkedReader {
                    bytes: body,
                    position: 0,
                    chunk,
                };
                gpui::http_client::http::Response::builder()
                    .status(status)
                    .body(AsyncBody::from_reader(reader))
                    .map_err(|error| anyhow::anyhow!("{error}"))
            })
        }
    }

    fn stream_params(
        token: Option<&str>,
        body: Vec<u8>,
        chunk: usize,
        status: u16,
        generation: Arc<AtomicU64>,
    ) -> (StreamParams, Arc<std::sync::Mutex<Vec<RecordedRequest>>>) {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http = FakeStreamHttpClient {
            requests: Arc::clone(&requests),
            body,
            chunk,
            status,
        };
        let config = AgentConfig::new("http://127.0.0.1:18787", token.map(str::to_string));
        let params = prepare_stream(&config, Arc::new(http), generation).expect("prepare stream");
        (params, requests)
    }

    /// A transport whose headers never arrive. The setup deadline must fire so
    /// the shell never stays in `sse=active` with polling suspended.
    struct HangingHttpClient;

    impl HttpClient for HangingHttpClient {
        fn user_agent(&self) -> Option<&header::HeaderValue> {
            None
        }

        fn proxy(&self) -> Option<&gpui::http_client::Url> {
            None
        }

        fn send(
            &self,
            _request: Request<AsyncBody>,
        ) -> futures::future::BoxFuture<
            'static,
            anyhow::Result<gpui::http_client::http::Response<AsyncBody>>,
        > {
            Box::pin(futures::future::pending())
        }
    }

    /// Test deadline that never fires: the stream itself is long-lived.
    fn never_timeout() -> impl Future<Output = ()> {
        futures::future::pending::<()>()
    }

    fn sse_snapshot_frame(total: usize) -> Vec<u8> {
        let json = serde_json::json!({
            "schema_version": WIRE_SCHEMA_VERSION,
            "total_process_count": total,
            "snapshot": tm_core::demo::demo_snapshot(0),
        })
        .to_string();
        format!("event: snapshot\ndata: {json}\n\n").into_bytes()
    }

    #[test]
    fn stream_request_keeps_the_token_out_of_the_url() {
        let (params, _) = stream_params(
            Some("secret-agent-token"),
            Vec::new(),
            8,
            200,
            Arc::new(AtomicU64::new(0)),
        );
        let uri = params.request.uri().to_string();
        assert_eq!(uri, "http://127.0.0.1:18787/api/v1/stream");
        assert!(!uri.contains("secret"), "{uri}");
        assert_eq!(
            params
                .request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer secret-agent-token")
        );
        // The browser adds Host itself; web_sys rejects a manually set one.
        assert!(!params.request.headers().contains_key(header::HOST));
    }

    #[test]
    fn sse_frame_splitter_handles_crlf_and_waits_for_partial_frames() {
        let frame = b"event: snapshot\ndata: 1";
        let mut buffer = frame.to_vec();
        buffer.extend_from_slice(b"\r\n\r\nrest");
        let (at, len) = find_sse_delimiter(&buffer, 0).expect("CRLF blank line ends a frame");
        assert_eq!(&buffer[..at], frame);
        assert_eq!(&buffer[at + len..], b"rest");
        assert!(
            find_sse_delimiter(&frame[..5], 0).is_none(),
            "an unterminated frame must stay buffered"
        );
    }

    #[test]
    fn sse_delimiter_search_resumes_across_a_read_boundary() {
        let mut buffer = b"event: snapshot\ndata: 1\r\n\r".to_vec();
        assert!(
            find_sse_delimiter(&buffer, 0).is_none(),
            "no complete delimiter yet"
        );
        // The caller saves this resume offset before the next chunk arrives.
        let resume = buffer.len().saturating_sub(SSE_DELIMITER_MAX_LEN - 1);
        buffer.push(b'\n'); // completes the `\r\n\r\n` delimiter
        let (at, len) = find_sse_delimiter(&buffer, resume).expect("straddling CRLF is found");
        assert_eq!(&buffer[at..at + len], b"\r\n\r\n");
        assert_eq!(&buffer[..at], b"event: snapshot\ndata: 1");
        // A resume offset past the delimiter never reports it a second time.
        assert!(find_sse_delimiter(&buffer, at + len).is_none());
    }

    #[test]
    fn sse_frames_split_across_chunks_are_delivered_in_order() {
        let mut body = sse_snapshot_frame(11);
        body.extend_from_slice(&sse_snapshot_frame(22));
        let (params, requests) = stream_params(
            Some("secret-agent-token"),
            body,
            7,
            200,
            Arc::new(AtomicU64::new(0)),
        );
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        futures::executor::block_on(drive_snapshot_stream(
            params,
            move |envelope| {
                sink.lock().unwrap().push(envelope.total_process_count);
            },
            never_timeout(),
        ))
        .expect("a well-formed stream drains to EOF");
        assert_eq!(*seen.lock().unwrap(), vec![11, 22]);
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].uri, "http://127.0.0.1:18787/api/v1/stream");
    }

    #[test]
    fn generation_change_aborts_the_live_stream() {
        let generation = Arc::new(AtomicU64::new(0));
        let mut body = sse_snapshot_frame(11);
        body.extend_from_slice(&sse_snapshot_frame(22));
        let (params, _) = stream_params(None, body, 16, 200, Arc::clone(&generation));
        let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = Arc::clone(&seen);
        let bump = Arc::clone(&generation);
        let error = futures::executor::block_on(drive_snapshot_stream(
            params,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                // Simulates `set_config` being called from a refreshed page URL.
                bump.fetch_add(1, Ordering::SeqCst);
            },
            never_timeout(),
        ))
        .expect_err("a settings change must abort the stream");
        assert_eq!(error, FetchError::BodyAborted);
        assert_eq!(error.to_string(), BODY_ABORTED_ERROR);
        assert_eq!(
            seen.load(Ordering::SeqCst),
            1,
            "no stale frame is delivered"
        );
    }

    #[test]
    fn stalled_stream_setup_times_out_and_reports_a_typed_error() {
        let config = AgentConfig::new(
            "http://127.0.0.1:18787",
            Some("secret-agent-token".to_string()),
        );
        let params = prepare_stream(
            &config,
            Arc::new(HangingHttpClient),
            Arc::new(AtomicU64::new(0)),
        )
        .expect("prepare stream");
        // An already-fired deadline stands in for `BackgroundExecutor::timer`.
        let error = futures::executor::block_on(drive_snapshot_stream(params, |_| {}, async {}))
            .expect_err("a stalled setup must time out");
        assert_eq!(error, FetchError::Timeout);
        assert_eq!(error.to_string(), fetch_timeout_error());
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn stream_errors_are_typed_and_never_echo_the_token() {
        let token = "secret-agent-token";
        let (unauthorized, _) =
            stream_params(Some(token), Vec::new(), 8, 401, Arc::new(AtomicU64::new(0)));
        let error = futures::executor::block_on(drive_snapshot_stream(
            unauthorized,
            |_| {},
            never_timeout(),
        ))
        .expect_err("401 must be an auth error");
        assert_eq!(error, FetchError::Auth);
        assert!(requires_reconnect(&error));
        assert!(!error.to_string().contains("secret"));

        let (malformed, _) = stream_params(
            Some(token),
            b"event: snapshot\ndata: {SECRET-PATH}\n\n".to_vec(),
            8,
            200,
            Arc::new(AtomicU64::new(0)),
        );
        let error =
            futures::executor::block_on(drive_snapshot_stream(malformed, |_| {}, never_timeout()))
                .expect_err("a malformed frame must end the stream");
        match error {
            FetchError::Parse(detail) => {
                assert_eq!(detail, "SSE データの解析に失敗しました");
                assert!(!detail.contains("SECRET"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
