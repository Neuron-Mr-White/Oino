//! `SectionLabel` (design.md A2.15, L9): 12/16, 500, uppercase, `fg_subtlest`. Callers write
//! sentence case; the label uppercases it. Margins 18 0 10 (4 0 10 when first).
//! [`section_row`] adds a right aside (count / link, 12 dim tabular).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};

/// The label text alone (no margins).
pub fn label(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    let text: SharedString = text.into();
    div()
        .text_role(t, TextRole::Sm)
        .weight_medium()
        .text_color(t.fg_subtlest())
        .truncate()
        .child(SharedString::from(text.to_uppercase()))
}

/// A section label row with the unipi margins and an optional aside.
pub fn section_row(
    t: &OinoTheme,
    text: impl Into<SharedString>,
    first: bool,
    aside: Option<AnyElement>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::S2))
        .pt(px(if first {
            space::S1
        } else {
            space::S4 + space::HALF
        }))
        .pb(px(space::S2_5))
        .child(label(t, text).flex_1().min_w_0())
        .when_some(aside, |el, a| {
            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .child(a),
            )
        })
}

/// A section label with a leading 14 px icon (settings sections).
pub fn with_icon(
    t: &OinoTheme,
    glyph: gpui_kit::assets::IconName,
    text: impl Into<SharedString>,
    first: bool,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::S1_5))
        .pt(px(if first { space::S1 } else { space::S4 }))
        .pb(px(space::S2_5))
        .child(icon(glyph, size::ICON_MD, t.fg_subtlest()))
        .child(label(t, text))
}

/// Count aside text.
#[must_use]
pub fn count(t: &OinoTheme, n: usize) -> AnyElement {
    div()
        .text_role(t, TextRole::Sm)
        .text_color(t.fg_subtlest())
        .child(SharedString::from(n.to_string()))
        .into_any_element()
}
