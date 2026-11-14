//! Palette and small presentational helpers for the browser shell.
//!
//! The colours mirror the native shell's Catppuccin Mocha-inspired tokens
//! (`app/src/theme.rs`). They are duplicated as `const`s on purpose: the wasm
//! crate must not depend on the native app crate, which pulls in `gpui-kit`.

/// Window background.
pub const C_BASE: u32 = 0x11111b;
/// Card / panel background.
pub const C_MANTLE: u32 = 0x181825;
/// Raised surface (hover, inactive buttons).
pub const C_SURFACE: u32 = 0x313244;
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

/// Height of one process table row (shared by the virtual list and header).
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

/// UTC clock label for a Unix timestamp (no timezone dependency in wasm).
pub fn format_clock(unix_secs: u64) -> String {
    let seconds_of_day = unix_secs % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}
