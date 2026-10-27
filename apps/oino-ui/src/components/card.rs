//! `Card` and grouped lists (design.md A2.12): `card` (surface-1) fill, radius 16, inset 1 px
//! `line`, no shadow. A grouped list is a card whose rows have inset dividers.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, Div, ParentElement as _, Styled as _, div, px};

use crate::design::theme::OinoTheme;
use crate::design::{radius, size};

/// An empty card shell (callers add padding 12–16 and content).
#[must_use]
pub fn card(t: &OinoTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(radius::R_CARD))
        .bg(t.card())
        .border_1()
        .border_color(t.border())
        .overflow_hidden()
}

/// A grouped card of rows with dividers inset `inset` px from the left (58 after a 28 tile
/// row, 14 for text rows).
#[must_use]
pub fn grouped(t: &OinoTheme, rows: Vec<AnyElement>, inset: f32) -> Div {
    let n = rows.len();
    let line = t.row_divider();
    card(t)
        .w_full()
        .children(rows.into_iter().enumerate().map(move |(i, r)| {
            div().relative().child(r).when(i + 1 < n, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(inset))
                        .right_0()
                        .h(px(size::HAIRLINE))
                        .bg(line),
                )
            })
        }))
}
