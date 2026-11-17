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

#[cfg(test)]
mod tests {
    use super::*;

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
}
