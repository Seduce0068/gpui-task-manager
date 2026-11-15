//! Small presentational widgets shared by the native shell.
//!
//! These helpers are deliberately stateless: they take data and return an
//! element. Interaction is injected by the caller through `on_click`, which
//! keeps `app.rs` focused on state transitions.

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::*;

/// A bordered overview card with a coloured accent bar and a title.
pub fn card(title: impl Into<SharedString>, accent: u32, body: impl IntoElement) -> Div {
    v_flex()
        .gap_2()
        .p_4()
        .rounded_lg()
        .bg(rgb(C_MANTLE))
        .border_1()
        .border_color(rgb(C_SURFACE))
        .child(
            h_flex()
                .gap_2()
                .items_center()
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

/// A thin utilisation bar (0..=100).
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
        .items_center()
        .gap_2()
        .text_xs()
        .child(div().text_color(rgb(C_MUTED)).child(label.into()))
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

/// A filled action button with hover and disabled states.
///
/// The caller supplies the click handler (usually `cx.listener(...)`), so the
/// widget itself never needs access to application state.
pub fn action_button(
    id: impl Into<ElementId>,
    label: &'static str,
    color: u32,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(30.0))
        .px_3()
        .rounded_md()
        .text_sm()
        .child(label);
    if enabled {
        base.bg(rgb(color))
            .text_color(rgb(C_BASE))
            .cursor_pointer()
            .hover(|style| style.opacity(0.88))
            .on_click(on_click)
    } else {
        base.bg(rgb(C_SURFACE)).text_color(rgb(C_MUTED))
    }
}
