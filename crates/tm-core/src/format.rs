//! Human-readable formatting for the numbers a task manager displays.
//!
//! Formatting lives in the core rather than in the UI so that the native shell
//! and the browser shell render identical strings, and so that golden tests can
//! pin the exact output.

/// Formats a byte count using binary prefixes (`KiB`, `MiB`, ...).
///
/// Values below one KiB keep the exact byte count; larger values use one
/// decimal place. This is the same convention as GNOME System Monitor and
/// `btop`, which keeps the UI familiar.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1_024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1_024.0 && unit + 1 < UNITS.len() {
        value /= 1_024.0;
        unit += 1;
    }
    // `{:.1}` rounds 1023.95.. up to 1024.0. Carry into the next unit so we
    // never print the physically nonsensical "1024.0 KiB" / "1024.0 MiB".
    if unit + 1 < UNITS.len() && (value * 10.0).round() >= 10_240.0 {
        value = 1.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Formats a byte-per-second rate.
pub fn format_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

/// Formats a percentage with one decimal place.
///
/// Non-finite values become `0.0%` so a broken probe can never render `NaN%`
/// or `inf%` in the UI.
pub fn format_percent(percent: f32) -> String {
    // `normalise_percent` also removes `-0.0`, which `clamp` alone keeps.
    let percent = crate::normalise_percent(percent).clamp(0.0, 100.0);
    format!("{percent:.1}%")
}

/// Formats a percentage that may exceed 100 (per-process CPU on multicore).
///
/// Unlike [`format_percent`] this is not capped at 100, but negative values are
/// floored at zero to match the chart history's clamp, and non-finite values
/// are replaced with zero for display safety.
pub fn format_cpu_percent(percent: f32) -> String {
    let percent = crate::normalise_percent(percent);
    format!("{percent:.1}%")
}

/// Formats a duration in seconds as `2d 3h`, `4h 5m`, `3m 2s` or `9s`.
pub fn format_duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    }
}

/// Formats the age of a process started at `start_secs`.
pub fn format_process_age(now_secs: u64, start_secs: u64) -> String {
    format_duration(now_secs.saturating_sub(start_secs))
}

/// Formats a three-value load average.
///
/// Non-finite components are shown as `0.00` instead of `NaN` / `inf`; negative
/// values are floored at zero to match the percentage formatters (R1S-DATA-09).
pub fn format_load_average(load: [f32; 3]) -> String {
    let finite = load.map(crate::normalise_percent);
    format!("{:.2} {:.2} {:.2}", finite[0], finite[1], finite[2])
}

/// Formats a count with thousands separators (`12,345`).
pub fn format_count(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Formats `used / total` memory with the same unit, for compact labels.
pub fn format_used_of_total(used_bytes: u64, total_bytes: u64) -> String {
    format!(
        "{} / {}",
        format_bytes(used_bytes),
        format_bytes(total_bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_use_binary_prefixes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1_023), "1023 B");
        assert_eq!(format_bytes(1_024), "1.0 KiB");
        assert_eq!(format_bytes(1_536), "1.5 KiB");
        assert_eq!(format_bytes(16 * 1_024 * 1_024 * 1_024), "16.0 GiB");
        // One decimal must not round up to "1024.0" in the lower unit.
        assert_eq!(format_bytes(1_048_575), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_823), "1.0 GiB");
        assert_eq!(format_bytes(1_099_511_627_775), "1.0 TiB");
        assert_eq!(format_rate(1_048_575), "1.0 MiB/s");
    }

    #[test]
    fn extreme_values_do_not_panic_or_overflow() {
        // Display helpers must stay total: a corrupt probe could hand them any
        // u64, and a panic in a render loop would take down the shell.
        let bytes = format_bytes(u64::MAX);
        assert_eq!(bytes, "16.0 EiB");
        assert_eq!(format_bytes(1_024_u64.pow(6)), "1.0 EiB");
        // Pin the full output, not just a suffix/comma: a broken renderer that
        // emits "0h" or "1,0" would otherwise pass these assertions (R38).
        assert_eq!(format_duration(u64::MAX), "213503982334601d 7h");
        assert_eq!(format_process_age(0, u64::MAX), "0s");
        assert_eq!(
            format_count(u64::MAX as usize),
            "18,446,744,073,709,551,615"
        );
    }

    #[test]
    fn rates_append_per_second() {
        assert_eq!(format_rate(2_048), "2.0 KiB/s");
    }

    #[test]
    fn percentages_are_clamped_and_rounded() {
        assert_eq!(format_percent(12.34), "12.3%");
        assert_eq!(format_percent(150.0), "100.0%");
        assert_eq!(format_cpu_percent(250.0), "250.0%");
    }

    #[test]
    fn non_finite_numbers_never_reach_the_display() {
        assert_eq!(format_percent(f32::NAN), "0.0%");
        assert_eq!(format_percent(f32::INFINITY), "0.0%");
        assert_eq!(format_cpu_percent(f32::NAN), "0.0%");
        assert_eq!(format_cpu_percent(f32::NEG_INFINITY), "0.0%");
        // Negative values are floored to match MetricHistory's 0..=100 clamp.
        assert_eq!(format_cpu_percent(-5.0), "0.0%");
        // `clamp` alone would keep -0.0 (bit sign) and render "-0.0%".
        assert_eq!(format_percent(-0.0), "0.0%");
        assert_eq!(format_percent(-5.0), "0.0%");
        assert_eq!(
            format_load_average([f32::NAN, 1.5, f32::INFINITY]),
            "0.00 1.50 0.00"
        );
        assert_eq!(format_load_average([-1.0, 1.5, f32::NAN]), "0.00 1.50 0.00");
        // `-0.0` would render as "-0.00" without the floor.
        assert_eq!(
            format_load_average([-0.0, 1.5, f32::NEG_INFINITY]),
            "0.00 1.50 0.00"
        );
    }

    #[test]
    fn durations_pick_the_two_most_significant_units() {
        assert_eq!(format_duration(9), "9s");
        assert_eq!(format_duration(62), "1m 2s");
        assert_eq!(format_duration(3_600 + 120), "1h 2m");
        assert_eq!(format_duration(2 * 86_400 + 3_600), "2d 1h");
        // Unit-switch boundaries.
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(60), "1m 0s");
        assert_eq!(format_duration(3_599), "59m 59s");
        assert_eq!(format_duration(3_600), "1h 0m");
        assert_eq!(format_duration(86_399), "23h 59m");
        assert_eq!(format_duration(86_400), "1d 0h");
    }

    #[test]
    fn process_age_is_saturating() {
        assert_eq!(format_process_age(1_000, 900), "1m 40s");
        assert_eq!(format_process_age(1_000, 2_000), "0s");
    }

    #[test]
    fn counts_use_grouping() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(12_345_678), "12,345,678");
    }

    #[test]
    fn used_of_total_shares_one_label() {
        assert_eq!(
            format_used_of_total(1_024 * 1_024, 2 * 1_024 * 1_024),
            "1.0 MiB / 2.0 MiB"
        );
    }

    #[test]
    fn load_average_has_two_decimals() {
        assert_eq!(format_load_average([0.5, 0.25, 0.0]), "0.50 0.25 0.00");
    }
}
