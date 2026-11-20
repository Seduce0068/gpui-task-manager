//! Token generation and request authentication for the read-only agent.
//!
//! The agent binds to loopback only, but loopback is not a security boundary:
//! any local process (or a browser page via CORS-free `no-cors` tricks) can
//! connect. Every snapshot request therefore has to present a bearer token that
//! is generated at start-up.
//!
//! Tokens are compared in constant time so a local attacker cannot learn the
//! secret byte by byte through timing.

use crate::http::Request;

/// Number of random bytes in a generated token.
///
/// Only Unix auto-generates a token (SC-WINTOKEN); other targets use an
/// explicitly supplied credential, so this contract is Unix-only.
#[cfg(unix)]
pub const TOKEN_BYTES: usize = 32;

/// Generates a fresh bearer token from the OS CSPRNG.
///
/// There is deliberately no time/PID fallback: a locally predictable token
/// would make the loopback "security boundary" meaningless. If the OS CSPRNG
/// cannot be read the agent fails to start rather than emitting a weak secret.
///
/// Platform branches (SC-WINTOKEN):
/// - `unix`: unchanged direct `/dev/urandom` read (std only, no `unsafe`).
/// - `windows`: `std` has no CSPRNG API. `getrandom` is present in `Cargo.lock`
///   only as a *transitive* dependency of other crates; declaring it here would
///   be a new dependency (out of scope for this round, AGENTS §2). Win32
///   `BCryptGenRandom` would require `unsafe` FFI, forbidden by
///   `#![forbid(unsafe_code)]` (AGENTS §15). Time/PID fallback is rejected
///   above. Therefore Windows fails fast with a fixed message that asks for an
///   explicit `--token` / `--token-file` / `TM_AGENT_TOKEN` instead of silently
///   emitting a weak credential.
/// - other targets: identical fixed error, identical rationale.
pub fn generate_token() -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::fs::File;
        use std::io::Read;

        let mut bytes = [0u8; TOKEN_BYTES];
        let mut device = File::open("/dev/urandom")
            .map_err(|err| format!("/dev/urandom を読めません: {err}"))?;
        device
            .read_exact(&mut bytes)
            .map_err(|err| format!("/dev/urandom から読み取れません: {err}"))?;
        Ok(hex_encode(&bytes))
    }
    #[cfg(windows)]
    {
        // Fixed, non-reflecting error: no token value, no path, no OS detail.
        Err(concat!(
            "Windows では --token 明示が必要です: ",
            "安全な乱数源を新規依存・unsafe なしで取得できないため、",
            "--token / --token-file / TM_AGENT_TOKEN のいずれかでトークンを指定してください。"
        )
        .to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(concat!(
            "このプラットフォームではトークンの自動生成はサポートされていません: ",
            "安全な乱数源を新規依存・unsafe なしで取得できないため、",
            "--token / --token-file / TM_AGENT_TOKEN のいずれかでトークンを指定してください。"
        )
        .to_string())
    }
}

/// Lower-case hex encoding (used for tokens only).
///
/// Only the Unix branch produces a token, so this helper is Unix-only too;
/// keeping it cfg-gated avoids a dead-code warning on Windows (`-D warnings`).
#[cfg(unix)]
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Compares two secrets without leaking their contents through timing.
///
/// The length is folded into the accumulator instead of causing an early
/// return, so the loop always runs for the longer input.
pub fn constant_time_eq(expected: &str, supplied: &str) -> bool {
    let left = expected.as_bytes();
    let right = supplied.as_bytes();
    let mut difference = (left.len() ^ right.len()) as u64;
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= u64::from(left_byte ^ right_byte);
    }
    difference == 0
}

/// Extracts the bearer token a request presents.
///
/// Only the `Authorization: Bearer <token>` header is accepted. Tokens are
/// deliberately **not** accepted from the query string: URLs leak through
/// browser history, `Referer` headers, screenshots and chat messages, and the
/// browser shell can set an `Authorization` header on its `fetch` request.
pub fn request_token(request: &Request) -> Option<String> {
    let header = request.header("authorization")?;
    let (scheme, value) = header.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") && !value.trim().is_empty() {
        Some(value.trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::parse_request;

    // Auto-generation only exists on Unix (SC-WINTOKEN); the Windows branch
    // is type-checked by native-windows `cargo check`, not exercised here.
    #[test]
    #[cfg(unix)]
    fn generated_tokens_are_hex_and_unique() {
        let first = generate_token().expect("urandom");
        let second = generate_token().expect("urandom");
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second, "two tokens must differ");
    }

    #[test]
    #[cfg(unix)]
    fn generated_tokens_are_lower_case_hex() {
        // Tokens are compared byte-wise (constant_time_eq), so a producer that
        // emitted upper-case hex would silently break every presented token.
        let token = generate_token().expect("urandom");
        assert_eq!(token, token.to_ascii_lowercase());
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert!(token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    #[test]
    fn presented_tokens_are_trimmed_before_comparison() {
        // All token sources store a trimmed value; the request side trims too,
        // so extra whitespace in the header must not break authentication.
        let request = parse_request(
            b"GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer   secret   \r\n",
        )
        .expect("valid request");
        assert_eq!(request_token(&request).as_deref(), Some("secret"));
    }

    #[test]
    fn constant_time_eq_matches_only_equal_inputs() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn request_token_accepts_only_the_authorization_header() {
        let request =
            parse_request(b"GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer header")
                .expect("valid request");
        assert_eq!(request_token(&request).as_deref(), Some("header"));

        // A query token is no longer accepted: URLs leak too easily.
        let request =
            parse_request(b"GET /api/v1/snapshot?token=query HTTP/1.1").expect("valid request");
        assert_eq!(request_token(&request), None);

        let request = parse_request(b"GET /api/v1/snapshot HTTP/1.1").expect("valid request");
        assert_eq!(request_token(&request), None);

        let request = parse_request(b"GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Basic abc")
            .expect("valid request");
        assert_eq!(request_token(&request), None);

        // Malformed bearer header must not fall through to anything else.
        let request = parse_request(b"GET /api/v1/snapshot HTTP/1.1\r\nAuthorization: Bearer")
            .expect("valid request");
        assert_eq!(request_token(&request), None);
    }
}
