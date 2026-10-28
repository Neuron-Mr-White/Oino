//! Chips (design.md A2.15): tool chip / pill (38, surface-2, active = `chip_active`),
//! segmented track (38 on surface-2, options 32, active surface-3), count badge, status
//! capsule (state colour 13 % + word, 22 high) and package / tag pill (tone 13 % + 45 %
//! border).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Div, ElementId, Hsla, InteractiveElement as _, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::automation::Automatable as _;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// Click callback carrying the chosen index.
pub type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A tool chip (toolbars): 38 high pill on surface-2, 14/500, optional icon.
pub fn tool_chip(
    t: &OinoTheme,
    id: impl Into<ElementId>,
    glyph: Option<IconName>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(space::S1_5))
        .h(px(size::CHIP))
        .px(px(space::S3_5))
        .rounded(px(radius::R_FULL))
        .bg(if active { t.chip_active() } else { t.chip() })
        .when(active, |el| {
            el.border_1().border_color(t.chip_active_border())
        })
        .text_role(t, TextRole::Base)
        .weight_medium()
        .text_color(t.fg())
        .cursor_pointer()
        .when(!active, |el| el.hover(|s| s.bg(t.bubble())))
        .when_some(glyph, |el, g| el.child(icon(g, size::ICON, t.fg_subtle())))
        .child(label.into())
}

/// A segmented control: 38 track on surface-2 with 32 pill options (active surface-3).
pub fn segmented(
    t: &OinoTheme,
    id: impl Into<SharedString>,
    labels: &[&str],
    active: usize,
    on_pick: OnPick,
) -> Div {
    let id: SharedString = id.into();
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(px(size::CHIP))
        .p(px((size::CHIP - size::SEGMENT_OPTION) / 2.0))
        .gap(px(space::HALF))
        .rounded(px(radius::R_FULL))
        .bg(t.chip())
        .children(labels.iter().enumerate().map(|(i, label)| {
            let pick = Rc::clone(&on_pick);
            let selected = i == active;
            div()
                .id(ElementId::Name(format!("{id}-{i}").into()))
                .flex()
                .items_center()
                .h(px(size::SEGMENT_OPTION))
                .px(px(space::S3))
                .rounded(px(radius::R_FULL))
                .cursor_pointer()
                .text_role(t, TextRole::Caption)
                .weight_medium()
                .text_color(if selected { t.fg() } else { t.fg_subtle() })
                .when(selected, |el| el.bg(t.bubble()))
                .when(!selected, |el| el.hover(|s| s.text_color(t.fg())))
                .on_click(move |_: &ClickEvent, w, cx| pick(i, w, cx))
                .child(SharedString::from((*label).to_owned()))
                .automation(
                    crate::automation::Meta::labelled(
                        crate::automation::Role::Segment,
                        (*label).to_owned(),
                    )
                    .selected(selected),
                )
        }))
}

/// A count badge (16–18 high, `Xs` 600 tabular). `amber` for needs-you counts.
#[must_use]
pub fn count_badge(t: &OinoTheme, n: usize, amber: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .min_w(px(size::CRAB_BADGE))
        .h(px(size::CRAB_BADGE))
        .px(px(space::S1))
        .rounded(px(radius::R_FULL))
        .bg(if amber { t.amber() } else { t.bubble() })
        .text_color(if amber { t.primary_fg() } else { t.fg_subtle() })
        .text_role(t, TextRole::Xs)
        .weight_semibold()
        .child(SharedString::from(n.to_string()))
}

/// A status capsule: state colour at 13 % + the word, 22 high.
pub fn status(t: &OinoTheme, color: Hsla, word: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(space::S1))
        .h(px(size::CAPSULE))
        .px(px(space::S2))
        .rounded(px(radius::R_FULL))
        .bg(color.opacity(0.13))
        .text_color(color)
        .text_role(t, TextRole::Sm)
        .weight_medium()
        .child(word.into())
}

/// A tag pill: tone at 13 % fill + 45 % border, `Xs`.
pub fn tag(t: &OinoTheme, color: Hsla, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(size::BADGE))
        .px(px(space::S1_5))
        .rounded(px(radius::R_FULL))
        .bg(color.opacity(0.13))
        .border_1()
        .border_color(color.opacity(0.45))
        .text_color(color)
        .text_role(t, TextRole::Xs)
        .weight_medium()
        .child(text.into())
}
