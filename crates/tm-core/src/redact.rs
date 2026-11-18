//! Pure helpers for redacting local identifiers from display strings.
//!
//! Both the native shell (`--redact`) and the agent's snapshot endpoint promise
//! to hide the host name, user names, home paths and tailnet domains. Keeping
//! the rules here (rather than duplicating them in each shell) prevents the two
//! paths from drifting. Reading the environment is deliberately left to the
//! callers: `tm-core` stays free of I/O and works on `wasm32`.

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

/// Redacts local identifiers from every display string in a snapshot.
///
/// Intended for the agent's "before sending" path, where the snapshot is owned.
/// Shells with an immutable view of the dashboard should use [`redact_text`]
/// on individual strings instead.
pub fn redact_snapshot(snapshot: &mut SystemSnapshot, ids: &RedactionIdentifiers<'_>) {
    if snapshot.hostname.is_some() {
        snapshot.hostname = Some("host".to_string());
    }
    for process in &mut snapshot.processes {
        if process.user.is_some() {
            process.user = Some("user".to_string());
        }
        process.command = redact_text(&process.command, ids);
        process.name = redact_text(&process.name, ids);
    }
    // Mount points can live under the user's home. Network interface names and
    // sensor labels are deliberately NOT touched: `is_virtual_interface()` uses
    // the name to classify traffic and would mis-count it after redaction.
    for disk in &mut snapshot.disks {
        disk.mount_point = redact_text(&disk.mount_point, ids);
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
        // An unrelated word containing a substring of the user name stays
        // intact (a tautological assertion here would verify nothing, R33).
        assert!(out.contains("java"), "short words untouched: {out}");
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
}
