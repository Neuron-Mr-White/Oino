//! Enter motion (design.md §2.10, unipi §1.4): overlays fade in with a 4 px rise over
//! `NORMAL_MS` (ease-out); nothing animates with reduce motion.

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, ElementId, IntoElement, Styled, ease_out_quint, px,
};

use crate::design::theme::OinoTheme;
use crate::design::tokens::motion::NORMAL_MS;

/// Rise distance of an entering overlay.
const RISE: f32 = 4.0;

/// `el` with the enter animation keyed by `id` (runs once per id while it stays rendered).
pub fn enter<E>(t: &OinoTheme, id: impl Into<ElementId>, el: E) -> AnyElement
where
    E: IntoElement + Styled + 'static,
{
    if t.reduce_motion {
        return el.into_any_element();
    }
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(NORMAL_MS)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).relative().top(px(RISE * (1.0 - d))),
    )
    .into_any_element()
}
