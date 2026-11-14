//! Token generation and request authentication for the read-only agent.
//!
//! The agent binds to loopback only, but loopback is not a security boundary:
//! any local process (or a browser page via CORS-free `no-cors` tricks) can
//! connect. Every snapshot request therefore has to present a bearer token that
//! is generated at start-up.
//!
//! Tokens are compared in constant time so a local attacker cannot learn the
//! secret byte by byte through timing.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::http::Request;

/// Number of random bytes in a generated token.
pub const TOKEN_BYTES: usize = 32;

/// Generates a fresh bearer token.
///
/// Uses `/dev/urandom` and falls back to a time/PID mix only when the device
/// cannot be read (a degraded container, for example). The fallback is clearly
/// weaker and is documented rather than hidden.
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    if let Ok(mut device) = File::open("/dev/urandom") {
        if device.read_exact(&mut bytes).is_ok() {
            return hex_encode(&bytes);
        }
    }
    eprintln!(
        "tm-agent: 警告: /dev/urandom を読めないため、弱いフォールバック乱数でトークンを生成しました"
    );
    fallback_token_bytes(&mut bytes);
    hex_encode(&bytes)
}

/// Lower-case hex encoding (used for tokens only).
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Best-effort entropy when `/dev/urandom` is unavailable.
fn fallback_token_bytes(out: &mut [u8; TOKEN_BYTES]) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let mut state = nanos ^ u64::from(std::process::id());
    for chunk in out.chunks_mut(8) {
        state = splitmix64(state);
        for (slot, byte) in chunk.iter_mut().zip(state.to_le_bytes().iter()) {
            *slot = *byte;
        }
    }
}

/// One round of SplitMix64, used to expand the fallback seed.
fn splitmix64(seed: u64) -> u64 {
    let mut value = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
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

    #[test]
    fn generated_tokens_are_hex_and_unique() {
        let first = generate_token();
        let second = generate_token();
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second, "two tokens must differ");
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
