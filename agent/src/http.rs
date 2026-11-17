//! Minimal HTTP/1.1 plumbing for the read-only agent.
//!
//! The agent deliberately avoids an HTTP framework: it serves two `GET`
//! endpoints on loopback, so the whole protocol layer is a few hundred lines of
//! `std` code that can be audited in one sitting.
//!
//! Design constraints:
//!
//! - one request per connection (`Connection: close`), which removes
//!   keep-alive state and request-smuggling surface,
//! - hard cap of [`MAX_HEADER_BYTES`] on the request head,
//! - strict CORS: only `http://127.0.0.1:<port>` and
//!   `http://localhost:<port>` origins are reflected.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

/// Upper bound for the request line plus all header lines.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Per-read timeout (a slow peer cannot block a thread indefinitely).
pub const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// Absolute deadline for the *whole* request head. Without this, a peer that
/// trickles one byte per second resets `READ_TIMEOUT` on every read and can
/// hold a connection slot for hours (slowloris).
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(5);

/// How long writing the response may take.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on the number of header lines.
const MAX_HEADERS: usize = 64;

/// A parsed HTTP request head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Method as sent by the client (compared case-insensitively).
    pub method: String,
    /// Path part of the request target, without the query string.
    pub path: String,
    /// Raw query string (everything after the first `?`), if any.
    pub query: Option<String>,
    /// Header name/value pairs; names are lower-cased.
    headers: Vec<(String, String)>,
}

impl Request {
    /// Returns the first header with the given (case-insensitive) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Why a request head could not be accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The head exceeded [`MAX_HEADER_BYTES`] or [`MAX_HEADERS`].
    TooLarge,
    /// The bytes were not a valid HTTP/1.x request head.
    Malformed,
    /// The head was not completed within [`HEAD_TIMEOUT`].
    Timeout,
}

/// Reads and parses one request head from `stream`.
///
/// Returns `Ok(None)` when the peer closed the connection without sending
/// anything, so an idle health check that connects and hangs up is not an
/// error.
pub fn read_request(stream: &mut TcpStream) -> Result<Option<Request>, ParseError> {
    read_request_with_timeout(stream, HEAD_TIMEOUT)
}

/// Same as [`read_request`] with an injectable deadline (used by tests).
fn read_request_with_timeout(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<Option<Request>, ParseError> {
    // Enforce the absolute deadline even if a caller forgot to set a socket
    // read timeout: cap each blocking read at the remaining budget.
    let _ = stream.set_read_timeout(Some(timeout.min(READ_TIMEOUT)));
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(position) = find_header_end(&buffer) {
            if position > MAX_HEADER_BYTES {
                return Err(ParseError::TooLarge);
            }
            return parse_request(&buffer[..position]).map(Some);
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(ParseError::TooLarge);
        }
        if Instant::now() >= deadline {
            return Err(ParseError::Timeout);
        }
        let read = stream.read(&mut chunk).map_err(|err| {
            if matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) {
                ParseError::Timeout
            } else {
                ParseError::Malformed
            }
        })?;
        if Instant::now() >= deadline {
            return Err(ParseError::Timeout);
        }
        if read == 0 {
            return if buffer.is_empty() {
                Ok(None)
            } else {
                Err(ParseError::Malformed)
            };
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// Parses a request head that does **not** include the final empty line.
pub fn parse_request(head: &[u8]) -> Result<Request, ParseError> {
    let text = std::str::from_utf8(head).map_err(|_| ParseError::Malformed)?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(ParseError::Malformed)?;

    let parts = request_line.split_whitespace().collect::<Vec<&str>>();
    if parts.len() != 3 {
        return Err(ParseError::Malformed);
    }
    let (method, target, version) = (parts[0], parts[1], parts[2]);
    if method.is_empty() {
        return Err(ParseError::Malformed);
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(ParseError::Malformed);
    }
    if !target.starts_with('/') {
        return Err(ParseError::Malformed);
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (target.to_string(), None),
    };

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(ParseError::TooLarge);
        }
        let (name, value) = line.split_once(':').ok_or(ParseError::Malformed)?;
        // RFC 9110 §5.1 forbids whitespace between the field name and colon,
        // and a line starting with SP/HTAB is an obs-fold continuation. Do NOT
        // trim the name: accepting `Host : x` (or a folded `Authorization`)
        // creates a header-interpretation ambiguity (R23 HTTP review).
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b':')
        {
            return Err(ParseError::Malformed);
        }
        let value = value.trim();
        if value
            .bytes()
            .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
        {
            return Err(ParseError::Malformed);
        }
        let name = name.to_ascii_lowercase();
        // Duplicate security-relevant headers would let a proxy-style
        // interpretation mismatch: `Request::header` takes the first value,
        // while a filter might inspect the last. Reject the whole request.
        if headers.iter().any(|(existing, _)| existing == &name) {
            return Err(ParseError::Malformed);
        }
        headers.push((name, value.to_string()));
    }

    Ok(Request {
        method: method.to_string(),
        path,
        query,
        headers,
    })
}

/// Finds the byte offset of the `\r\n\r\n` that ends the request head.
fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// A response ready to be written to the socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// Numeric HTTP status code.
    pub status: u16,
    /// Reason phrase written after the status code.
    pub reason: &'static str,
    /// `Content-Type` value used for responses that carry a body.
    pub content_type: &'static str,
    /// Response body; empty for `204 No Content`.
    pub body: Vec<u8>,
    /// Additional headers such as CORS or `WWW-Authenticate`.
    pub extra_headers: Vec<(String, String)>,
}

impl Response {
    /// Creates a JSON response from an already serialised body.
    pub fn json(status: u16, reason: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason,
            content_type: "application/json; charset=utf-8",
            body,
            extra_headers: Vec::new(),
        }
    }

    /// Creates a response without a body (`204 No Content`).
    pub fn empty(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "",
            body: Vec::new(),
            extra_headers: Vec::new(),
        }
    }
}

/// Writes `response` and closes the write side of the connection.
pub fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
    let mut head = String::with_capacity(256);
    head.push_str("HTTP/1.1 ");
    head.push_str(&response.status.to_string());
    head.push(' ');
    head.push_str(response.reason);
    head.push_str("\r\n");
    // 204 must not carry a body and must not advertise one.
    if response.status != 204 {
        head.push_str("Content-Type: ");
        head.push_str(response.content_type);
        head.push_str("\r\nContent-Length: ");
        head.push_str(&response.body.len().to_string());
        head.push_str("\r\n");
    }
    head.push_str("Connection: close\r\n");
    head.push_str("Cache-Control: no-store\r\n");
    head.push_str("X-Content-Type-Options: nosniff\r\n");
    for (name, value) in &response.extra_headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes())?;
    if response.status != 204 {
        stream.write_all(&response.body)?;
    }
    stream.flush()?;
    stream.shutdown(Shutdown::Write)?;
    Ok(())
}

/// Validates the port part of a `Host` header (digits only, never empty).
///
/// [`Iterator::all`] returns `true` for an empty slice, so an explicit
/// non-empty check is required to reject `Host: localhost:` and similar.
fn valid_port_suffix(port: &str) -> bool {
    !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit())
}

/// Returns `true` when the `Host` header names the loopback interface.
///
/// This is a defence-in-depth measure against DNS rebinding: a page on an
/// attacker-controlled domain that resolves to 127.0.0.1 would still send its
/// own host name in the `Host` header. Token authentication already blocks it,
/// but rejecting the request earlier keeps the attack surface smaller.
pub fn is_allowed_host(host: &str) -> bool {
    let host = host.trim();
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        // IPv6 literal: [::1]:port — validate everything after `]` as a port.
        match rest.split_once(']') {
            Some((address, suffix)) => match suffix.strip_prefix(':') {
                Some(port) if valid_port_suffix(port) => address,
                None if suffix.is_empty() => address,
                _ => return false,
            },
            None => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((address, port)) if valid_port_suffix(port) => address,
            // A stray colon that is not a valid port makes the whole Host
            // malformed; do not silently treat it as a hostname.
            Some(_) => return false,
            None => host,
        }
    };
    hostname.eq_ignore_ascii_case("localhost") || hostname == "127.0.0.1" || hostname == "::1"
}

/// Returns `true` for browser origins the agent is willing to expose data to.
///
/// Only plain `http` origins on the loopback host are accepted: the agent
/// serves a local development UI, not a public API. Any port is allowed because
/// the browser shell and its dev server pick their own ports.
pub fn is_allowed_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    if rest.is_empty()
        || rest
            .chars()
            .any(|ch| matches!(ch, '/' | '?' | '#' | ' ' | '@' | '\r' | '\n'))
    {
        return false;
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    if let Some(port) = port {
        if port.is_empty()
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || port.parse::<u16>().is_err()
        {
            return false;
        }
    }
    host.eq_ignore_ascii_case("127.0.0.1") || host.eq_ignore_ascii_case("localhost")
}

/// Adds the CORS headers that expose a response to an allowed browser origin.
pub fn add_cors_headers(response: &mut Response, origin: &str) {
    response.extra_headers.push((
        "Access-Control-Allow-Origin".to_string(),
        origin.to_string(),
    ));
    response
        .extra_headers
        .push(("Vary".to_string(), "Origin".to_string()));
}

/// Returns the percent-decoded value of `name` from a raw query string.
pub fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => (pair, ""),
        };
        if key == name {
            let decoded = percent_decode(value)?;
            // A decoded control character (e.g. %0d%0a) must never reach a
            // parsed parameter: treat the whole query as invalid. First value
            // wins on duplicates, so a second injected value cannot override.
            if decoded.chars().any(|ch| ch.is_control()) {
                return None;
            }
            return Some(decoded);
        }
    }
    None
}

/// Decodes `%XX` escapes and `+` in a query-string component.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = hex_value(*bytes.get(index + 1)?)?;
                let low = hex_value(*bytes.get(index + 2)?)?;
                out.push((high << 4) | low);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Decodes one hexadecimal digit.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_get_request() {
        let request =
            parse_request(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:8787").expect("valid request");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/healthz");
        assert_eq!(request.query, None);
        assert_eq!(request.header("host"), Some("127.0.0.1:8787"));
        assert_eq!(request.header("HOST"), Some("127.0.0.1:8787"));
    }

    #[test]
    fn accepts_http_1_0_request_lines() {
        // The parser accepts both 1.0 and 1.1 (simple clients may use 1.0);
        // responses are always written as HTTP/1.1 by `write_response`.
        let request = parse_request(b"GET /healthz HTTP/1.0").expect("http/1.0");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/healthz");
    }

    #[test]
    fn splits_path_and_query() {
        let request =
            parse_request(b"GET /api/v1/snapshot?token=abc HTTP/1.1").expect("valid request");
        assert_eq!(request.path, "/api/v1/snapshot");
        assert_eq!(request.query.as_deref(), Some("token=abc"));
    }

    #[test]
    fn malformed_header_names_and_folds_are_rejected() {
        // RFC 9110 §5.1: no whitespace before the colon, no obs-fold lines,
        // and no C0 controls in values (HTAB remains legal OWS).
        for raw in [
            &b"GET / HTTP/1.1\r\nHost : a"[..],
            b"GET / HTTP/1.1\r\nHost\t: a",
            "GET / HTTP/1.1\r\nHost\u{a0}: a".as_bytes(),
            b"GET / HTTP/1.1\r\nHost: a\r\n Authorization: Bearer x",
            b"GET / HTTP/1.1\r\nX-Test: a\x1bb",
            b"GET / HTTP/1.1\r\nX-Test: a\x7fb",
        ] {
            assert_eq!(parse_request(raw), Err(ParseError::Malformed), "{raw:?}");
        }
        assert!(parse_request(b"GET / HTTP/1.1\r\nX-Test: a\tb").is_ok());
    }

    #[test]
    fn duplicate_headers_are_rejected() {
        assert_eq!(
            parse_request(b"GET / HTTP/1.1\r\nHost: a\r\nHost: b"),
            Err(ParseError::Malformed)
        );
        assert_eq!(
            parse_request(b"GET / HTTP/1.1\r\nX-Test: a\r\nx-test: b"),
            Err(ParseError::Malformed)
        );
        // Distinct headers remain fine.
        assert!(parse_request(b"GET / HTTP/1.1\r\nHost: a\r\nX-Test: b").is_ok());
    }

    #[test]
    fn lowercases_header_names() {
        let request = parse_request(b"GET / HTTP/1.1\r\nAuthorization: Bearer x").expect("valid");
        assert_eq!(request.header("authorization"), Some("Bearer x"));
    }

    #[test]
    fn rejects_malformed_request_lines() {
        for head in [
            &b""[..],
            b"GET",
            b"GET /",
            b"GET / HTTP/2.0",
            b"GET  HTTP/1.1",
            b"GET / HTTP/1.1 extra",
            b"GET healthz HTTP/1.1",
        ] {
            assert_eq!(
                parse_request(head),
                Err(ParseError::Malformed),
                "head {head:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_malformed_headers() {
        assert_eq!(
            parse_request(b"GET / HTTP/1.1\r\nno-colon-here"),
            Err(ParseError::Malformed)
        );
        assert_eq!(
            parse_request(b"GET / HTTP/1.1\r\n: empty-name"),
            Err(ParseError::Malformed)
        );
        assert_eq!(
            parse_request(b"GET / HTTP/1.1\r\nX: a\rb"),
            Err(ParseError::Malformed)
        );
    }

    #[test]
    fn rejects_invalid_utf8() {
        assert_eq!(
            parse_request(b"GET /\xff HTTP/1.1"),
            Err(ParseError::Malformed)
        );
    }

    /// Feeds `head` to `read_request` over a loopback socket and returns its result.
    fn read_head(head: &[u8]) -> Result<Option<Request>, ParseError> {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let client = std::net::TcpStream::connect(address).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let mut client = client;
        client.write_all(head).expect("write head");
        drop(client);
        let mut server = server;
        read_request(&mut server)
    }

    #[test]
    fn accepts_a_head_at_the_byte_limit() {
        // A single padded header: the terminating CRLF ends exactly at
        // MAX_HEADER_BYTES, exercising only the byte limit (not MAX_HEADERS).
        let mut head = String::from("GET / HTTP/1.1\r\nX: ");
        let padding = MAX_HEADER_BYTES - head.len() - 4; // + final "\r\n\r\n"
        head.push_str(&"a".repeat(padding));
        head.push_str("\r\n\r\n");
        assert_eq!(head.len(), MAX_HEADER_BYTES);
        let result = read_head(head.as_bytes());
        assert!(
            result.is_ok(),
            "a head exactly at the byte limit must be accepted: {result:?}"
        );
    }

    #[test]
    fn empty_connection_returns_none() {
        // A health check that connects and closes without sending a head must
        // be a clean no-op, not a parse error (the agent's read timeout path).
        assert_eq!(read_head(b""), Ok(None));
    }

    #[test]
    fn partial_head_then_close_is_malformed() {
        // Bytes without the final CRLF CRLF must not be treated as a request.
        assert_eq!(
            read_head(b"GET / HTTP/1.1\r\nHost: 127.0.0.1"),
            Err(ParseError::Malformed)
        );
    }

    #[test]
    fn accepts_a_head_spanning_a_read_chunk_boundary() {
        // The reader consumes 1024-byte chunks; put the terminating CRLF across
        // that boundary to verify `find_header_end` sees the accumulated buffer.
        let mut head = String::from("GET / HTTP/1.1\r\nX: ");
        // The header's own CRLF starts at 1023 and ends at 1024, so the final
        // empty-line CRLF sits in the next chunk: 1024 bytes "GET ... a\r\n" +
        // "\r\n" = 1026 total.
        let padding = 1_024 - head.len() - 2;
        head.push_str(&"a".repeat(padding));
        head.push_str("\r\n\r\n");
        assert_eq!(head.len(), 1_026);
        assert_eq!(&head.as_bytes()[1022..1024], b"\r\n");
        assert_eq!(&head.as_bytes()[1024..1026], b"\r\n");
        assert!(read_head(head.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_a_head_over_the_byte_limit() {
        let mut head = String::from("GET / HTTP/1.1\r\n");
        while head.len() < MAX_HEADER_BYTES + 32 {
            head.push_str("X: 0123456789\r\n");
        }
        head.push_str("\r\n");
        assert_eq!(
            read_head(head.as_bytes()).err(),
            Some(ParseError::TooLarge),
            "a head over the byte limit must be rejected"
        );
    }

    #[test]
    fn accepts_the_maximum_number_of_headers() {
        let mut head = String::from("GET / HTTP/1.1");
        for index in 0..MAX_HEADERS {
            head.push_str(&format!("\r\nX-{index}: v"));
        }
        assert!(parse_request(head.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_too_many_headers() {
        let mut head = String::from("GET / HTTP/1.1");
        for index in 0..=MAX_HEADERS {
            head.push_str(&format!("\r\nX-{index}: v"));
        }
        assert_eq!(parse_request(head.as_bytes()), Err(ParseError::TooLarge));
    }

    #[test]
    fn allows_only_loopback_hosts() {
        for allowed in [
            "127.0.0.1",
            "127.0.0.1:18787",
            "localhost",
            "localhost:18787",
            "[::1]",
            "[::1]:18787",
        ] {
            assert!(is_allowed_host(allowed), "{allowed} should be allowed");
        }
        for rejected in [
            "evil.example",
            "evil.example:80",
            "10.0.0.1:18787",
            "[::1",
            "localhost:",
            "127.0.0.1:",
            "[::1]:",
            "[::1]:garbage",
            "localhost:80:80",
            "",
        ] {
            assert!(!is_allowed_host(rejected), "{rejected} should be rejected");
        }
    }

    #[test]
    fn allows_only_loopback_http_origins() {
        for allowed in [
            "http://127.0.0.1:8787",
            "http://localhost:5173",
            "http://127.0.0.1",
            "http://localhost",
        ] {
            assert!(is_allowed_origin(allowed), "{allowed} should be allowed");
        }
        for rejected in [
            "https://127.0.0.1:8787",
            "http://example.com",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil.example:80",
            "http://127.0.0.1:99999",
            "http://127.0.0.1:abc",
            "http://127.0.0.1/path",
            "http://user@127.0.0.1:80",
            "http://127.0.0.1:8787\r\nX-Injected: 1",
            "null",
            "",
            "http://[::1]:8787",
        ] {
            assert!(
                !is_allowed_origin(rejected),
                "{rejected} should be rejected"
            );
        }
    }

    #[test]
    fn writes_a_well_formed_response() {
        let mut response = Response::json(200, "OK", b"{\"ok\":true}".to_vec());
        add_cors_headers(&mut response, "http://127.0.0.1:5173");
        // The writer itself is exercised through the agent's socket tests; here
        // only the CORS header composition is checked.
        assert!(response
            .extra_headers
            .contains(&("Vary".to_string(), "Origin".to_string())));
        assert!(response.extra_headers.contains(&(
            "Access-Control-Allow-Origin".to_string(),
            "http://127.0.0.1:5173".to_string()
        )));
        // The header echoes the validated origin verbatim, so no wildcard:
        // credentials and loopback scoping stay meaningful.
        assert!(response
            .extra_headers
            .iter()
            .all(|(name, _)| name != "Access-Control-Allow-Credentials"));
    }
    /// Writes a response over a loopback socket and returns the raw bytes.
    fn write_response_bytes(response: &Response) -> Vec<u8> {
        use std::io::Read;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let client = std::net::TcpStream::connect(address).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let mut server = server;
        write_response(&mut server, response).expect("write response");
        let mut client = client;
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).expect("read response");
        bytes
    }

    #[test]
    fn head_read_times_out_absolutely_even_when_bytes_trickle() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let address = listener.local_addr().expect("local addr");
        let client = std::net::TcpStream::connect(address).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        // Send one byte, then keep the connection open without completing the
        // head. A per-read timeout would keep resetting; the absolute deadline
        // must still fire.
        let mut client = client;
        client.write_all(b"G").expect("write partial head");
        let mut server = server;
        assert_eq!(
            read_request_with_timeout(&mut server, Duration::from_millis(100)),
            Err(ParseError::Timeout)
        );
    }

    #[test]
    fn response_headers_are_safe_and_204_omits_body_headers() {
        let ok = String::from_utf8(write_response_bytes(&Response::json(
            200,
            "OK",
            b"{\"ok\":true}".to_vec(),
        )))
        .expect("utf8");
        assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(ok.contains("Content-Type: application/json; charset=utf-8\r\n"));
        assert!(ok.contains("Content-Length: 11\r\n"));
        assert!(ok.contains("Cache-Control: no-store\r\n"));
        assert!(ok.contains("X-Content-Type-Options: nosniff\r\n"));
        assert!(ok.contains("Connection: close\r\n"));

        let empty = String::from_utf8(write_response_bytes(&Response::empty(204, "No Content")))
            .expect("utf8");
        assert!(empty.starts_with("HTTP/1.1 204 No Content\r\n"));
        // Match header lines, not substrings: "X-Content-Type-Options" contains
        // "Content-Type" but is a different header.
        assert!(!empty.contains("\r\nContent-Type:"));
        assert!(!empty.contains("\r\nContent-Length:"));
        assert!(empty.ends_with("\r\n\r\n"));
    }

    #[test]
    fn query_param_extracts_values() {
        let query = Some("foo=1&token=abc%2Bdef&bar=2");
        assert_eq!(query_param(query, "token").as_deref(), Some("abc+def"));
        assert_eq!(query_param(query, "foo").as_deref(), Some("1"));
        assert_eq!(query_param(query, "missing"), None);
        assert_eq!(query_param(None, "token"), None);
    }

    #[test]
    fn query_param_rejects_controls_and_keeps_first_duplicate() {
        // Decoded CRLF/NUL would allow header/parameter poisoning downstream.
        assert_eq!(query_param(Some("limit=5%0d%0afoo"), "limit"), None);
        assert_eq!(query_param(Some("limit=%00"), "limit"), None);
        // Deterministic first-wins: a second injected value cannot override.
        assert_eq!(
            query_param(Some("limit=5&limit=9999"), "limit").as_deref(),
            Some("5")
        );
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b"), Some("a b".to_string()));
        assert_eq!(percent_decode("a+b"), Some("a b".to_string()));
        assert_eq!(percent_decode("a%2Bb"), Some("a+b".to_string()));
        assert_eq!(percent_decode("%E3%81%82"), Some("あ".to_string()));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%2"), None);
    }
}
