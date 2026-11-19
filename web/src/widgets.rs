//! Small presentational widgets shared by the browser shell.
//!
//! They are deliberately stateless: data in, element out. Interaction (clicks,
//! focus) is wired by `shell.rs`, which keeps the layout code readable and makes
//! it obvious that the browser shell offers no process control.

use gpui::accesskit::Role;
use gpui::{
    Div, ElementId, FontWeight, IntoElement, SharedString, Stateful, div, prelude::*, px, rgb,
};

use crate::theme::*;
use tm_viewmodel::{FilterPreset, TrendDelta, TrendDirection};

/// A horizontal flex container (mirrors the native shell's `h_flex`).
pub fn h_flex() -> Div {
    div().flex().items_center()
}

/// A vertical flex container (mirrors the native shell's `v_flex`).
pub fn v_flex() -> Div {
    div().flex().flex_col()
}

/// A bordered card with a coloured accent bar and a small title.
pub fn card(title: impl Into<SharedString>, accent: u32, body: impl IntoElement) -> Div {
    v_flex()
        .gap_2()
        .p_3()
        .rounded_lg()
        .bg(rgb(palette().mantle))
        .border_1()
        .border_color(rgb(palette().surface))
        .child(
            h_flex()
                .gap_2()
                .child(div().w(px(3.0)).h(px(14.0)).rounded_full().bg(rgb(accent)))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().subtext))
                        .child(title.into()),
                ),
        )
        .child(body)
}

/// A thin utilisation bar (0..=100) drawn with two flex children.
pub fn metric_bar(percent: f32, color: u32) -> Div {
    let percent = percent.clamp(0.0, 100.0);
    div()
        .w_full()
        .h(px(6.0))
        .rounded_full()
        .overflow_hidden()
        .flex()
        .bg(rgb(palette().surface))
        .child(div().h_full().bg(rgb(color)).flex_grow(percent.max(0.5)))
        .child(div().h_full().flex_grow((100.0 - percent).max(0.5)))
}

/// A compact bar sparkline drawn with plain flex elements (no chart dependency).
///
/// The browser shell cannot use `gpui-kit`'s helpers because that crate pulls in
/// `wasm_thread` (nightly-only on wasm), so this is a small local reimplementation
/// of the native shell's widget.
pub fn sparkline(values: &[f32], max: f32, color: u32, height: f32) -> Div {
    let max = max.max(0.001);
    let bars: Vec<Div> = values
        .iter()
        .map(|&value| {
            let fraction = (value / max).clamp(0.0, 1.0);
            div()
                .flex_1()
                .h(px((fraction * height).max(1.0)))
                .rounded_sm()
                .bg(rgb(color))
        })
        .collect();
    h_flex()
        .items_end()
        .gap(px(1.0))
        .w_full()
        .h(px(height))
        .children(bars)
}

// --------------------------------------------------- selected process sparkline (R1-I08)
//
// The overview sparklines are scaled to fixed 0..100 (or network) ranges. A
// single process has no meaningful fixed full scale — CPU can exceed 100 % on a
// multicore machine — so these helpers derive the scale from the retained
// samples and keep the newest reading in text for assistive technology.

/// Full scale for a process sparkline: the observed peak, with a small floor so
/// an all-zero series still has a visible baseline.
pub fn process_sparkline_max(values: &[f32]) -> f32 {
    let peak = values
        .iter()
        .copied()
        .fold(0.0_f32, |peak, value| peak.max(value));
    if peak.is_finite() { peak.max(1.0) } else { 1.0 }
}

/// Visible text for the newest CPU sample of a process series.
pub fn process_sparkline_value_text(values: &[f32]) -> String {
    match values.last().copied() {
        Some(value) if value.is_finite() => format!("{value:.1}%"),
        _ => "—".to_string(),
    }
}

/// Accessible name (`aria-label` equivalent) for a process CPU sparkline.
///
/// The process name, latest value and sample count are spelled out so the chart
/// is understandable without colour perception or a screen-space reading.
pub fn process_sparkline_a11y_label(process: &str, values: &[f32]) -> String {
    if values.is_empty() {
        return format!("{process} の CPU 使用率の推移、データなし");
    }
    format!(
        "{process} の CPU 使用率の推移、最新 {value}、{count} サンプル",
        value = process_sparkline_value_text(values),
        count = values.len()
    )
}

/// A sparkline for the selected process with a stable ElementId and AccessKit name.
///
/// `id` must be unique within a frame — pass a constant such as
/// `process-cpu-sparkline`. The underlying [`sparkline`] and the existing
/// overview DOM hooks / aria identities are left untouched; this is an extra
/// wrapper for the details panel that `shell.rs` will mount.
pub fn process_sparkline(
    id: impl Into<ElementId>,
    process: impl Into<SharedString>,
    values: &[f32],
    color: u32,
    height: f32,
) -> Stateful<Div> {
    let process: SharedString = process.into();
    let label = process_sparkline_a11y_label(&process, values);
    let max = process_sparkline_max(values);
    sparkline(values, max, color, height)
        .id(id)
        .role(Role::Image)
        .aria_label(label)
}

/// A small rounded status label.
pub fn pill(text: impl Into<SharedString>, fg: u32, bg: u32) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded_full()
        .text_xs()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .child(text.into())
}

/// A label/value row for the details panel.
pub fn kv(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Div {
    h_flex()
        .justify_between()
        .gap_2()
        .text_xs()
        .child(div().text_color(rgb(palette().muted)).child(label.into()))
        .child(
            div()
                .text_color(rgb(palette().subtext))
                .truncate()
                .child(value.into()),
        )
}

/// A small section heading.
pub fn section_title(title: &'static str) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette().muted))
        .child(title)
}

// --------------------------------------------------------------- accessibility
//
// The browser shell draws everything on a canvas, so the visible colour of a
// pill or a bar is not reachable by assistive technology. These wrappers add
// stable GPUI/AccessKit identities and textual names to the presentational
// widgets. Callers must pass an id that is unique within a frame (use a tuple
// with the row index for repeated widgets such as process-row status pills).

/// A status pill with an explicit accessible name.
///
/// `text` stays visible so the state is never communicated by colour alone;
/// `label` is exposed as the AccessKit name (the Web/aria equivalent of
/// `aria-label`).
pub fn accessible_pill(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    fg: u32,
    bg: u32,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    pill(text, fg, bg)
        .id(id)
        .role(Role::Label)
        .aria_label(label)
}

// ------------------------------------------------------------ filter preset chip (R1-I06)
//
// Presets are built-in, in-memory shortcuts from the view-model (ADR-012 keeps
// filters out of persisted settings). The chip is purely presentational: the
// shell attaches the click handler and decides where the row is mounted.

/// Visible text for a filter preset chip; the active marker is textual, not colour-only.
pub fn filter_preset_text(preset: &FilterPreset, active: bool) -> String {
    if active {
        format!("✓ {}", preset.label)
    } else {
        preset.label.to_string()
    }
}

/// Accessible name (`aria-label` equivalent) for a filter preset chip.
pub fn filter_preset_a11y_label(preset: &FilterPreset, active: bool) -> String {
    if active {
        format!("フィルタプリセット {} を適用中", preset.label)
    } else {
        format!("フィルタプリセット {} を適用", preset.label)
    }
}

/// AccessKit role for a filter preset chip.
///
/// [`accessible_pill`] marks its pill as `Role::Label`, but a preset chip is
/// clickable (the shell attaches `on_click`), so it must be exposed as a
/// `Role::Button`. Kept as a pure helper so the a11y contract is testable
/// without rendering a window.
pub fn filter_preset_a11y_role() -> Role {
    Role::Button
}

/// A preset chip with a stable ElementId and AccessKit name.
///
/// `id` must be unique within a frame; pass `(id, index)` when building the
/// preset row. Reuses [`accessible_pill`], so no existing DOM hook / aria
/// contract changes; only the role is overridden from `Label` to `Button`
/// because the chip is an action.
pub fn filter_preset_chip(
    id: impl Into<ElementId>,
    preset: &FilterPreset,
    active: bool,
) -> Stateful<Div> {
    let palette = palette();
    let (fg, bg) = if active {
        (palette.base, palette.blue)
    } else {
        (palette.subtext, palette.surface)
    };
    accessible_pill(
        id,
        filter_preset_text(preset, active),
        fg,
        bg,
        filter_preset_a11y_label(preset, active),
    )
    // Override accessible_pill's Role::Label: this pill is clickable.
    .role(filter_preset_a11y_role())
}

/// A utilisation bar with a numeric accessible value and a textual name.
pub fn accessible_metric_bar(
    id: impl Into<ElementId>,
    percent: f32,
    color: u32,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let percent = percent.clamp(0.0, 100.0);
    metric_bar(percent, color)
        .id(id)
        .role(Role::ProgressIndicator)
        .aria_label(label)
        .aria_numeric_value(percent as f64)
        .aria_value(format!("{percent:.0}%"))
}

/// Textual severity for a 0..=100 utilisation value.
///
/// The thresholds mirror [`crate::theme::usage_color`]: a colour-blind reader
/// can still tell whether a value is low, elevated or critical.
pub fn usage_severity(percent: f32) -> &'static str {
    if percent >= 85.0 {
        "高"
    } else if percent >= 60.0 {
        "中"
    } else {
        "低"
    }
}

/// Visible utilisation text: the number followed by its textual severity.
///
/// Mirrors [`temperature_value_text`]: the colour of the number is not the
/// only signal. The same 高/中/低 word used by the accessible name is drawn
/// next to the value, so a colour-blind reader sees the state as well.
pub fn usage_value_text(percent: f32) -> String {
    let percent = percent.clamp(0.0, 100.0);
    format!(
        "{} {}",
        tm_core::format::format_percent(percent),
        usage_severity(percent)
    )
}

// ------------------------------------------------------------ temperature gauge
//
// `critical_c` is the halt threshold and `max_c` the upper operating limit
// reported by the probe. A gauge compares the live reading against both and
// always spells out the resulting state in text: a colour (or a bar length)
// alone is not an accessible state.

/// State of a temperature reading relative to its thresholds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TemperatureSeverity {
    /// Reading or thresholds are missing, so no judgement is possible.
    Unknown,
    /// Below both thresholds.
    Normal,
    /// At or above `max_c`, below `critical_c`.
    Warning,
    /// At or above `critical_c`.
    Critical,
}

impl TemperatureSeverity {
    /// Japanese state label shared by the visible gauge and its AccessKit name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "不明",
            Self::Normal => "正常",
            Self::Warning => "注意",
            Self::Critical => "危険",
        }
    }
}

/// Only finite readings are compared: a NaN from a driver must never be
/// reported as "正常".
fn finite(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite())
}

/// Thresholds must also be strictly positive: a negative `critical_c` is
/// invalid probe data and must not turn every reading into a false alarm.
fn positive(value: Option<f32>) -> Option<f32> {
    finite(value).filter(|value| *value > 0.0)
}

/// Compares `temperature_c` against `critical_c` first, then `max_c`.
///
/// When neither threshold is available the result is [`TemperatureSeverity::Unknown`]
/// rather than an optimistic "正常" — a bare temperature cannot be classified.
pub fn temperature_severity(
    temperature_c: Option<f32>,
    max_c: Option<f32>,
    critical_c: Option<f32>,
) -> TemperatureSeverity {
    let Some(temperature) = finite(temperature_c) else {
        return TemperatureSeverity::Unknown;
    };
    let max = positive(max_c);
    let critical = positive(critical_c);
    if max.is_none() && critical.is_none() {
        return TemperatureSeverity::Unknown;
    }
    if critical.is_some_and(|critical| temperature >= critical) {
        return TemperatureSeverity::Critical;
    }
    if max.is_some_and(|max| temperature >= max) {
        return TemperatureSeverity::Warning;
    }
    TemperatureSeverity::Normal
}

/// Full-scale value for the gauge: `critical_c`, then `max_c`, then 100 °C.
pub fn temperature_reference(max_c: Option<f32>, critical_c: Option<f32>) -> f32 {
    positive(critical_c)
        .or_else(|| positive(max_c))
        .unwrap_or(100.0)
}

/// Visible text for one gauge: value, both thresholds and the textual state.
///
/// Keeping the state in text is what makes the gauge readable without colour
/// perception and in the accessibility tree.
pub fn temperature_value_text(
    temperature_c: Option<f32>,
    max_c: Option<f32>,
    critical_c: Option<f32>,
) -> String {
    let temperature = finite(temperature_c)
        .map(|temperature| format!("{temperature:.0}°C"))
        .unwrap_or_else(|| "—".to_string());
    let max = positive(max_c)
        .map(|max| format!(" / max {max:.0}°C"))
        .unwrap_or_default();
    let critical = positive(critical_c)
        .map(|critical| format!(" / crit {critical:.0}°C"))
        .unwrap_or_default();
    let state = temperature_severity(temperature_c, max_c, critical_c).label();
    format!("{temperature}{max}{critical} {state}")
}

/// Accessible name (`aria-label` equivalent) for a temperature gauge.
pub fn temperature_a11y_label(sensor: &str, value_text: &str) -> String {
    format!("{sensor} 温度 {value_text}")
}

/// A temperature gauge drawn as a thin bar plus a text row.
///
/// The bar is scaled to [`temperature_reference`] and coloured by
/// [`temperature_severity`]; the text repeats the value, thresholds and state so
/// nothing depends on colour alone. `id` must be unique within a frame — pass a
/// `(name, index)` tuple for repeated sensors. The group and the bar get stable
/// AccessKit identities and labels; no DOM hook is touched here.
pub fn temperature_gauge(
    id: impl Into<ElementId>,
    sensor: impl Into<SharedString>,
    temperature_c: Option<f32>,
    max_c: Option<f32>,
    critical_c: Option<f32>,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let sensor: SharedString = sensor.into();
    let severity = temperature_severity(temperature_c, max_c, critical_c);
    let color = match severity {
        TemperatureSeverity::Critical => palette().red,
        TemperatureSeverity::Warning => palette().yellow,
        TemperatureSeverity::Normal => palette().green,
        TemperatureSeverity::Unknown => palette().muted,
    };
    let value_text = temperature_value_text(temperature_c, max_c, critical_c);
    let a11y_label = temperature_a11y_label(&sensor, &value_text);
    let reference = temperature_reference(max_c, critical_c);
    let percent = finite(temperature_c)
        .map(|temperature| (temperature.max(0.0) / reference).clamp(0.0, 1.0) * 100.0)
        .unwrap_or(0.0);

    v_flex()
        .gap_1()
        .id(id.clone())
        .role(Role::Group)
        .aria_label(a11y_label.clone())
        .child(
            h_flex()
                .justify_between()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .min_w_0()
                        .text_color(rgb(palette().muted))
                        .truncate()
                        .child(sensor),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(color))
                        .child(value_text),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(5.0))
                .rounded_full()
                .overflow_hidden()
                .flex()
                .bg(rgb(palette().surface))
                .id((id, "bar"))
                .role(Role::ProgressIndicator)
                .aria_label(a11y_label)
                .aria_numeric_value(percent as f64)
                .aria_value(format!("{percent:.0}%"))
                .child(div().h_full().bg(rgb(color)).flex_grow(percent.max(1.0)))
                .child(div().h_full().flex_grow((100.0 - percent).max(1.0))),
        )
}

// ------------------------------------------------------------- trend badge
//
// A history trend is meaningful only when the direction is spelled out: a
// coloured arrow alone is invisible to assistive technology. The amount and
// the Japanese state label come from the viewmodel, so both shells show the
// same wording. The widget reuses `accessible_pill`, keeping every existing
// DOM hook / ElementId / role / aria contract untouched.

/// Foreground colour for a trend badge (R5-I05).
///
/// A rise means more load (red), a fall means less load (green); flat and
/// unknown states stay neutral so a missing sample never looks like good news.
pub fn trend_badge_color(direction: TrendDirection) -> u32 {
    let palette = palette();
    match direction {
        TrendDirection::Up => palette.red,
        TrendDirection::Down => palette.green,
        TrendDirection::Flat | TrendDirection::Unknown => palette.muted,
    }
}

/// Visible text for a trend badge: cue, signed amount and textual state.
pub fn trend_badge_text(delta: &TrendDelta) -> String {
    delta.badge_text()
}

/// Accessible name (`aria-label` equivalent) for a trend badge.
pub fn trend_a11y_label(metric: &str, delta: &TrendDelta) -> String {
    delta.a11y_label(metric)
}

/// A coloured, textual trend badge with a stable ElementId and AccessKit name.
///
/// `id` must be unique within a frame; pass a metric-specific string such as
/// `trend-cpu`. The existing [`accessible_pill`] applies the id, `Role::Label`
/// and aria-label, so no DOM hook contract is changed here.
pub fn trend_badge(
    id: impl Into<ElementId>,
    metric: impl Into<SharedString>,
    delta: &TrendDelta,
) -> Stateful<Div> {
    let metric: SharedString = metric.into();
    let label = trend_a11y_label(&metric, delta);
    accessible_pill(
        id,
        trend_badge_text(delta),
        trend_badge_color(delta.direction),
        palette().surface,
        label,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_viewmodel::{FilterPresetId, TrendUnit, filter_preset, trend_delta};

    #[test]
    fn usage_severity_has_the_same_thresholds_as_usage_color() {
        // theme::usage_color switches at 60 and 85; keep the textual cue in
        // lockstep so a colour and its label never disagree.
        assert_eq!(usage_severity(0.0), "低");
        assert_eq!(usage_severity(59.9), "低");
        assert_eq!(usage_severity(60.0), "中");
        assert_eq!(usage_severity(84.9), "中");
        assert_eq!(usage_severity(85.0), "高");
        assert_eq!(usage_severity(100.0), "高");
    }

    #[test]
    fn usage_severity_clamps_against_nan_like_inputs() {
        // Negative values cannot come from a probe, but must not be "高".
        assert_eq!(usage_severity(-1.0), "低");
    }

    #[test]
    fn usage_value_text_keeps_the_number_and_spells_out_the_severity() {
        assert_eq!(usage_value_text(42.0), "42.0% 低");
        assert_eq!(usage_value_text(70.0), "70.0% 中");
        assert_eq!(usage_value_text(95.5), "95.5% 高");
        // Non-finite input must not render NaN% or contradict the a11y name.
        assert_eq!(usage_value_text(f32::NAN), "0.0% 低");
    }

    #[test]
    fn temperature_severity_uses_critical_then_max_threshold() {
        let (max, critical) = (Some(68.0), Some(100.0));
        assert_eq!(
            temperature_severity(Some(40.0), max, critical),
            TemperatureSeverity::Normal
        );
        assert_eq!(
            temperature_severity(Some(67.9), max, critical),
            TemperatureSeverity::Normal
        );
        assert_eq!(
            temperature_severity(Some(68.0), max, critical),
            TemperatureSeverity::Warning
        );
        assert_eq!(
            temperature_severity(Some(99.9), max, critical),
            TemperatureSeverity::Warning
        );
        assert_eq!(
            temperature_severity(Some(100.0), max, critical),
            TemperatureSeverity::Critical
        );
        assert_eq!(
            temperature_severity(Some(120.0), max, critical),
            TemperatureSeverity::Critical
        );
    }

    #[test]
    fn temperature_severity_without_a_threshold_is_unknown_not_normal() {
        assert_eq!(
            temperature_severity(Some(42.0), None, None),
            TemperatureSeverity::Unknown
        );
        assert_eq!(
            temperature_severity(None, Some(68.0), Some(100.0)),
            TemperatureSeverity::Unknown
        );
        // A driver may report NaN; that must not be classified as safe.
        assert_eq!(
            temperature_severity(Some(f32::NAN), Some(68.0), Some(100.0)),
            TemperatureSeverity::Unknown
        );
        // A NaN threshold is ignored, not treated as an always-triggered alarm.
        assert_eq!(
            temperature_severity(Some(42.0), None, Some(f32::NAN)),
            TemperatureSeverity::Unknown
        );
        // The same holds for a non-positive threshold.
        assert_eq!(
            temperature_severity(Some(42.0), None, Some(-5.0)),
            TemperatureSeverity::Unknown
        );
    }

    #[test]
    fn temperature_reference_prefers_critical_and_falls_back_safely() {
        assert_eq!(temperature_reference(Some(68.0), Some(100.0)), 100.0);
        assert_eq!(temperature_reference(Some(68.0), None), 68.0);
        assert_eq!(temperature_reference(None, None), 100.0);
        assert_eq!(temperature_reference(Some(0.0), Some(-5.0)), 100.0);
    }

    #[test]
    fn temperature_value_text_spells_out_state_and_thresholds() {
        let warning = temperature_value_text(Some(70.0), Some(68.0), Some(100.0));
        assert!(warning.contains("70°C"), "{warning}");
        assert!(warning.contains("max 68°C"), "{warning}");
        assert!(warning.contains("crit 100°C"), "{warning}");
        assert!(warning.contains("注意"), "{warning}");
        let critical = temperature_value_text(Some(101.0), Some(68.0), Some(100.0));
        assert!(critical.contains("危険"), "{critical}");
        let normal = temperature_value_text(Some(30.0), Some(68.0), Some(100.0));
        assert!(normal.contains("正常"), "{normal}");
        let missing = temperature_value_text(None, None, None);
        assert!(missing.contains("—"), "{missing}");
        assert!(missing.contains("不明"), "{missing}");
    }

    #[test]
    fn temperature_gauge_a11y_label_carries_the_textual_state() {
        let value = temperature_value_text(Some(101.0), Some(68.0), Some(100.0));
        let label = temperature_a11y_label("CPU Packages", &value);
        assert!(label.starts_with("CPU Packages 温度"), "{label}");
        assert!(label.contains("危険"), "{label}");
        assert!(label.contains("max 68°C"), "{label}");
        assert!(label.contains("crit 100°C"), "{label}");
    }

    #[test]
    fn trend_badge_text_carries_direction_without_colour() {
        let up = trend_delta(Some(40.0), Some(42.5), 0.0, TrendUnit::PercentPoints);
        let text = trend_badge_text(&up);
        assert!(text.contains('↑'), "{text}");
        assert!(text.contains("上昇"), "{text}");
        assert!(text.contains("+2.5pt"), "{text}");

        let unknown = trend_delta(None, None, 0.0, TrendUnit::PercentPoints);
        assert_eq!(trend_badge_text(&unknown), "— 不明");
    }

    #[test]
    fn trend_badge_colour_distinguishes_rise_from_fall() {
        let up = trend_badge_color(TrendDirection::Up);
        let down = trend_badge_color(TrendDirection::Down);
        let flat = trend_badge_color(TrendDirection::Flat);
        let unknown = trend_badge_color(TrendDirection::Unknown);
        assert_ne!(up, down, "a rise and a fall must never share a colour");
        assert_ne!(up, flat);
        assert_ne!(down, flat);
        assert_eq!(flat, unknown, "no data must not look like a real fall");
    }

    #[test]
    fn trend_badge_a11y_label_keeps_metric_direction_and_amount() {
        let down = trend_delta(Some(1_556_023.0), Some(0.0), 0.0, TrendUnit::BytesPerSecond);
        let label = trend_a11y_label("ネットワーク", &down);
        assert!(label.starts_with("ネットワーク の推移"), "{label}");
        assert!(label.contains("下降"), "{label}");
        assert!(label.contains("-1.5 MiB/s"), "{label}");
    }

    #[test]
    fn process_sparkline_max_follows_the_peak_with_a_zero_floor() {
        assert_eq!(process_sparkline_max(&[]), 1.0);
        assert_eq!(process_sparkline_max(&[0.0, 0.0]), 1.0);
        assert_eq!(process_sparkline_max(&[0.0, 12.5, 3.0]), 12.5);
        // A NaN reading from a probe must never become a zero-height chart.
        assert_eq!(process_sparkline_max(&[f32::NAN]), 1.0);
    }

    #[test]
    fn process_sparkline_value_text_spells_out_latest_and_missing() {
        assert_eq!(process_sparkline_value_text(&[1.0, 12.34]), "12.3%");
        assert_eq!(process_sparkline_value_text(&[]), "—");
        assert_eq!(process_sparkline_value_text(&[f32::NAN]), "—");
    }

    #[test]
    fn process_sparkline_a11y_label_carries_process_value_and_count() {
        let label = process_sparkline_a11y_label("firefox", &[1.0, 12.34]);
        assert!(label.starts_with("firefox の CPU 使用率の推移"), "{label}");
        assert!(label.contains("12.3%"), "{label}");
        assert!(label.contains("2 サンプル"), "{label}");
        let empty = process_sparkline_a11y_label("firefox", &[]);
        assert!(empty.contains("データなし"), "{empty}");
    }

    #[test]
    fn filter_preset_chip_text_and_label_mark_active_state_without_colour() {
        let preset = filter_preset(FilterPresetId::HighCpu);
        let active = filter_preset_text(preset, true);
        assert!(active.contains('✓'), "{active}");
        assert!(active.contains("高 CPU"), "{active}");
        assert_eq!(filter_preset_text(preset, false), "高 CPU");
        let label = filter_preset_a11y_label(preset, true);
        assert!(label.contains("適用中"), "{label}");
        assert!(label.contains("高 CPU"), "{label}");
    }

    #[test]
    fn filter_preset_chip_is_a_named_button() {
        // The chip overrides accessible_pill's Role::Label because it is
        // clickable, and its accessible name still contains the preset name.
        assert_eq!(filter_preset_a11y_role(), Role::Button);
        let preset = filter_preset(FilterPresetId::HighCpu);
        let name = preset.label;
        for active in [false, true] {
            let label = filter_preset_a11y_label(preset, active);
            assert!(label.contains(name), "{label}");
            let _chip = filter_preset_chip(("filter-preset-test", active as usize), preset, active);
        }
    }

    #[test]
    fn existing_widget_helpers_keep_their_accessible_wording() {
        // Regression guard: adding the new widgets must not disturb the strings
        // relied on by already-wired overview/trend widgets and their DOM hooks.
        let delta = trend_delta(Some(40.0), Some(42.5), 0.0, TrendUnit::PercentPoints);
        assert_eq!(trend_badge_text(&delta), "↑ +2.5pt 上昇");
        assert_eq!(trend_a11y_label("CPU", &delta), "CPU の推移 上昇 +2.5pt");
        assert_eq!(usage_severity(85.0), "高");
    }
}
