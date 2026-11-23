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
//!
//! The envelope's `total_process_count` means the number of processes on the
//! whole machine **before** `?limit=N` truncated `snapshot.processes`. A caller
//! that owns the wire envelope passes that value explicitly, so an export
//! cannot silently shrink it to the number of published rows. A local probe has
//! no envelope and passes `None`, keeping the historical row-count behaviour.

use std::fmt::Write as _;

use crate::model::{BatteryState, PowerSource, ProcessStatus, SamplingProfile, SystemSnapshot};
use crate::redact::{redact_snapshot, RedactionIdentifiers};
use crate::sanitize::sanitize_snapshot;

/// Header row of the process CSV written by [`snapshot_to_csv`].
///
/// `sampled_at_unix_millis` is repeated on every row so several exports can be
/// concatenated into one time series without losing the sample time.
pub const PROCESS_CSV_HEADER: &str = "sampled_at_unix_millis,pid,start_time_secs,name,user,status,cpu_percent,memory_bytes,virtual_memory_bytes,threads,parent_pid,command";

/// Returns an owned, fully redacted copy of `snapshot`.
///
/// This is the exact data the exporters convert. [`crate::redact::redact_snapshot`]
/// is the single source of truth and now masks every identifier-bearing display
/// string: host name, OS name, kernel version, process user/name/command, mount
/// points, interface names and sensor labels. It applies
/// [`crate::redact::redact_text`] to each string field (including component
/// labels, battery names and cgroup labels).
/// Re-applying it to a private clone is therefore enough, with no field-specific
/// masking left here; the operation is idempotent if the caller already redacted
/// its snapshot.
///
/// [`crate::sanitize::sanitize_snapshot`] is applied to the same clone as a
/// second, defensive boundary: an exporter must not emit ANSI escapes or other
/// control characters even when the caller skipped sanitisation. Both passes
/// are idempotent, so re-applying them to a snapshot that was already redacted
/// and sanitised (as the agent shell does) is safe.
///
/// The input snapshot is never mutated and `ids` must be built from values
/// captured **before** any redaction (see [`RedactionIdentifiers`]).
pub fn redact_for_export(
    snapshot: &SystemSnapshot,
    ids: &RedactionIdentifiers<'_>,
) -> SystemSnapshot {
    let mut copy = snapshot.clone();
    redact_snapshot(&mut copy, ids);
    sanitize_snapshot(&mut copy);
    copy
}

/// Converts a snapshot into CSV using only redacted data.
///
/// The first line is [`PROCESS_CSV_HEADER`]; each following line describes one
/// process, in snapshot order. Text fields use RFC 4180 quoting (a field is
/// quoted when it contains a comma, quote or line break, and embedded quotes
/// are doubled) and lines end with `\n`. Text fields starting with a formula
/// character get a leading apostrophe so a spreadsheet cannot interpret an
/// attacker-supplied command line as a formula. Besides the four ASCII starters
/// (`=`, `+`, `-`, `@`) this includes their fullwidth / Unicode lookalikes
/// (e.g. `＝`, `＋`, `－`, `＠`, `−`); see is_formula_starter. Leading
/// newline, space, tab, carriage return, VT and FF are neutralised too
/// (spreadsheet importers may skip them). Numeric fields are never neutralised
/// this way.
///
/// `cpu_percent` is rounded to two decimal places **at serialisation time only**
/// (e.g. `4.3799996` becomes `4.38`); the snapshot's raw `f32` still drives
/// filtering and sorting, and the JSON exporter keeps wire-compatible numbers.
pub fn snapshot_to_csv(snapshot: &SystemSnapshot, ids: &RedactionIdentifiers<'_>) -> String {
    // A local caller has no wire envelope, so the published rows are the only
    // process count available.
    snapshot_to_csv_with_total(snapshot, ids, None)
}

/// CSV variant of [`snapshot_to_csv`] that keeps the wire envelope's
/// `total_process_count` in its signature for a uniform exporter API.
///
/// The CSV deliberately keeps its fixed 12-column process-row contract: the
/// envelope total describes the whole machine, not a row, so it is **not
/// written to the output** and changing this argument never changes the result
/// (hence `_ignored_wire_total`). This differs from
/// [`snapshot_to_json_with_total`], which preserves an explicit envelope total
/// verbatim. The argument exists only so a shell that owns a wire envelope can
/// call both exporters through one API; callers need no changes.
pub fn snapshot_to_csv_with_total(
    snapshot: &SystemSnapshot,
    ids: &RedactionIdentifiers<'_>,
    _ignored_wire_total: Option<u64>,
) -> String {
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
        push_csv_percent(&mut out, process.cpu_percent);
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
    // Backward-compatible entry point: no envelope is available here, so the
    // historical published-row fallback applies.
    snapshot_to_json_with_total(snapshot, ids, None)
}

/// JSON variant of [`snapshot_to_json`] that preserves an explicit wire total.
///
/// `total_process_count` is the machine-wide count from the agent's wire
/// envelope, captured before `?limit=N` truncated `snapshot.processes`. `Some`
/// is written through verbatim; `None` (a local probe with no envelope) falls
/// back to the number of published snapshot rows.
pub fn snapshot_to_json_with_total(
    snapshot: &SystemSnapshot,
    ids: &RedactionIdentifiers<'_>,
    total_process_count: Option<u64>,
) -> String {
    let snap = redact_for_export(snapshot, ids);
    let mut out = String::with_capacity(2_048 + snap.processes.len() * 512);

    out.push_str("{\n");
    push_key(&mut out, 1, "schema_version");
    push_display(&mut out, crate::WIRE_SCHEMA_VERSION);
    out.push_str(",\n");
    push_key(&mut out, 1, "total_process_count");
    // Wire semantics: the machine-wide process count *before* `?limit=N`
    // truncation. Keep an explicitly supplied envelope value verbatim; only a
    // local probe without an envelope falls back to the published row count.
    match total_process_count {
        Some(total) => push_display(&mut out, total),
        None => push_display(&mut out, snap.processes.len()),
    }
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

    // `std::time::Duration` is a `{ "secs": u64, "nanos": u32 }` object on the
    // wire, exactly as serde serialises `SystemSnapshot::sampled_elapsed`.
    push_key(&mut out, 2, "sampled_elapsed");
    out.push_str("{\n");
    push_key(&mut out, 3, "secs");
    push_display(&mut out, snap.sampled_elapsed.as_secs());
    out.push_str(",\n");
    push_key(&mut out, 3, "nanos");
    push_display(&mut out, snap.sampled_elapsed.subsec_nanos());
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
            out.push_str(",\n");
            push_key(&mut out, 4, "swap_bytes");
            push_display(&mut out, process.swap_bytes);
            out.push_str(",\n");
            // Cumulative block I/O in bytes, in the same order as the serde
            // fields on `ProcessSnapshot` (C1-IODATA). `rchar`/`wchar` are not
            // exported: only actual block-layer reads/writes are meaningful.
            push_key(&mut out, 4, "read_bytes");
            push_display(&mut out, process.read_bytes);
            out.push_str(",\n");
            push_key(&mut out, 4, "write_bytes");
            push_display(&mut out, process.write_bytes);
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
        push_optional(out, component.temperature_c, push_temperature);
        out.push_str(",\n");
        push_key(out, 4, "max_c");
        push_optional(out, component.max_c, push_temperature);
        out.push_str(",\n");
        push_key(out, 4, "critical_c");
        push_optional(out, component.critical_c, push_temperature);
        out.push('\n');
    });
    out.push_str(",\n");

    // Additive wire fields (R1-I10 / R1-I11 / R1-I12). The keys and enum
    // spellings mirror serde's representation of the model types, so a consumer
    // can parse both the live endpoint and an exported file with one struct.
    push_key(&mut out, 2, "power");
    out.push_str("{\n");
    push_key(&mut out, 3, "source");
    push_json_string(&mut out, json_power_source(snap.power.source));
    out.push_str(",\n");
    push_key(&mut out, 3, "batteries");
    push_json_object_array(&mut out, 3, snap.power.batteries.len(), |out, index| {
        let battery = &snap.power.batteries[index];
        push_key(out, 5, "name");
        push_json_string(out, &battery.name);
        out.push_str(",\n");
        push_key(out, 5, "capacity_percent");
        push_optional(out, battery.capacity_percent, push_display);
        out.push_str(",\n");
        push_key(out, 5, "state");
        push_json_string(out, json_battery_state(battery.state));
        out.push('\n');
    });
    out.push('\n');
    push_indent(&mut out, 2);
    out.push_str("},\n");

    push_key(&mut out, 2, "sampling");
    out.push_str("{\n");
    push_key(&mut out, 3, "interval_ms");
    push_display(&mut out, snap.sampling.interval_ms);
    out.push_str(",\n");
    push_key(&mut out, 3, "profile");
    push_json_string(&mut out, json_sampling_profile(snap.sampling.profile));
    out.push('\n');
    push_indent(&mut out, 2);
    out.push_str("},\n");

    push_key(&mut out, 2, "cgroups");
    push_json_object_array(&mut out, 2, snap.cgroups.len(), |out, index| {
        let group = &snap.cgroups[index];
        push_key(out, 4, "label");
        push_json_string(out, &group.label);
        out.push_str(",\n");
        push_key(out, 4, "members");
        push_json_object_array(out, 4, group.members.len(), |out, member_index| {
            let key = &group.members[member_index];
            push_key(out, 6, "pid");
            push_display(out, key.pid);
            out.push_str(",\n");
            push_key(out, 6, "start_time_secs");
            push_display(out, key.start_time_secs);
            out.push('\n');
        });
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

/// Temperatures are not percentages: do not clamp them to 0..100.
fn push_temperature(out: &mut String, value: f32) {
    push_display(out, crate::normalise_temperature(value));
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

/// Serde serialises unit enum variants by their Rust name on the wire.
fn json_power_source(source: PowerSource) -> &'static str {
    match source {
        PowerSource::Unknown => "Unknown",
        PowerSource::Ac => "Ac",
        PowerSource::Battery => "Battery",
    }
}

/// Serde serialises unit enum variants by their Rust name on the wire.
fn json_battery_state(state: BatteryState) -> &'static str {
    match state {
        BatteryState::Unknown => "Unknown",
        BatteryState::Charging => "Charging",
        BatteryState::Discharging => "Discharging",
        BatteryState::Full => "Full",
        BatteryState::NotCharging => "NotCharging",
    }
}

/// Serde serialises unit enum variants by their Rust name on the wire.
fn json_sampling_profile(profile: SamplingProfile) -> &'static str {
    match profile {
        SamplingProfile::Interactive => "Interactive",
        SamplingProfile::Battery => "Battery",
        SamplingProfile::BatterySaver => "BatterySaver",
        SamplingProfile::Background => "Background",
    }
}

/// CSV uses the stable lowercase identifier instead of the serde variant name.
fn csv_status(status: ProcessStatus) -> &'static str {
    status.id()
}

/// Returns `true` when `ch` can start a spreadsheet formula and must therefore
/// be neutralised at the beginning of a CSV text field.
///
/// The four ASCII starters are the historical contract. A spreadsheet may
/// however normalise fullwidth / compatibility characters back to ASCII before
/// evaluating a cell, so a leading `＝` (U+FF1D) is just as dangerous as `=`
/// (PC-CSVBOM). The list below covers the required fullwidth forms, the typographic
/// minus sign and the other common lookalikes that fold to an ASCII starter.
fn is_formula_starter(ch: char) -> bool {
    matches!(
        ch,
        // Historical ASCII contract.
        '=' | '+' | '-' | '@'
        // Fullwidth forms required by PC-CSVBOM.
        | '\u{ff1d}' // ＝ FULLWIDTH EQUALS SIGN
        | '\u{ff0b}' // ＋ FULLWIDTH PLUS SIGN
        | '\u{ff0d}' // － FULLWIDTH HYPHEN-MINUS
        | '\u{ff20}' // ＠ FULLWIDTH COMMERCIAL AT
        // Other lookalikes commonly folded to an ASCII starter by importers.
        | '\u{2212}' // − MINUS SIGN
        | '\u{fe63}' // ﹣ SMALL HYPHEN-MINUS
        | '\u{fe66}' // ﹦ SMALL EQUALS SIGN
        | '\u{fe62}' // ﹢ SMALL PLUS SIGN
        | '\u{fe6b}' // ﹫ SMALL COMMERCIAL AT
        | '\u{207a}' // ⁺ SUPERSCRIPT PLUS SIGN
        | '\u{207b}' // ⁻ SUPERSCRIPT MINUS SIGN
        | '\u{207c}' // ⁼ SUPERSCRIPT EQUALS SIGN
        | '\u{208a}' // ₊ SUBSCRIPT PLUS SIGN
        | '\u{208b}' // ₋ SUBSCRIPT MINUS SIGN
        | '\u{208c}' // ₌ SUBSCRIPT EQUALS SIGN
    )
}

fn csv_text(value: &str) -> String {
    // A leading newline/space (or VT/FF) can be ignored by spreadsheet
    // importers, exposing the next character as a formula. The apostrophe must
    // go *inside* the RFC 4180 quotes: placing it before the opening quote
    // makes parsers treat the field as unquoted, so commas/newlines split it
    // and a later field can start with a formula (injection bypass, R29).
    let neutralise = value.chars().next().is_some_and(|ch| {
        is_formula_starter(ch) || matches!(ch, '\t' | '\r' | '\n' | ' ' | '\u{0b}' | '\u{0c}')
    });
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

/// Writes one CPU percentage for the CSV at two decimal places.
///
/// The probe's `f32` values carry representation noise (`4.3799996`,
/// `1.9399999`) that a plain `Display` prints verbatim. The UI already renders
/// one or two decimals, so the export rounds at this single serialisation point
/// and never mutates the snapshot: filtering and sorting keep using the raw
/// value, and the JSON exporter keeps its wire-compatible number. Trailing
/// zeroes are trimmed so integral values stay `0` / `100` as before.
fn push_csv_percent(out: &mut String, value: f32) {
    let rounded = format!("{:.2}", crate::normalise_percent(value));
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    out.push_str(trimmed);
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
                swap_bytes: 4_096,
                read_bytes: 8_192,
                write_bytes: 16_384,
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
            power: crate::model::PowerSnapshot {
                source: PowerSource::Battery,
                batteries: vec![crate::model::BatterySnapshot {
                    name: "alice-my-host-BAT0".into(),
                    capacity_percent: Some(12),
                    state: BatteryState::Discharging,
                }],
            },
            sampling: crate::model::SamplingPlan {
                interval_ms: 5_000,
                profile: SamplingProfile::BatterySaver,
            },
            cgroups: vec![crate::model::CgroupGroup {
                label: "/home/alice/my-host.slice/app.service".into(),
                members: vec![ProcessKey::new(4242, 1_700_000_000)],
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
            assert!(
                !dump.contains(secret),
                "redact_for_export leaked {secret:?}"
            );
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
        // The exporter now sanitises before serialising (R2-DATA), so the CRLF
        // is folded into a space: only the header and the row terminator remain
        // and no raw control character reaches the file.
        assert_eq!(csv.matches('\n').count(), 2, "{csv}");
        assert!(!csv.contains('\r'), "{csv}");
        // RFC 4180 quoting is still applied to names with commas/quotes.
        assert!(csv.contains("\"a,b\"\"c\""), "{csv}");
        // The spreadsheet formula is still neutralised after sanitisation.
        assert!(csv.contains("'=cmd|' /C calc'!A0 next"), "{csv}");
        // Empty Option fields stay empty instead of inventing a value.
        assert!(csv.contains("0,0,0,,,'=cmd|'"), "{csv}");
    }

    #[test]
    fn csv_rounds_cpu_percent_to_two_decimals_without_touching_the_model() {
        let mut snapshot = SystemSnapshot::default();
        for (pid, cpu) in [
            (1, 4.3799996_f32),
            (2, 1.9399999_f32),
            (3, 100.0_f32),
            (4, f32::NAN),
        ] {
            snapshot.processes.push(crate::model::ProcessSnapshot {
                key: ProcessKey::new(pid, 1),
                cpu_percent: cpu,
                ..crate::model::ProcessSnapshot::default()
            });
        }
        let original = snapshot.clone();
        let csv = snapshot_to_csv(&snapshot, &secret_ids());

        // Column 7 (0-based 6) is cpu_percent; raw f32 noise must be gone.
        let percents: Vec<&str> = csv
            .lines()
            .skip(1)
            .map(|row| row.split(',').nth(6).expect("cpu_percent column"))
            .collect();
        assert_eq!(percents, ["4.38", "1.94", "100", "0"], "{csv}");
        assert!(!csv.contains("4.3799996"), "{csv}");
        assert!(!csv.contains("1.9399999"), "{csv}");

        // Formatting is the only place that rounds: the snapshot keeps the
        // original values for filtering and sorting, and JSON keeps its
        // wire-compatible number.
        assert_eq!(snapshot.processes[0].cpu_percent, 4.3799996_f32);
        assert_eq!(snapshot.processes[1].cpu_percent, 1.9399999_f32);
        assert!(snapshot.processes[3].cpu_percent.is_nan());
        assert_eq!(format!("{snapshot:?}"), format!("{original:?}"));
        let json = snapshot_to_json(&snapshot, &secret_ids());
        assert!(json.contains("\"cpu_percent\": 4.3799996"), "{json}");
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
        // exposing the '=' that follows as a formula (R28 export audit). Keep
        // the CSV-layer guard pinned directly: the exporter sanitises first, so
        // these raw values no longer reach `csv_text` through the public path.
        for value in ["\n=1+1", " =1+1", "\u{0b}=1+1", "\u{0c}=1+1"] {
            let field = csv_text(value);
            // A quoted field puts the apostrophe after the opening quote.
            let content = field.strip_prefix('"').unwrap_or(&field);
            assert!(content.starts_with('\''), "{value:?} -> {field:?}");
            assert!(field.contains("=1+1"), "{value:?} -> {field:?}");
        }
        // Through the export path the control character is removed by the new
        // sanitisation pass, and the leading '=' still gets its apostrophe.
        for value in ["\n=1+1", " =1+1", "\u{0b}=1+1", "\u{0c}=1+1"] {
            let mut snapshot = SystemSnapshot::default();
            snapshot.processes.push(crate::model::ProcessSnapshot {
                command: value.into(),
                ..crate::model::ProcessSnapshot::default()
            });
            let csv = snapshot_to_csv(&snapshot, &secret_ids());
            assert!(csv.ends_with("'=1+1\n"), "unexpected field: {csv:?}");
        }
    }

    #[test]
    fn csv_neutralises_fullwidth_and_unicode_formula_lookalikes() {
        // PC-CSVBOM: a leading fullwidth '＝' (or another Unicode lookalike) must
        // receive the same apostrophe guard as ASCII '='. Check the pure helper
        // first so every listed lookalike is pinned, then the public export path.
        let starters = [
            '\u{ff1d}', // ＝ FULLWIDTH EQUALS SIGN (required)
            '\u{ff0b}', // ＋ FULLWIDTH PLUS SIGN (required)
            '\u{ff0d}', // － FULLWIDTH HYPHEN-MINUS (required)
            '\u{ff20}', // ＠ FULLWIDTH COMMERCIAL AT (required)
            '\u{2212}', // − MINUS SIGN (required)
            '\u{fe63}', // ﹣ SMALL HYPHEN-MINUS
            '\u{fe66}', // ﹦ SMALL EQUALS SIGN
            '\u{fe62}', // ﹢ SMALL PLUS SIGN
            '\u{fe6b}', // ﹫ SMALL COMMERCIAL AT
            '\u{207a}', // ⁺ SUPERSCRIPT PLUS SIGN
            '\u{207b}', // ⁻ SUPERSCRIPT MINUS SIGN
            '\u{207c}', // ⁼ SUPERSCRIPT EQUALS SIGN
            '\u{208a}', // ₊ SUBSCRIPT PLUS SIGN
            '\u{208b}', // ₋ SUBSCRIPT MINUS SIGN
            '\u{208c}', // ₌ SUBSCRIPT EQUALS SIGN
        ];
        for &starter in &starters {
            assert!(
                is_formula_starter(starter),
                "{starter:?} must be neutralised"
            );
            let field = csv_text(&format!("{starter}1+1"));
            assert!(
                field.starts_with('\''),
                "{starter:?} did not get an apostrophe: {field:?}"
            );
        }
        // Only a *leading* lookalike matters; an interior one is left intact.
        assert_eq!(csv_text("a\u{ff1d}b"), "a\u{ff1d}b");
        // The ASCII contract is unchanged.
        for starter in ['=', '+', '-', '@'] {
            assert!(csv_text(&format!("{starter}cmd")).starts_with('\''));
        }

        // Public path: a process name beginning with '＝' is exported with the
        // apostrophe inside the field, without changing the 12-column layout or
        // the RFC 4180 escaping of the other fields.
        let mut snapshot = SystemSnapshot::default();
        snapshot.processes.push(crate::model::ProcessSnapshot {
            name: "\u{ff1d}1+1".into(),
            command: "a,b\"c".into(),
            ..crate::model::ProcessSnapshot::default()
        });
        let csv = snapshot_to_csv(&snapshot, &secret_ids());
        assert!(csv.contains("'\u{ff1d}1+1"), "{csv}");
        let row = csv.lines().nth(1).expect("data row");
        assert!(
            row.contains("\"a,b\"\"c\""),
            "RFC 4180 escaping broke: {row}"
        );
        // 12 columns + one embedded comma => 12 commas in the row.
        assert_eq!(row.matches(',').count(), 12, "column contract broke: {row}");
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
        // Control characters are removed by the exporter's sanitisation pass,
        // so the JSON escaper no longer needs to emit `\t` / `\n` escapes for
        // process-supplied text (R2-DATA).
        assert!(
            json.contains("quote \\\" backslash \\\\ tab newline"),
            "{json}"
        );
        assert!(!json.contains("\\t"), "{json}");
        assert!(!json.contains("\\n"), "{json}");
        assert!(json.contains("\"cpu_percent\": 0"), "{json}");
        assert!(json.contains("\"total_percent\": 0"), "{json}");
    }

    #[test]
    fn export_sanitises_untrusted_text_even_without_a_prior_pass() {
        let mut snapshot = SystemSnapshot::default();
        snapshot.processes.push(crate::model::ProcessSnapshot {
            name: "wor\u{1b}[31mker\u{202e}".into(),
            command: "--home /home/alice https://my-host.tail1234.ts.net\u{1b}]0;title\u{7}".into(),
            ..crate::model::ProcessSnapshot::default()
        });
        snapshot
            .power
            .batteries
            .push(crate::model::BatterySnapshot {
                name: "BA\u{1b}[31mT0".into(),
                ..crate::model::BatterySnapshot::default()
            });
        snapshot.cgroups.push(crate::model::CgroupGroup {
            label: "a.slice/w\u{1b}[31morker.service".into(),
            members: vec![crate::model::ProcessKey::new(4242, 1)],
        });

        let json = snapshot_to_json(&snapshot, &secret_ids());
        assert_well_formed_json(&json);
        for control in ['\u{1b}', '\u{7}', '\u{202e}'] {
            assert!(!json.contains(control), "control character leaked: {json}");
        }
        assert!(!json.contains("\\u001b"), "{json}");
        assert!(!json.contains("alice"), "{json}");
        assert!(!json.contains("my-host"), "{json}");
        assert!(json.contains("\"name\": \"worker\""), "{json}");
        assert!(json.contains("\"name\": \"BAT0\""), "{json}");
        assert!(
            json.contains("\"label\": \"a.slice/worker.service\""),
            "{json}"
        );

        // CSV takes the same defensive path.
        let csv = snapshot_to_csv(&snapshot, &secret_ids());
        for control in ['\u{1b}', '\u{7}', '\u{202e}', '\r'] {
            assert!(!csv.contains(control), "control character leaked: {csv}");
        }
        // The caller's snapshot is never mutated by the extra pass.
        assert!(snapshot.processes[0].name.contains('\u{1b}'));
        assert!(snapshot.power.batteries[0].name.contains('\u{1b}'));
    }

    #[test]
    fn temperatures_are_not_clamped_like_percentages() {
        // Sensors can legitimately exceed 100 °C or read below zero; only
        // non-finite values are unusable in JSON (R32 audit).
        let mut snapshot = SystemSnapshot::default();
        snapshot.components.push(crate::model::ComponentSnapshot {
            label: "CPU".into(),
            temperature_c: Some(120.5),
            max_c: Some(-20.0),
            critical_c: Some(f32::NAN),
        });
        let json = snapshot_to_json(&snapshot, &secret_ids());
        assert_well_formed_json(&json);
        assert!(json.contains("\"temperature_c\": 120.5"), "{json}");
        assert!(json.contains("\"max_c\": -20"), "{json}");
        assert!(json.contains("\"critical_c\": 0"), "{json}");
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

    /// The exporter is checked against the one shared wire type instead of a
    /// local mirror, so a field-order or serde-attribute change in `wire.rs`
    /// cannot silently drift away from this test.
    #[cfg(feature = "serde")]
    type SnapshotEnvelope<'a> = crate::wire::SnapshotEnvelope<&'a SystemSnapshot>;

    /// Byte-invariance guard: FNV-1a of the serialised wire envelope.
    ///
    /// The value was captured from the pre-refactor local mirror; moving the
    /// type to `tm_core::wire` must not change a single JSON byte.
    #[test]
    #[cfg(feature = "serde")]
    fn wire_envelope_json_bytes_are_stable() {
        let snapshot = SystemSnapshot::default();
        let bytes = serde_json::to_string(&SnapshotEnvelope {
            schema_version: crate::WIRE_SCHEMA_VERSION,
            total_process_count: 0,
            snapshot: &snapshot,
        })
        .expect("envelope must serialise");
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        eprintln!("WIRE_ENVELOPE_HASH {hash:016x} len={}", bytes.len());
        assert_eq!(hash, 0x1265_fcb5_b2d4_e6f4, "wire bytes changed: {bytes}");
        assert_eq!(bytes.len(), 595, "wire byte count changed");
    }

    /// Replaces every scalar with `null` while preserving object keys and array
    /// structure. Redacted strings and normalised floats cannot cause false
    /// positives, but a field added to any snapshot type changes this shape.
    #[cfg(feature = "serde")]
    fn shape_only(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(entries) => serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, shape_only(value)))
                    .collect(),
            ),
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(shape_only).collect())
            }
            _ => serde_json::Value::Null,
        }
    }

    #[test]
    #[cfg(feature = "serde")]
    fn json_matches_the_wire_envelope_shape() {
        let mut snapshot = secret_snapshot();
        // A non-zero duration makes the secs/nanos split observable.
        snapshot.sampled_elapsed = std::time::Duration::new(3, 456_000_000);
        let json = snapshot_to_json(&snapshot, &secret_ids());
        let exported: serde_json::Value =
            serde_json::from_str(&json).expect("export must be valid JSON");

        // The reference is serialised from the model itself, so a new model
        // field appears here automatically and a stale exporter fails below.
        let wire = serde_json::to_value(SnapshotEnvelope {
            schema_version: crate::WIRE_SCHEMA_VERSION,
            total_process_count: snapshot.processes.len(),
            snapshot: &snapshot,
        })
        .expect("envelope must serialise");

        // Simulate a future model field: the shape diff must notice it, which
        // is the whole point of comparing against the model-derived envelope.
        let mut with_future_field = wire.clone();
        with_future_field["snapshot"]["future_field"] = serde_json::json!(1);
        assert_ne!(
            shape_only(exported.clone()),
            shape_only(with_future_field),
            "a new model field must change the detected shape"
        );

        assert_eq!(
            exported["snapshot"]["sampled_elapsed"], wire["snapshot"]["sampled_elapsed"],
            "sampled_elapsed must match serde's Duration representation: {json}"
        );
        assert_eq!(
            shape_only(exported),
            shape_only(wire),
            "JSON shape diverged from the wire envelope: {json}"
        );
        assert!(json.contains("\"threads\": 4"), "{json}");
        assert!(json.contains("\"parent_pid\": null"), "{json}");
        assert!(json.contains("\"swap_bytes\": 4096"), "{json}");
        // C1-IODATA: the cumulative block-IO fields follow `swap_bytes` in the
        // same order serde declares them, so exported files parse with the
        // exact same model struct as the live wire endpoint.
        let swap_at = json.find("\"swap_bytes\": 4096").expect("swap_bytes field");
        let read_at = json.find("\"read_bytes\": 8192").expect("read_bytes field");
        let write_at = json
            .find("\"write_bytes\": 16384")
            .expect("write_bytes field");
        assert!(swap_at < read_at && read_at < write_at, "{json}");
        assert!(json.contains("\"max_c\": null"), "{json}");
    }

    /// Without tm-core's `serde` feature the model cannot be serialised, so the
    /// full envelope diff is unavailable. Still check the new field's shape
    /// when running the plain `cargo test -p tm-core` gate.
    #[test]
    #[cfg(not(feature = "serde"))]
    fn json_export_includes_sampled_elapsed_shape() {
        let json = snapshot_to_json(&secret_snapshot(), &secret_ids());
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let elapsed = &value["snapshot"]["sampled_elapsed"];
        assert!(elapsed["secs"].is_number(), "{json}");
        assert!(elapsed["nanos"].is_number(), "{json}");
        assert!(json.contains("\"threads\": 4"), "{json}");
        assert!(json.contains("\"parent_pid\": null"), "{json}");
        assert!(json.contains("\"swap_bytes\": 4096"), "{json}");
        assert!(json.contains("\"read_bytes\": 8192"), "{json}");
        assert!(json.contains("\"write_bytes\": 16384"), "{json}");
        assert!(json.contains("\"max_c\": null"), "{json}");
    }

    #[test]
    fn power_sampling_and_cgroups_are_registered_in_the_wire_export() {
        let json = snapshot_to_json(&secret_snapshot(), &secret_ids());
        assert_well_formed_json(&json);
        for key in [
            "\"power\"",
            "\"source\": \"Battery\"",
            "\"batteries\"",
            "\"capacity_percent\": 12",
            "\"state\": \"Discharging\"",
            "\"sampling\"",
            "\"interval_ms\": 5000",
            "\"profile\": \"BatterySaver\"",
            "\"cgroups\"",
            "\"members\"",
            "\"start_time_secs\": 1700000000",
        ] {
            assert!(
                json.contains(key),
                "missing additive wire key {key}: {json}"
            );
        }
        // The stable lowercase identifiers are only used by CSV/status labels;
        // the JSON envelope must keep serde's variant spelling.
        assert!(!json.contains("\"source\": \"battery\""), "{json}");
        assert!(!json.contains("\"profile\": \"battery_saver\""), "{json}");
        // An empty snapshot still emits well-formed, parseable additive fields.
        let empty = snapshot_to_json(&SystemSnapshot::default(), &secret_ids());
        assert_well_formed_json(&empty);
        assert!(empty.contains("\"batteries\": []"), "{empty}");
        assert!(empty.contains("\"cgroups\": []"), "{empty}");
        assert!(empty.contains("\"source\": \"Unknown\""), "{empty}");
        assert!(empty.contains("\"profile\": \"Interactive\""), "{empty}");
        // The process CSV is a fixed header + one row per process; additive
        // snapshot fields must not change it.
        assert_eq!(
            snapshot_to_csv(&secret_snapshot(), &secret_ids())
                .lines()
                .next()
                .expect("header"),
            PROCESS_CSV_HEADER
        );
    }

    #[test]
    fn json_export_preserves_explicit_wire_total_and_falls_back_to_row_count() {
        // `GET /api/v1/snapshot?limit=N` publishes 3 rows while the envelope's
        // `total_process_count` still describes the whole machine (30).
        let mut snapshot = SystemSnapshot::default();
        for pid in [10, 20, 30] {
            snapshot.processes.push(crate::model::ProcessSnapshot {
                key: ProcessKey::new(pid, 1),
                ..crate::model::ProcessSnapshot::default()
            });
        }
        let ids = secret_ids();

        let wire = snapshot_to_json_with_total(&snapshot, &ids, Some(30));
        assert_well_formed_json(&wire);
        let value: serde_json::Value = serde_json::from_str(&wire).expect("wire export is JSON");
        assert_eq!(value["total_process_count"].as_u64(), Some(30), "{wire}");
        assert_eq!(
            value["snapshot"]["processes"]
                .as_array()
                .map(|rows| rows.len()),
            Some(3),
            "only published rows may appear inside `snapshot`"
        );

        // A CLI / local-probe export has no envelope: keep the row count.
        let cli = snapshot_to_json_with_total(&snapshot, &ids, None);
        let value: serde_json::Value = serde_json::from_str(&cli).expect("cli export is JSON");
        assert_eq!(value["total_process_count"].as_u64(), Some(3), "{cli}");
        assert_eq!(
            snapshot_to_json(&snapshot, &ids),
            cli,
            "the 2-argument wrapper must remain backward compatible"
        );
    }

    #[test]
    fn csv_export_keeps_twelve_columns_with_an_explicit_wire_total() {
        let mut snapshot = SystemSnapshot::default();
        for pid in [10, 20, 30] {
            snapshot.processes.push(crate::model::ProcessSnapshot {
                key: ProcessKey::new(pid, 1),
                ..crate::model::ProcessSnapshot::default()
            });
        }
        let ids = secret_ids();
        let with_total = snapshot_to_csv_with_total(&snapshot, &ids, Some(30));
        let without_total = snapshot_to_csv(&snapshot, &ids);
        assert_eq!(with_total, without_total, "CSV has no envelope field");
        assert_eq!(PROCESS_CSV_HEADER.matches(',').count(), 11);
        let lines: Vec<&str> = with_total.lines().collect();
        assert_eq!(lines.len(), 4, "header + 3 rows: {with_total}");
        for line in lines {
            assert_eq!(line.matches(',').count(), 11, "12 columns: {line}");
        }
    }

    #[test]
    fn csv_ignores_the_wire_total_argument_while_json_preserves_it() {
        let mut snapshot = SystemSnapshot::default();
        for pid in [10, 20, 30] {
            snapshot.processes.push(crate::model::ProcessSnapshot {
                key: ProcessKey::new(pid, 1),
                ..crate::model::ProcessSnapshot::default()
            });
        }
        let ids = secret_ids();
        let baseline = snapshot_to_csv(&snapshot, &ids);

        // The renamed `_ignored_wire_total` argument must be observationally
        // inert: CSV has no envelope field, only one row per published process.
        let huge_total = 9_999_999_u64;
        for total in [None, Some(30), Some(huge_total)] {
            let csv = snapshot_to_csv_with_total(&snapshot, &ids, total);
            assert_eq!(csv, baseline, "CSV must not depend on {total:?}");
            assert!(
                !csv.contains(&huge_total.to_string()),
                "CSV emitted an envelope total: {csv}"
            );
        }

        // The JSON counterpart deliberately differs: it writes the total.
        let json = snapshot_to_json_with_total(&snapshot, &ids, Some(30));
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["total_process_count"].as_u64(), Some(30), "{json}");
    }
}
