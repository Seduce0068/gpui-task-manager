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
//!   response before sleeping, so a slow agent cannot build a request pile-up.

use std::sync::Arc;

use futures::AsyncReadExt as _;
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
}

impl AgentClient {
    /// Creates a client for the given configuration.
    pub fn new(config: AgentConfig, http: Arc<dyn HttpClient>) -> Self {
        Self { config, http }
    }

    /// Endpoint label for the status bar.
    pub fn endpoint_label(&self) -> &str {
        self.config.endpoint_label()
    }

    /// Requests and parses one snapshot.
    ///
    /// Errors are returned as user-facing Japanese strings with the token
    /// redacted; they never trigger a silent fallback to demo data.
    pub async fn fetch_snapshot(&self) -> Result<SnapshotResponse, String> {
        let url = snapshot_url(&self.config.endpoint);
        let mut builder = Request::builder().method("GET").uri(&url);
        if let Some(token) = &self.config.token {
            // クエリではなくヘッダで渡す（URL・Referer・アクセスログに残さない）。
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = builder
            .body(AsyncBody::empty())
            .map_err(|error| format!("リクエストの組み立てに失敗しました: {error}"))?;
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

        let envelope: SnapshotEnvelope = serde_json::from_slice(&buffer)
            .map_err(|error| format!("スナップショット JSON の解析に失敗しました: {error}"))?;
        check_schema_version(envelope.schema_version)?;
        Ok(SnapshotResponse {
            snapshot: envelope.snapshot,
            total_process_count: envelope.total_process_count,
        })
    }
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
        assert!(MAX_SNAPSHOT_BYTES > 1_000_000);
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
}
