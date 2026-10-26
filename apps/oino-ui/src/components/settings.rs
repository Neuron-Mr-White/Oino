//! Settings parts (design.md §4.29): page title, section title, bordered `R_XL` groups and
//! `SettingRow`s (label, description, control on the right, min height 48).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// The page title (`Display`/600).
pub fn page_title(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    div()
        .pb(px(space::S6))
        .text_role(t, TextRole::Display)
        .weight_semibold()
        .text_color(t.fg())
        .child(text.into())
}

/// A section title (`Lg`/500).
pub fn section_title(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(space::S4))
        .pb(px(space::S2))
        .text_role(t, TextRole::Lg)
        .weight_medium()
        .text_color(t.fg())
        .child(text.into())
}

/// A bordered group of rows.
#[must_use]
pub fn group(t: &OinoTheme, rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(space::S4))
        .rounded(px(radius::R_CARD))
        .border_1()
        .border_color(t.border())
        .children(rows.into_iter().enumerate().map(move |(i, r)| {
            div()
                .when(i + 1 < n, |el| el.border_b_1().border_color(t.border()))
                .child(r)
        }))
}

/// One row: label (+ modified dot), description, control.
pub fn row(
    t: &OinoTheme,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    modified: bool,
    control: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::S4))
        .min_h(px(size::SETTING_ROW))
        .py(px(space::S2))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(space::HALF))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S1_5))
                        .text_role(t, TextRole::Base)
                        .weight_medium()
                        .text_color(t.fg())
                        .child(label.into())
                        .when(modified, |el| {
                            el.child(
                                div()
                                    .size(px(size::DOT))
                                    .rounded(px(radius::R_FULL))
                                    .bg(t.brand()),
                            )
                        }),
                )
                .when_some(description, |el, d| {
                    el.child(
                        div()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(d),
                    )
                }),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(size::SIDE_PANE_MIN))
                .child(control),
        )
        .into_any_element()
}
