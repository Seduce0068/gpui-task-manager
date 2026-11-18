//! Pure CSV / JSON exporters for snapshots.
//!
//! Both exporters are pure functions: they take a [`SystemSnapshot`] and the
//! [`RedactionIdentifiers`] captured from the environment by the caller, and
//! return an owned `String`. There is no file I/O, no clock and no global state,
//! so this module also compiles for `wasm32-unknown-unknown`.
//!
//! The exporters deliberately **re-apply redaction to a private clone** instead
//! of trusting the caller to have called [`crate::redact::redact_snapshot`].
//! That makes an unredacted snapshot unable to leak local identifiers through
//! this module, even when a shell forgets its `--redact` flag. The JSON output
//! mirrors the agent's wire envelope (`schema_version`, `total_process_count`,
//! `snapshot`) without adding a `serde_json` dependency (see ADR-009); the CSV
//! output is one header row plus one row per process.

use std::fmt::Write as _;

use crate::model::{ProcessStatus, SystemSnapshot};
use crate::redact::{redact_snapshot, redact_text, RedactionIdentifiers};

/// Header row of the process CSV written by [`snapshot_to_csv`].
///
/// `sampled_at_unix_millis` is repeated on every row so several exports can be
/// concatenated into one time series without losing the sample time.
pub const PROCESS_CSV_HEADER: &str = "sampled_at_unix_millis,pid,start_time_secs,name,user,status,cpu_percent,memory_bytes,virtual_memory_bytes,threads,parent_pid,command";

/// Returns an owned, fully redacted copy of `snapshot`.
///
/// This is the exact data the exporters convert. [`crate::redact::redact_snapshot`]
/// covers the host name, process user/name/command and disk mount points, but
/// deliberately leaves network interface names untouched because
/// `is_virtual_interface()` classifies traffic by name. Classification has
/// already happened by the time a snapshot is exported, so the exporters also
/// mask the remaining display strings (OS name, kernel version, interface names
/// and sensor labels) on their private copy.
///
/// The input snapshot is never mutated and `ids` must be built from values
/// captured **before** any redaction (see [`RedactionIdentifiers`]).
pub fn redact_for_export(
    snapshot: &SystemSnapshot,
    ids: &RedactionIdentifiers<'_>,
) -> SystemSnapshot {
    let mut copy = snapshot.clone();
    redact_snapshot(&mut copy, ids);
    if let Some(os_name) = copy.os_name.take() {
        copy.os_name = Some(redact_text(&os_name, ids));
    }
    if let Some(kernel_version) = copy.kernel_version.take() {
        copy.kernel_version = Some(redact_text(&kernel_version, ids));
    }
    for interface in &mut copy.network.interfaces {
        interface.name = redact_text(&interface.name, ids);
    }
    for component in &mut copy.components {
        component.label = redact_text(&component.label, ids);
    }
    copy
}

/// Converts a snapshot into CSV using only redacted data.
///
/// The first line is [`PROCESS_CSV_HEADER`]; each following line describes one
/// process, in snapshot order. Text fields use RFC 4180 quoting (a field is
/// quoted when it contains a comma, quote or line break, and embedded quotes
/// are doubled) and lines end with `\n`. Text fields starting with `=`, `+`,
/// `-`, `@`, tab or carriage return get a leading apostrophe so a spreadsheet
/// cannot interpret an attacker-supplied command line as a formula. Leading
/// newline, space, VT and FF are neutralised too (spreadsheet importers may
/// skip them). Numeric fields are never neutralised this way.
pub fn snapshot_to_csv(snapshot: &SystemSnapshot, ids: &RedactionIdentifiers<'_>) -> String {
    let snap = redact_for_export(snapshot, ids);
    let mut out = String::with_capacity(PROCESS_CSV_HEADER.len() + 1 + snap.processes.len() * 192);
    out.push_str(PROCESS_CSV_HEADER);
    out.push('\n');
    for process in &snap.processes {
        push_display(&mut out, snap.sampled_at.unix_millis);
        out.push(',');
        push_display(&mut out, process.key.pid);
        out.push(',');
        push_display(&mut out, process.key.start_time_secs);
        out.push(',');
        out.push_str(&csv_text(&process.name));
        out.push(',');
        // A missing `Option` is an empty field on the wire, not the two
        // apostrophes that `csv_text` would emit for an empty string.
        if let Some(user) = &process.user {
            out.push_str(&csv_text(user));
        }
        out.push(',');
        out.push_str(csv_status(process.status));
        out.push(',');
        push_display(&mut out, crate::normalise_percent(process.cpu_percent));
        out.push(',');
        push_display(&mut out, process.memory_bytes);
        out.push(',');
        push_display(&mut out, process.virtual_memory_bytes);
        out.push(',');
        push_optional_csv(&mut out, process.threads);
        out.push(',');
        push_optional_csv(&mut out, process.parent_pid);
        out.push(',');
        out.push_str(&csv_text(&process.command));
        out.push('\n');
    }
    out
}

/// Converts a snapshot into JSON using only redacted data.
///
/// The result has the same envelope keys as `GET /api/v1/snapshot` and uses
/// [`crate::WIRE_SCHEMA_VERSION`]. Field names and enum spellings match serde's
/// representation of the model types, so consumers can parse both the live
/// endpoint and an exported file with the same struct. Non-finite `f32` values
/// are replaced with `0.0` because JSON has no `NaN` / `Infinity` literals.
pub fn snapshot_to_json(snapshot: &SystemSnapshot, ids: &RedactionIdentifiers<'_>) -> String {
    let snap = redact_for_export(snapshot, ids);
    let mut out = String::with_capacity(2_048 + snap.processes.len() * 512);

    out.push_str("{\n");
    push_key(&mut out, 1, "schema_version");
    push_display(&mut out, crate::WIRE_SCHEMA_VERSION);
    out.push_str(",\n");
    push_key(&mut out, 1, "total_process_count");
    push_display(&mut out, snap.processes.len());
    out.push_str(",\n");
    push_key(&mut out, 1, "snapshot");
    out.push_str("{\n");

    // `sampled_at` is an object on the wire, not a bare integer.
    push_key(&mut out, 2, "sampled_at");
    out.push_str("{\n");
    push_key(&mut out, 3, "unix_millis");
    push_display(&mut out, snap.sampled_at.unix_millis);
    out.push('\n');
    push_indent(&mut out, 2);
    out.push_str("},\n");

    push_key(&mut out, 2, "hostname");
    push_json_opt_string(&mut out, snap.hostname.as_deref());
    out.push_str(",\n");
    push_key(&mut out, 2, "os_name");
    push_json_opt_string(&mut out, snap.os_name.as_deref());
    out.push_str(",\n");
    push_key(&mut out, 2, "kernel_version");
    push_json_opt_string(&mut out, snap.kernel_version.as_deref());
    out.push_str(",\n");

    push_key(&mut out, 2, "cpu");
    out.push_str("{\n");
    push_key(&mut out, 3, "total_percent");
    push_f32(&mut out, snap.cpu.total_percent);
    out.push_str(",\n");
    push_key(&mut out, 3, "per_core_percent");
    push_json_f32_array(&mut out, &snap.cpu.per_core_percent);
    out.push_str(",\n");
    push_key(&mut out, 3, "load_average");
    push_json_f32_array(&mut out, &snap.cpu.load_average);
    out.push_str(",\n");
    push_key(&mut out, 3, "uptime_secs");
    push_display(&mut out, snap.cpu.uptime_secs);
    out.push_str(",\n");
    push_key(&mut out, 3, "frequency_mhz");
    push_optional(&mut out, snap.cpu.frequency_mhz, push_display);
    out.push('\n');
    push_indent(&mut out, 2);
    out.push_str("},\n");

    push_key(&mut out, 2, "memory");
    out.push_str("{\n");
    push_key(&mut out, 3, "total_bytes");
    push_display(&mut out, snap.memory.total_bytes);
    out.push_str(",\n");
    push_key(&mut out, 3, "used_bytes");
    push_display(&mut out, snap.memory.used_bytes);
    out.push_str(",\n");
    push_key(&mut out, 3, "available_bytes");
    push_display(&mut out, snap.memory.available_bytes);
    out.push_str(",\n");
    push_key(&mut out, 3, "swap_total_bytes");
    push_display(&mut out, snap.memory.swap_total_bytes);
    out.push_str(",\n");
    push_key(&mut out, 3, "swap_used_bytes");
    push_display(&mut out, snap.memory.swap_used_bytes);
    out.push('\n');
    push_indent(&mut out, 2);
    out.push_str("},\n");

    push_key(&mut out, 2, "processes");
    if snap.processes.is_empty() {
        out.push_str("[],\n");
    } else {
        out.push_str("[\n");
        let last = snap.processes.len() - 1;
        for (index, process) in snap.processes.iter().enumerate() {
            push_indent(&mut out, 3);
            out.push_str("{\n");
            push_key(&mut out, 4, "key");
            out.push_str("{\n");
            push_key(&mut out, 5, "pid");
            push_display(&mut out, process.key.pid);
            out.push_str(",\n");
            push_key(&mut out, 5, "start_time_secs");
            push_display(&mut out, process.key.start_time_secs);
            out.push('\n');
            push_indent(&mut out, 4);
            out.push_str("},\n");

            push_key(&mut out, 4, "name");
            push_json_string(&mut out, &process.name);
            out.push_str(",\n");
            push_key(&mut out, 4, "command");
            push_json_string(&mut out, &process.command);
            out.push_str(",\n");
            push_key(&mut out, 4, "user");
            push_json_opt_string(&mut out, process.user.as_deref());
            out.push_str(",\n");
            push_key(&mut out, 4, "status");
            push_json_string(&mut out, json_status(process.status));
            out.push_str(",\n");
            push_key(&mut out, 4, "cpu_percent");
            push_f32(&mut out, process.cpu_percent);
            out.push_str(",\n");
            push_key(&mut out, 4, "memory_bytes");
            push_display(&mut out, process.memory_bytes);
            out.push_str(",\n");
            push_key(&mut out, 4, "virtual_memory_bytes");
            push_display(&mut out, process.virtual_memory_bytes);
            out.push_str(",\n");
            push_key(&mut out, 4, "threads");
            push_optional(&mut out, process.threads, push_display);
            out.push_str(",\n");
            push_key(&mut out, 4, "parent_pid");
            push_optional(&mut out, process.parent_pid, push_display);
            out.push('\n');

            push_indent(&mut out, 3);
            out.push('}');
            if index != last {
                out.push(',');
            }
            out.push('\n');
        }
        push_indent(&mut out, 2);
        out.push_str("],\n");
    }

    push_key(&mut out, 2, "network");
    out.push_str("{\n");
    push_key(&mut out, 3, "interfaces");
    if snap.network.interfaces.is_empty() {
        out.push_str("[]\n");
    } else {
        out.push_str("[\n");
        let last = snap.network.interfaces.len() - 1;
        for (index, interface) in snap.network.interfaces.iter().enumerate() {
            push_indent(&mut out, 4);
            out.push_str("{\n");
            push_key(&mut out, 5, "name");
            push_json_string(&mut out, &interface.name);
            out.push_str(",\n");
            push_key(&mut out, 5, "received_bytes");
            push_display(&mut out, interface.received_bytes);
            out.push_str(",\n");
            push_key(&mut out, 5, "transmitted_bytes");
            push_display(&mut out, interface.transmitted_bytes);
            out.push('\n');
            push_indent(&mut out, 4);
            out.push('}');
            if index != last {
                out.push(',');
            }
            out.push('\n');
        }
        push_indent(&mut out, 3);
        out.push_str("]\n");
    }
    push_indent(&mut out, 2);
    out.push_str("},\n");

    push_key(&mut out, 2, "disks");
    push_json_object_array(&mut out, 2, snap.disks.len(), |out, index| {
        let disk = &snap.disks[index];
        push_key(out, 4, "mount_point");
        push_json_string(out, &disk.mount_point);
        out.push_str(",\n");
        push_key(out, 4, "total_bytes");
        push_display(out, disk.total_bytes);
        out.push_str(",\n");
        push_key(out, 4, "available_bytes");
        push_display(out, disk.available_bytes);
        out.push_str(",\n");
        push_key(out, 4, "used_bytes");
        push_display(out, disk.used_bytes);
        out.push('\n');
    });
    out.push_str(",\n");

    push_key(&mut out, 2, "components");
    push_json_object_array(&mut out, 2, snap.components.len(), |out, index| {
        let component = &snap.components[index];
        push_key(out, 4, "label");
        push_json_string(out, &component.label);
        out.push_str(",\n");
        push_key(out, 4, "temperature_c");
        push_optional(out, component.temperature_c, push_f32);
        out.push_str(",\n");
        push_key(out, 4, "max_c");
        push_optional(out, component.max_c, push_f32);
        out.push_str(",\n");
        push_key(out, 4, "critical_c");
        push_optional(out, component.critical_c, push_f32);
        out.push('\n');
    });
    out.push_str(",\n");

    push_key(&mut out, 2, "self_pid");
    push_optional(&mut out, snap.self_pid, push_display);
    out.push('\n');

    push_indent(&mut out, 1);
    out.push_str("}\n}\n");
    out
}

// --- internal helpers -------------------------------------------------------

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_display<T: std::fmt::Display>(out: &mut String, value: T) {
    write!(out, "{value}").expect("writing to a String cannot fail");
}

fn push_f32(out: &mut String, value: f32) {
    push_display(out, crate::normalise_percent(value));
}

fn push_optional<T>(out: &mut String, value: Option<T>, push: fn(&mut String, T)) {
    match value {
        Some(value) => push(out, value),
        None => out.push_str("null"),
    }
}

fn push_key(out: &mut String, depth: usize, key: &str) {
    push_indent(out, depth);
    push_json_string(out, key);
    out.push_str(": ");
}

fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if ch.is_control() => {
                write!(out, "\\u{:04x}", ch as u32).expect("writing to a String cannot fail");
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn push_json_opt_string(out: &mut String, value: Option<&str>) {
    match value {
        Some(value) => push_json_string(out, value),
        None => out.push_str("null"),
    }
}

fn push_json_f32_array(out: &mut String, values: &[f32]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_f32(out, *value);
    }
    out.push(']');
}

/// Writes one JSON object per element, with the caller filling the fields at
/// field-indent level 4 inside an array opened at `parent_depth`.
fn push_json_object_array(
    out: &mut String,
    parent_depth: usize,
    len: usize,
    write_fields: impl Fn(&mut String, usize),
) {
    if len == 0 {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for index in 0..len {
        push_indent(out, parent_depth + 1);
        out.push_str("{\n");
        write_fields(out, index);
        push_indent(out, parent_depth + 1);
        out.push('}');
        if index + 1 != len {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, parent_depth);
    out.push(']');
}

/// Serde serialises unit enum variants by their Rust name on the wire.
fn json_status(status: ProcessStatus) -> &'static str {
    match status {
        ProcessStatus::Running => "Running",
        ProcessStatus::Sleeping => "Sleeping",
        ProcessStatus::Idle => "Idle",
        ProcessStatus::Stopped => "Stopped",
        ProcessStatus::Zombie => "Zombie",
        ProcessStatus::Tracing => "Tracing",
        ProcessStatus::Unknown => "Unknown",
    }
}

/// CSV uses the stable lowercase identifier instead of the serde variant name.
fn csv_status(status: ProcessStatus) -> &'static str {
    status.id()
}

fn csv_text(value: &str) -> String {
    // A leading newline/space (or VT/FF) can be ignored by spreadsheet
    // importers, exposing the next character as a formula. The apostrophe must
    // go *inside* the RFC 4180 quotes: placing it before the opening quote
    // makes parsers treat the field as unquoted, so commas/newlines split it
    // and a later field can start with '=' (formula-injection bypass, R29).
    let neutralise = matches!(
        value.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r' | '\n' | ' ' | '\u{0b}' | '\u{0c}')
    );
    let quoted = value
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b',' | b'"' | b'\n' | b'\r'));
    if quoted {
        // Build the quoted field first, then insert the apostrophe right after
        // the opening quote so the whole value stays one field.
        let mut escaped = String::with_capacity(value.len() + 2);
        if neutralise {
            escaped.push('\'');
        }
        for ch in value.chars() {
            if ch == '"' {
                escaped.push('"');
            }
            escaped.push(ch);
        }
        format!("\"{escaped}\"")
    } else if neutralise {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

fn push_optional_csv<T: std::fmt::Display>(out: &mut String, value: Option<T>) {
    if let Some(value) = value {
        push_display(out, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ComponentSnapshot, DiskUsageSnapshot, NetworkInterfaceSnapshot, NetworkSnapshot,
        ProcessKey, Timestamp,
    };

    /// Identifiers that must never appear in an exported file.
    const SECRETS: [&str; 5] = ["alice", "my-host", "/home/alice", "tail1234", "ts.net"];

    fn secret_ids() -> RedactionIdentifiers<'static> {
        RedactionIdentifiers {
            home: "/home/alice",
            user: "alice",
            hostname: "my-host",
        }
    }

    fn secret_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            sampled_at: Timestamp::from_millis(1_700_000_000_123),
            hostname: Some("my-host".into()),
            os_name: Some("Linux on my-host".into()),
            kernel_version: Some("6.11-my-host".into()),
            processes: vec![crate::model::ProcessSnapshot {
                key: ProcessKey::new(4242, 1_700_000_000),
                name: "alice-worker".into(),
                command: "alice --home /home/alice https://my-host.tail1234.ts.net:8443".into(),
                user: Some("alice".into()),
                status: ProcessStatus::Running,
                cpu_percent: f32::NAN,
                memory_bytes: 1_024,
                virtual_memory_bytes: 2_048,
                threads: Some(4),
                parent_pid: None,
            }],
            network: NetworkSnapshot {
                interfaces: vec![NetworkInterfaceSnapshot {
                    name: "my-host-eth0".into(),
                    received_bytes: 10,
                    transmitted_bytes: 20,
                }],
            },
            disks: vec![DiskUsageSnapshot {
                mount_point: "/home/alice/work".into(),
                total_bytes: 100,
                available_bytes: 40,
                used_bytes: 60,
            }],
            components: vec![ComponentSnapshot {
                label: "my-host CPU Package".into(),
                temperature_c: Some(42.0),
                max_c: None,
                critical_c: Some(100.0),
            }],
            self_pid: Some(4242),
            ..SystemSnapshot::default()
        }
    }

    /// Minimal well-formedness check for the JSON subset this module emits.
    /// Catches unbalanced containers, unterminated strings, raw control
    /// characters and stray identifiers such as `NaN`.
    fn assert_well_formed_json(input: &str) {
        let mut stack: Vec<char> = Vec::new();
        let mut in_string = false;
        let mut escaped = false;
        for ch in input.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else {
                    match ch {
                        '\\' => escaped = true,
                        '"' => in_string = false,
                        c if c.is_control() => panic!("raw control character in JSON: {c:?}"),
                        _ => {}
                    }
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '{' | '[' => stack.push(ch),
                '}' => assert_eq!(stack.pop(), Some('{'), "unbalanced '}}'"),
                ']' => assert_eq!(stack.pop(), Some('['), "unbalanced ']'"),
                c if c.is_whitespace() => {}
                c if c.is_ascii_alphanumeric() || ".,:+-_".contains(c) => {}
                other => panic!("unexpected character in JSON: {other:?}"),
            }
        }
        assert!(!in_string, "unterminated JSON string");
        assert!(stack.is_empty(), "unclosed JSON containers: {stack:?}");
        for forbidden in ["NaN", "Infinity", "-Infinity"] {
            assert!(
                !input.contains(forbidden),
                "invalid JSON literal {forbidden}"
            );
        }
    }

    #[test]
    fn redact_for_export_covers_every_field_including_future_ones() {
        // Debug prints every field, so adding a new string field to
        // SystemSnapshot without redacting it fails here (R30 export audit).
        let redacted = redact_for_export(&secret_snapshot(), &secret_ids());
        let dump = format!("{redacted:?}");
        for secret in SECRETS {
            assert!(!dump.contains(secret), "redact_for_export leaked {secret:?}");
        }
    }

    #[test]
    fn csv_export_redacts_every_display_string() {
        let csv = snapshot_to_csv(&secret_snapshot(), &secret_ids());
        for secret in SECRETS {
            assert!(!csv.contains(secret), "CSV leaked {secret:?}: {csv}");
        }
        assert!(csv.contains("host"), "{csv}");
        assert!(csv.contains("user"), "{csv}");
        assert!(csv.contains("running"), "{csv}");
    }

    #[test]
    fn json_export_redacts_every_display_string() {
        let json = snapshot_to_json(&secret_snapshot(), &secret_ids());
        for secret in SECRETS {
            assert!(!json.contains(secret), "JSON leaked {secret:?}: {json}");
        }
        assert!(json.contains("\"hostname\": \"host\""), "{json}");
        assert!(json.contains("\"user\": \"user\""), "{json}");
        assert!(json.contains("\"status\": \"Running\""), "{json}");
        assert_well_formed_json(&json);
    }

    #[test]
    fn redaction_is_applied_even_to_an_already_redacted_snapshot() {
        let mut snapshot = secret_snapshot();
        let ids = secret_ids();
        crate::redact::redact_snapshot(&mut snapshot, &ids);
        let csv = snapshot_to_csv(&snapshot, &ids);
        let json = snapshot_to_json(&snapshot, &ids);
        for output in [&csv, &json] {
            for secret in SECRETS {
                assert!(!output.contains(secret), "leaked {secret:?}: {output}");
            }
        }
    }

    #[test]
    fn exporters_are_pure_and_deterministic() {
        let snapshot = secret_snapshot();
        let original = snapshot.clone();
        let ids = secret_ids();
        assert_eq!(
            snapshot_to_csv(&snapshot, &ids),
            snapshot_to_csv(&snapshot, &ids)
        );
        assert_eq!(
            snapshot_to_json(&snapshot, &ids),
            snapshot_to_json(&snapshot, &ids)
        );
        // `SystemSnapshot` compares floats with `PartialEq`, so NaN != NaN;
        // compare the debug representation to check for structural mutation.
        assert_eq!(
            format!("{snapshot:?}"),
            format!("{original:?}"),
            "export must not mutate its input"
        );
    }

    #[test]
    fn csv_escapes_fields_and_neutralises_spreadsheet_formulas() {
        let mut snapshot = SystemSnapshot::default();
        snapshot.processes.push(crate::model::ProcessSnapshot {
            name: "a,b\"c".into(),
            command: "=cmd|' /C calc'!A0\r\nnext".into(),
            ..crate::model::ProcessSnapshot::default()
        });
        let csv = snapshot_to_csv(&snapshot, &secret_ids());
        // Header terminator, the quoted CRLF inside the command, and the row
        // terminator: three LFs total. The CRLF must remain inside one field.
        assert_eq!(
            csv.matches('\n').count(),
            3,
            "a command newline must stay quoted: {csv}"
        );
        assert!(csv.contains("\"a,b\"\"c\""), "{csv}");
        // The apostrophe sits *inside* the RFC 4180 quotes so a comma or
        // newline in the value cannot split the field (R29 export audit).
        assert!(csv.contains("\"'=cmd|' /C calc'!A0\r\nnext\""), "{csv}");
        // Empty Option fields stay empty instead of inventing a value.
        assert!(csv.contains("0,0,0,,,\"'=cmd|'"), "{csv}");
    }

    #[test]
    fn csv_neutralising_apostrophe_stays_inside_quotes() {
        // If the apostrophe precedes the opening quote, RFC 4180 parsers treat
        // the field as unquoted and a comma splits it into a formula field.
        let mut snapshot = SystemSnapshot::default();
        snapshot.processes.push(crate::model::ProcessSnapshot {
            command: "=a,=1+1".into(),
            ..crate::model::ProcessSnapshot::default()
        });
        let csv = snapshot_to_csv(&snapshot, &secret_ids());
        let row = csv.lines().nth(1).expect("data row");
        assert!(row.contains("\"'=a,=1+1\""), "{row}");
        // 12 columns + 1 embedded comma => 12 commas; a split field adds one.
        assert_eq!(row.matches(',').count(), 12, "field must not split: {row}");
    }

    #[test]
    fn csv_neutralises_leading_whitespace_and_newline_before_formulas() {
        // A leading newline or space can be ignored by a spreadsheet importer,
        // exposing the '=' that follows as a formula (R28 export audit).
        for value in ["\n=1+1", " =1+1", "\u{0b}=1+1", "\u{0c}=1+1"] {
            let mut snapshot = SystemSnapshot::default();
            snapshot.processes.push(crate::model::ProcessSnapshot {
                command: value.into(),
                ..crate::model::ProcessSnapshot::default()
            });
            let csv = snapshot_to_csv(&snapshot, &secret_ids());
            let row = csv.lines().nth(1).expect("data row");
            assert!(
                row.contains("'"),
                "leading whitespace must be neutralised: {row:?}"
            );
            assert!(
                !row.contains(",\n="),
                "formula must not start a row: {row:?}"
            );
        }
    }

    #[test]
    fn json_escapes_control_characters_and_normalises_non_finite_numbers() {
        let mut snapshot = SystemSnapshot::default();
        snapshot.processes.push(crate::model::ProcessSnapshot {
            command: "quote \" backslash \\ tab \t newline \n".into(),
            cpu_percent: f32::INFINITY,
            memory_bytes: 7,
            ..crate::model::ProcessSnapshot::default()
        });
        snapshot.cpu.total_percent = f32::NAN;
        let json = snapshot_to_json(&snapshot, &secret_ids());
        assert_well_formed_json(&json);
        assert!(
            json.contains("quote \\\" backslash \\\\ tab \\t newline \\n"),
            "{json}"
        );
        assert!(json.contains("\"cpu_percent\": 0"), "{json}");
        assert!(json.contains("\"total_percent\": 0"), "{json}");
    }

    #[test]
    fn empty_snapshot_still_produces_usable_documents() {
        let snapshot = SystemSnapshot::default();
        let csv = snapshot_to_csv(&snapshot, &secret_ids());
        assert_eq!(csv, format!("{PROCESS_CSV_HEADER}\n"));
        let json = snapshot_to_json(&snapshot, &secret_ids());
        assert_well_formed_json(&json);
        assert!(json.contains("\"processes\": []"), "{json}");
        assert!(json.contains("\"interfaces\": []"), "{json}");
        assert!(json.contains("\"disks\": []"), "{json}");
        assert!(json.contains("\"components\": []"), "{json}");
        assert!(json.contains("\"self_pid\": null"), "{json}");
        assert!(json.contains("\"schema_version\": 1"), "{json}");
        assert!(json.contains("\"total_process_count\": 0"), "{json}");
    }

    #[test]
    fn json_matches_the_wire_envelope_shape() {
        let json = snapshot_to_json(&secret_snapshot(), &secret_ids());
        for key in [
            "\"schema_version\"",
            "\"total_process_count\"",
            "\"snapshot\"",
            "\"sampled_at\"",
            "\"unix_millis\"",
            "\"cpu\"",
            "\"per_core_percent\"",
            "\"load_average\"",
            "\"memory\"",
            "\"network\"",
            "\"interfaces\"",
            "\"disks\"",
            "\"components\"",
            "\"key\"",
            "\"start_time_secs\"",
            "\"threads\"",
            "\"parent_pid\"",
        ] {
            assert!(json.contains(key), "missing wire key {key}: {json}");
        }
        assert!(json.contains("\"threads\": 4"), "{json}");
        assert!(json.contains("\"parent_pid\": null"), "{json}");
        assert!(json.contains("\"max_c\": null"), "{json}");
    }
}
