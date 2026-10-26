//! The Unicrab and the Oino mark (design.md §4.28), painted as quads from
//! [`crate::design::brand`] data (no SVG, no per-cell elements).

use gpui_kit::{
    AnyElement, App, Bounds, Hsla, IntoElement, ParentElement as _, Pixels, Styled as _, Window,
    canvas, fill, point, px, size as gsize,
};

use crate::design::brand::{CRAB_14, CRAB_22, CrabState, cell_color};
use crate::design::theme::{OinoTheme, color};
use crate::design::ticker::Ticker;
use crate::design::{radius, size};

/// The glyph color of a state (16 px silhouette).
#[must_use]
pub fn state_color(t: &OinoTheme, state: CrabState) -> Hsla {
    match state {
        CrabState::NeedsYou => t.brand(),
        CrabState::Working => t.working(),
        CrabState::Done => t.success(),
        CrabState::Idle => t.fg_subtle(),
        CrabState::Offline | CrabState::Sleep => t.fg_subtlest(),
    }
}

/// Opacity of the needs-you pulse (unipi §1.3 "pulsing", 1.2 s period on the 12 fps
/// ticker; held at 1 with reduce motion).
pub fn pulse(cx: &mut App) -> f32 {
    let frame = Ticker::frame(cx);
    if OinoTheme::get(cx).reduce_motion {
        return 1.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "frame modulo 14")]
    let phase = (frame % 14) as f32 / 14.0;
    0.55 + 0.45 * (phase * std::f32::consts::PI).sin()
}

/// The needs-you icon: crab eyes with a `!` (unipi §1.3), pulsing in `fg`.
pub fn needs_you_eyes(px_size: f32, fg: Hsla, cx: &mut App) -> AnyElement {
    let fg = fg.opacity(pulse(cx));
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
            let u = px_size / 6.0;
            let o = bounds.origin;
            let cell = |x: f32, y: f32, w: f32, h: f32| {
                fill(
                    Bounds::new(
                        point(o.x + px(x * u), o.y + px(y * u)),
                        gsize(px(w * u), px(h * u)),
                    ),
                    fg,
                )
            };
            // Two eyes on the visor row, the `!` between and above them.
            window.paint_quad(cell(0.5, 3.0, 1.5, 2.0));
            window.paint_quad(cell(4.0, 3.0, 1.5, 2.0));
            window.paint_quad(cell(2.6, 0.0, 0.8, 2.6));
            window.paint_quad(cell(2.6, 3.4, 0.8, 0.8));
        },
    )
    .size(px(px_size))
    .flex_shrink_0()
    .into_any_element()
}

/// A Unicrab of `px_size` (16 glyph = silhouette in the state color; ≥ 64 animated).
pub fn unicrab(state: CrabState, px_size: f32, cx: &mut App) -> AnyElement {
    let t = OinoTheme::get(cx);
    let glyph = px_size < size::CRAB.1;
    // Only transient states animate (working / needs-you / done): an idle, sleeping or
    // offline crab holds frame 0 so an idle window renders nothing (idle CPU ≈ 0, §2.11).
    let still = matches!(
        state,
        CrabState::Idle | CrabState::Sleep | CrabState::Offline
    );
    let frame = if glyph || t.reduce_motion || still {
        0
    } else {
        // 8 fps from the 12 fps ticker.
        Ticker::frame(cx) * 2 / 3
    };
    let tint = glyph.then(|| state_color(&t, state));
    let dim = matches!(state, CrabState::Offline | CrabState::Sleep);
    let grid: &[&str] = if px_size >= size::CRAB.1 {
        &CRAB_22
    } else {
        &CRAB_14
    };
    let cols = grid.first().map_or(1, |r| r.len());
    let rows = grid.len() + 2;
    #[allow(clippy::cast_precision_loss, reason = "≤ 22 cells")]
    let cell = px_size / cols as f32;
    let hop = state.hop(frame);
    let claws = state.claws_open(frame);
    #[allow(clippy::cast_precision_loss, reason = "≤ 21 rows")]
    let height = cell * rows as f32;
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
            for (r, line) in grid.iter().enumerate() {
                for (c, b) in line.bytes().enumerate() {
                    let Some(cc) = cell_color(b) else {
                        continue;
                    };
                    let mut col = tint.unwrap_or_else(|| color(cc));
                    if dim {
                        col = col.opacity(0.45);
                    }
                    let shift = if claws && r < 4 {
                        if c < cols / 2 { -1.0 } else { 1.0 }
                    } else {
                        0.0
                    };
                    #[allow(clippy::cast_precision_loss, reason = "small grid indices")]
                    let (x, y) = (
                        (c as f32 + shift) * cell,
                        (r as f32 + 2.0 - f32::from(u16::try_from(hop).unwrap_or(0))) * cell,
                    );
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                            gsize(px(cell + 0.5), px(cell + 0.5)),
                        ),
                        col,
                    ));
                }
            }
        },
    )
    .w(px(px_size))
    .h(px(height))
    .flex_shrink_0()
    .into_any_element()
}

/// The Oino mark: a rounded tile with an inner ring (drawn, not an image).
#[must_use]
pub fn oino_mark(t: &OinoTheme, px_size: f32, opacity: f32) -> AnyElement {
    let fg = t.fg().opacity(opacity);
    let ring = px_size * 0.5;
    gpui_kit::div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(px_size))
        .rounded(px(px_size * 0.25))
        .border_2()
        .border_color(fg)
        .child(
            gpui_kit::div()
                .size(px(ring))
                .rounded(px(radius::R_FULL))
                .border_2()
                .border_color(t.brand().opacity(opacity.max(0.6))),
        )
        .into_any_element()
}
