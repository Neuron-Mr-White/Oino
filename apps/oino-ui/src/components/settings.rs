//! Settings parts (design.md A2.16): page title, section labels, grouped cards (radius 16,
//! inset dividers at 58) and `SettingRow`s (min 56, padding 12 16, optional 28 icon tile,
//! label 15/500, description 12.5 dim, control on the right).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// The page title (`Display`/600).
pub fn page_title(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    div()
        .pb(px(space::S4))
        .text_role(t, TextRole::Display)
        .weight_semibold()
        .text_color(t.fg())
        .child(text.into())
}

/// A section title: an uppercase section label (A2.15) above a grouped card.
pub fn section_title(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    crate::components::section_label::section_row(t, text, false, None)
}

/// A grouped card of rows (radius 16, `card` fill, inset dividers).
#[must_use]
pub fn group(t: &OinoTheme, rows: Vec<AnyElement>) -> Div {
    crate::components::card::grouped(t, rows, size::ROW_DIVIDER_INSET)
}

/// One row without an icon tile: label (+ modified dot), description, control.
pub fn row(
    t: &OinoTheme,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    modified: bool,
    control: AnyElement,
) -> AnyElement {
    row_with(t, None, label, description, modified, control)
}

/// One row with a 28 px icon tile (radius 8, surface-2, icon 16 muted).
pub fn row_icon(
    t: &OinoTheme,
    glyph: IconName,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    modified: bool,
    control: AnyElement,
) -> AnyElement {
    row_with(t, Some(glyph), label, description, modified, control)
}

/// The 28 px settings icon tile.
#[must_use]
pub fn tile(t: &OinoTheme, glyph: IconName) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(size::SETTING_TILE))
        .rounded(px(radius::R_TILE_SM))
        .bg(t.chip())
        .child(icon(glyph, size::ICON, t.fg_subtle()))
}

fn row_with(
    t: &OinoTheme,
    glyph: Option<IconName>,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    modified: bool,
    control: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::S3_5))
        .min_h(px(size::SETTING_ROW))
        .py(px(space::S3))
        .px(px(space::S4))
        .when_some(glyph, |el, g| el.child(tile(t, g)))
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
                        .text_role(t, TextRole::Prose)
                        .line_height(t.text(TextRole::Base).1)
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
                            .text_color(t.fg_subtlest())
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
