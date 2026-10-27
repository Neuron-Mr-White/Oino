//! Form controls (design.md §4.3): `Segmented`, `Switch`, `Checkbox`, `Radio`, the
//! `ThinkingLevel` bars and the composer context ring.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, Div, ElementId, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathBuilder, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};
use oino_ai::ThinkingLevel;

use crate::design::icons::icon;
use crate::design::theme::OinoTheme;
use crate::design::{radius, size, space};

/// Click callback carrying the chosen index.
pub type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A segmented control (2–5 choices): the A2.15 38 px track with 32 px pill options.
pub fn segmented(
    id: impl Into<SharedString>,
    t: &OinoTheme,
    labels: &[&str],
    active: usize,
    on_pick: OnPick,
) -> Div {
    crate::components::chip::segmented(t, id, labels, active, on_pick)
}

/// A 44 × 26 switch (A2.16): on = `shell` with a cream knob, off = surface-3 with a muted
/// knob.
pub fn switch(
    id: impl Into<ElementId>,
    t: &OinoTheme,
    on: bool,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (w, h) = size::SWITCH_LG;
    let knob = h - space::S1;
    div()
        .id(id.into())
        .flex_shrink_0()
        .w(px(w))
        .h(px(h))
        .rounded(px(radius::R_FULL))
        .bg(if on { t.primary() } else { t.secondary() })
        .cursor_pointer()
        .relative()
        .on_click(move |_: &ClickEvent, win, cx| on_toggle(!on, win, cx))
        .child(
            div()
                .absolute()
                .top(px(space::HALF))
                .left(px(if on {
                    w - knob - space::HALF
                } else {
                    space::HALF
                }))
                .size(px(knob))
                .rounded(px(radius::R_FULL))
                .bg(if on { t.cream() } else { t.fg_subtle() }),
        )
}

/// A 16 px checkbox (or radio when `round`).
#[must_use]
pub fn check(t: &OinoTheme, checked: bool, round: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(size::CHECK))
        .rounded(px(if round { radius::R_FULL } else { radius::R_SM }))
        .border_1()
        .border_color(if checked {
            t.primary()
        } else {
            t.border_hover()
        })
        .when(checked, |el| {
            el.bg(t.primary()).when(round, |el| {
                el.child(
                    div()
                        .size(px(size::DOT))
                        .rounded(px(radius::R_FULL))
                        .bg(t.primary_fg()),
                )
            })
        })
        .when(checked && !round, |el| {
            el.child(icon(IconName::Check, size::ICON_SM, t.primary_fg()))
        })
}

/// Thinking levels in order.
pub const LEVELS: [ThinkingLevel; 7] = [
    ThinkingLevel::Off,
    ThinkingLevel::Minimal,
    ThinkingLevel::Low,
    ThinkingLevel::Medium,
    ThinkingLevel::High,
    ThinkingLevel::Xhigh,
    ThinkingLevel::Max,
];

/// Index of a thinking level.
#[must_use]
pub fn level_index(level: ThinkingLevel) -> usize {
    LEVELS.iter().position(|l| *l == level).unwrap_or(0)
}

/// Short label of a level.
#[must_use]
pub const fn level_label(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "Off",
        ThinkingLevel::Minimal => "Minimal",
        ThinkingLevel::Low => "Low",
        ThinkingLevel::Medium => "Medium",
        ThinkingLevel::High => "High",
        ThinkingLevel::Xhigh => "XHigh",
        ThinkingLevel::Max => "Max",
    }
}

/// The thinking bars (one cell per level above off of 4 × 10, filled `success`).
#[must_use]
pub fn thinking_bars(t: &OinoTheme, level: ThinkingLevel) -> Div {
    let filled = level_index(level);
    let (w, h) = size::THINK_BAR;
    div()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(space::HALF))
        .children((1..LEVELS.len()).map(move |i| {
            div()
                .w(px(w))
                .h(px(h))
                .rounded(px(radius::R_SM / 4.0))
                .bg(if i <= filled { t.success() } else { t.border() })
        }))
}

/// The context ring (`ratio` 0..1; `warning` ≥ 80 %, `danger` ≥ 95 %).
#[must_use]
pub fn context_ring(t: &OinoTheme, ratio: f32) -> AnyElement {
    let ratio = ratio.clamp(0.0, 1.0);
    let fill: Hsla = if ratio >= 0.95 {
        t.danger()
    } else if ratio >= 0.8 {
        t.warning()
    } else {
        t.fg_subtle()
    };
    let track = t.border();
    let d = size::RING;
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
            let r = (d / 2.0) - space::HALF;
            let c = bounds.center();
            let paint = |from: f32, to: f32, color: Hsla, window: &mut Window| {
                if to - from <= 0.0001 {
                    return;
                }
                let mut b = PathBuilder::stroke(px(space::HALF));
                let steps = 48;
                for s in 0..=steps {
                    #[allow(clippy::cast_precision_loss, reason = "≤ 48 steps")]
                    let a = from + (to - from) * (s as f32 / steps as f32);
                    let ang = a * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let p = point(c.x + px(r * ang.cos()), c.y + px(r * ang.sin()));
                    if s == 0 {
                        b.move_to(p);
                    } else {
                        b.line_to(p);
                    }
                }
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            paint(0.0, 1.0, track, window);
            paint(0.0, ratio, fill, window);
        },
    )
    .size(px(d))
    .flex_shrink_0()
    .into_any_element()
}
