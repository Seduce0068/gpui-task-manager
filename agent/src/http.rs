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
use std::time::Duration;

/// Upper bound for the request line plus all header lines.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// How long a client may take to send the request head.
pub const READ_TIMEOUT: Duration = Duration::from_secs(1);

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
}

/// Reads and parses one request head from `stream`.
///
/// Returns `Ok(None)` when the peer closed the connection without sending
/// anything, so an idle health check that connects and hangs up is not an
/// error.
pub fn read_request(stream: &mut TcpStream) -> Result<Option<Request>, ParseError> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
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
        let read = stream.read(&mut chunk).map_err(|_| ParseError::Malformed)?;
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
        let name = name.trim();
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
            .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0)
        {
            return Err(ParseError::Malformed);
        }
        headers.push((name.to_ascii_lowercase(), value.to_string()));
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

/// Returns `true` when the `Host` header names the loopback interface.
///
/// This is a defence-in-depth measure against DNS rebinding: a page on an
/// attacker-controlled domain that resolves to 127.0.0.1 would still send its
/// own host name in the `Host` header. Token authentication already blocks it,
/// but rejecting the request earlier keeps the attack surface smaller.
pub fn is_allowed_host(host: &str) -> bool {
    let host = host.trim();
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        // IPv6 literal: [::1]:port
        match rest.split_once(']') {
            Some((address, _)) => address,
            None => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((address, port)) if port.chars().all(|ch| ch.is_ascii_digit()) => address,
            _ => host,
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
            return percent_decode(value);
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
    fn splits_path_and_query() {
        let request =
            parse_request(b"GET /api/v1/snapshot?token=abc HTTP/1.1").expect("valid request");
        assert_eq!(request.path, "/api/v1/snapshot");
        assert_eq!(request.query.as_deref(), Some("token=abc"));
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
            "[::1]:18787",
        ] {
            assert!(is_allowed_host(allowed), "{allowed} should be allowed");
        }
        for rejected in [
            "evil.example",
            "evil.example:80",
            "10.0.0.1:18787",
            "[::1",
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
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b"), Some("a b".to_string()));
        assert_eq!(percent_decode("a+b"), Some("a b".to_string()));
        assert_eq!(percent_decode("a%2Bb"), Some("a+b".to_string()));
        assert_eq!(percent_decode("%E3%81%82"), Some("あ".to_string()));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%2"), None);
    }
}
