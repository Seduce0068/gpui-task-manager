//! Palette and small presentational helpers for the browser shell.
//!
//! The colours mirror the native shell's Catppuccin-inspired tokens
//! (`app/src/theme.rs`). They are duplicated as `const`s on purpose: the wasm
//! crate must not depend on the native app crate, which pulls in `gpui-kit`.
//! Two variants are provided — **Dark** (Mocha) and **Light** (Latte) — and the
//! active one is selected at runtime by [`set_theme`] and read via [`palette`].

use std::sync::atomic::{AtomicU8, Ordering};

use tm_viewmodel::Theme;

/// Full colour palette for one theme variant.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub base: u32,
    pub mantle: u32,
    pub surface: u32,
    pub surface_hi: u32,
    pub text: u32,
    pub subtext: u32,
    pub muted: u32,
    pub dim: u32,
    pub blue: u32,
    pub green: u32,
    pub yellow: u32,
    pub red: u32,
    pub mauve: u32,
    pub teal: u32,
    pub peach: u32,
}

/// Catppuccin Mocha (dark).
pub const DARK: Palette = Palette {
    base: 0x11111b,
    mantle: 0x181825,
    surface: 0x313244,
    surface_hi: 0x45475a,
    text: 0xcdd6f4,
    subtext: 0xa6adc8,
    muted: 0x7f849c,
    dim: 0x585b70,
    blue: 0x89b4fa,
    green: 0xa6e3a1,
    yellow: 0xf9e2af,
    red: 0xf38ba8,
    mauve: 0xcba6f7,
    teal: 0x94e2d5,
    peach: 0xfab387,
};

/// Catppuccin Latte (light).
pub const LIGHT: Palette = Palette {
    base: 0xeff1f5,
    mantle: 0xe6e9ef,
    surface: 0xccd0da,
    surface_hi: 0xbcc0cc,
    text: 0x4c4f69,
    subtext: 0x5c5f77,
    muted: 0x6c6f85,
    dim: 0x8c8fa1,
    blue: 0x1e66f5,
    green: 0x40a02b,
    yellow: 0xdf8e1d,
    red: 0xd20f39,
    mauve: 0x8839ef,
    teal: 0x179299,
    peach: 0xfe640b,
};

static CURRENT_THEME: AtomicU8 = AtomicU8::new(0);

/// Returns the currently active theme.
pub fn theme() -> Theme {
    match CURRENT_THEME.load(Ordering::Relaxed) {
        1 => Theme::Light,
        _ => Theme::Dark,
    }
}

/// Switches the active theme at runtime.
pub fn set_theme(t: Theme) {
    let value = match t {
        Theme::Dark => 0u8,
        Theme::Light => 1u8,
    };
    CURRENT_THEME.store(value, Ordering::Relaxed);
}

/// Returns the palette for the currently active theme.
pub fn palette() -> &'static Palette {
    match theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// Height of one process table row (shared by the virtual list and the header).
pub const ROW_HEIGHT: f32 = 30.0;

/// Colour for a 0..=100 utilisation value: green → yellow → red.
pub fn usage_color(percent: f32) -> u32 {
    let p = palette();
    if percent >= 85.0 {
        p.red
    } else if percent >= 60.0 {
        p.yellow
    } else {
        p.green
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
        let p = palette();
        assert_eq!(usage_color(0.0), p.green);
        assert_eq!(usage_color(59.9), p.green);
        assert_eq!(usage_color(60.0), p.yellow);
        assert_eq!(usage_color(85.0), p.red);
    }

    #[test]
    fn light_theme_differs_from_dark() {
        assert_ne!(DARK.base, LIGHT.base);
        assert_ne!(DARK.text, LIGHT.text);
    }

    #[test]
    fn set_theme_switches_palette() {
        set_theme(Theme::Light);
        assert_eq!(palette().base, LIGHT.base);
        set_theme(Theme::Dark);
        assert_eq!(palette().base, DARK.base);
    }
}
