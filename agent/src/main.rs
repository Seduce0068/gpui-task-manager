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

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tm_core::demo::DemoProbe;
use tm_core::model::SystemSnapshot;
use tm_core::port::SystemProbe;
use tm_platform::NativeProbe;

/// Health endpoint (no authentication, reveals nothing sensitive).
const HEALTH_PATH: &str = "/healthz";
/// Snapshot endpoint (bearer token required).
const SNAPSHOT_PATH: &str = "/api/v1/snapshot";

/// Number of processes returned when the client does not ask for a limit.
///
/// A desktop can easily run 3,000+ processes; sending all of them once per
/// second would produce multi-megabyte responses in the browser. The UI only
/// renders a table page, so the agent truncates by default and reports the
/// full count alongside the list.
const DEFAULT_PROCESS_LIMIT: usize = 500;

/// Hard upper bound for `?limit=` (a larger value is clamped, not rejected).
const MAX_PROCESS_LIMIT: usize = 2000;

/// Maximum concurrent connections. The extra ones get an immediate 503 instead
/// of waiting behind idle peers; the agent is loopback-only, but an unbounded
/// thread-per-connection server would still be a local resource exhaustion vector.
const MAX_CONNECTIONS: usize = 8;

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

/// Wire body returned for error responses.
#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

fn main() {
    if let Err(message) = run() {
        eprintln!("tm-agent: {message}");
        std::process::exit(1);
    }
}

/// Runs the agent and returns a human-readable error on failure.
fn run() -> Result<(), String> {
    let config = args::parse(std::env::args().skip(1))?;
    if config.help {
        print!("{}", args::USAGE);
        return Ok(());
    }

    let mut probe = build_probe(&config)?;

    if config.once {
        return print_once(probe.as_mut(), config.redact);
    }

    let token = match (config.token, config.token_file.as_deref()) {
        (Some(_), Some(_)) => {
            return Err("--token と --token-file は同時に指定できません".to_string());
        }
        (Some(token), None) => {
            eprintln!(
                "tm-agent: 警告: --token の値は /proc/<pid>/cmdline から同一ユーザーに読まれ得ます。常用時は --token-file か TM_AGENT_TOKEN を使ってください。"
            );
            // `request_token` trims the presented value, so store the trimmed
            // token for every source (the file path already does this).
            token.trim().to_string()
        }
        (None, Some(path)) => read_token_file(path)?,
        (None, None) => match std::env::var("TM_AGENT_TOKEN") {
            Ok(token) if !token.trim().is_empty() => token.trim().to_string(),
            _ => {
                let token = auth::generate_token();
                // The operator needs the token to talk to the agent; this is the
                // only place it is written to the log.
                eprintln!("tm-agent: 認証トークン (生成): {token}");
                token
            }
        },
    };

    let listener = TcpListener::bind(config.listen)
        .map_err(|err| format!("{} を bind できません: {err}", config.listen))?;
    let local = listener
        .local_addr()
        .map_err(|err| format!("待ち受けアドレスを取得できません: {err}"))?;
    eprintln!(
        "tm-agent: http://{local} で待ち受けます (読み取り専用, demo={}, schema_version={})",
        config.demo,
        tm_core::WIRE_SCHEMA_VERSION
    );
    serve(&listener, probe, &token, config.redact)
}

/// Reads a bearer token from a file, trimming the trailing newline.
///
/// Preferred over `--token`: file contents are not exposed through
/// `/proc/<pid>/cmdline`. Empty files are rejected so a misconfiguration cannot
/// start an agent with an empty token.
fn read_token_file(path: &std::path::Path) -> Result<String, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|err| format!("トークンファイルを読めません ({}): {err}", path.display()))?;
    let token = contents.trim();
    if token.is_empty() {
        return Err(format!("トークンファイルが空です: {}", path.display()));
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
            .map_err(|err| err.to_string())
    }
}

/// Implements `--once`: one snapshot as JSON on stdout, then exit.
fn print_once(probe: &mut dyn SystemProbe, redact: bool) -> Result<(), String> {
    // CPU percentages are deltas to the previous sample, so the first sample
    // is a warm-up and is not printed.
    probe.sample().map_err(|err| err.to_string())?;
    std::thread::sleep(Duration::from_millis(250));
    let mut snapshot = probe.sample().map_err(|err| err.to_string())?;
    if redact {
        redact_snapshot(&mut snapshot);
    }
    let total = apply_process_limit(&mut snapshot, DEFAULT_PROCESS_LIMIT);
    let envelope = SnapshotEnvelope {
        schema_version: tm_core::WIRE_SCHEMA_VERSION,
        total_process_count: total,
        snapshot: &snapshot,
    };
    let json = serde_json::to_string(&envelope)
        .map_err(|err| format!("JSON への変換に失敗しました: {err}"))?;
    println!("{json}");
    Ok(())
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
}

impl SharedState {
    fn new(probe: Box<dyn SystemProbe>, token: &str, redact: bool) -> Self {
        Self {
            probe: Mutex::new(probe),
            token: token.to_string(),
            redact,
            active: AtomicUsize::new(0),
        }
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
) -> Result<(), String> {
    let state = Arc::new(SharedState::new(probe, token, redact));
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => spawn_connection(Arc::clone(&state), stream),
            Err(err) => eprintln!("tm-agent: accept に失敗: {err}"),
        }
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
fn spawn_connection(state: Arc<SharedState>, mut stream: TcpStream) {
    let previous = state.active.fetch_add(1, Ordering::AcqRel);
    if previous >= MAX_CONNECTIONS {
        // Drop guard returns the slot even if writing the rejection panics.
        let _slot = ActiveConnection(state);
        let response = http::Response::json(
            503,
            "Service Unavailable",
            error_body("同時接続数が上限に達しました"),
        );
        // The 503 path bypasses `handle_connection`, so set the timeout here:
        // a peer that stops reading must not block the accept loop.
        if let Err(err) = stream.set_write_timeout(Some(http::WRITE_TIMEOUT)) {
            eprintln!("tm-agent: 503 のタイムアウト設定に失敗: {err}");
        }
        if let Err(err) = http::write_response(&mut stream, &response) {
            eprintln!("tm-agent: 503 応答に失敗: {err}");
        }
        return;
    }

    let _slot = ActiveConnection(Arc::clone(&state));
    std::thread::spawn(move || {
        // `_slot` is moved into the worker so the counter is released when the
        // worker exits — or unwinds after a panic — never on the accept thread.
        let _slot = _slot;
        let result = handle_connection(&mut stream, |request| {
            // Rust の Mutex はパニックで poison する可能性があるが、プローブ側の
            // 致命的失敗でエージェント全体を止めないよう guard を回収して続行する。
            // 回収した場合はログに残し、無ログの fail-open にならないようにする。
            let mut guard = match state.probe.lock() {
                Ok(guard) => guard,
                Err(err) => {
                    eprintln!("tm-agent: プローブの mutex が poison しました; 回収して続行します");
                    err.into_inner()
                }
            };
            route(request, guard.as_mut(), &state.token, state.redact)
        });
        if let Err(err) = result {
            eprintln!("tm-agent: 接続処理に失敗: {err}");
        }
    });
}

/// Handles exactly one HTTP request on `stream`.
///
/// The caller supplies `route` so the shared probe is locked only after a
/// complete request head has been read.
fn handle_connection(
    stream: &mut TcpStream,
    mut route: impl FnMut(&http::Request) -> http::Response,
) -> io::Result<()> {
    stream.set_read_timeout(Some(http::READ_TIMEOUT))?;
    stream.set_write_timeout(Some(http::WRITE_TIMEOUT))?;

    let request = match http::read_request(stream) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(http::ParseError::TooLarge) => {
            let response = http::Response::json(
                431,
                "Request Header Fields Too Large",
                error_body("リクエストヘッダが大きすぎます"),
            );
            return http::write_response(stream, &response);
        }
        Err(http::ParseError::Malformed) => {
            let response =
                http::Response::json(400, "Bad Request", error_body("リクエストを解釈できません"));
            return http::write_response(stream, &response);
        }
    };

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
                error_body("Host ヘッダがループバックではありません"),
            );
            if let Some(origin) = allowed_origin {
                http::add_cors_headers(&mut response, origin);
            }
            return http::write_response(stream, &response);
        }
    }

    let mut response = route(&request);
    if let Some(origin) = allowed_origin {
        http::add_cors_headers(&mut response, origin);
    }
    http::write_response(stream, &response)
}

/// Routes a parsed request to its handler.
fn route(
    request: &http::Request,
    probe: &mut dyn SystemProbe,
    token: &str,
    redact: bool,
) -> http::Response {
    if request.method.eq_ignore_ascii_case("OPTIONS") {
        return preflight_response(request);
    }
    if !request.method.eq_ignore_ascii_case("GET") {
        let mut response = http::Response::json(
            405,
            "Method Not Allowed",
            error_body("このエージェントは GET のみ対応しています"),
        );
        response
            .extra_headers
            .push(("Allow".to_string(), "GET, OPTIONS".to_string()));
        return response;
    }

    match request.path.as_str() {
        HEALTH_PATH => health_response(),
        SNAPSHOT_PATH => snapshot_response(request, probe, token, redact),
        _ => http::Response::json(
            404,
            "Not Found",
            error_body("エンドポイントが見つかりません"),
        ),
    }
}

/// Answers a CORS preflight request.
///
/// Preflights carry no credentials, so they are answered without a token; the
/// actual `GET` still needs one.
fn preflight_response(request: &http::Request) -> http::Response {
    let Some(origin) = request.header("origin") else {
        return http::Response::json(403, "Forbidden", error_body("Origin ヘッダが必要です"));
    };
    if !http::is_allowed_origin(origin) {
        return http::Response::json(403, "Forbidden", error_body("許可されていない Origin です"));
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

/// `/api/v1/snapshot`: authenticated, read-only snapshot.
fn snapshot_response(
    request: &http::Request,
    probe: &mut dyn SystemProbe,
    token: &str,
    redact: bool,
) -> http::Response {
    let Some(supplied) = auth::request_token(request) else {
        return unauthorized_response();
    };
    if !auth::constant_time_eq(token, &supplied) {
        return unauthorized_response();
    }

    let limit = match parse_process_limit(request.query.as_deref()) {
        Ok(limit) => limit,
        Err(message) => return http::Response::json(400, "Bad Request", error_body(&message)),
    };

    match probe.sample() {
        Ok(mut snapshot) => {
            if redact {
                redact_snapshot(&mut snapshot);
            }
            let total = apply_process_limit(&mut snapshot, limit);
            let envelope = SnapshotEnvelope {
                schema_version: tm_core::WIRE_SCHEMA_VERSION,
                total_process_count: total,
                snapshot: &snapshot,
            };
            json_response(200, "OK", &envelope)
        }
        Err(err) => json_response(
            500,
            "Internal Server Error",
            &ErrorBody {
                error: &err.to_string(),
            },
        ),
    }
}

/// Removes local identifiers from a snapshot before it is serialised.
///
/// Used by `--redact` so screenshots and JSON evidence can be published without
/// exposing the host name, user names or home directory layout.
fn redact_snapshot(snapshot: &mut tm_core::SystemSnapshot) {
    let original_hostname = snapshot.hostname.clone().unwrap_or_default();
    if snapshot.hostname.is_some() {
        snapshot.hostname = Some("host".to_string());
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    // Host names also appear inside command lines (for example OIDC issuer
    // URLs, `--trusted-host` flags or avahi's advertised name). Replace the
    // full host name, its first label and any `*.ts.net` style tailnet domain
    // so `--redact` really removes the machine identity.
    let short_host = original_hostname
        .split(['.', '-'])
        .next()
        .unwrap_or("")
        .to_string();
    let redact_text = |value: &str| {
        let mut out = value.to_string();
        if !home.is_empty() {
            out = out.replace(&home, "/home/user");
        }
        if !user.is_empty() {
            out = out.replace(&user, "user");
        }
        if !original_hostname.is_empty() {
            out = out.replace(&original_hostname, "host");
        }
        if short_host.len() >= 3 {
            out = out.replace(&short_host, "host");
        }
        out = redact_tailnet_domains(&out);
        out = redact_host_subdomains(&out);
        redact_tailnet_tokens(&out)
    };
    for process in &mut snapshot.processes {
        if process.user.is_some() {
            process.user = Some("user".to_string());
        }
        process.command = redact_text(&process.command);
        process.name = redact_text(&process.name);
    }
}

/// Replaces `host.tailXXXX.ts.net` style domains with `host`.
///
/// Commands are truncated to 240 characters before they reach the agent, so a
/// full `.ts.net` suffix may be missing; [`redact_host_subdomains`] and
/// [`redact_tailnet_tokens`] cover those fragments as well.
fn redact_tailnet_domains(value: &str) -> String {
    const SUFFIX: &str = ".ts.net";
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(position) = rest.find(SUFFIX) {
        let before = &rest[..position];
        let start = before
            .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
            .map(|index| index + 1)
            .unwrap_or(0);
        out.push_str(&before[..start]);
        out.push_str("host");
        rest = &rest[position + SUFFIX.len()..];
    }
    out.push_str(rest);
    out
}

/// Replaces `host.<tailnet-id>` style suffixes with `host`.
///
/// This catches truncated commands such as `https://host.tail1234.t…` where the
/// `.ts.net` suffix was cut off by the 240-character command limit.
fn redact_host_subdomains(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(position) = rest.find("host.") {
        out.push_str(&rest[..position]);
        out.push_str("host");
        let suffix = &rest[position + "host.".len()..];
        let consumed = suffix
            .char_indices()
            .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '.'))
            .map(|(index, _)| index)
            .unwrap_or(suffix.len());
        rest = &suffix[consumed..];
    }
    out.push_str(rest);
    out
}

/// Replaces bare `tail` + hex identifiers (tailnet IDs) with `tailnet`.
fn redact_tailnet_tokens(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(position) = rest.find("tail") {
        out.push_str(&rest[..position]);
        let candidate = &rest[position + "tail".len()..];
        let hex_len = candidate
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .count();
        if hex_len >= 6 {
            out.push_str("tailnet");
            rest = &candidate[hex_len..];
        } else {
            out.push_str("tail");
            rest = candidate;
        }
    }
    out.push_str(rest);
    out
}

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
        .map_err(|_| format!("limit には 1 以上の整数を指定してください: {raw}"))?;
    if parsed == 0 {
        return Err("limit には 1 以上の整数を指定してください: 0".to_string());
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
        right
            .cpu_percent
            .total_cmp(&left.cpu_percent)
            .then_with(|| left.key.pid.cmp(&right.key.pid))
    });
    snapshot.processes.truncate(limit);
    total
}

/// `401` with the standard bearer challenge.
fn unauthorized_response() -> http::Response {
    let mut response =
        http::Response::json(401, "Unauthorized", error_body("認証トークンが必要です"));
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
            http::Response::json(
                500,
                "Internal Server Error",
                error_body("JSON への変換に失敗しました"),
            )
        }
    }
}

/// Builds an `{"error": "..."}` body.
fn error_body(message: &str) -> Vec<u8> {
    serde_json::to_vec(&ErrorBody { error: message })
        .unwrap_or_else(|_| b"{\"error\":\"internal error\"}".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{Shutdown, SocketAddr};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    const TEST_TOKEN: &str = "test-token";

    /// Starts a demo-mode server that accepts exactly `connections` requests.
    fn spawn_demo_server(connections: usize) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let mut probe = DemoProbe::new();
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().expect("accept");
                handle_connection(&mut stream, |request| {
                    route(request, &mut probe, TEST_TOKEN, false)
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

    #[test]
    fn health_endpoint_is_public() {
        let (address, handle) = spawn_demo_server(1);
        let response = send_request(address, "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        handle.join().expect("server thread");

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

        handle.join().expect("server thread");
    }

    #[test]
    fn foreign_host_headers_are_rejected() {
        let (address, handle) = spawn_demo_server(1);
        let response = send_request(
            address,
            "GET /api/v1/snapshot HTTP/1.1\r\nHost: evil.example\r\nAuthorization: Bearer test-token\r\n\r\n",
        );
        handle.join().expect("server thread");
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
        handle.join().expect("server thread");

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
        handle.join().expect("server thread");

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

        handle.join().expect("server thread");
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

        handle.join().expect("server thread");
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

        handle.join().expect("server thread");
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
        handle.join().expect("server thread");
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
        handle.join().expect("server thread");
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
        // A malformed percent escape decodes to None, which is treated the same
        // as an absent `limit` (default) rather than a rejection. Pin this so a
        // future parser change cannot silently alter the response size.
        assert_eq!(
            parse_process_limit(Some("limit=%zz")).expect("malformed escape -> default"),
            DEFAULT_PROCESS_LIMIT
        );
    }

    #[test]
    fn token_file_is_trimmed_and_empty_files_are_rejected() {
        let path =
            std::env::temp_dir().join(format!("tm-agent-token-{}-{}", std::process::id(), "read"));
        std::fs::write(&path, "secret-token\n").expect("write token file");
        assert_eq!(read_token_file(&path).expect("read token"), "secret-token");
        std::fs::write(&path, "   \n").expect("write empty token file");
        assert!(read_token_file(&path).is_err());
        let _ = std::fs::remove_file(&path);
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
        redact_snapshot(&mut snapshot);
        assert_eq!(snapshot.hostname.as_deref(), Some("host"));
        assert_eq!(snapshot.processes[0].user.as_deref(), Some("user"));
        assert!(!snapshot.processes[0].command.contains(&home));
        assert!(!snapshot.processes[0].command.contains(&user));
        assert!(!snapshot.processes[0].command.contains("my-host"));
        assert!(!snapshot.processes[0].command.contains("tail1234"));
        assert!(!snapshot.processes[0].command.contains(".ts.net"));
        // Truncated commands (240 chars) can lose the `.ts.net` suffix.
        snapshot.processes[0].command =
            "app --url=https://host.tail1234.t… --id=tail1234abcd".to_string();
        redact_snapshot(&mut snapshot);
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

        handle.join().expect("server thread");
    }

    /// Accepts one connection from `listener` and hands it to the production
    /// dispatcher, so tests exercise the same shared-state path as `serve()`.
    fn accept_and_spawn(listener: &TcpListener, state: &Arc<SharedState>) {
        let (stream, _) = listener.accept().expect("accept");
        spawn_connection(Arc::clone(state), stream);
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
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");

        // Idle peers that connect but never send a request head. Under the old
        // serial server each one stalled the loop for READ_TIMEOUT (1 s), so
        // /healthz took about 2 s behind two of them.
        let idle = [
            TcpStream::connect(address).expect("connect idle 1"),
            TcpStream::connect(address).expect("connect idle 2"),
        ];
        for _ in 0..idle.len() {
            accept_and_spawn(&listener, &state);
        }

        let start = Instant::now();
        let mut client = TcpStream::connect(address).expect("connect health client");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        client
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("write request");
        accept_and_spawn(&listener, &state);
        let mut response = String::new();
        client.read_to_string(&mut response).expect("read response");
        let elapsed = start.elapsed();

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(
            elapsed < Duration::from_millis(800),
            "health check waited {elapsed:?} behind idle peers"
        );
        drop(idle);
    }

    #[test]
    fn excess_connections_are_rejected_immediately() {
        let state = Arc::new(SharedState::new(
            Box::new(DemoProbe::new()),
            TEST_TOKEN,
            false,
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");

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
            accept_and_spawn(&listener, &state);
        }
        assert_eq!(state.active.load(Ordering::Acquire), MAX_CONNECTIONS);

        let mut client = TcpStream::connect(address).expect("connect extra client");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        client
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("write request");
        accept_and_spawn(&listener, &state);
        let mut response = String::new();
        client.read_to_string(&mut response).expect("read response");

        assert!(
            response.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{response}"
        );
        assert_eq!(state.active.load(Ordering::Acquire), MAX_CONNECTIONS);
        drop(idle);
        for keeper in keepers {
            keeper.join().expect("keeper thread");
        }
    }
}
