//! `EmptyState` (design.md §4.28).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// A centred empty state: icon tile, title (`Lg`/500), one line, up to two actions.
pub fn empty_state(
    t: &OinoTheme,
    glyph: IconName,
    title: impl Into<SharedString>,
    line: Option<SharedString>,
    actions: Vec<AnyElement>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(space::S3))
        .p(px(space::S8))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(size::EMPTY_TILE))
                .rounded(px(radius::R_XL))
                .bg(t.surface())
                .child(icon(glyph, size::ICON_LG, t.fg_subtle())),
        )
        .child(
            div()
                .text_role(t, TextRole::Lg)
                .weight_medium()
                .text_color(t.fg())
                .child(title.into()),
        )
        .when_some(line, |el, l| {
            el.child(
                div()
                    .text_role(t, TextRole::Base)
                    .text_color(t.fg_subtle())
                    .child(l),
            )
        })
        .when(!actions.is_empty(), |el| {
            el.child(div().flex().gap(px(space::S2)).children(actions))
        })
}

/// The compact variant: icon 20, `Base` title, inline.
pub fn empty_compact(t: &OinoTheme, glyph: IconName, title: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(space::S2))
        .py(px(space::S2))
        .text_color(t.fg_subtle())
        .text_role(t, TextRole::Base)
        .child(icon(glyph, size::ICON_LG, t.fg_subtlest()))
        .child(title.into())
        .into_any_element()
}
