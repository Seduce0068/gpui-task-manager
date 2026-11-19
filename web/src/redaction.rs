//! Wire-side redaction for the browser shell.
//!
//! `tm-agent`'s `--redact` flag defaults to `false`, so a live HTTP response can
//! still carry the real host name, process users, home paths and tailnet domains.
//! The browser cannot read the agent host's environment, so instead of trusting
//! the agent flag we recover the identifiers from the raw wire snapshot, then
//! redact, then sanitise. Redaction must see the untouched wire strings:
//! sanitisation replaces Cf/control characters with spaces and truncates display
//! strings, which would otherwise stop the recovered identifiers from matching
//! their own display strings. The pure rules are shared with the native / agent
//! shells ([`tm_core::redact`]). This keeps the dashboard and its exports free
//! of local identifiers even when an unredacted agent is connected.
//!
//! Identifier strings are owned here rather than borrowed from the snapshot so a
//! mutable snapshot can be redacted / sanitised alongside them.

use tm_core::SystemSnapshot;
use tm_core::redact::{RedactionIdentifiers, redact_snapshot};
use tm_core::sanitize::sanitize_snapshot;

/// Owned identifiers recovered from a snapshot.
///
/// Keeping the strings here (instead of inside the snapshot) lets the caller pass
/// `&mut SystemSnapshot` to the pure redaction helpers at the same time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnedIdentifiers {
    /// Home directory seen in mount points / commands (e.g. `/home/alice`).
    pub(crate) home: String,
    /// Account name seen on a process (e.g. `alice`).
    pub(crate) user: String,
    /// Machine host name from the snapshot.
    pub(crate) hostname: String,
}

impl OwnedIdentifiers {
    /// Borrows the owned strings as the view expected by `tm_core`'s helpers.
    pub(crate) fn as_redaction_ids(&self) -> RedactionIdentifiers<'_> {
        RedactionIdentifiers {
            home: &self.home,
            user: &self.user,
            hostname: &self.hostname,
        }
    }
}

/// Recovers identifier values from an (as yet unredacted) snapshot.
///
/// The host name is taken directly from `snapshot.hostname`, the account name
/// from the first process that reports one, and the home path is read from disk
/// mount points or command lines when present (with the conventional
/// `/home/<user>` layout as a fallback). Every value is owned so the caller can
/// mutate the snapshot immediately afterwards.
pub(crate) fn identifiers_from_snapshot(snapshot: &SystemSnapshot) -> OwnedIdentifiers {
    let hostname = snapshot.hostname.clone().unwrap_or_default();
    let user = snapshot
        .processes
        .iter()
        .find_map(|process| process.user.as_deref())
        .filter(|name| !name.is_empty())
        .unwrap_or("")
        .to_string();
    let home = derive_home(snapshot, &user);
    OwnedIdentifiers {
        home,
        user,
        hostname,
    }
}

/// Finds a concrete home path in the snapshot, falling back to the POSIX layout.
fn derive_home(snapshot: &SystemSnapshot, user: &str) -> String {
    for text in snapshot
        .disks
        .iter()
        .map(|disk| disk.mount_point.as_str())
        .chain(
            snapshot
                .processes
                .iter()
                .map(|process| process.command.as_str()),
        )
    {
        for prefix in ["/home/", "/Users/", "C:\\Users\\"] {
            if let Some(index) = text.find(prefix) {
                let tail = &text[index + prefix.len()..];
                let name: String = tail
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
                    .collect();
                if !name.is_empty() {
                    return format!("{prefix}{name}");
                }
            }
        }
    }
    if user.is_empty() {
        String::new()
    } else {
        format!("/home/{user}")
    }
}

/// Redacts and then sanitises a snapshot that arrived over the HTTP wire.
///
/// The order is deliberately **identify → redact → sanitise** and must not be
/// swapped; the name follows that order for the two mutation steps:
///
/// 1. Capture identifiers from the raw wire snapshot, before any mutation.
/// 2. Redact against those raw strings. Sanitisation turns Cf/control characters
///    into spaces and truncates commands at 240 chars, so sanitizing first makes
///    the recovered ids stop matching their own display strings (ZWSP inside a
///    host name, or an identifier crossing the truncation cut).
/// 3. Sanitise last, so ANSI/control/Cf injection is still stripped and every
///    rendered string stays bounded regardless of the agent's `--redact` flag.
pub(crate) fn redact_and_sanitize_wire_snapshot(snapshot: &mut SystemSnapshot) {
    let ids = identifiers_from_snapshot(snapshot);
    redact_snapshot(snapshot, &ids.as_redaction_ids());
    sanitize_snapshot(snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::model::{DiskUsageSnapshot, ProcessKey, ProcessSnapshot, ProcessStatus};
    use tm_core::sanitize::MAX_COMMAND_CHARS;

    fn wire_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            hostname: Some("my-host".into()),
            processes: vec![ProcessSnapshot {
                key: ProcessKey::new(4242, 1_700_000_000),
                name: "alice-worker".into(),
                command:
                    "alice --home /home/alice https://my-host.tail1234.ts.net:8443 --id=tail1234abcd"
                        .into(),
                user: Some("alice".into()),
                status: ProcessStatus::Running,
                ..ProcessSnapshot::default()
            }],
            disks: vec![DiskUsageSnapshot {
                mount_point: "/home/alice/work".into(),
                ..DiskUsageSnapshot::default()
            }],
            ..SystemSnapshot::default()
        }
    }

    #[test]
    fn identifiers_are_recovered_from_the_snapshot() {
        let ids = identifiers_from_snapshot(&wire_snapshot());
        assert_eq!(ids.hostname, "my-host");
        assert_eq!(ids.user, "alice");
        assert_eq!(ids.home, "/home/alice");
    }

    #[test]
    fn wire_snapshot_is_sanitized_and_redacted_before_display() {
        let mut snapshot = wire_snapshot();
        redact_and_sanitize_wire_snapshot(&mut snapshot);

        assert_eq!(snapshot.hostname.as_deref(), Some("host"));
        let process = &snapshot.processes[0];
        assert_eq!(process.user.as_deref(), Some("user"));
        for secret in ["alice", "my-host", "/home/alice", "tail1234", "ts.net"] {
            assert!(
                !process.command.contains(secret),
                "command leaked {secret}: {}",
                process.command
            );
            assert!(
                !process.name.contains(secret),
                "name leaked {secret}: {}",
                process.name
            );
        }
        assert_eq!(snapshot.disks[0].mount_point, "<home>/work");
    }

    #[test]
    fn renamed_wire_gate_keeps_the_redact_then_sanitize_contract_and_is_idempotent() {
        // The renamed entry point must keep the old function's exact contract:
        // recover ids from the raw wire, redact them, then sanitise last.
        let mut once = wire_snapshot();
        redact_and_sanitize_wire_snapshot(&mut once);

        assert_eq!(once.hostname.as_deref(), Some("host"));
        assert_eq!(once.processes[0].user.as_deref(), Some("user"));
        assert_eq!(once.disks[0].mount_point, "<home>/work");

        // Re-deriving ids from an already redacted / sanitised snapshot must be
        // a no-op, so applying the gate twice yields the same output as once.
        let mut twice = once.clone();
        redact_and_sanitize_wire_snapshot(&mut twice);
        assert_eq!(twice, once, "the wire gate must be idempotent");
    }

    #[test]
    fn missing_identifiers_are_handled_without_panicking() {
        let ids = identifiers_from_snapshot(&SystemSnapshot::default());
        assert_eq!(ids, OwnedIdentifiers::default());

        let mut empty = SystemSnapshot::default();
        redact_and_sanitize_wire_snapshot(&mut empty);
        assert_eq!(empty.hostname, None);
    }

    #[test]
    fn home_falls_back_to_the_conventional_user_path() {
        let mut snapshot = SystemSnapshot::default();
        snapshot.processes.push(ProcessSnapshot {
            user: Some("alice".into()),
            command: "worker --no-home-here".into(),
            ..ProcessSnapshot::default()
        });
        assert_eq!(identifiers_from_snapshot(&snapshot).home, "/home/alice");
    }

    #[test]
    fn cf_characters_do_not_disable_redaction() {
        // A ZWSP inside an identifier must not let that identifier survive.
        // Redaction runs on the raw wire strings, where the Cf char is still
        // present and the recovered id matches exactly; a sanitize-first order
        // turned it into a space ("al ice" / "my host") that no full-id
        // replacement could match.
        let mut snapshot = SystemSnapshot {
            hostname: Some("my\u{200b}host".into()),
            processes: vec![ProcessSnapshot {
                name: "al\u{200b}ice-worker".into(),
                command: "run as al\u{200b}ice on my\u{200b}host".into(),
                user: Some("al\u{200b}ice".into()),
                ..ProcessSnapshot::default()
            }],
            ..SystemSnapshot::default()
        };
        redact_and_sanitize_wire_snapshot(&mut snapshot);

        let process = &snapshot.processes[0];
        assert_eq!(process.user.as_deref(), Some("user"));
        assert_eq!(process.name, "user-worker");
        assert_eq!(process.command, "run as user on host");
        assert_eq!(snapshot.hostname.as_deref(), Some("host"));
    }

    #[test]
    fn identifiers_straddling_the_truncation_boundary_are_redacted() {
        // Commands are truncated to MAX_COMMAND_CHARS by sanitisation. Place the
        // host name across that boundary: redacting first replaces the whole id
        // while it is still intact, whereas sanitize-first cut it into a "my-"
        // fragment that the full-id replacement could no longer match.
        let prefix = "x".repeat(MAX_COMMAND_CHARS - 4);
        let mut snapshot = SystemSnapshot {
            hostname: Some("my-host".into()),
            processes: vec![ProcessSnapshot {
                command: format!("{prefix} my-host"),
                ..ProcessSnapshot::default()
            }],
            ..SystemSnapshot::default()
        };
        redact_and_sanitize_wire_snapshot(&mut snapshot);

        let command = &snapshot.processes[0].command;
        assert!(!command.contains("my-host"), "hostname survived: {command}");
        assert_eq!(command, &format!("{prefix} hos…"));
    }
}
