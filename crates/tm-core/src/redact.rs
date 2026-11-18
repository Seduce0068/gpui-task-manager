//! Pure helpers for redacting local identifiers from display strings.
//!
//! Both the native shell (`--redact`) and the agent's snapshot endpoint promise
//! to hide every identifier-bearing display string: host name, OS name, kernel
//! version, user names, home paths, process names and commands, mount points,
//! interface names and sensor labels (plus tailnet domains). Keeping the rules
//! here (rather than duplicating them in each shell) prevents the snapshot and
//! export paths from drifting. Reading the environment is deliberately left to
//! the callers: `tm-core` stays free of I/O and works on `wasm32`.

use crate::model::SystemSnapshot;

/// The local identifiers to remove from a string.
///
/// Build these from values captured *before* any redaction, and own the strings
/// (e.g. `hostname.clone()`) so a mutable snapshot can be passed alongside them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedactionIdentifiers<'a> {
    /// The user's home directory (from `HOME`), or an empty string.
    pub home: &'a str,
    /// The account name (from `USER` / `LOGNAME`), or an empty string.
    pub user: &'a str,
    /// The machine host name, or an empty string.
    pub hostname: &'a str,
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
        // `rfind` returns a byte index; advancing by 1 byte can land inside a
        // multibyte character (e.g. a Japanese label before ".ts.net") and
        // panic on the slice below. Advance by the character's UTF-8 length.
        let start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '-'))
            .map(|(index, ch)| index + ch.len_utf8())
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
/// Catches truncated commands such as `https://host.tail1234.t…` where the
/// `.ts.net` suffix was cut off by the command length limit.
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

/// Redacts local identifiers from one display string.
///
/// The order matters: longer, more specific identifiers are replaced first so
/// a short user name cannot partially rewrite a host name (e.g. `USER=alice`,
/// `hostname=alice-laptop`). Very short identifiers are skipped because
/// `String::replace` would corrupt unrelated words (`a` in `java`).
pub fn redact_text(value: &str, ids: &RedactionIdentifiers<'_>) -> String {
    let mut out = value.to_string();
    if ids.home.chars().count() >= 3 {
        out = out.replace(ids.home, "/home/user");
    }
    // The first label of the host name is also used in service names; derive
    // it here so every caller applies the same rule.
    let short_host = ids.hostname.split(['.', '-']).next().unwrap_or("");
    if ids.hostname.chars().count() >= 3 && !ids.hostname.is_empty() {
        out = out.replace(ids.hostname, "host");
    }
    if short_host.chars().count() >= 3 {
        out = out.replace(short_host, "host");
    }
    if !ids.user.is_empty() && ids.user.chars().count() >= 2 {
        out = out.replace(ids.user, "user");
    }
    out = redact_tailnet_domains(&out);
    out = redact_host_subdomains(&out);
    redact_tailnet_tokens(&out)
}

/// Redacts every identifier-bearing display string in a snapshot.
///
/// This covers the host name, OS name, kernel version, process user/name/command,
/// network interface names, mount points and sensor labels. Display strings are
/// rewritten with [`redact_text`] and the supplied `ids`; the host name and user
/// are replaced with fixed placeholders. Interface names are masked here too:
/// this is the "before sending" boundary (`--once`, the snapshot API and SSE),
/// where leaving a local identifier in a published payload is worse than losing
/// a name-based classification hint. The function is idempotent, so
/// [`crate::export::redact_for_export`] can safely re-apply it to a clone.
/// Shells with an immutable view of the dashboard should use [`redact_text`]
/// on individual strings instead.
pub fn redact_snapshot(snapshot: &mut SystemSnapshot, ids: &RedactionIdentifiers<'_>) {
    if snapshot.hostname.is_some() {
        snapshot.hostname = Some("host".to_string());
    }
    if let Some(os_name) = snapshot.os_name.take() {
        snapshot.os_name = Some(redact_text(&os_name, ids));
    }
    if let Some(kernel_version) = snapshot.kernel_version.take() {
        snapshot.kernel_version = Some(redact_text(&kernel_version, ids));
    }
    for process in &mut snapshot.processes {
        if process.user.is_some() {
            process.user = Some("user".to_string());
        }
        process.command = redact_text(&process.command, ids);
        process.name = redact_text(&process.name, ids);
    }
    for interface in &mut snapshot.network.interfaces {
        interface.name = redact_text(&interface.name, ids);
    }
    // Mount points can live under the user's home.
    for disk in &mut snapshot.disks {
        disk.mount_point = redact_text(&disk.mount_point, ids);
    }
    for component in &mut snapshot.components {
        component.label = redact_text(&component.label, ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_home_host_and_tailnet() {
        let ids = RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "my-host",
        };
        let value = "/home/alice/work https://my-host.tail1234.ts.net:8443 alice";
        assert_eq!(
            redact_text(value, &ids),
            "/home/user/work https://host:8443 user"
        );
    }

    #[test]
    fn user_inside_hostname_does_not_break_redaction() {
        // A short user name must not rewrite the longer host name first.
        let ids = RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "alice-laptop",
        };
        let value = "https://alice-laptop.tail1234.ts.net alice java";
        let out = redact_text(value, &ids);
        assert!(!out.contains("alice"), "{out}");
        assert!(!out.contains("user-laptop"), "{out}");
        // Pin the complete output: "java" contains a substring of the user
        // name and must survive. Both the full hostname ("alice-laptop") and
        // the short host ("alice") map to "host", as does the user name.
        assert_eq!(out, "https://host host java");
    }

    #[test]
    fn short_and_empty_identifiers_are_ignored() {
        let ids = RedactionIdentifiers {
            home: "/a",
            user: "a",
            hostname: "",
        };
        assert_eq!(redact_text("java cache /app", &ids), "java cache /app");

        let empty = RedactionIdentifiers {
            home: "",
            user: "",
            hostname: "",
        };
        assert_eq!(redact_text("hello world", &empty), "hello world");
    }

    #[test]
    fn minimum_length_guards_are_exact() {
        let ids =
            |home: &'static str, user: &'static str, hostname: &'static str| RedactionIdentifiers {
                home,
                user,
                hostname,
            };
        // home / hostname: values shorter than 3 chars stay, 3+ are replaced.
        // "/ab" is 3 chars including the leading slash, so it is replaced.
        assert_eq!(redact_text("/a", &ids("/a", "", "")), "/a");
        assert_eq!(redact_text("/ab", &ids("/ab", "", "")), "/home/user");
        assert_eq!(redact_text("ab", &ids("", "", "ab")), "ab");
        assert_eq!(redact_text("abc", &ids("", "", "abc")), "host");
        // user: 1 char stays, 2 chars are replaced.
        assert_eq!(redact_text("java", &ids("", "a", "")), "java");
        assert_eq!(redact_text("ab-ab", &ids("", "ab", "")), "user-user");
        // tail token: 5 hex digits stay, 6 are replaced.
        assert_eq!(redact_text("tail12345", &ids("", "", "")), "tail12345");
        assert_eq!(redact_text("tail123456", &ids("", "", "")), "tailnet");
    }

    #[test]
    fn truncated_tailnet_fragments_are_redacted() {
        let ids = RedactionIdentifiers {
            home: "",
            user: "",
            hostname: "my-host",
        };
        assert_eq!(
            redact_text("https://host.tail1234.t… --id=tail1234abcd", &ids),
            "https://host… --id=tailnet"
        );
    }

    #[test]
    fn non_ascii_label_before_ts_net_does_not_panic() {
        // `rfind` returns a byte index; "あ.ts.net" used to slice inside the
        // multibyte character and panic (R29 core audit).
        // The character immediately before ".ts.net" is kept (it is not a
        // delimiter); the important part is that slicing never panics.
        for (input, expected) in [
            ("echo \"あ.ts.net\"", "echo \"あhost\""),
            ("https://ホスト.ts.net/path", "https://ホストhost/path"),
        ] {
            assert_eq!(
                redact_text(
                    input,
                    &RedactionIdentifiers {
                        home: "",
                        user: "",
                        hostname: "",
                    }
                ),
                expected,
                "{input}"
            );
        }
    }

    /// Identifiers that must never appear in a redacted snapshot (R4-OB01).
    const SECRETS: [&str; 5] = ["alice", "my-host", "/home/alice", "tail1234", "ts.net"];

    fn secret_ids() -> RedactionIdentifiers<'static> {
        RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "my-host",
        }
    }

    /// Fills every display string with the identifiers from [`SECRETS`].
    fn secret_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            hostname: Some("my-host".into()),
            os_name: Some("Linux on my-host".into()),
            kernel_version: Some("6.11-my-host".into()),
            processes: vec![crate::model::ProcessSnapshot {
                name: "alice-worker".into(),
                command: "alice --home /home/alice https://my-host.tail1234.ts.net:8443".into(),
                user: Some("alice".into()),
                ..crate::model::ProcessSnapshot::default()
            }],
            network: crate::model::NetworkSnapshot {
                interfaces: vec![crate::model::NetworkInterfaceSnapshot {
                    name: "my-host-eth0".into(),
                    ..crate::model::NetworkInterfaceSnapshot::default()
                }],
            },
            disks: vec![crate::model::DiskUsageSnapshot {
                mount_point: "/home/alice/work".into(),
                ..crate::model::DiskUsageSnapshot::default()
            }],
            components: vec![crate::model::ComponentSnapshot {
                label: "my-host CPU Package".into(),
                temperature_c: Some(42.0),
                ..crate::model::ComponentSnapshot::default()
            }],
            ..SystemSnapshot::default()
        }
    }

    #[test]
    fn redact_snapshot_covers_every_display_string() {
        // Debug prints every field, so a new string field in SystemSnapshot that
        // is not redacted makes this test fail (same guard as the export tests).
        let mut snapshot = secret_snapshot();
        let ids = secret_ids();
        redact_snapshot(&mut snapshot, &ids);
        let dump = format!("{snapshot:?}");
        for secret in SECRETS {
            assert!(
                !dump.contains(secret),
                "redact_snapshot leaked {secret:?}: {dump}"
            );
        }
        // The agent and the exporters may both redact the same snapshot, so a
        // second pass must be a no-op.
        redact_snapshot(&mut snapshot, &ids);
        assert_eq!(
            format!("{snapshot:?}"),
            dump,
            "redact_snapshot must be idempotent"
        );
    }
}
