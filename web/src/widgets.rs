//! Small presentational widgets shared by the browser shell.
//!
//! They are deliberately stateless: data in, element out. Interaction (clicks,
//! focus) is wired by `shell.rs`, which keeps the layout code readable and makes
//! it obvious that the browser shell offers no process control.

use gpui::{Div, FontWeight, IntoElement, SharedString, div, prelude::*, px, rgb};

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
        .bg(rgb(C_MANTLE))
        .border_1()
        .border_color(rgb(C_SURFACE))
        .child(
            h_flex()
                .gap_2()
                .child(div().w(px(3.0)).h(px(14.0)).rounded_full().bg(rgb(accent)))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(C_SUBTEXT))
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
        .bg(rgb(C_SURFACE))
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
pub fn kv(label: &'static str, value: impl Into<SharedString>) -> Div {
    h_flex()
        .justify_between()
        .gap_2()
        .text_xs()
        .child(div().text_color(rgb(C_MUTED)).child(label))
        .child(
            div()
                .text_color(rgb(C_SUBTEXT))
                .truncate()
                .child(value.into()),
        )
}

/// A small section heading.
pub fn section_title(title: &'static str) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(C_MUTED))
        .child(title)
}
