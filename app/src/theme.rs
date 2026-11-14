//! Palette and layout tokens for the native shell.
//!
//! The colours are a Catppuccin Mocha-inspired dark palette. They are defined in
//! one place so the overview cards, process table, dialogs and status bar stay
//! visually consistent.

/// Window background.
pub const C_BASE: u32 = 0x11111b;
/// Card / panel background.
pub const C_MANTLE: u32 = 0x181825;
/// Raised surface (hover, inactive buttons).
pub const C_SURFACE: u32 = 0x313244;
/// Brighter raised surface.
pub const C_SURFACE_HI: u32 = 0x45475a;
/// Primary text.
pub const C_TEXT: u32 = 0xcdd6f4;
/// Secondary text.
pub const C_SUBTEXT: u32 = 0xa6adc8;
/// Muted labels.
pub const C_MUTED: u32 = 0x7f849c;
/// Dim text / separators.
pub const C_DIM: u32 = 0x585b70;
/// Accent blue.
pub const C_BLUE: u32 = 0x89b4fa;
/// Accent green.
pub const C_GREEN: u32 = 0xa6e3a1;
/// Accent yellow.
pub const C_YELLOW: u32 = 0xf9e2af;
/// Accent red.
pub const C_RED: u32 = 0xf38ba8;
/// Accent mauve.
pub const C_MAUVE: u32 = 0xcba6f7;
/// Accent teal.
pub const C_TEAL: u32 = 0x94e2d5;
/// Accent peach.
pub const C_PEACH: u32 = 0xfab387;

/// Height of one process table row, shared by the virtual list and the header.
pub const ROW_HEIGHT: f32 = 30.0;

/// Colour for a 0..=100 utilisation value: green → yellow → red.
pub fn usage_color(percent: f32) -> u32 {
    if percent >= 85.0 {
        C_RED
    } else if percent >= 60.0 {
        C_YELLOW
    } else {
        C_GREEN
    }
}

/// UTC clock label for a Unix timestamp (no timezone dependency).
pub fn format_clock(unix_secs: u64) -> String {
    let seconds_of_day = unix_secs % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats_utc_time_of_day() {
        assert_eq!(format_clock(0), "00:00:00 UTC");
        assert_eq!(format_clock(3_661), "01:01:01 UTC");
        assert_eq!(format_clock(86_399), "23:59:59 UTC");
    }

    #[test]
    fn usage_colours_follow_thresholds() {
        assert_eq!(usage_color(0.0), C_GREEN);
        assert_eq!(usage_color(59.9), C_GREEN);
        assert_eq!(usage_color(60.0), C_YELLOW);
        assert_eq!(usage_color(85.0), C_RED);
    }
}
