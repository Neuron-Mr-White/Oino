//! Enter and exit motion (design.md §2.10/A2.5, unipi §1.4, `ZCode`: overlays fade, nothing is
//! springy): overlays fade in with a small rise (`NORMAL_MS` menus, `DIALOG_MS` dialogs,
//! `PALETTE_MS` palette; ease-out); detail views slide in 16 px (`PANE_MS`). A dismissed
//! overlay fades out and drops back over `EXIT_MS` ([`exit`]) instead of vanishing, so the
//! owner keeps it one beat longer (toasts do; see `components::toast`). GPUI divs have no
//! scale transform, so the unipi scale-in is drawn as the rise. Nothing animates with reduce
//! motion, and an exit under reduce motion is a plain removal.

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, ElementId, IntoElement, Styled, ease_out_quint, px,
};

use crate::design::theme::OinoTheme;
use crate::design::tokens::motion::{EXIT_MS, NORMAL_MS, PANE_MS};

/// Rise distance of an entering overlay.
const RISE: f32 = 4.0;
/// Slide distance of an entering detail view.
const SLIDE: f32 = 16.0;

/// `el` with the enter animation keyed by `id` (runs once per id while it stays rendered).
pub fn enter<E>(t: &OinoTheme, id: impl Into<ElementId>, el: E) -> AnyElement
where
    E: IntoElement + Styled + 'static,
{
    enter_ms(t, id, el, NORMAL_MS)
}

/// `el` with a fade + rise over `ms` (dialogs: `DIALOG_MS`, palette: `PALETTE_MS`).
pub fn enter_ms<E>(t: &OinoTheme, id: impl Into<ElementId>, el: E, ms: u64) -> AnyElement
where
    E: IntoElement + Styled + 'static,
{
    if t.reduce_motion {
        return el.into_any_element();
    }
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).relative().top(px(RISE * (1.0 - d))),
    )
    .into_any_element()
}

/// `el` on its way out: fade 1 → 0 and drop by [`RISE`] over `ms` (`EXIT_MS` by default).
/// The caller keeps the element mounted for `ms` (the animation drives its own frames), then
/// removes it.
pub fn exit<E>(t: &OinoTheme, id: impl Into<ElementId>, el: E) -> AnyElement
where
    E: IntoElement + Styled + 'static,
{
    exit_ms(t, id, el, EXIT_MS)
}

/// [`exit`] with an explicit duration.
pub fn exit_ms<E>(t: &OinoTheme, id: impl Into<ElementId>, el: E, ms: u64) -> AnyElement
where
    E: IntoElement + Styled + 'static,
{
    if t.reduce_motion {
        return el.into_any_element();
    }
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()),
        |el, d| el.opacity(1.0 - d).relative().top(px(RISE * d)),
    )
    .into_any_element()
}

/// A detail view sliding in 16 px from the right with a fade (`PANE_MS`, A2.5 `pane-in`).
pub fn pane_in<E>(t: &OinoTheme, id: impl Into<ElementId>, el: E) -> AnyElement
where
    E: IntoElement + Styled + 'static,
{
    if t.reduce_motion {
        return el.into_any_element();
    }
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(PANE_MS)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).relative().left(px(SLIDE * (1.0 - d))),
    )
    .into_any_element()
}
