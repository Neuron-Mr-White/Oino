//! Settings parts (round5 #14, `ZCode` `SettingsPage` grammar): 24/30 section titles and
//! `rounded-xl` cards of hairline-divided rows — `px-4 py-3`, label 14 medium,
//! description the same size in the subtle colour, control right-aligned
//! ([scout-ref A(#14)](../../../../../docs/dev/v1/round5/scout-ref.md)). Rows and their
//! descriptions register for the automation driver (`gui-driver.md` §2) so scenarios can
//! assert presence, absence and the Wayland restart note.

use gpui_kit::InteractiveElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::automation::{Automatable as _, Meta, Role};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::type_scale;
use crate::design::{TextRole, radius, size, space};

/// Row horizontal padding (`ZCode` `px-4`).
pub const ROW_PX: f32 = 16.0;
/// Row vertical padding (`ZCode` `py-3`).
pub const ROW_PY: f32 = 12.0;
/// Group card corner radius (`ZCode` `rounded-xl`).
pub const CARD_R: f32 = 12.0;
/// Minimum row height (one 14/20 label line plus `py-3`).
pub const ROW_MIN_H: f32 = 44.0;

/// A section title: 24/30 semibold (`ZCode` `text-ui-settings-title`,
/// `type_scale::zcode::SETTINGS_TITLE`).
pub fn title(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(type_scale::zcode::SETTINGS_TITLE.0))
        .line_height(px(type_scale::zcode::SETTINGS_TITLE.1))
        .weight_semibold()
        .text_color(t.fg())
        .child(text.into())
}

/// A section title with an aside on the right (the hub scope control, a count).
pub fn title_row(t: &OinoTheme, text: impl Into<SharedString>, aside: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(space::S4))
        .pb(px(space::S3))
        .child(title(t, text))
        .child(div().flex_shrink_0().child(aside))
}

/// A grouped card of rows: `rounded-xl`, hairline dividers between rows inset by the row
/// padding (`ZCode` `SettingsGroupCard` + `border-t` rows).
#[must_use]
pub fn group(t: &OinoTheme, rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    let line = t.row_divider();
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(CARD_R))
        .bg(t.card())
        .border_1()
        .border_color(t.border())
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(move |(i, r)| {
            div().relative().child(r).when(i + 1 < n, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(ROW_PX))
                        .right_0()
                        .h(px(size::HAIRLINE))
                        .bg(line),
                )
            })
        }))
}

/// [`group`] only when at least one row survived the search filter (a query that hides
/// every row hides the page's card entirely, like `ZCode`'s filtered groups).
#[must_use]
pub fn group_if(t: &OinoTheme, rows: Vec<AnyElement>) -> Option<Div> {
    (!rows.is_empty()).then(|| group(t, rows))
}

/// One row: label (+ modified dot, + optional badge) and description on the left, the
/// control right-aligned. The row registers as `role: row` with the label, the
/// description as a `text` node, so driver scenarios can select and assert them.
pub fn row(
    t: &OinoTheme,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    modified: bool,
    control: AnyElement,
) -> AnyElement {
    row_with_badge(t, label, description, modified, None, control)
}

/// [`row`] with a small badge after the label (the hub's "Set in project" scope badge,
/// round5 #22e).
pub fn row_with_badge(
    t: &OinoTheme,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    modified: bool,
    badge: Option<AnyElement>,
    control: AnyElement,
) -> AnyElement {
    let label_text: SharedString = label.into();
    div()
        .id(label_text.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::S4))
        .min_h(px(ROW_MIN_H))
        .px(px(ROW_PX))
        .py(px(ROW_PY))
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
                        .flex_wrap()
                        .text_role(t, TextRole::Base)
                        .weight_medium()
                        .text_color(t.fg())
                        .child(label_text.clone())
                        .when_some(badge, Div::child)
                        .when(modified, |el| {
                            el.child(
                                div()
                                    .size(px(space::S1_5))
                                    .rounded(px(radius::R_FULL))
                                    .bg(t.brand()),
                            )
                        }),
                )
                .when_some(description, |el, d| {
                    el.child(
                        div()
                            .id(SharedString::from(format!("{label_text}-desc")))
                            .text_role(t, TextRole::Base)
                            .text_color(t.fg_subtle())
                            .child(d.clone())
                            .automation(Meta::labelled(Role::Text, d.to_string())),
                    )
                }),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(size::SIDE_PANE_MIN))
                .child(control),
        )
        .automation(Meta::labelled(Role::Row, label_text.to_string()))
        .into_any_element()
}
