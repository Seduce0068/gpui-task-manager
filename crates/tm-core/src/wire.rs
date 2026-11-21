//! Wire contract shared by the native agent and the browser shell.
//!
//! Both ends of the loopback protocol need the same three things: the fixed
//! `403` guidance text, the ceiling for a numeric `Retry-After` hint, and the
//! JSON envelope that carries a snapshot. Defining them here makes agreement
//! structural instead of a matter of mirrored comments.
//!
//! [`SnapshotEnvelope`]'s field order and spelling are part of the wire
//! contract. Do not reorder the fields or add `serde` attributes: the agent
//! serialises it directly and the browser deserialises it directly, so any
//! change here changes the JSON bytes on the wire.

use crate::SystemSnapshot;

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
