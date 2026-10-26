//! Touch gestures on top of GPUI's recognizer (taps → clicks, pans → [`ScrollEvent`]s, long
//! presses → [`LongPressEvent`]s):
//!
//! - [`swipe_row`]: a row that scrolls horizontally to reveal trailing actions (swipe left),
//!   snapping open or shut when the finger lifts. Vertical pans fall through to the list.
//! - [`long_press`]: an invisible overlay that claims a long press inside its bounds.
//! - [`Pull`]: pull-to-refresh accounting for a vertical list at its top.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, DispatchPhase, ElementId, InteractiveElement as _, IntoElement,
    LongPressEvent, ParentElement as _, Pixels, Point, ScrollEvent, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, TouchPhase, Window, canvas, div, point, px,
};

use crate::design::tokens::touch;

/// Long-press callback with the press position.
pub(crate) type OnPress = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

/// A row whose `actions` (each [`touch::SWIPE_ACTION`] wide) sit behind its trailing edge.
/// `width` is the row's visible width; `handle` keeps the offset between frames.
pub(crate) fn swipe_row(
    id: impl Into<ElementId>,
    width: Pixels,
    content: AnyElement,
    actions: Vec<AnyElement>,
    handle: &ScrollHandle,
) -> AnyElement {
    let reveal = px(touch::SWIPE_ACTION) * actions.len() as f32;
    let snap = handle.clone();
    div()
        .id(id.into())
        .w(width)
        .flex()
        .flex_row()
        .overflow_x_scroll()
        .restrict_scroll_to_axis()
        .track_scroll(handle)
        .on_scroll(move |e: &ScrollEvent, _, _| {
            // Snap when the finger lifts: past a third of the actions → open, else shut.
            if matches!(e.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                let x = -snap.offset().x;
                let open = x > reveal / 3.0;
                snap.set_offset(point(
                    if open { -reveal } else { Pixels::ZERO },
                    Pixels::ZERO,
                ));
            }
        })
        .child(div().flex_none().w(width).child(content))
        .child(div().flex_none().flex().flex_row().children(actions))
        .into_any_element()
}

/// Closes a swiped row.
pub(crate) fn close_row(handle: &ScrollHandle) {
    handle.set_offset(point(Pixels::ZERO, Pixels::ZERO));
}

/// Opens a swiped row fully (screenshots, accessibility action).
pub(crate) fn open_row(handle: &ScrollHandle, actions: usize) {
    handle.set_offset(point(
        -px(touch::SWIPE_ACTION) * actions as f32,
        Pixels::ZERO,
    ));
}

/// An absolutely positioned overlay that claims long presses starting inside it.
/// `enabled` lets the owner ignore presses while a sheet covers the row.
pub(crate) fn long_press(enabled: Rc<Cell<bool>>, on_press: OnPress) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let on_press = Rc::clone(&on_press);
            let enabled = Rc::clone(&enabled);
            window.on_mouse_event(move |e: &LongPressEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || e.phase != TouchPhase::Started
                    || !enabled.get()
                    || !bounds.contains(&e.start_position)
                {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                on_press(e.start_position, window, cx);
            });
        },
    )
    .absolute()
    .size_full()
    .into_any_element()
}

/// Pull-to-refresh state of one list: distance pulled past the top while the touch is down.
#[derive(Debug, Default)]
pub(crate) struct Pull {
    /// Pulled distance (px, ≥ 0).
    pub(crate) distance: f32,
}

impl Pull {
    /// Feeds a scroll event of the list whose current offset is `offset_y` (0 = top).
    /// Returns `true` when the pull was released past [`touch::PULL_REFRESH`].
    pub(crate) fn feed(&mut self, e: &ScrollEvent, offset_y: Pixels, line: Pixels) -> bool {
        let dy = f32::from(e.delta.pixel_delta(line).y);
        match e.touch_phase {
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let fire = self.distance >= touch::PULL_REFRESH;
                self.distance = 0.0;
                fire
            }
            _ => {
                if offset_y >= Pixels::ZERO && dy > 0.0 {
                    self.distance += dy;
                } else if dy < 0.0 {
                    self.distance = (self.distance + dy).max(0.0);
                }
                false
            }
        }
    }

    /// Progress toward a refresh (0–1).
    pub(crate) fn progress(&self) -> f32 {
        (self.distance / touch::PULL_REFRESH).clamp(0.0, 1.0)
    }
}
