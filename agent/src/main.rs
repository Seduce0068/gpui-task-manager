//! Read-only loopback snapshot agent.
//!
//! The browser shell cannot read `/proc`, so the native side runs this small
//! HTTP server on loopback and the wasm UI fetches snapshots from it.
//!
//! # Security posture
//!
//! - binds to a loopback address only (validated before `bind`),
//! - every snapshot request must present a bearer token (header or query),
//! - the API is **read-only**: there is no endpoint that signals a process,
//! - one request per connection, 1 s read / 5 s write timeouts, 8 KiB request head,
//! - connections are handled concurrently (up to 8) so an idle peer cannot stall
//!   the whole agent,
//! - CORS is limited to `http://127.0.0.1:<port>` / `http://localhost:<port>`.
//!
//! `--demo` serves deterministic data for UI development; `--once` prints one
//! snapshot as JSON and exits, which is handy for smoke tests.

#![forbid(unsafe_code)]

mod args;
mod auth;
mod http;
mod telemetry;

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tm_core::demo::DemoProbe;
use tm_core::model::SystemSnapshot;
use tm_core::port::SystemProbe;
use tm_platform::NativeProbe;

use telemetry::{AgentError, ExitCategory, Level, Metrics, RequestTrace};

/// Health endpoint (no authentication, reveals nothing sensitive).
const HEALTH_PATH: &str = "/healthz";
/// Snapshot endpoint (bearer token required).
const SNAPSHOT_PATH: &str = "/api/v1/snapshot";
/// Metrics endpoint (no authentication; counters/gauges only, no machine data).
const METRICS_PATH: &str = "/metrics";

/// Number of processes returned when the client does not ask for a limit.
///
/// A desktop can easily run 3,000+ processes; sending all of them once per
/// second would produce multi-megabyte responses in the browser. The UI only
/// renders a table page, so the agent truncates by default and reports the
/// full count alongside the list.
const DEFAULT_PROCESS_LIMIT: usize = 500;

/// Delay between the discarded warm-up sample and the first published sample.
/// Must exceed sysinfo's `MINIMUM_CPU_UPDATE_INTERVAL` (~200 ms).
const WARMUP_INTERVAL: Duration = Duration::from_millis(250);

/// Hard upper bound for `?limit=` (a larger value is clamped, not rejected).
const MAX_PROCESS_LIMIT: usize = 2000;

/// Maximum concurrent connections. The extra ones get an immediate 503 instead
/// of waiting behind idle peers; the agent is loopback-only, but an unbounded
/// thread-per-connection server would still be a local resource exhaustion vector.
const MAX_CONNECTIONS: usize = 8;

/// How long the accept loop waits for in-flight connection workers once it
/// stops accepting. A worker can hold a 1 s read timeout plus a ~250 ms probe
/// warm-up, so 2 s is enough for a well-behaved peer without blocking shutdown
/// forever (R2-X03).
const SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// `JoinHandle::join` has no deadline, so the tracker checks `is_finished`
/// between short sleeps to bound the wait without spawning a helper thread.
const JOIN_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Warning emitted whenever `--token` is present on the command line.
///
/// `/proc/<pid>/cmdline` is world-readable by default, so the credential is
/// visible to every user on the host — not just the owner. Kept as a constant
/// so the exact wording can be asserted in tests and stays in sync with the
/// `args::USAGE` authentication section.
const TOKEN_ARGV_WARNING: &str = concat!(
    "警告: --token の値は /proc/<pid>/cmdline に残り、既定では同一ホストの全ユーザーが読めます。",
    "推奨順は --token-file > TM_AGENT_TOKEN > --token です。",
    "TM_AGENT_TOKEN は /proc/<pid>/environ に残存し得ます。"
);

/// Minimum severity for structured events, set once from `--log-level`.
///
/// `0` = error, `1` = warn, `2` = info. `debug` and `trace` map to `2` because
/// the agent currently emits only Info/Warn/Error events, so there is nothing
/// lower-severity to reveal at those settings.
static LOG_THRESHOLD: AtomicU8 = AtomicU8::new(2);

/// Lower ranks are more severe; used for both `Level` and the CLI threshold.
fn level_rank(level: Level) -> u8 {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
    }
}

fn threshold_rank(level: args::LogLevel) -> u8 {
    match level {
        args::LogLevel::Error => 0,
        args::LogLevel::Warn => 1,
        args::LogLevel::Info | args::LogLevel::Debug | args::LogLevel::Trace => 2,
    }
}

/// Pure form of the threshold check (kept separate so it can be tested).
fn level_allowed(level: Level, minimum: args::LogLevel) -> bool {
    level_rank(level) <= threshold_rank(minimum)
}

/// Emits a structured event unless `--log-level` suppresses it.
fn emit(level: Level, event: &str, fields: serde_json::Value) {
    let minimum = match LOG_THRESHOLD.load(Ordering::Relaxed) {
        0 => args::LogLevel::Error,
        1 => args::LogLevel::Warn,
        _ => args::LogLevel::Info,
    };
    if level_allowed(level, minimum) {
        telemetry::emit(level, event, fields);
    }
}

/// Wire envelope returned by `/api/v1/snapshot` and by `--once`.
#[derive(Serialize)]
struct SnapshotEnvelope<'a> {
    schema_version: u32,
    /// Number of processes in the machine, before `?limit=` truncation.
    total_process_count: usize,
    snapshot: &'a SystemSnapshot,
}

/// Wire body returned by `/healthz`.
#[derive(Serialize)]
struct HealthBody {
    ok: bool,
    schema_version: u32,
}

/// Stable, machine-readable codes carried by every HTTP error body.
///
/// Keep this list in sync with the `HTTP エラーコード` section of `args::USAGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorCode {
    BadRequest,
    InvalidLimit,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    RequestTooLarge,
    RequestTimeout,
    Internal,
    Busy,
}

impl ErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::InvalidLimit => "invalid_limit",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::RequestTooLarge => "request_too_large",
            Self::RequestTimeout => "request_timeout",
            Self::Internal => "internal",
            Self::Busy => "busy",
        }
    }
}

/// Wire body returned for error responses.
///
/// `code` is the stable machine-readable contract; `message` is its canonical
/// human-readable companion. `error` is a deprecated alias of `message` kept
/// so existing clients continue to work.
#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    error: &'a str,
}

fn main() {
    match run() {
        Ok(()) => emit(
            Level::Info,
            "agent.exit",
            json!({ "exit_code": 0, "category": ExitCategory::Success.as_str() }),
        ),
        Err(error) => {
            // Structured first (machine-readable), then the unchanged human line.
            emit(
                Level::Error,
                "agent.exit",
                json!({
                    "exit_code": error.code(),
                    "category": error.category().as_str(),
                    "error": error.message(),
                }),
            );
            eprintln!("tm-agent: {}", error.message());
            std::process::exit(error.code());
        }
    }
}

/// Runs the agent and returns a categorised error on failure.
///
/// Error categories map to stable exit codes (see [`ExitCategory`]); the legacy
/// human-readable message is preserved verbatim on stderr.
fn run() -> Result<(), AgentError> {
    // `args()` panics on non-UTF-8 argv; `args_os()` keeps a stray byte from
    // aborting the agent. Reject it as a usage error without echoing the raw
    // bytes (tokens must never be logged, R49 audit).
    let arguments = std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument.into_string().map_err(|_| {
                AgentError::new(
                    ExitCategory::Usage,
                    "引数に UTF-8 として解釈できないバイトがあります",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let config =
        args::parse(arguments).map_err(|message| AgentError::new(ExitCategory::Usage, message))?;
    LOG_THRESHOLD.store(threshold_rank(config.log_level), Ordering::Relaxed);
    if config.version {
        // Same one-line shape as `gpui-task-manager --version`; stdout only.
        println!("tm-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if config.help {
        print!("{}", args::USAGE);
        return Ok(());
    }

    let mut probe =
        build_probe(&config).map_err(|message| AgentError::new(ExitCategory::Probe, message))?;

    // Warn about argv exposure before the `--once` early return so operators
    // running a one-shot snapshot with `--token` see the same warning.
    if config.token.is_some() {
        eprintln!("tm-agent: {TOKEN_ARGV_WARNING}");
    }

    if config.once {
        return print_once(
            probe.as_mut(),
            config.redact,
            config.token.as_deref().unwrap_or_default(),
        )
        .map_err(|message| AgentError::new(ExitCategory::Probe, message));
    }

    let token = match (config.token, config.token_file.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(AgentError::new(
                ExitCategory::Usage,
                "--token と --token-file は同時に指定できません",
            ));
        }
        (Some(token), None) => {
            // The argv warning was already emitted above (also for --once).
            // `request_token` trims the presented value, so store the trimmed
            // token for every source (the file path already does this).
            token.trim().to_string()
        }
        (None, Some(path)) => {
            read_token_file(path).map_err(|message| AgentError::new(ExitCategory::Auth, message))?
        }
        (None, None) => match std::env::var("TM_AGENT_TOKEN") {
            Ok(token) if !token.trim().is_empty() => token.trim().to_string(),
            _ => {
                let token = auth::generate_token()
                    .map_err(|message| AgentError::new(ExitCategory::Auth, message))?;
                // Never log the generated token: stderr is routinely captured
                // in CI output and support bundles. Operators who need to know
                // the credential pass --token-file or TM_AGENT_TOKEN instead.
                eprintln!("tm-agent: 認証トークンを自動生成しました（値はログに出力しません）");
                token
            }
        },
    };

    let listener = TcpListener::bind(config.listen).map_err(|err| {
        AgentError::new(
            ExitCategory::Network,
            format!("{} を bind できません: {err}", config.listen),
        )
    })?;
    let local = listener.local_addr().map_err(|err| {
        AgentError::new(
            ExitCategory::Network,
            format!("待ち受けアドレスを取得できません: {err}"),
        )
    })?;
    eprintln!(
        "tm-agent: http://{local} で待ち受けます (読み取り専用, demo={}, schema_version={})",
        config.demo,
        tm_core::WIRE_SCHEMA_VERSION
    );
    emit(
        Level::Info,
        "agent.listening",
        json!({
            "address": local.to_string(),
            "demo": config.demo,
            "redact": config.redact,
            "schema_version": tm_core::WIRE_SCHEMA_VERSION,
        }),
    );
    // `serve` still returns `Result<(), String>`; `From<String>` maps it to
    // the historical Internal category / exit code 1.
    serve(&listener, probe, &token, config.redact, !config.demo).map_err(AgentError::from)
}

/// Returns only the leaf name of a token file for operator messages.
///
/// The full path can include the user's home directory or an internal mount
/// point, so error messages must never echo it.
fn token_file_name(path: &std::path::Path) -> std::borrow::Cow<'_, str> {
    path.file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("(名前なし)"))
}

/// Classifies a probe error without echoing its free-form detail string.
///
/// `ProbeError` payloads can embed internal paths or host names. Logs and
/// structured events keep only the variant, which is enough to triage a
/// failure without leaking identifiers.
fn probe_error_label(error: &tm_core::ProbeError) -> &'static str {
    match error {
        tm_core::ProbeError::Unsupported(_) => "未対応",
        tm_core::ProbeError::Io(_) => "I/O エラー",
        tm_core::ProbeError::Permission(_) => "権限不足",
    }
}

/// Reads a bearer token from a file, trimming the trailing newline.
///
/// Preferred over `--token`: file contents are not exposed through
/// `/proc/<pid>/cmdline`. Empty files are rejected so a misconfiguration cannot
/// start an agent with an empty token. Errors name the file only, never its
/// parent directories.
fn read_token_file(path: &std::path::Path) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|err| {
                format!(
                    "トークンファイルの属性を読めません ({}): {err}",
                    token_file_name(path)
                )
            })?
            .permissions()
            .mode();
        // Group/other readability would expose the secret to other local users.
        if mode & 0o044 != 0 {
            return Err(format!(
                "トークンファイルの権限が緩すぎます ({}): 0600 にしてください (現在 {:04o})",
                token_file_name(path),
                mode & 0o777
            ));
        }
    }
    let contents = std::fs::read_to_string(path).map_err(|err| {
        format!(
            "トークンファイルを読めません ({}): {err}",
            token_file_name(path)
        )
    })?;
    let token = contents.trim();
    if token.is_empty() {
        return Err(format!(
            "トークンファイルが空です: {}",
            token_file_name(path)
        ));
    }
    Ok(token.to_string())
}

/// Builds the probe selected by the command line.
fn build_probe(config: &args::Config) -> Result<Box<dyn SystemProbe>, String> {
    if config.demo {
        Ok(Box::new(DemoProbe::new()))
    } else {
        NativeProbe::new()
            .map(|probe| Box::new(probe) as Box<dyn SystemProbe>)
            .map_err(|err| {
                format!(
                    "スナップショット収集を初期化できません ({})",
                    probe_error_label(&err)
                )
            })
    }
}

/// Implements `--once`: one snapshot as JSON on stdout, then exit.
fn print_once(probe: &mut dyn SystemProbe, redact: bool, token: &str) -> Result<(), String> {
    println!("{}", once_json(probe, redact, token)?);
    Ok(())
}

/// Builds the `--once` JSON line without printing it (unit-testable).
fn once_json(probe: &mut dyn SystemProbe, redact: bool, token: &str) -> Result<String, String> {
    // CPU percentages are deltas to the previous sample, so the first sample
    // is a warm-up and is not printed.
    probe
        .sample()
        .map_err(|err| format!("スナップショット取得に失敗 ({})", probe_error_label(&err)))?;
    std::thread::sleep(WARMUP_INTERVAL);
    let mut snapshot = probe
        .sample()
        .map_err(|err| format!("スナップショット取得に失敗 ({})", probe_error_label(&err)))?;
    if redact {
        redact_snapshot(&mut snapshot, token);
    } else {
        // `--once --token <secret>` without `--redact` still publishes stdout
        // JSON. The token lives in this process's argv, which the probe copies
        // into `process.command`/`process.name`; scrub it unconditionally.
        scrub_argv_token(&mut snapshot, token);
    }
    let total = apply_process_limit(&mut snapshot, DEFAULT_PROCESS_LIMIT);
    let envelope = SnapshotEnvelope {
        schema_version: tm_core::WIRE_SCHEMA_VERSION,
        total_process_count: total,
        snapshot: &snapshot,
    };
    serde_json::to_string(&envelope).map_err(|err| format!("JSON への変換に失敗しました: {err}"))
}

/// State shared by connection threads.
///
/// `SystemProbe` is `Send`, so one probe can be shared behind a mutex. Sampling
/// stays serial (a snapshot costs ~250 ms), but reading a request head no longer
/// blocks other connections: the lock is taken only while routing a full request.
struct SharedState {
    probe: Mutex<Box<dyn SystemProbe>>,
    token: String,
    redact: bool,
    active: AtomicUsize,
    /// `false` until the first real-probe snapshot has consumed a warm-up
    /// sample. Demo data starts ready (its ticks are deterministic).
    warmed_up: AtomicBool,
    /// Request counters/gauges exposed read-only at [`METRICS_PATH`].
    metrics: Arc<Metrics>,
}

impl SharedState {
    fn new(probe: Box<dyn SystemProbe>, token: &str, redact: bool, needs_warmup: bool) -> Self {
        Self {
            probe: Mutex::new(probe),
            token: token.to_string(),
            redact,
            active: AtomicUsize::new(0),
            warmed_up: AtomicBool::new(!needs_warmup),
            metrics: Arc::new(Metrics::new()),
        }
    }
}

/// Outcome of [`JoinTracker::join_all`].
#[derive(Debug, Default, PartialEq, Eq)]
struct JoinOutcome {
    /// Workers that exited and were joined.
    joined: usize,
    /// Joined workers that unwound instead of returning normally.
    panicked: usize,
    /// Whether the deadline expired before every tracked worker was joined.
    timed_out: bool,
}

/// Tracks the `JoinHandle` of every spawned worker thread (R2-X03).
///
/// `spawn_connection` used to drop each handle, so shutdown could neither
/// observe nor wait for in-flight connections. Handles live behind a mutex
/// because the accept loop and the workers run on different threads. Handles
/// whose thread already exited are pruned on registration, keeping the vector
/// bounded during a long-running service.
#[derive(Clone)]
struct JoinTracker {
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl JoinTracker {
    fn new() -> Self {
        Self {
            handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Registers a worker handle, dropping handles whose thread already exited.
    fn track(&self, handle: JoinHandle<()>) {
        let mut handles = self.lock_handles();
        handles.retain(|candidate| !candidate.is_finished());
        handles.push(handle);
    }

    /// Number of handles still awaiting a join.
    fn tracked_count(&self) -> usize {
        self.lock_handles().len()
    }

    /// Joins every tracked worker, returning when they are all done or when
    /// `timeout` elapses. Workers still running on timeout stay tracked so a
    /// later call can retry; this method never waits longer than `timeout`.
    fn join_all(&self, timeout: Duration) -> JoinOutcome {
        let deadline = std::time::Instant::now() + timeout;
        let mut outcome = JoinOutcome::default();

        loop {
            let handles = std::mem::take(&mut *self.lock_handles());
            let mut pending = Vec::with_capacity(handles.len());
            for handle in handles {
                if handle.is_finished() {
                    if handle.join().is_err() {
                        outcome.panicked += 1;
                    }
                    outcome.joined += 1;
                } else {
                    pending.push(handle);
                }
            }
            {
                let mut live = self.lock_handles();
                // Preserve handles registered while we were joining.
                pending.append(&mut live);
                *live = pending;
            }

            if self.tracked_count() == 0 {
                return outcome;
            }
            if std::time::Instant::now() >= deadline {
                outcome.timed_out = true;
                return outcome;
            }
            std::thread::sleep(JOIN_POLL_INTERVAL);
        }
    }

    /// Locks the handle list, recovering it if a previous holder panicked.
    fn lock_handles(&self) -> std::sync::MutexGuard<'_, Vec<JoinHandle<()>>> {
        self.handles.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// Accepts connections until the process is terminated.
///
/// One thread per connection. An idle peer waiting for the read timeout can no
/// longer stall `/healthz` or another snapshot (review item s1); the probe mutex
/// is held only during routing, not while reading the request head.
fn serve(
    listener: &TcpListener,
    probe: Box<dyn SystemProbe>,
    token: &str,
    redact: bool,
    needs_warmup: bool,
) -> Result<(), String> {
    let state = Arc::new(SharedState::new(probe, token, redact, needs_warmup));
    let workers = JoinTracker::new();
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                if let Err(err) = spawn_connection(Arc::clone(&state), stream, &workers) {
                    eprintln!("tm-agent: {err}");
                }
            }
            // A persistent accept error (e.g. EMFILE) would otherwise spin the
            // loop at full speed; pause briefly before retrying (R2X-FR1).
            Err(err) => {
                eprintln!("tm-agent: accept に失敗: {err}");
                emit(Level::Warn, "accept.failed", json!({}));
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    // R2-X03: the accept loop has ended, so no new workers can appear. Reap the
    // in-flight ones with a deadline instead of dropping their handles. This
    // adds no telemetry events and leaves the CLI/error contract untouched.
    let outcome = workers.join_all(SHUTDOWN_JOIN_TIMEOUT);
    if outcome.timed_out || outcome.panicked > 0 {
        eprintln!(
            "tm-agent: 接続スレッドの終了を確認できませんでした (join={}, panic={}, 未完了={})",
            outcome.joined,
            outcome.panicked,
            workers.tracked_count()
        );
    }
    Ok(())
}

/// Decrements the live-connection counter on every exit path, including panic.
struct ActiveConnection(Arc<SharedState>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Handles one accepted connection, enforcing [`MAX_CONNECTIONS`].
///
/// The worker's `JoinHandle` is registered with `workers` (R2-X03); the 503
/// rejection path spawns no thread and therefore registers nothing.
fn spawn_connection(
    state: Arc<SharedState>,
    mut stream: TcpStream,
    workers: &JoinTracker,
) -> Result<(), String> {
    let previous = state.active.fetch_add(1, Ordering::AcqRel);
    if previous >= MAX_CONNECTIONS {
        // Drop guard returns the slot even if writing the rejection panics.
        let metrics = Arc::clone(&state.metrics);
        let _slot = ActiveConnection(state);
        let mut trace = RequestTrace::begin_unknown(&metrics);
        trace.finish(503);
        let response = http::Response::json(
            503,
            "Service Unavailable",
            error_body(ErrorCode::Busy, "同時接続数が上限に達しました"),
        );
        // The 503 path is written synchronously on the accept thread, bounded
        // by WRITE_TIMEOUT (5 s). In practice the ~240-byte response fits in
        // the loopback send buffer and returns immediately; a peer stopping
        // reads alone cannot stall the loop. Do NOT move this to a worker:
        // spawning a thread per rejected connection would defeat MAX_CONNECTIONS
        // and open a local resource-exhaustion vector.
        if let Err(err) = stream.set_write_timeout(Some(http::WRITE_TIMEOUT)) {
            eprintln!("tm-agent: 503 のタイムアウト設定に失敗: {err}");
        }
        if let Err(err) = http::write_response(&mut stream, &response) {
            eprintln!("tm-agent: 503 応答に失敗: {err}");
        }
        return Ok(());
    }

    let _slot = ActiveConnection(Arc::clone(&state));
    let worker_state = Arc::clone(&state);
    let worker = std::thread::Builder::new()
        .name("tm-agent-connection".to_string())
        .spawn(move || {
            // `_slot` is moved into the worker so the counter is released when the
            // worker exits — or unwinds after a panic — never on the accept thread.
            let _slot = _slot;
            let result = handle_connection(&mut stream, &worker_state.metrics, |request| {
                // Only snapshot requests need the probe. Acquiring it for
                // /healthz and /metrics would make liveness wait for a slow
                // sample (including the 250 ms warm-up sleep) — R40 audit.
                let needs_probe = request.path == SNAPSHOT_PATH;
                let mut guard = if needs_probe {
                    // Rust の Mutex はパニックで poison する可能性があるが、
                    // プローブ側の致命的失敗でエージェント全体を止めないよう
                    // guard を回収して続行する（無ログの fail-open は避ける）。
                    Some(match worker_state.probe.lock() {
                        Ok(guard) => guard,
                        Err(err) => {
                            eprintln!(
                                "tm-agent: プローブの mutex が poison しました; 回収して続行します"
                            );
                            emit(
                                Level::Warn,
                                "probe.mutex_poisoned",
                                json!({ "recovered": true }),
                            );
                            err.into_inner()
                        }
                    })
                } else {
                    None
                };
                route(
                    request,
                    guard
                        .as_deref_mut()
                        .map(|guard| &mut **guard as &mut dyn SystemProbe),
                    &worker_state.token,
                    worker_state.redact,
                    &worker_state.warmed_up,
                    &worker_state.metrics,
                    &worker_state.active,
                )
            });
            if let Err(err) = result {
                eprintln!("tm-agent: 接続処理に失敗: {err}");
            }
        })
        .map_err(|err| {
            // Do NOT decrement `active` here: `std::thread` drops the spawn
            // closure (and thus `_slot`) when `Thread::new` fails, so the drop
            // guard already released the slot. A manual decrement would make it
            // wrap to usize::MAX and reject every later connection with 503.
            format!("接続スレッドを起動できません: {err}")
        })?;
    // Only a successfully spawned worker is tracked; on spawn failure the
    // builder already dropped the closure (and its connection-slot guard).
    workers.track(worker);
    Ok(())
}

/// Handles exactly one HTTP request on `stream`.
///
/// The caller supplies `route` so the shared probe is locked only after a
/// complete request head has been read. Every request gets a [`RequestTrace`]:
/// its `Drop` implementation guarantees even an early error return is recorded
/// in the structured log and metrics.
fn handle_connection(
    stream: &mut TcpStream,
    metrics: &Metrics,
    mut route: impl FnMut(&http::Request) -> http::Response,
) -> io::Result<()> {
    stream.set_read_timeout(Some(http::READ_TIMEOUT))?;
    stream.set_write_timeout(Some(http::WRITE_TIMEOUT))?;

    let request = match http::read_request(stream) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(parse_error) => {
            let (status, reason, code, message) = match parse_error {
                http::ParseError::TooLarge => (
                    431,
                    "Request Header Fields Too Large",
                    ErrorCode::RequestTooLarge,
                    "リクエストヘッダが大きすぎます",
                ),
                http::ParseError::Malformed => (
                    400,
                    "Bad Request",
                    ErrorCode::BadRequest,
                    "リクエストを解釈できません",
                ),
                http::ParseError::Timeout => (
                    408,
                    "Request Timeout",
                    ErrorCode::RequestTimeout,
                    "リクエストの送信がタイムアウトしました",
                ),
            };
            let mut trace = RequestTrace::begin_unknown(metrics);
            trace.finish(status);
            let response = http::Response::json(status, reason, error_body(code, message));
            return http::write_response(stream, &response);
        }
    };

    // Only `path` is traced. The raw target (with `?token=...`) and the
    // Authorization header must never reach a log line.
    let mut trace = RequestTrace::begin(metrics, &request.method, &request.path);

    // Compute the allowed Origin before any early return so error responses
    // (e.g. the Host check below) carry CORS headers too. Without this, the
    // browser UI gets an opaque CORS failure instead of the 403 body.
    let origin = request.header("origin").map(str::to_string);
    let allowed_origin = origin
        .as_deref()
        .filter(|origin| http::is_allowed_origin(origin));

    if let Some(host) = request.header("host") {
        if !http::is_allowed_host(host) {
            let mut response = http::Response::json(
                403,
                "Forbidden",
                error_body(
                    ErrorCode::Forbidden,
                    "Host ヘッダがループバックではありません",
                ),
            );
            if let Some(origin) = allowed_origin {
                http::add_cors_headers(&mut response, origin);
            }
            trace.finish(response.status);
            return http::write_response(stream, &response);
        }
    }

    let mut response = route(&request);
    trace.finish(response.status);
    if let Some(origin) = allowed_origin {
        http::add_cors_headers(&mut response, origin);
    }
    http::write_response(stream, &response)
}

/// Routes a parsed request to its handler.
fn route(
    request: &http::Request,
    probe: Option<&mut dyn SystemProbe>,
    token: &str,
    redact: bool,
    warmed_up: &AtomicBool,
    metrics: &Metrics,
    active: &AtomicUsize,
) -> http::Response {
    if request.method.eq_ignore_ascii_case("OPTIONS") {
        return preflight_response(request);
    }
    if !request.method.eq_ignore_ascii_case("GET") {
        let mut response = http::Response::json(
            405,
            "Method Not Allowed",
            error_body(
                ErrorCode::MethodNotAllowed,
                "このエージェントは GET のみ対応しています",
            ),
        );
        response
            .extra_headers
            .push(("Allow".to_string(), "GET, OPTIONS".to_string()));
        return response;
    }

    match request.path.as_str() {
        HEALTH_PATH => health_response(),
        SNAPSHOT_PATH => match probe {
            Some(probe) => snapshot_response(request, probe, token, redact, warmed_up),
            // The caller only acquires the probe mutex for snapshot requests;
            // reaching here means the lock failed to be passed through.
            None => http::Response::json(
                500,
                "Internal Server Error",
                error_body(
                    ErrorCode::Internal,
                    "スナップショットを取得できませんでした",
                ),
            ),
        },
        METRICS_PATH => metrics_response(metrics, active),
        _ => http::Response::json(
            404,
            "Not Found",
            error_body(ErrorCode::NotFound, "エンドポイントが見つかりません"),
        ),
    }
}

/// Answers a CORS preflight request.
///
/// Preflights carry no credentials, so they are answered without a token; the
/// actual `GET` still needs one.
fn preflight_response(request: &http::Request) -> http::Response {
    let Some(origin) = request.header("origin") else {
        return http::Response::json(
            403,
            "Forbidden",
            error_body(ErrorCode::Forbidden, "Origin ヘッダが必要です"),
        );
    };
    if !http::is_allowed_origin(origin) {
        return http::Response::json(
            403,
            "Forbidden",
            error_body(ErrorCode::Forbidden, "許可されていない Origin です"),
        );
    }
    let mut response = http::Response::empty(204, "No Content");
    response.extra_headers.push((
        "Access-Control-Allow-Methods".to_string(),
        "GET, OPTIONS".to_string(),
    ));
    response.extra_headers.push((
        "Access-Control-Allow-Headers".to_string(),
        "Authorization, Content-Type".to_string(),
    ));
    response
        .extra_headers
        .push(("Access-Control-Max-Age".to_string(), "600".to_string()));
    response
}

/// `/healthz`: liveness only, no machine data, no token.
fn health_response() -> http::Response {
    json_response(
        200,
        "OK",
        &HealthBody {
            ok: true,
            schema_version: tm_core::WIRE_SCHEMA_VERSION,
        },
    )
}

/// `/metrics`: process-local counters/gauges in Prometheus text format.
///
/// Deliberately public like `/healthz` (loopback-only listener, no machine
/// data). It does not replace or alter any existing endpoint.
fn metrics_response(metrics: &Metrics, active: &AtomicUsize) -> http::Response {
    let body = metrics.render_prometheus(active.load(Ordering::Acquire));
    let mut response = http::Response::json(200, "OK", body);
    response.content_type = "text/plain; version=0.0.4; charset=utf-8";
    response
}

/// `/api/v1/snapshot`: authenticated, read-only snapshot.
fn snapshot_response(
    request: &http::Request,
    probe: &mut dyn SystemProbe,
    token: &str,
    redact: bool,
    warmed_up: &AtomicBool,
) -> http::Response {
    let Some(supplied) = auth::request_token(request) else {
        // Never log the supplied value; only the fact that it was absent.
        emit(
            Level::Warn,
            "auth.denied",
            json!({ "reason": "missing_token", "path": request.path }),
        );
        return unauthorized_response();
    };
    if !auth::constant_time_eq(token, &supplied) {
        emit(
            Level::Warn,
            "auth.denied",
            json!({ "reason": "invalid_token", "path": request.path }),
        );
        return unauthorized_response();
    }

    let limit = match parse_process_limit(request.query.as_deref()) {
        Ok(limit) => limit,
        Err(message) => {
            return http::Response::json(
                400,
                "Bad Request",
                error_body(ErrorCode::InvalidLimit, &message),
            )
        }
    };

    // CPU percentages are deltas; the first native sample is always 0.0, which
    // makes every process tie and turns `limit` into an arbitrary PID subset
    // instead of the busiest processes. Consume one discarded sample on the
    // first authenticated snapshot only. The probe mutex is held by the caller
    // (`route`), so concurrent connections cannot race this check.
    if !warmed_up.load(Ordering::Acquire) {
        if let Err(err) = probe.sample() {
            eprintln!(
                "tm-agent: ウォームアップサンプルに失敗しました ({})",
                probe_error_label(&err)
            );
            emit(
                Level::Error,
                "probe.sample_failed",
                json!({ "phase": "warmup" }),
            );
            return http::Response::json(
                500,
                "Internal Server Error",
                error_body(
                    ErrorCode::Internal,
                    "スナップショットを取得できませんでした",
                ),
            );
        }
        std::thread::sleep(WARMUP_INTERVAL);
        warmed_up.store(true, Ordering::Release);
    }

    match probe.sample() {
        Ok(mut snapshot) => {
            if redact {
                redact_snapshot(&mut snapshot, token);
            } else {
                // Same argv echo as `--once`: a non-redacted HTTP snapshot
                // must not carry our own `--token` command line either.
                scrub_argv_token(&mut snapshot, token);
            }
            let total = apply_process_limit(&mut snapshot, limit);
            let envelope = SnapshotEnvelope {
                schema_version: tm_core::WIRE_SCHEMA_VERSION,
                total_process_count: total,
                snapshot: &snapshot,
            };
            json_response(200, "OK", &envelope)
        }
        Err(err) => {
            // Only the error class reaches the operator's log; the free-form
            // detail can contain paths or host names. The client always gets a
            // fixed string as well.
            eprintln!(
                "tm-agent: スナップショット取得に失敗しました ({})",
                probe_error_label(&err)
            );
            emit(
                Level::Error,
                "probe.sample_failed",
                json!({ "phase": "snapshot" }),
            );
            http::Response::json(
                500,
                "Internal Server Error",
                error_body(
                    ErrorCode::Internal,
                    "スナップショットを取得できませんでした",
                ),
            )
        }
    }
}

/// Removes this process's `--token` value from the argv echo in a snapshot.
///
/// A token passed via `--token` lives in our own argv, which the native probe
/// copies into `process.command` and `process.name`. `tm_core::redact` cannot
/// know that value, so it must be scrubbed explicitly. This has to run even
/// without `--redact`: `--once --token <secret>` still publishes stdout JSON.
fn scrub_argv_token(snapshot: &mut tm_core::SystemSnapshot, token: &str) {
    if token.is_empty() {
        return;
    }
    for process in &mut snapshot.processes {
        if process.command.contains(token) {
            process.command = process.command.replace(token, "***");
        }
        if process.name.contains(token) {
            process.name = process.name.replace(token, "***");
        }
    }
}

/// Removes local identifiers from a snapshot before it is serialised.
///
/// Used by `--redact` so screenshots and JSON evidence can be published without
/// exposing the host name, user names or home directory layout.
///
/// The rules live in `tm_core::redact` so the native shell (`--redact`) and
/// this endpoint cannot drift. Values are captured before redaction and owned
/// as Strings to avoid borrowing a snapshot we are about to mutate.
fn redact_snapshot(snapshot: &mut tm_core::SystemSnapshot, token: &str) {
    let original_hostname = snapshot.hostname.clone().unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    let ids = tm_core::redact::RedactionIdentifiers {
        home: &home,
        user: &user,
        hostname: &original_hostname,
    };
    tm_core::redact::redact_snapshot(snapshot, &ids);
    // "Redacted" evidence must not leak the Bearer token either (R45).
    scrub_argv_token(snapshot, token);
}

/// Fixed message for an invalid `?limit=`; the raw input is never echoed
/// because it may contain a credential or other client-controlled identifier.
const INVALID_LIMIT_MESSAGE: &str = "limit には 1 以上の整数を指定してください";

/// Parses `?limit=`: default [`DEFAULT_PROCESS_LIMIT`], clamped to
/// [`MAX_PROCESS_LIMIT`].
///
/// `limit=0` and non-numeric values are rejected instead of being silently
/// coerced, so a client bug shows up as `400` rather than as an empty table.
fn parse_process_limit(query: Option<&str>) -> Result<usize, String> {
    let Some(raw) = http::query_param(query, "limit") else {
        return Ok(DEFAULT_PROCESS_LIMIT);
    };
    let parsed = raw
        .trim()
        .parse::<u64>()
        .map_err(|_| INVALID_LIMIT_MESSAGE.to_string())?;
    if parsed == 0 {
        return Err(INVALID_LIMIT_MESSAGE.to_string());
    }
    Ok(usize::try_from(parsed)
        .unwrap_or(MAX_PROCESS_LIMIT)
        .min(MAX_PROCESS_LIMIT))
}

/// Sorts processes by CPU (descending) then PID, truncates to `limit`, and
/// returns the number of processes that existed before truncation.
fn apply_process_limit(snapshot: &mut SystemSnapshot, limit: usize) -> usize {
    let total = snapshot.processes.len();
    snapshot.processes.sort_by(|left, right| {
        // Sort by the displayed value: a negative/NaN CPU percentage renders as
        // 0.0% and must not jump ahead of real work when `limit` truncates.
        tm_core::normalise_percent(right.cpu_percent)
            .total_cmp(&tm_core::normalise_percent(left.cpu_percent))
            .then_with(|| left.key.pid.cmp(&right.key.pid))
    });
    snapshot.processes.truncate(limit);
    total
}

/// `401` with the standard bearer challenge.
fn unauthorized_response() -> http::Response {
    let mut response = http::Response::json(
        401,
        "Unauthorized",
        error_body(ErrorCode::Unauthorized, "認証トークンが必要です"),
    );
    response
        .extra_headers
        .push(("WWW-Authenticate".to_string(), "Bearer".to_string()));
    response
}

/// Serialises `value` as JSON, mapping serialisation failures to a `500`.
fn json_response<T: Serialize>(status: u16, reason: &'static str, value: &T) -> http::Response {
    match serde_json::to_vec(value) {
        Ok(body) => http::Response::json(status, reason, body),
        Err(err) => {
            eprintln!("tm-agent: JSON への変換に失敗: {err}");
            emit(
                Level::Error,
                "response.serialize_failed",
                json!({ "status": status }),
            );
            http::Response::json(
                500,
                "Internal Server Error",
                error_body(ErrorCode::Internal, "JSON への変換に失敗しました"),
            )
        }
    }
}

/// Builds a `{"code":"...","message":"...","error":"..."}` body.
///
/// `error` duplicates `message` for backward compatibility; `code` is the
/// stable, machine-readable contract documented in `args::USAGE`.
fn error_body(code: ErrorCode, message: &str) -> Vec<u8> {
    serde_json::to_vec(&ErrorBody {
        code: code.as_str(),
        message,
        error: message,
    })
    .unwrap_or_else(|_| {
        b"{\"code\":\"internal\",\"message\":\"internal error\",\"error\":\"internal error\"}"
            .to_vec()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{Shutdown, SocketAddr};
    use std::time::Instant;

    const TEST_TOKEN: &str = "test-token";

    /// Bounded join for demo-server threads: a stuck server must fail its test
    /// instead of hanging the whole suite (R2-X03).
    const TEST_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

    /// Joins a test server thread through the same tracker as production
    /// workers, asserting that it stopped cleanly within [`TEST_JOIN_TIMEOUT`].
    fn join_demo_server(handle: JoinHandle<()>) {
        let tracker = JoinTracker::new();
        tracker.track(handle);
        let outcome = tracker.join_all(TEST_JOIN_TIMEOUT);
        assert!(
            !outcome.timed_out,
            "demo server did not exit within {TEST_JOIN_TIMEOUT:?}"
        );
        assert_eq!(outcome.panicked, 0, "demo server thread panicked");
        assert_eq!(outcome.joined, 1);
        assert_eq!(tracker.tracked_count(), 0);
    }

    /// Starts a demo-mode server that accepts exactly `connections` requests.
    fn spawn_demo_server(connections: usize) -> (SocketAddr, JoinHandle<()>) {
        spawn_demo_server_with_redaction(connections, false)
    }

    /// Same as [`spawn_demo_server`] with an explicit `--redact` setting.
    fn spawn_demo_server_with_redaction(
        connections: usize,
        redact: bool,
    ) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let mut probe = DemoProbe::new();
            let warm = AtomicBool::new(true); // demo data needs no warm-up
            let metrics = Metrics::new();
            let active = AtomicUsize::new(0);
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().expect("accept");
                handle_connection(&mut stream, &metrics, |request| {
                    route(
                        request,
                        Some(&mut probe),
                        TEST_TOKEN,
                        redact,
                        &warm,
                        &metrics,
                        &active,
                    )
                })
                .expect("handle connection");
            }
        });
        (address, handle)
    }

    /// Sends one raw request and returns the full raw response.
    fn send_request(address: SocketAddr, raw: &str) -> String {
        let mut stream = TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        stream.write_all(raw.as_bytes()).expect("write request");
        // The server may close the connection immediately after rejecting a
        // request (e.g. oversized headers), making half-close fail with
        // NotConnected/BrokenPipe on some platforms and schedules.
        let _ = stream.shutdown(Shutdown::Write);
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        response
    }

    fn parse_body(response: &str) -> serde_json::Value {
        let (_, body) = response
            .split_once("\r\n\r\n")
            .expect("response must contain a body separator");
        serde_json::from_str(body).expect("body must be JSON")
    }

    struct FailingProbe;

    impl SystemProbe for FailingProbe {
        fn platform_info(&self) -> tm_core::PlatformInfo {
            tm_core::PlatformInfo::demo()
        }
        fn sample(&mut self) -> Result<tm_core::SystemSnapshot, tm_core::ProbeError> {
            Err(tm_core::ProbeError::Io(
                "SECRET-PATH /home/victim".to_string(),
            ))
        }
    }

    /// Demo snapshot whose own process echoes a token on its command line,
    /// mimicking how a native probe copies this process's argv into a snapshot.
    struct TokenEchoProbe {
        inner: DemoProbe,
        token: &'static str,
    }

    impl SystemProbe for TokenEchoProbe {
        fn platform_info(&self) -> tm_core::PlatformInfo {
            self.inner.platform_info()
        }
        fn sample(&mut self) -> Result<tm_core::SystemSnapshot, tm_core::ProbeError> {
            let mut snapshot = self.inner.sample()?;
            snapshot.processes[0].command = format!("tm-agent --token {} --once", self.token);
            snapshot.processes[0].name = format!("tm-agent-{}", self.token);
            Ok(snapshot)
        }
    }

    #[test]
    fn snapshot_error_body_does_not_echo_probe_details() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let mut probe = FailingProbe;
            let warm = AtomicBool::new(true); // the error path never warms up
            let metrics = Metrics::new();
            let active = AtomicUsize::new(0);
            let (mut stream, _) = listener.accept().expect("accept");
            handle_connection(&mut stream, &metrics, |request| {
                route(
                    request,
                    Some(&mut probe),
                    TEST_TOKEN,
                    false,
                    &warm,
                    &metrics,
                    &active,
                )
            })
            .expect("handle connection");
        });
        let response = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        join_demo_server(handle);

        assert!(response.starts_with("HTTP/1.1 500 Internal Server Error"));
        assert!(!response.contains("SECRET-PATH"));
        assert!(!response.contains("/home/victim"));
        let body = parse_body(&response);
        assert_eq!(body["error"], "スナップショットを取得できませんでした");
        assert_eq!(body["code"], "internal");
    }

    #[test]
    fn health_endpoint_is_public() {
        let (address, handle) = spawn_demo_server(1);
        let response = send_request(address, "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        join_demo_server(handle);

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let body = parse_body(&response);
        assert_eq!(body["ok"], true);
        assert_eq!(body["schema_version"], 1);
    }

    #[test]
    fn snapshot_requires_the_bearer_token() {
        let (address, handle) = spawn_demo_server(4);

        let unauthorized = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        );
        assert!(unauthorized.starts_with("HTTP/1.1 401"), "{unauthorized}");
        assert!(unauthorized.contains("WWW-Authenticate: Bearer"));

        let wrong = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer nope\r\n\r\n",
        );
        assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");

        let authorized = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        assert!(authorized.starts_with("HTTP/1.1 200 OK"), "{authorized}");

        // Tokens in the query string are rejected: URLs leak through history,
        // Referer headers and screenshots.
        let via_query = send_request(
            address,
            "GET /api/v1/snapshot?token=test-token HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        );
        assert!(via_query.starts_with("HTTP/1.1 401"), "{via_query}");

        join_demo_server(handle);
    }

    #[test]
    fn foreign_host_headers_are_rejected() {
        let (address, handle) = spawn_demo_server(1);
        let response = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: evil.example\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        join_demo_server(handle);
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    }

    #[test]
    fn forbidden_host_response_carries_cors_only_for_allowed_origins() {
        // The browser UI must be able to read the 403 body; a foreign Origin
        // must still never be reflected.
        let (address, handle) = spawn_demo_server(2);
        let allowed = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: evil.example\r\nOrigin: http://127.0.0.1:5173\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        let foreign = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: evil.example\r\nOrigin: https://evil.example\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        join_demo_server(handle);

        assert!(allowed.starts_with("HTTP/1.1 403"), "{allowed}");
        assert!(
            allowed.contains("Access-Control-Allow-Origin: http://127.0.0.1:5173"),
            "{allowed}"
        );
        assert!(allowed.contains("Vary: Origin"), "{allowed}");

        assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");
        assert!(
            !foreign.contains("Access-Control-Allow-Origin"),
            "{foreign}"
        );
    }

    #[test]
    fn snapshot_body_uses_the_wire_schema() {
        let (address, handle) = spawn_demo_server(1);
        let response = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        join_demo_server(handle);

        let body = parse_body(&response);
        assert_eq!(body["schema_version"], 1);
        let snapshot = &body["snapshot"];
        assert!(snapshot.is_object(), "snapshot must be an object");
        let processes = snapshot["processes"]
            .as_array()
            .expect("processes must be an array");
        assert!(
            !processes.is_empty(),
            "demo snapshot must contain processes"
        );
        assert_eq!(
            body["total_process_count"],
            tm_core::demo::demo_snapshot(0).processes.len()
        );
        assert_eq!(
            processes.len(),
            body["total_process_count"].as_u64().unwrap() as usize
        );
        assert_eq!(snapshot["self_pid"], 4242);
        assert!(snapshot["cpu"]["total_percent"].is_number());
        assert!(snapshot["sampled_at"]["unix_millis"].is_number());
    }

    #[test]
    fn cors_is_limited_to_loopback_origins() {
        let (address, handle) = spawn_demo_server(2);

        let allowed = send_request(
            address,
            "GET /healthz HTTP/1.1\r\nOrigin: http://127.0.0.1:5173\r\n\r\n",
        );
        assert!(allowed.starts_with("HTTP/1.1 200 OK"));
        assert!(
            allowed.contains("Access-Control-Allow-Origin: http://127.0.0.1:5173"),
            "{allowed}"
        );
        assert!(allowed.contains("Vary: Origin"));

        let foreign = send_request(
            address,
            "GET /healthz HTTP/1.1\r\nOrigin: https://evil.example\r\n\r\n",
        );
        assert!(foreign.starts_with("HTTP/1.1 200 OK"));
        assert!(
            !foreign.contains("Access-Control-Allow-Origin"),
            "foreign origins must not receive CORS headers: {foreign}"
        );

        join_demo_server(handle);
    }

    #[test]
    fn preflight_answers_loopback_origins() {
        let (address, handle) = spawn_demo_server(2);

        let allowed = send_request(
            address,
            "OPTIONS /api/v1/snapshot HTTP/1.1\r\nOrigin: http://localhost:5173\r\n\r\n",
        );
        assert!(allowed.starts_with("HTTP/1.1 204 No Content"), "{allowed}");
        assert!(allowed.contains("Access-Control-Allow-Origin: http://localhost:5173"));
        assert!(allowed.contains("Access-Control-Allow-Methods: GET, OPTIONS"));

        let foreign = send_request(
            address,
            "OPTIONS /api/v1/snapshot HTTP/1.1\r\nOrigin: http://evil.example\r\n\r\n",
        );
        assert!(foreign.starts_with("HTTP/1.1 403 Forbidden"), "{foreign}");

        join_demo_server(handle);
    }

    #[test]
    fn rejects_unknown_paths_and_methods() {
        let (address, handle) = spawn_demo_server(2);

        let not_found = send_request(address, "GET /nope HTTP/1.1\r\n\r\n");
        assert!(
            not_found.starts_with("HTTP/1.1 404 Not Found"),
            "{not_found}"
        );

        let method_not_allowed = send_request(address, "POST /api/v1/snapshot HTTP/1.1\r\n\r\n");
        assert!(
            method_not_allowed.starts_with("HTTP/1.1 405 Method Not Allowed"),
            "{method_not_allowed}"
        );
        assert!(method_not_allowed.contains("Allow: GET, OPTIONS"));

        join_demo_server(handle);
    }

    #[test]
    fn error_bodies_keep_the_legacy_error_field_alongside_code() {
        for (code, expected) in [
            (ErrorCode::BadRequest, "bad_request"),
            (ErrorCode::InvalidLimit, "invalid_limit"),
            (ErrorCode::Unauthorized, "unauthorized"),
            (ErrorCode::Forbidden, "forbidden"),
            (ErrorCode::NotFound, "not_found"),
            (ErrorCode::MethodNotAllowed, "method_not_allowed"),
            (ErrorCode::RequestTooLarge, "request_too_large"),
            (ErrorCode::RequestTimeout, "request_timeout"),
            (ErrorCode::Internal, "internal"),
            (ErrorCode::Busy, "busy"),
        ] {
            assert_eq!(code.as_str(), expected);
            let body: serde_json::Value =
                serde_json::from_slice(&error_body(code, "message")).expect("error body");
            assert_eq!(body["code"], expected);
            assert_eq!(body["message"], "message");
            assert_eq!(body["error"], "message", "legacy field must stay");
            assert_eq!(body.as_object().expect("object").len(), 3);
        }
    }

    #[test]
    fn http_errors_carry_machine_readable_codes_and_never_echo_raw_limit() {
        let (address, handle) = spawn_demo_server(6);
        let cases = [
            (
                "GET /api/v1/snapshot?limit=SECRET-RAW HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
                400,
                "invalid_limit",
            ),
            (
                "GET /api/v1/snapshot HTTP/1.1\r\n\r\n",
                401,
                "unauthorized",
            ),
            (
                "GET /api/v1/snapshot HTTP/1.1\r\nHost: evil.example\r\nAuthorization: Bearer test-token\r\n\r\n",
                403,
                "forbidden",
            ),
            (
                "GET /nope HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
                404,
                "not_found",
            ),
            (
                "POST /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
                405,
                "method_not_allowed",
            ),
            (
                "OPTIONS /api/v1/snapshot HTTP/1.1\r\n\r\n",
                403,
                "forbidden",
            ),
        ];
        for (raw, expected_status, expected_code) in cases {
            let response = send_request(address, raw);
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected_status} ")),
                "{response}"
            );
            let body = parse_body(&response);
            assert_eq!(body["code"], expected_code, "{response}");
            assert!(body["error"].is_string(), "{response}");
            assert!(
                !response.contains("SECRET-RAW"),
                "the raw limit value must not be echoed: {response}"
            );
        }
        join_demo_server(handle);
    }

    #[test]
    fn rejects_oversized_request_heads() {
        let (address, handle) = spawn_demo_server(1);
        let mut raw = String::from("GET /healthz HTTP/1.1\r\n");
        for index in 0..600 {
            raw.push_str(&format!("X-Padding-{index}: 0123456789abcdef\r\n"));
        }
        raw.push_str("\r\n");
        let response = send_request(address, &raw);
        join_demo_server(handle);
        assert!(
            response.starts_with("HTTP/1.1 431"),
            "oversized head must be rejected: {}",
            &response[..response.len().min(120)]
        );
    }

    #[test]
    fn empty_connection_is_ignored() {
        let (address, handle) = spawn_demo_server(1);
        let stream = TcpStream::connect(address).expect("connect");
        // Half-close can race with server-side handling on some platforms.
        let _ = stream.shutdown(Shutdown::Write);
        join_demo_server(handle);
    }

    #[test]
    fn once_mode_serialises_a_demo_snapshot() {
        let mut probe = DemoProbe::new();
        let snapshot = probe.sample().expect("demo sample");
        let envelope = SnapshotEnvelope {
            schema_version: tm_core::WIRE_SCHEMA_VERSION,
            total_process_count: snapshot.processes.len(),
            snapshot: &snapshot,
        };
        let json = serde_json::to_vec(&envelope).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_slice(&json).expect("parse");
        assert_eq!(parsed["schema_version"], 1);
        assert!(parsed["snapshot"]["processes"].is_array());
    }

    #[test]
    fn once_mode_scrubs_the_token_with_or_without_redact() {
        // `--once --token <secret>` publishes stdout JSON even without
        // `--redact`; the credential must never appear in that JSON (R45).
        const SECRET: &str = "supersecret-argv-token";
        for redact in [false, true] {
            let mut probe = TokenEchoProbe {
                inner: DemoProbe::new(),
                token: SECRET,
            };
            let json = once_json(&mut probe, redact, SECRET).expect("once json");
            assert!(!json.contains(SECRET), "redact={redact}: {json}");
            assert!(json.contains("***"), "redact={redact}: {json}");
        }
    }

    #[test]
    fn http_snapshot_scrubs_the_token_without_redact() {
        // The HTTP path has the same argv echo as `--once`; a non-redacted
        // response must not carry our own --token command line either.
        const SECRET: &str = "http-argv-secret";
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let mut probe = TokenEchoProbe {
                inner: DemoProbe::new(),
                token: SECRET,
            };
            let warm = AtomicBool::new(true); // demo data needs no warm-up
            let metrics = Metrics::new();
            let active = AtomicUsize::new(0);
            let (mut stream, _) = listener.accept().expect("accept");
            handle_connection(&mut stream, &metrics, |request| {
                route(
                    request,
                    Some(&mut probe),
                    SECRET,
                    false, // no --redact
                    &warm,
                    &metrics,
                    &active,
                )
            })
            .expect("handle connection");
        });
        let response = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer http-argv-secret\r\n\r\n",
        );
        join_demo_server(handle);

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(!response.contains(SECRET), "{response}");
    }

    #[test]
    fn token_argv_warning_states_world_readable_cmdline_and_preferred_sources() {
        assert!(TOKEN_ARGV_WARNING.contains("/proc/<pid>/cmdline"));
        assert!(TOKEN_ARGV_WARNING.contains("同一ホストの全ユーザー"));
        assert!(!TOKEN_ARGV_WARNING.contains("同一ユーザーに読まれ得ます"));
        assert!(TOKEN_ARGV_WARNING.contains("/proc/<pid>/environ"));
        let file = TOKEN_ARGV_WARNING
            .find("--token-file")
            .expect("--token-file must be recommended");
        let env = TOKEN_ARGV_WARNING
            .find("TM_AGENT_TOKEN")
            .expect("TM_AGENT_TOKEN must be mentioned");
        let token = TOKEN_ARGV_WARNING
            .find("> --token ")
            .expect("recommendation order must be present");
        assert!(
            file < env && env < token,
            "expected --token-file > TM_AGENT_TOKEN > --token: {TOKEN_ARGV_WARNING}"
        );
    }

    #[test]
    fn process_limit_defaults_and_clamps() {
        assert_eq!(
            parse_process_limit(None).expect("default limit"),
            DEFAULT_PROCESS_LIMIT
        );
        assert_eq!(
            parse_process_limit(Some("limit=10")).expect("explicit limit"),
            10
        );
        assert_eq!(
            parse_process_limit(Some("foo=1&limit=2000")).expect("max limit"),
            MAX_PROCESS_LIMIT
        );
        assert_eq!(
            parse_process_limit(Some("limit=99999")).expect("clamped limit"),
            MAX_PROCESS_LIMIT
        );
        assert!(parse_process_limit(Some("limit=0")).is_err());
        assert!(parse_process_limit(Some("limit=abc")).is_err());
        assert!(parse_process_limit(Some("limit=")).is_err());
        // Invalid input is never echoed back (it may be a credential).
        for raw in ["limit=SECRET-RAW", "limit=000", "limit=-1"] {
            let error = parse_process_limit(Some(raw)).expect_err("invalid limit");
            assert_eq!(error, INVALID_LIMIT_MESSAGE);
            assert!(!error.contains("SECRET-RAW"), "{error}");
        }
        // A malformed percent escape decodes to None, which is treated the same
        // as an absent `limit` (default) rather than a rejection. Pin this so a
        // future parser change cannot silently alter the response size.
        assert_eq!(
            parse_process_limit(Some("limit=%zz")).expect("malformed escape -> default"),
            DEFAULT_PROCESS_LIMIT
        );
    }

    #[test]
    fn token_file_errors_name_only_the_file() {
        let path = std::path::Path::new("/home/alice/private/token.txt");
        assert_eq!(token_file_name(path), "token.txt");
        assert!(!token_file_name(path).contains("alice"));
        assert_eq!(token_file_name(std::path::Path::new("/")), "(名前なし)");
    }

    #[test]
    fn probe_error_label_never_echoes_the_detail() {
        for error in [
            tm_core::ProbeError::Unsupported("SECRET-PATH /home/victim".to_string()),
            tm_core::ProbeError::Io("SECRET-PATH /home/victim".to_string()),
            tm_core::ProbeError::Permission("SECRET-PATH /home/victim".to_string()),
        ] {
            let label = probe_error_label(&error);
            assert!(!label.contains("SECRET-PATH"), "{label}");
            assert!(!label.contains("/home/victim"), "{label}");
            assert!(!label.is_empty());
        }
        assert_eq!(
            probe_error_label(&tm_core::ProbeError::Io("anything".to_string())),
            "I/O エラー"
        );
    }

    #[test]
    fn token_file_is_trimmed_and_empty_files_are_rejected() {
        let path =
            std::env::temp_dir().join(format!("tm-agent-token-{}-{}", std::process::id(), "read"));
        std::fs::write(&path, "secret-token\n").expect("write token file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("set 0600");
        }
        assert_eq!(read_token_file(&path).expect("read token"), "secret-token");
        std::fs::write(&path, "   \n").expect("write empty token file");
        let error = read_token_file(&path).expect_err("empty token file must fail");
        assert!(error.contains("トークンファイルが空です"), "{error}");
        assert!(
            !error.contains(path.to_string_lossy().as_ref()),
            "the full token file path must not leak: {error}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn token_file_rejects_group_or_other_readability() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("tm-agent-token-{}-mode", std::process::id()));
        std::fs::write(&path, "secret-token\n").expect("write token file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("set 0644");
        let error = read_token_file(&path).expect_err("0644 must be rejected");
        assert!(error.contains("0600"), "{error}");
        assert!(
            !error.contains(path.to_string_lossy().as_ref()),
            "the full token file path must not leak: {error}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Asserts the token-file error contract: the leaf name may be shown to the
    /// operator, but no parent component (home path, mount point, temp dir) may
    /// appear anywhere in the message.
    fn assert_token_error_is_leaf_only(error: &str, path: &std::path::Path) {
        let leaf = path
            .file_name()
            .expect("token file has a name")
            .to_string_lossy();
        assert!(error.contains(leaf.as_ref()), "leaf name missing: {error}");
        assert!(
            !error.contains(path.to_string_lossy().as_ref()),
            "full token path leaked: {error}"
        );
        let parent = path.parent().expect("token file has a parent");
        assert!(
            !error.contains(parent.to_string_lossy().as_ref()),
            "parent directory leaked: {error}"
        );
        let parent_name = parent
            .file_name()
            .expect("parent has a name")
            .to_string_lossy();
        assert!(
            !error.contains(parent_name.as_ref()),
            "parent directory name leaked: {error}"
        );
    }

    #[test]
    fn token_file_io_errors_hide_parent_directories() {
        let dir = std::env::temp_dir().join(format!("tm-agent-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // metadata() failure: the leaf may be printed, the parent may not.
        let missing = dir.join("missing-token.txt");
        let error = read_token_file(&missing).expect_err("missing token file must fail");
        assert!(error.contains("属性"), "{error}");
        assert_token_error_is_leaf_only(&error, &missing);

        // read_to_string() failure caused by invalid UTF-8. Set 0600 first so
        // the Unix permission pre-check does not mask this branch.
        let invalid = dir.join("invalid-utf8-token.txt");
        std::fs::write(&invalid, [0xff, 0xfe, b'A', 0x80]).expect("write invalid UTF-8");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&invalid, std::fs::Permissions::from_mode(0o600))
                .expect("set 0600");
        }
        let error = read_token_file(&invalid).expect_err("invalid UTF-8 must fail");
        assert!(error.contains("読めません"), "{error}");
        assert_token_error_is_leaf_only(&error, &invalid);

        let _ = std::fs::remove_file(&invalid);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn token_file_unreadable_error_hides_parent_directories() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tm-agent-io-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("unreadable-token.txt");
        std::fs::write(&path, "secret-token\n").expect("write token file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).expect("set 0000");

        // Root (or CAP_DAC_OVERRIDE) can still open a 0000 file. Guard the
        // permission-dependent assertion so CI stays deterministic everywhere.
        let enforced = std::fs::File::open(&path).is_err();
        if enforced {
            let error = read_token_file(&path).expect_err("0000 file must be unreadable");
            assert!(error.contains("読めません"), "{error}");
            assert_token_error_is_leaf_only(&error, &path);
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn redaction_scrubs_the_bearer_token_from_argv_echo() {
        // `--token` ends up in our own process command; "redacted" JSON must
        // not carry the credential itself (R45 / R2X-A1).
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        snapshot.processes[0].command = "tm-agent --token supersecret123 --once".to_string();
        snapshot.processes[0].name = "tm-agent-supersecret123".to_string();
        redact_snapshot(&mut snapshot, "supersecret123");
        assert!(!snapshot.processes[0].command.contains("supersecret123"));
        assert!(!snapshot.processes[0].name.contains("supersecret123"));
        assert!(snapshot.processes[0].command.contains("***"));
    }

    #[test]
    fn redaction_removes_host_user_and_home_paths() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/alice".to_string());
        let user = std::env::var("USER").unwrap_or_else(|_| "alice".to_string());
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        snapshot.hostname = Some("my-host".to_string());
        snapshot.processes[0].user = Some(user.clone());
        snapshot.processes[0].command = format!(
            "{home}/private/app --issuer=https://my-host.tail1234.ts.net:8443 --host=my-host"
        );
        snapshot.disks[0].mount_point = format!("{home}/work/project");
        redact_snapshot(&mut snapshot, "");
        assert_eq!(snapshot.hostname.as_deref(), Some("host"));
        assert_eq!(snapshot.processes[0].user.as_deref(), Some("user"));
        assert!(!snapshot.processes[0].command.contains(&home));
        assert!(!snapshot.processes[0].command.contains(&user));
        assert!(!snapshot.processes[0].command.contains("my-host"));
        assert!(!snapshot.processes[0].command.contains("tail1234"));
        assert!(!snapshot.processes[0].command.contains(".ts.net"));
        assert_eq!(snapshot.disks[0].mount_point, "/home/user/work/project");
        // Truncated commands (240 chars) can lose the `.ts.net` suffix.
        snapshot.processes[0].command =
            "app --url=https://host.tail1234.t… --id=tail1234abcd".to_string();
        redact_snapshot(&mut snapshot, "");
        assert!(!snapshot.processes[0].command.contains("tail1234"));
        assert!(snapshot.processes[0].command.contains("host"));
    }

    #[test]
    fn process_limit_sorts_by_cpu_then_pid_and_reports_the_total() {
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        let total = snapshot.processes.len();
        let returned = apply_process_limit(&mut snapshot, 5);

        assert_eq!(returned, total);
        assert_eq!(snapshot.processes.len(), 5);
        for window in snapshot.processes.windows(2) {
            let (left, right) = (&window[0], &window[1]);
            assert!(
                left.cpu_percent > right.cpu_percent
                    || (left.cpu_percent == right.cpu_percent && left.key.pid <= right.key.pid),
                "processes must be sorted by CPU desc then PID asc"
            );
        }
    }

    #[test]
    fn process_limit_treats_negative_cpu_as_zero_when_truncating() {
        let mut snapshot = tm_core::demo::demo_snapshot(0);
        snapshot.processes.truncate(3);
        snapshot.processes[0].cpu_percent = -5.0;
        snapshot.processes[1].cpu_percent = 0.0;
        snapshot.processes[2].cpu_percent = 30.0;

        // Truncate to one row: before normalising, the -5.0 row sorts first
        // (total_cmp) and would be kept instead of the real 30% process.
        apply_process_limit(&mut snapshot, 1);
        assert_eq!(snapshot.processes[0].cpu_percent, 30.0);
    }

    #[test]
    fn snapshot_limit_truncates_and_reports_the_total() {
        let (address, handle) = spawn_demo_server(3);

        let limited = send_request(
            address,
            "GET /api/v1/snapshot?limit=3 HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        assert!(limited.starts_with("HTTP/1.1 200 OK"), "{limited}");
        let body = parse_body(&limited);
        let total = tm_core::demo::demo_snapshot(0).processes.len() as u64;
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["total_process_count"], total);
        assert_eq!(
            body["snapshot"]["processes"]
                .as_array()
                .expect("processes")
                .len(),
            3
        );

        let default_limit = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        let body = parse_body(&default_limit);
        assert_eq!(
            body["snapshot"]["processes"]
                .as_array()
                .expect("processes")
                .len() as u64,
            total,
            "the demo process count is below the default limit"
        );

        let invalid = send_request(
            address,
            "GET /api/v1/snapshot?limit=abc HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        assert!(invalid.starts_with("HTTP/1.1 400 Bad Request"), "{invalid}");

        join_demo_server(handle);
    }

    /// Accepts one connection from `listener` and hands it to the production
    /// dispatcher, so tests exercise the same shared-state path as `serve()`.
    fn accept_and_spawn(listener: &TcpListener, state: &Arc<SharedState>, workers: &JoinTracker) {
        let (stream, _) = listener.accept().expect("accept");
        spawn_connection(Arc::clone(state), stream, workers).expect("spawn connection");
    }

    struct CountingProbe {
        inner: DemoProbe,
        calls: Arc<AtomicUsize>,
    }

    impl SystemProbe for CountingProbe {
        fn platform_info(&self) -> tm_core::PlatformInfo {
            self.inner.platform_info()
        }
        fn sample(&mut self) -> Result<tm_core::SystemSnapshot, tm_core::ProbeError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.inner.sample()
        }
    }

    #[test]
    fn first_authenticated_snapshot_consumes_one_warmup_sample() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(SharedState::new(
            Box::new(CountingProbe {
                inner: DemoProbe::new(),
                calls: Arc::clone(&calls),
            }),
            TEST_TOKEN,
            false,
            true, // simulate a real probe needing warm-up
        ));
        // One accept per request, synchronously driven by the test so there is
        // no race between `send_request` and the listener accepting.
        let request = |raw: &str, state: &Arc<SharedState>| -> String {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
            let address = listener.local_addr().expect("local addr");
            let workers = JoinTracker::new();
            let server = std::thread::spawn({
                let state = Arc::clone(state);
                let workers = workers.clone();
                move || accept_and_spawn(&listener, &state, &workers)
            });
            let _ = address;
            let response = send_request(address, raw);
            join_demo_server(server);
            let outcome = workers.join_all(TEST_JOIN_TIMEOUT);
            assert!(
                !outcome.timed_out && outcome.panicked == 0,
                "connection worker did not exit cleanly: {outcome:?}"
            );
            assert_eq!(outcome.joined, 1);
            response
        };

        // Unauthorized, public health check and bad limit must NOT consume
        // warm-up. `/healthz` legitimately returns 200 without a token.
        let health = request("GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", &state);
        assert!(health.starts_with("HTTP/1.1 200"));
        for raw in [
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "GET /api/v1/snapshot?limit=abc HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        ] {
            let response = request(raw, &state);
            assert!(!response.starts_with("HTTP/1.1 200"), "unexpected: {response}");
        }
        assert_eq!(calls.load(Ordering::Acquire), 0);

        let auth = "GET /api/v1/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer test-token\r\n\r\n";
        assert!(request(auth, &state).starts_with("HTTP/1.1 200"));
        assert_eq!(calls.load(Ordering::Acquire), 2); // discarded + published
        assert!(request(auth, &state).starts_with("HTTP/1.1 200"));
        assert_eq!(calls.load(Ordering::Acquire), 3); // one per later request
    }

    #[test]
    fn concurrent_authenticated_requests_share_one_warmup_sample() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(SharedState::new(
            Box::new(CountingProbe {
                inner: DemoProbe::new(),
                calls: Arc::clone(&calls),
            }),
            TEST_TOKEN,
            false,
            true, // real probe: warm-up required
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let workers = JoinTracker::new();

        // Both peers send a complete authenticated request before the server
        // accepts either one, so the two workers genuinely overlap. The probe
        // mutex serialises routing, which must make the warm-up check one-shot.
        let mut clients = Vec::with_capacity(2);
        for _ in 0..2 {
            let mut client = TcpStream::connect(address).expect("connect");
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");
            client
                .write_all(b"GET /api/v1/snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer test-token\r\n\r\n")
                .expect("write request");
            let _ = client.shutdown(Shutdown::Write);
            clients.push(client);
        }
        for _ in 0..clients.len() {
            accept_and_spawn(&listener, &state, &workers);
        }
        for mut client in clients {
            let mut response = String::new();
            client.read_to_string(&mut response).expect("read response");
            assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        }
        let outcome = workers.join_all(TEST_JOIN_TIMEOUT);
        assert!(!outcome.timed_out && outcome.panicked == 0, "{outcome:?}");
        assert_eq!(outcome.joined, 2);

        // One discarded warm-up sample plus one published sample per request.
        assert_eq!(calls.load(Ordering::Acquire), 3);
    }

    #[test]
    fn active_counter_is_released_when_worker_panics() {
        // Exercises the Drop guard: a panic inside the worker must not leak a
        // connection slot (otherwise 8 panics would make the agent return 503
        // forever).
        let state = Arc::new(SharedState::new(
            Box::new(DemoProbe::new()),
            TEST_TOKEN,
            false,
            false, // demo data needs no warm-up
        ));
        assert_eq!(state.active.load(Ordering::Acquire), 0);
        let worker = {
            // Mirrors `spawn_connection`: increment on admission, then move the
            // guard into the worker so it decrements even on unwind.
            state.active.fetch_add(1, Ordering::AcqRel);
            let _slot = ActiveConnection(Arc::clone(&state));
            std::thread::spawn(move || {
                let _slot = _slot;
                panic!("simulated probe failure");
            })
        };
        assert!(worker.join().is_err());
        assert_eq!(state.active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn idle_connections_do_not_block_health_check() {
        let state = Arc::new(SharedState::new(
            Box::new(DemoProbe::new()),
            TEST_TOKEN,
            false,
            false, // demo data needs no warm-up
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let workers = JoinTracker::new();

        // Idle peers that connect but never send a request head. Under the old
        // serial server each one stalled the loop for READ_TIMEOUT (1 s), so
        // /healthz took about 2 s behind two of them.
        let idle = [
            TcpStream::connect(address).expect("connect idle 1"),
            TcpStream::connect(address).expect("connect idle 2"),
        ];
        for _ in 0..idle.len() {
            accept_and_spawn(&listener, &state, &workers);
        }

        let start = Instant::now();
        let mut client = TcpStream::connect(address).expect("connect health client");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        client
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("write request");
        accept_and_spawn(&listener, &state, &workers);
        let mut response = String::new();
        client.read_to_string(&mut response).expect("read response");
        let elapsed = start.elapsed();

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(
            elapsed < Duration::from_millis(800),
            "health check waited {elapsed:?} behind idle peers"
        );
        let expected_workers = idle.len() + 1;
        assert_eq!(workers.tracked_count(), expected_workers);
        drop(idle);
        let outcome = workers.join_all(TEST_JOIN_TIMEOUT);
        assert!(!outcome.timed_out && outcome.panicked == 0, "{outcome:?}");
        assert_eq!(outcome.joined, expected_workers);
    }

    #[test]
    fn excess_connections_are_rejected_immediately() {
        let state = Arc::new(SharedState::new(
            Box::new(DemoProbe::new()),
            TEST_TOKEN,
            false,
            false, // demo data needs no warm-up
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let workers = JoinTracker::new();

        // Hold every slot with a partial request head. A peer that keeps
        // sending bytes (without the final CRLF) never hits READ_TIMEOUT, so
        // the counter cannot drop below MAX_CONNECTIONS mid-test — no reliance
        // on finishing the whole loop within one second.
        let mut idle = Vec::with_capacity(MAX_CONNECTIONS);
        let mut keepers = Vec::with_capacity(MAX_CONNECTIONS);
        for _ in 0..MAX_CONNECTIONS {
            let peer = TcpStream::connect(address).expect("connect idle");
            let mut keeper = peer.try_clone().expect("clone idle peer");
            keepers.push(std::thread::spawn(move || {
                // An incomplete head keeps `read_request` waiting for "\r\n\r\n".
                // Send it once, then refresh only to stay under READ_TIMEOUT:
                // the bytes must not accumulate past MAX_HEADER_BYTES (8 KiB).
                if keeper
                    .write_all(b"GET /healthz HTTP/1.1\r\nHost: x")
                    .is_err()
                {
                    return;
                }
                std::thread::sleep(Duration::from_millis(300));
                let _ = keeper.write_all(b"\r\nX: keepalive");
                std::thread::sleep(Duration::from_millis(300));
                let _ = keeper.shutdown(Shutdown::Write);
            }));
            idle.push(peer);
            accept_and_spawn(&listener, &state, &workers);
        }
        assert_eq!(state.active.load(Ordering::Acquire), MAX_CONNECTIONS);
        assert_eq!(workers.tracked_count(), MAX_CONNECTIONS);

        let mut client = TcpStream::connect(address).expect("connect extra client");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        client
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("write request");
        accept_and_spawn(&listener, &state, &workers);
        let mut response = String::new();
        client.read_to_string(&mut response).expect("read response");

        assert!(
            response.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{response}"
        );
        assert_eq!(state.active.load(Ordering::Acquire), MAX_CONNECTIONS);
        // The 503 path is synchronous on the accept thread: it must not add a
        // worker handle, or repeated rejections would grow the tracker forever.
        assert_eq!(workers.tracked_count(), MAX_CONNECTIONS);
        drop(idle);
        for keeper in keepers {
            keeper.join().expect("keeper thread");
        }
        let outcome = workers.join_all(TEST_JOIN_TIMEOUT);
        assert!(!outcome.timed_out && outcome.panicked == 0, "{outcome:?}");
        assert_eq!(outcome.joined, MAX_CONNECTIONS);
    }

    #[test]
    fn metrics_endpoint_is_public_and_never_leaks_the_token() {
        let (address, handle) = spawn_demo_server(2);
        // A first request must be counted before /metrics is rendered.
        let health = send_request(address, "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        assert!(health.starts_with("HTTP/1.1 200 OK"), "{health}");

        let metrics = send_request(address, "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        join_demo_server(handle);

        assert!(metrics.starts_with("HTTP/1.1 200 OK"), "{metrics}");
        let (head, body) = metrics.split_once("\r\n\r\n").expect("body separator");
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: text/plain"),
            "{head}"
        );
        assert!(body.contains("tm_agent_requests_total 1"), "{body}");
        assert!(
            body.contains("tm_agent_responses_total{class=\"2xx\"} 1"),
            "{body}"
        );
        assert!(
            body.contains("# TYPE tm_agent_active_connections gauge"),
            "{body}"
        );
        assert!(
            !body.contains(TEST_TOKEN),
            "metrics must never expose the bearer token"
        );
        assert!(
            !body.contains("kubuntu-demo"),
            "metrics must never expose machine data"
        );
    }

    #[test]
    fn redacted_snapshot_http_response_keeps_the_wire_envelope() {
        let (address, handle) = spawn_demo_server_with_redaction(1, true);
        let response = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        join_demo_server(handle);

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let body = parse_body(&response);
        // These envelope keys are an existing contract: adding telemetry must
        // not change them.
        assert_eq!(body["schema_version"], json!(tm_core::WIRE_SCHEMA_VERSION));
        assert!(body["total_process_count"].is_number());
        assert!(body["snapshot"]["processes"].is_array());
        assert_eq!(body["snapshot"]["hostname"], "host");
        assert!(!response.contains("kubuntu-demo"));
    }

    #[test]
    fn log_level_filters_structured_events_without_changing_levels() {
        // Default keeps every existing event (no behaviour change by default).
        assert!(level_allowed(Level::Info, args::LogLevel::Info));
        assert!(level_allowed(Level::Warn, args::LogLevel::Info));
        assert!(level_allowed(Level::Error, args::LogLevel::Info));

        assert!(level_allowed(Level::Error, args::LogLevel::Error));
        assert!(!level_allowed(Level::Warn, args::LogLevel::Error));
        assert!(!level_allowed(Level::Info, args::LogLevel::Error));

        assert!(level_allowed(Level::Warn, args::LogLevel::Warn));
        assert!(!level_allowed(Level::Info, args::LogLevel::Warn));

        // The agent emits no Debug/Trace events, so those settings behave like
        // info; error-level exit telemetry is never suppressed.
        for minimum in [args::LogLevel::Debug, args::LogLevel::Trace] {
            assert!(level_allowed(Level::Info, minimum));
            assert!(level_allowed(Level::Error, minimum));
        }
        assert_eq!(
            LOG_THRESHOLD.load(Ordering::Relaxed),
            threshold_rank(args::LogLevel::Info)
        );
    }

    #[test]
    fn join_tracker_empty_returns_immediately() {
        let tracker = JoinTracker::new();
        let start = Instant::now();
        let outcome = tracker.join_all(TEST_JOIN_TIMEOUT);
        assert_eq!(outcome, JoinOutcome::default());
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "an empty tracker must not wait for the deadline"
        );
    }

    #[test]
    fn join_tracker_joins_finished_workers_and_counts_panics() {
        let tracker = JoinTracker::new();
        // Keep every worker alive until all four are registered; otherwise the
        // quick ones are pruned by `track` before the final assertion.
        let release = Arc::new(AtomicBool::new(false));
        for _ in 0..3 {
            let flag = Arc::clone(&release);
            tracker.track(std::thread::spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }));
        }
        let flag = Arc::clone(&release);
        tracker.track(std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            panic!("simulated worker failure");
        }));
        assert_eq!(tracker.tracked_count(), 4);

        release.store(true, Ordering::Release);
        let outcome = tracker.join_all(TEST_JOIN_TIMEOUT);
        assert_eq!(
            outcome,
            JoinOutcome {
                joined: 4,
                panicked: 1,
                timed_out: false,
            }
        );
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn join_tracker_timeout_is_bounded_and_retryable() {
        let tracker = JoinTracker::new();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        tracker.track(std::thread::spawn(move || {
            let _ = release_rx.recv();
        }));

        let start = Instant::now();
        let outcome = tracker.join_all(Duration::from_millis(120));
        let elapsed = start.elapsed();
        assert!(outcome.timed_out, "a blocked worker must hit the deadline");
        assert_eq!(outcome.joined, 0);
        assert!(
            elapsed < Duration::from_secs(1),
            "join_all waited {elapsed:?} despite a 120 ms deadline"
        );
        assert_eq!(
            tracker.tracked_count(),
            1,
            "a worker still running on timeout must stay tracked"
        );

        release_tx.send(()).expect("release worker");
        let outcome = tracker.join_all(TEST_JOIN_TIMEOUT);
        assert_eq!(
            outcome,
            JoinOutcome {
                joined: 1,
                panicked: 0,
                timed_out: false,
            }
        );
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn join_tracker_prunes_finished_handles_on_track() {
        let tracker = JoinTracker::new();
        let finished = std::thread::spawn(|| {});
        while !finished.is_finished() {
            std::thread::yield_now();
        }
        tracker.track(finished);
        assert_eq!(tracker.tracked_count(), 1);

        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let live = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        tracker.track(live);
        assert_eq!(
            tracker.tracked_count(),
            1,
            "registering a new worker must prune the already-finished one"
        );

        release_tx.send(()).expect("release worker");
        let outcome = tracker.join_all(TEST_JOIN_TIMEOUT);
        assert_eq!(
            outcome,
            JoinOutcome {
                joined: 1,
                panicked: 0,
                timed_out: false,
            }
        );
    }
}
