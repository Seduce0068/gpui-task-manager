//! Pure helpers for redacting local identifiers from display strings.
//!
//! Both the native shell (`--redact`) and the agent's snapshot endpoint promise
//! to hide every identifier-bearing display string: host name, OS name, kernel
//! version, user names, home paths, process names and commands, mount points,
//! interface names and sensor labels (plus tailnet domains). Keeping the rules
//! here (rather than duplicating them in each shell) prevents the snapshot and
//! export paths from drifting. Reading the environment is deliberately left to
//! the callers: `tm-core` stays free of I/O and works on `wasm32`.

use crate::model::{group_by_cgroup, ProcessCgroup, SystemSnapshot};

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

/// Byte ranges in `value` whose text equals `needle`, ignoring case.
///
/// Matching runs on a lowercased working copy so identifiers that differ only
/// in case (`MY-HOST` vs `my-host`) are found too. The ranges are mapped back
/// onto `value`'s byte offsets: lowercasing can change a character's byte
/// length (e.g. `İ` becomes `i` plus a combining dot), so using the working
/// copy's offsets directly could split a UTF-8 code point in the original
/// string.
fn case_insensitive_ranges(value: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let needle_lower = needle.to_lowercase();
    let lowered = value.to_lowercase();

    // For every byte of `lowered`, keep the original character's start and end
    // byte offsets. `str::to_lowercase` concatenates the per-character
    // lowercase mappings, so the two byte streams stay index-aligned.
    let mut starts = Vec::with_capacity(lowered.len());
    let mut ends = Vec::with_capacity(lowered.len());
    let mut original_offset = 0usize;
    for ch in value.chars() {
        let start = original_offset;
        let end = start + ch.len_utf8();
        original_offset = end;
        for lowered_ch in ch.to_lowercase() {
            for _ in 0..lowered_ch.len_utf8() {
                starts.push(start);
                ends.push(end);
            }
        }
    }
    debug_assert_eq!(starts.len(), lowered.len());
    debug_assert_eq!(ends.len(), lowered.len());

    lowered
        .match_indices(&needle_lower)
        .map(|(start, matched)| (starts[start], ends[start + matched.len() - 1]))
        .collect()
}

/// Replaces every case-insensitive occurrence of `needle` in `value` with
/// `replacement`, leaving all other bytes (including multibyte characters)
/// at their original positions.
fn replace_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut cursor = 0usize;
    for (start, end) in case_insensitive_ranges(value, needle) {
        if start < cursor {
            // Overlapping matches: keep the earliest replacement only.
            continue;
        }
        out.push_str(&value[cursor..start]);
        out.push_str(replacement);
        cursor = end;
    }
    out.push_str(&value[cursor..]);
    out
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
    while let Some(&(position, end)) = case_insensitive_ranges(rest, SUFFIX).first() {
        let before = &rest[..position];
        // The match position is a byte index; advancing by 1 byte can land inside a
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
        rest = &rest[end..];
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
    while let Some(&(position, end)) = case_insensitive_ranges(rest, "host.").first() {
        out.push_str(&rest[..position]);
        out.push_str("host");
        let suffix = &rest[end..];
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
    while let Some(&(position, end)) = case_insensitive_ranges(rest, "tail").first() {
        out.push_str(&rest[..position]);
        let candidate = &rest[end..];
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
/// a case-insensitive replacement would corrupt unrelated words (`a` in
/// `java`). Matching ignores case, so `MY-HOST` is hidden just like `my-host`.
pub fn redact_text(value: &str, ids: &RedactionIdentifiers<'_>) -> String {
    let mut out = value.to_string();
    if ids.home.chars().count() >= 3 {
        out = replace_case_insensitive(&out, ids.home, "/home/user");
    }
    // The first label of the host name is also used in service names; derive
    // it here so every caller applies the same rule.
    let short_host = ids.hostname.split(['.', '-']).next().unwrap_or("");
    if ids.hostname.chars().count() >= 3 && !ids.hostname.is_empty() {
        out = replace_case_insensitive(&out, ids.hostname, "host");
    }
    if short_host.chars().count() >= 3 {
        out = replace_case_insensitive(&out, short_host, "host");
    }
    if !ids.user.is_empty() && ids.user.chars().count() >= 2 {
        out = replace_case_insensitive(&out, ids.user, "user");
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
    // Battery names come from sysfs and cgroup labels are derived from /proc
    // paths; both can embed a user name or host-specific identifier. Neither is
    // used for classification, so they are redacted like any other display text
    // (R1-I10 / R1-I12).
    for battery in &mut snapshot.power.batteries {
        battery.name = redact_text(&battery.name, ids);
    }
    for group in &mut snapshot.cgroups {
        group.label = redact_text(&group.label, ids);
    }

    // Redaction can rewrite two distinct names/labels into the same text (for
    // example `host.slice/app.service` loses `slice.` when `redact_host_subdomains`
    // consumes `host.`), and it can reorder names by replacing their leading
    // identifier. Re-establish the probe's invariants on the *redacted* values
    // so exported snapshots stay unique, sorted and deterministic (R2-DATA).
    snapshot
        .power
        .batteries
        .sort_by(|left, right| left.name.cmp(&right.name));
    let memberships = snapshot
        .cgroups
        .iter()
        .flat_map(|group| {
            group.members.iter().map(move |key| ProcessCgroup {
                key: *key,
                label: group.label.clone(),
            })
        })
        .collect::<Vec<_>>();
    snapshot.cgroups = group_by_cgroup(&memberships);
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
    fn redacts_battery_names_and_cgroup_labels() {
        use crate::model::{BatterySnapshot, CgroupGroup};

        let ids = RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "my-host",
        };
        let mut snapshot = SystemSnapshot {
            power: crate::model::PowerSnapshot {
                source: crate::model::PowerSource::Battery,
                batteries: vec![BatterySnapshot {
                    name: "alice-my-host-BAT0".into(),
                    capacity_percent: Some(42),
                    state: crate::model::BatteryState::Discharging,
                }],
            },
            cgroups: vec![CgroupGroup {
                label: "/home/alice/my-host.slice/app.service".into(),
                members: vec![crate::model::ProcessKey::new(7, 1)],
            }],
            ..SystemSnapshot::default()
        };
        // The same membership must not create a second group after redaction.
        let before = snapshot.cgroups.len();
        redact_snapshot(&mut snapshot, &ids);
        assert_eq!(
            snapshot.cgroups.len(),
            before,
            "grouping shape must be kept"
        );
        assert_eq!(snapshot.power.batteries[0].name, "user-host-BAT0");
        assert!(!snapshot.cgroups[0].label.contains("alice"), "{snapshot:?}");
        assert!(
            !snapshot.cgroups[0].label.contains("my-host"),
            "{snapshot:?}"
        );
        assert_eq!(snapshot.power.batteries[0].capacity_percent, Some(42));
    }

    #[test]
    fn redaction_restores_battery_order_and_cgroup_uniqueness() {
        use crate::model::{BatterySnapshot, BatteryState, CgroupGroup, PowerSnapshot, ProcessKey};

        let ids = RedactionIdentifiers {
            home: "",
            user: "alice",
            hostname: "my-host",
        };
        // Before redaction the battery names are sorted ("alice-" < "my-host-").
        // After replacement they are not ("user-" > "host-"), so the post-pass
        // must re-sort instead of trusting the probe's original order.
        // The two `host...` labels collide once `host.` has been consumed by
        // `redact_host_subdomains`, so grouping must be rebuilt from the
        // redacted labels to keep one group per label with deduplicated members.
        let mut snapshot = SystemSnapshot {
            power: PowerSnapshot {
                source: crate::model::PowerSource::Battery,
                batteries: vec![
                    BatterySnapshot {
                        name: "alice-AAA".into(),
                        capacity_percent: Some(30),
                        state: BatteryState::Charging,
                    },
                    BatterySnapshot {
                        name: "my-host-BAT0".into(),
                        capacity_percent: Some(70),
                        state: BatteryState::Discharging,
                    },
                ],
            },
            cgroups: vec![
                CgroupGroup {
                    label: "host.slice/app.service".into(),
                    members: vec![ProcessKey::new(200, 20), ProcessKey::new(100, 10)],
                },
                CgroupGroup {
                    label: "host/app.service".into(),
                    members: vec![ProcessKey::new(100, 10), ProcessKey::new(300, 30)],
                },
                CgroupGroup {
                    label: "z.slice/app.service".into(),
                    members: vec![ProcessKey::new(400, 40)],
                },
            ],
            ..SystemSnapshot::default()
        };

        redact_snapshot(&mut snapshot, &ids);
        let names: Vec<_> = snapshot
            .power
            .batteries
            .iter()
            .map(|battery| battery.name.as_str())
            .collect();
        assert_eq!(names, ["host-BAT0", "user-AAA"]);
        assert_eq!(
            snapshot.power.batteries[0].capacity_percent,
            Some(70),
            "the battery payload must follow its re-sorted name"
        );

        let labels: Vec<_> = snapshot
            .cgroups
            .iter()
            .map(|group| group.label.as_str())
            .collect();
        assert_eq!(labels, ["host/app.service", "z.slice/app.service"]);
        assert_eq!(
            snapshot.cgroups[0].members,
            vec![
                ProcessKey::new(100, 10),
                ProcessKey::new(200, 20),
                ProcessKey::new(300, 30),
            ],
            "colliding groups must merge with sorted, deduplicated members"
        );

        // Re-applying redaction (as the export path does) is still a no-op.
        let before = format!("{:?}", snapshot.cgroups);
        redact_snapshot(&mut snapshot, &ids);
        assert_eq!(format!("{:?}", snapshot.cgroups), before);
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

    #[test]
    fn uppercase_identifiers_are_redacted() {
        let ids = RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "my-host",
        };
        let value = "/HOME/ALICE/work https://MY-HOST.TAIL1234.TS.NET:8443 ALICE";
        assert_eq!(
            redact_text(value, &ids),
            "/home/user/work https://host:8443 user"
        );
        // Mixed case is covered too, including the uppercase `.Ts.Net` suffix.
        let mixed = "/Home/Alice https://My-Host.TailAbCd.Ts.Net AlIcE";
        assert_eq!(redact_text(mixed, &ids), "/home/user https://host user");
        // A bare uppercase tail token goes through `redact_tailnet_tokens`.
        assert_eq!(
            redact_text("--id=TAIL1234ABCD --id=TaIlAbCdEf", &ids),
            "--id=tailnet --id=tailnet"
        );
    }

    #[test]
    fn case_insensitive_replacement_preserves_multibyte_prefix() {
        let ids = RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "my-host",
        };
        // Multibyte characters before the identifier must not shift the byte
        // offsets used for the replacement or split a UTF-8 code point.
        let value = "日本語 /HOME/ALICE/仕事 MY-HOST ALICE";
        assert_eq!(redact_text(value, &ids), "日本語 /home/user/仕事 host user");
    }

    #[test]
    fn lowercased_character_expansion_does_not_split_utf8() {
        // `İ` grows from 2 bytes to 3 when lowercased (`i` + combining dot), so
        // the working copy is longer than the original. Offsets must be mapped
        // back onto `value` or the replacement would split a UTF-8 code point.
        let ids = RedactionIdentifiers {
            home: "",
            user: "İA",
            hostname: "",
        };
        assert_eq!(redact_text("İA worker", &ids), "user worker");
        assert_eq!(redact_text("i\u{0307}a worker", &ids), "user worker");
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
