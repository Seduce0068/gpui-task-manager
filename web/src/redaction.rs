//! Wire-side redaction for the browser shell.
//!
//! `tm-agent`'s `--redact` flag defaults to `false`, so a live HTTP response can
//! still carry the real host name, process users, home paths and tailnet domains.
//! The browser cannot read the agent host's environment, so instead of trusting
//! the agent flag we recover the identifiers from the snapshot itself **before**
//! redacting it and reuse the same pure rules as the native / agent shells
//! ([`tm_core::redact`]). This keeps the dashboard and its exports free of local
//! identifiers even when an unredacted agent is connected.
//!
//! Identifier strings are owned here rather than borrowed from the snapshot so a
//! mutable snapshot can be sanitised / redacted alongside them.

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

/// Sanitises and then redacts a snapshot that arrived over the HTTP wire.
///
/// Identifiers are captured first, while the snapshot still holds the real
/// values; sanitisation only strips control characters / truncates display
/// strings and must not hide the values we are about to mask. Redaction runs
/// last so the stored snapshot (and therefore every export of it) is safe to
/// render and share regardless of the agent's `--redact` flag.
pub(crate) fn sanitize_and_redact_wire_snapshot(snapshot: &mut SystemSnapshot) {
    let ids = identifiers_from_snapshot(snapshot);
    sanitize_snapshot(snapshot);
    redact_snapshot(snapshot, &ids.as_redaction_ids());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::model::{DiskUsageSnapshot, ProcessKey, ProcessSnapshot, ProcessStatus};

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
        sanitize_and_redact_wire_snapshot(&mut snapshot);

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
        assert_eq!(snapshot.disks[0].mount_point, "/home/user/work");
    }

    #[test]
    fn missing_identifiers_are_handled_without_panicking() {
        let ids = identifiers_from_snapshot(&SystemSnapshot::default());
        assert_eq!(ids, OwnedIdentifiers::default());

        let mut empty = SystemSnapshot::default();
        sanitize_and_redact_wire_snapshot(&mut empty);
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
}
