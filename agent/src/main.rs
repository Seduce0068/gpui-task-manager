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
//! - one request per connection, 5 s read/write timeouts, 8 KiB request head,
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
            token
        }
        (None, Some(path)) => read_token_file(path)?,
        (None, None) => match std::env::var("TM_AGENT_TOKEN") {
            Ok(token) if !token.trim().is_empty() => token,
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
    serve(&listener, probe.as_mut(), &token, config.redact)
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

/// Accepts connections until the process is terminated.
///
/// Connections are handled one at a time: the agent serves a single local UI,
/// and serial handling keeps the code free of shared mutable state.
fn serve(
    listener: &TcpListener,
    probe: &mut dyn SystemProbe,
    token: &str,
    redact: bool,
) -> Result<(), String> {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream, probe, token, redact) {
                    eprintln!("tm-agent: 接続処理に失敗: {err}");
                }
            }
            Err(err) => eprintln!("tm-agent: accept に失敗: {err}"),
        }
    }
    Ok(())
}

/// Handles exactly one HTTP request on `stream`.
fn handle_connection(
    mut stream: TcpStream,
    probe: &mut dyn SystemProbe,
    token: &str,
    redact: bool,
) -> io::Result<()> {
    stream.set_read_timeout(Some(http::READ_TIMEOUT))?;
    stream.set_write_timeout(Some(http::WRITE_TIMEOUT))?;

    let request = match http::read_request(&mut stream) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(http::ParseError::TooLarge) => {
            let response = http::Response::json(
                431,
                "Request Header Fields Too Large",
                error_body("リクエストヘッダが大きすぎます"),
            );
            return http::write_response(&mut stream, &response);
        }
        Err(http::ParseError::Malformed) => {
            let response =
                http::Response::json(400, "Bad Request", error_body("リクエストを解釈できません"));
            return http::write_response(&mut stream, &response);
        }
    };

    if let Some(host) = request.header("host") {
        if !http::is_allowed_host(host) {
            let response = http::Response::json(
                403,
                "Forbidden",
                error_body("Host ヘッダがループバックではありません"),
            );
            return http::write_response(&mut stream, &response);
        }
    }

    let origin = request.header("origin").map(str::to_string);
    let allowed_origin = origin
        .as_deref()
        .filter(|origin| http::is_allowed_origin(origin));

    let mut response = route(&request, probe, token, redact);
    if let Some(origin) = allowed_origin {
        http::add_cors_headers(&mut response, origin);
    }
    http::write_response(&mut stream, &response)
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
    use std::time::Duration;

    const TEST_TOKEN: &str = "test-token";

    /// Starts a demo-mode server that accepts exactly `connections` requests.
    fn spawn_demo_server(connections: usize) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let mut probe = DemoProbe::new();
            for _ in 0..connections {
                let (stream, _) = listener.accept().expect("accept");
                handle_connection(stream, &mut probe, TEST_TOKEN, false)
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
        stream.shutdown(Shutdown::Write).expect("shutdown write");
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
        stream.shutdown(Shutdown::Write).expect("shutdown");
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
}
