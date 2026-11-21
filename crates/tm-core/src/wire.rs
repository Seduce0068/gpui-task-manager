//! Wire contract shared by the native agent and the browser shell.
//!
//! Both ends of the loopback protocol need one definition of each of these:
//! the fixed `403` guidance text, the ceiling for a numeric `Retry-After` hint,
//! the JSON envelope that carries a snapshot, the two endpoint paths, the
//! default and maximum `?limit=` values, the default listen port, and the set
//! of machine-readable error codes. Defining them here makes agreement
//! structural instead of a matter of mirrored comments.
//!
//! [`SnapshotEnvelope`]'s field order and spelling are part of the wire
//! contract. Do not reorder the fields or add `serde` attributes: the agent
//! serialises it directly and the browser deserialises it directly, so any
//! change here changes the JSON bytes on the wire.

use crate::SystemSnapshot;

/// Shared reason for a `503 Busy` response when a connection limit is reached.
///
/// The ordinary-connection cap uses this verbatim as its body message; the
/// SSE-stream cap derives a prefixed variant (`ストリーミング<reason>`) in the
/// agent because `concat!` cannot expand a `const`. The browser shows fixed
/// actionable guidance from this phrase and deliberately never echoes the
/// server body. Keeping the shared phrase here lets both agent call sites and
/// the web hint be cross-checked against one definition.
pub const SERVICE_UNAVAILABLE_REASON: &str = "同時接続数が上限に達しました";

/// Path of the snapshot endpoint (GET, bearer token required).
///
/// The agent routes on this exact string and the browser builds its poll URL
/// from the same constant, so a rename cannot desynchronise the two sides.
pub const SNAPSHOT_PATH: &str = "/api/v1/snapshot";

/// Path of the long-lived SSE endpoint (GET, bearer token required).
pub const STREAM_PATH: &str = "/api/v1/stream";

/// Number of processes returned when `?limit=` is absent.
///
/// The agent truncates to this many busiest processes; the browser asks for
/// exactly this number so the two sides cannot silently drift (500 today).
pub const DEFAULT_SNAPSHOT_LIMIT: usize = 500;

/// Hard upper bound for `?limit=`. A larger value is clamped, not rejected.
pub const MAX_SNAPSHOT_LIMIT: usize = 2000;

/// Default loopback TCP port of `tm-agent`; the listen address is
/// `127.0.0.1:<DEFAULT_PORT>`.
pub const DEFAULT_PORT: u16 = 8787;

/// Stable, machine-readable codes carried by every HTTP error body.
///
/// The enum, [`ErrorCode::ALL`] and [`ErrorCode::as_str`] are the single wire
/// table. The agent's help text is cross-checked against `ALL` in both
/// directions (every variant documented, every documented code a variant), so
/// adding a code without documenting it — or documenting one that does not
/// exist — fails a test instead of silently diverging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// Malformed request (e.g. an unparsable HTTP head).
    BadRequest,
    /// `?limit=` was present but not a number.
    InvalidLimit,
    /// Missing or invalid bearer token (`401`).
    Unauthorized,
    /// Rejected Host/Origin (`403`).
    Forbidden,
    /// Unknown endpoint (`404`).
    NotFound,
    /// Method other than GET/OPTIONS (`405`).
    MethodNotAllowed,
    /// Request head exceeded the size cap (`413`).
    RequestTooLarge,
    /// Request head read timed out (`408`).
    RequestTimeout,
    /// Internal failure (`500`).
    Internal,
    /// Concurrent connection/stream cap reached (`503`).
    Busy,
}

impl ErrorCode {
    /// All variants in wire order — the one table used for cross-validation.
    pub const ALL: [Self; 10] = [
        Self::BadRequest,
        Self::InvalidLimit,
        Self::Unauthorized,
        Self::Forbidden,
        Self::NotFound,
        Self::MethodNotAllowed,
        Self::RequestTooLarge,
        Self::RequestTimeout,
        Self::Internal,
        Self::Busy,
    ];

    /// Stable spelling emitted in the `code` field of an error body.
    pub fn as_str(self) -> &'static str {
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

/// User-facing body for every `403` response, byte-identical on both sides.
///
/// The message describes an origin constraint the user can actually act on
/// (run the app and the browser on the same host). It is static text: no
/// request-derived Host/Origin value is ever echoed back. The 401 guidance is
/// deliberately *not* shared — only the Web shell presents a reconnect hint
/// for a token problem, so it stays in `web::agent::RECONNECT_GUIDANCE`.
pub const FORBIDDEN_GUIDANCE: &str = "別ホスト／別オリジンからのアクセスはサポートしていません。アプリとブラウザを同一ホストで起動してください";

/// Upper bound for a numeric `Retry-After` delta-seconds hint, in seconds.
///
/// The agent clamps outgoing hints at this ceiling and the browser clamps
/// incoming hints at the same value, so a misconfigured or hostile hint can
/// never park the Web shell's poll indefinitely. `0` is normalised to `1` by
/// the agent to avoid a hot retry loop.
pub const MAX_RETRY_AFTER_SECS: u64 = 30;

/// Wire envelope returned by `/api/v1/snapshot` and by SSE `snapshot` events.
///
/// The default parameter is the owned form used by the browser shell's
/// `Deserialize` path. The agent uses `SnapshotEnvelope<&SystemSnapshot>` for
/// its `Serialize` path so a snapshot is never cloned on the way out. Field
/// order (`schema_version`, `total_process_count`, `snapshot`) is intentional
/// and must remain stable: it is observed byte-for-byte by the wire tests.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SnapshotEnvelope<S = SystemSnapshot> {
    /// Version of the JSON wire schema carried by this envelope.
    pub schema_version: u32,
    /// Number of processes on the machine before `?limit=` truncation.
    pub total_process_count: usize,
    /// The snapshot itself.
    pub snapshot: S,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_guidance_is_a_fixed_single_line_message() {
        assert!(
            FORBIDDEN_GUIDANCE.contains("別ホスト／別オリジンからのアクセスはサポートしていません")
        );
        assert!(FORBIDDEN_GUIDANCE.contains("同一ホストで起動してください"));
        assert!(!FORBIDDEN_GUIDANCE.contains('#'), "no token hint in a 403");
        assert!(
            !FORBIDDEN_GUIDANCE.contains('\n'),
            "single-line status text"
        );
        assert!(!FORBIDDEN_GUIDANCE.contains(';'), "DOM proof delimiter");
    }

    #[test]
    fn retry_after_ceiling_is_thirty_seconds() {
        assert_eq!(MAX_RETRY_AFTER_SECS, 30);
    }

    #[test]
    fn endpoint_paths_and_limits_are_the_single_wire_contract() {
        assert_eq!(SNAPSHOT_PATH, "/api/v1/snapshot");
        assert_eq!(STREAM_PATH, "/api/v1/stream");
        assert_eq!(DEFAULT_SNAPSHOT_LIMIT, 500);
        assert_eq!(MAX_SNAPSHOT_LIMIT, 2000);
        const { assert!(DEFAULT_SNAPSHOT_LIMIT <= MAX_SNAPSHOT_LIMIT) };
        assert_eq!(DEFAULT_PORT, 8787);
    }

    #[test]
    fn service_unavailable_reason_is_fixed_and_shared() {
        // Both agent 503 call sites and the web hint derive from this phrase;
        // keep it a single user-facing reason without proof delimiters.
        assert_eq!(SERVICE_UNAVAILABLE_REASON, "同時接続数が上限に達しました");
        assert!(!SERVICE_UNAVAILABLE_REASON.contains(';'));
        assert!(!SERVICE_UNAVAILABLE_REASON.contains('\n'));
    }

    #[test]
    fn error_code_table_is_complete_and_distinct() {
        // Canonical spellings, asserted once here where the table is defined.
        // `args::USAGE` cross-checks the same `ALL` table from the other side.
        let expected = [
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
        ];
        assert_eq!(ErrorCode::ALL.len(), expected.len());
        for (variant, spelling) in expected {
            assert!(
                ErrorCode::ALL.contains(&variant),
                "ALL is missing {variant:?}"
            );
            assert_eq!(variant.as_str(), spelling);
        }
        let mut spellings: Vec<_> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        spellings.sort_unstable();
        let count = spellings.len();
        spellings.dedup();
        assert_eq!(
            spellings.len(),
            count,
            "error code spellings must be unique"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn envelope_bytes_keep_the_agreed_field_order() {
        let snapshot = SystemSnapshot::default();
        let json = serde_json::to_string(&SnapshotEnvelope {
            schema_version: crate::WIRE_SCHEMA_VERSION,
            total_process_count: 7,
            snapshot: &snapshot,
        })
        .expect("envelope must serialise");
        assert!(
            json.starts_with("{\"schema_version\":1,\"total_process_count\":7,\"snapshot\":"),
            "wire field order changed: {json}"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn envelope_round_trips_schema_version_and_total() {
        let snapshot = SystemSnapshot::default();
        let value = serde_json::json!({
            "schema_version": crate::WIRE_SCHEMA_VERSION,
            "total_process_count": 3,
            "snapshot": snapshot,
        });
        let envelope: SnapshotEnvelope =
            serde_json::from_value(value).expect("envelope must deserialise");
        assert_eq!(envelope.schema_version, crate::WIRE_SCHEMA_VERSION);
        assert_eq!(envelope.total_process_count, 3);
        assert_eq!(envelope.snapshot, snapshot);
    }
}
