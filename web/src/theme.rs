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
    muted: 0x585b70,
    dim: 0x64677c,
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

/// Minimum contrast ratio for normal text under WCAG 2.x AA (SC 1.4.3).
pub const WCAG_AA_NORMAL: f64 = 4.5;

/// Linearises one 8-bit sRGB channel for WCAG relative-luminance math.
///
/// Pure integer-in / float-out helper: no theme state is consulted.
fn linearize(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.x relative luminance of an `0xRRGGBB` colour, in `0.0..=1.0`.
pub fn relative_luminance(color: u32) -> f64 {
    let r = linearize(((color >> 16) & 0xff) as u8);
    let g = linearize(((color >> 8) & 0xff) as u8);
    let b = linearize((color & 0xff) as u8);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG 2.x contrast ratio between two `0xRRGGBB` colours, in `1.0..=21.0`.
///
/// Symmetric by construction; which colour is "foreground" does not matter.
pub fn contrast_ratio(a: u32, b: u32) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// High-contrast foreground for text drawn on a filled `bg`.
///
/// Keeps the caller's `fg` when it already clears [`WCAG_AA_NORMAL`]; otherwise
/// picks whichever of near-black / near-white has more contrast on `bg`. This
/// keeps `base`-coloured text on light accent fills legible without changing
/// the palette or the caller's colour semantics.
pub fn on_fill_text_color(fg: u32, bg: u32) -> u32 {
    if contrast_ratio(fg, bg) >= WCAG_AA_NORMAL {
        return fg;
    }
    const NEAR_BLACK: u32 = 0x11111b;
    const NEAR_WHITE: u32 = 0xffffff;
    if contrast_ratio(NEAR_WHITE, bg) >= contrast_ratio(NEAR_BLACK, bg) {
        NEAR_WHITE
    } else {
        NEAR_BLACK
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
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    /// Serialises the tests that read or mutate the process-global theme.
    static THEME_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Holds `THEME_TEST_LOCK` for the whole test and restores the theme that
    /// was active when it was created. Restoration runs from `Drop`, so the
    /// global theme is left untouched even if the test panics.
    struct ThemeGuard {
        original: Theme,
        // Held until after `Drop::drop` restores `original`; field order matters.
        _lock: MutexGuard<'static, ()>,
    }

    impl ThemeGuard {
        fn new() -> Self {
            // Recover from a poisoned lock so one panicking test does not
            // cascade into unrelated theme tests.
            let lock = match THEME_TEST_LOCK.lock() {
                Ok(lock) => lock,
                Err(poisoned) => poisoned.into_inner(),
            };
            Self {
                original: theme(),
                _lock: lock,
            }
        }
    }

    impl Drop for ThemeGuard {
        fn drop(&mut self) {
            set_theme(self.original);
        }
    }

    #[test]
    fn clock_formats_utc_time_of_day() {
        assert_eq!(format_clock(0), "00:00:00 UTC");
        assert_eq!(format_clock(3_661), "01:01:01 UTC");
        assert_eq!(format_clock(86_399), "23:59:59 UTC");
    }

    #[test]
    fn usage_colours_follow_thresholds() {
        let _theme_guard = ThemeGuard::new();
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
        let _theme_guard = ThemeGuard::new();
        set_theme(Theme::Light);
        assert_eq!(palette().base, LIGHT.base);
        set_theme(Theme::Dark);
        assert_eq!(palette().base, DARK.base);
    }

    #[test]
    fn wcag_contrast_helpers_are_pure_and_symmetric() {
        assert_eq!(relative_luminance(0x000000), 0.0);
        assert!((relative_luminance(0xffffff) - 1.0).abs() < 1e-9);
        let black_white = contrast_ratio(0x000000, 0xffffff);
        assert!((black_white - 21.0).abs() < 1e-9, "{black_white}");
        assert!((contrast_ratio(0xffffff, 0x000000) - black_white).abs() < 1e-9);
        assert!((contrast_ratio(0x12345678, 0x12345678) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn light_theme_normal_text_meets_wcag_aa() {
        for (name, fg) in [
            ("text", LIGHT.text),
            ("subtext", LIGHT.subtext),
            ("muted", LIGHT.muted),
            ("dim", LIGHT.dim),
        ] {
            for (bg_name, bg) in [("base", LIGHT.base), ("mantle", LIGHT.mantle)] {
                let ratio = contrast_ratio(fg, bg);
                assert!(
                    ratio >= WCAG_AA_NORMAL,
                    "LIGHT {name} on {bg_name}: {ratio:.2}:1 < {WCAG_AA_NORMAL}:1"
                );
            }
        }
    }

    #[test]
    fn on_fill_text_color_clears_wcag_aa_on_every_light_fill() {
        for (name, fill) in [
            ("blue", LIGHT.blue),
            ("green", LIGHT.green),
            ("yellow", LIGHT.yellow),
            ("red", LIGHT.red),
            ("mauve", LIGHT.mauve),
            ("teal", LIGHT.teal),
            ("peach", LIGHT.peach),
            ("surface_hi", LIGHT.surface_hi),
        ] {
            let fg = on_fill_text_color(LIGHT.base, fill);
            let ratio = contrast_ratio(fg, fill);
            assert!(
                ratio >= WCAG_AA_NORMAL,
                "LIGHT {name} fill {fill:06x}: {fg:06x} gives {ratio:.2}:1"
            );
        }
    }

    /// Extracts the verbatim `DARK` / `LIGHT` palette literal blocks from a source file.
    fn palette_literals(source: &str) -> String {
        let mut out = String::new();
        for name in ["DARK", "LIGHT"] {
            let marker = format!("pub const {name}: Palette = Palette {{");
            let start = source
                .find(&marker)
                .expect("{name} palette literal must exist");
            let block = &source[start..];
            let end = block
                .find("\n};")
                .expect("{name} palette literal must terminate")
                + 3;
            out.push_str(&block[..end]);
            out.push('\n');
        }
        out
    }

    #[test]
    fn native_and_web_palettes_are_verbatim_copies() {
        let here = include_str!("theme.rs");
        let other = include_str!("../../app/src/theme.rs");
        assert_eq!(palette_literals(here), palette_literals(other));
    }
}
