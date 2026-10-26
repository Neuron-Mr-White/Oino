//! `KanbanBoard` (design.md §4.30): the native view of the `unipi.kanboard/board` panel.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_ai::JsonValue;

use crate::components::badge::{badge, pill};
use crate::components::ous::{ActionFn, Ous, render};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::kanban::{Board, Lane, visible};
use crate::model::ous::Tone;

fn lane_color(t: &OinoTheme, id: &str) -> Hsla {
    match id {
        "in_progress" => t.warning(),
        "blocked" => t.danger(),
        "in_review" => t.ansi(12),
        "done" => t.success(),
        "cancelled" => t.fg_subtlest(),
        _ => t.fg_subtle(),
    }
}

fn lane_icon(id: &str) -> IconName {
    match id {
        "in_progress" => IconName::CircleDot,
        "blocked" => IconName::CircleAlert,
        "in_review" => IconName::Eye,
        "done" => IconName::CircleCheck,
        "cancelled" => IconName::CircleX,
        _ => IconName::CircleDashed,
    }
}

fn lane(t: &OinoTheme, lane: &Lane, select: Option<&str>, on_action: &ActionFn) -> AnyElement {
    let c = lane_color(t, &lane.id);
    div()
        .flex()
        .flex_col()
        .w(px(size::LANE))
        .flex_shrink_0()
        .gap(px(space::S2))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .h(px(size::CONTROL))
                .px(px(space::S1))
                .child(icon(lane_icon(&lane.id), size::ICON_MD, c))
                .child(
                    div()
                        .text_role(t, TextRole::Sm)
                        .weight_medium()
                        .child(SharedString::from(lane.title.clone())),
                )
                .child(pill(
                    t,
                    t.tag(),
                    t.fg_subtle(),
                    lane.cards.len().to_string(),
                )),
        )
        .children(lane.cards.iter().map(|card| {
            let cb = Rc::clone(on_action);
            let action = select.map(str::to_owned);
            let id = card.id.clone();
            div()
                .id(ElementId::Name(format!("card-{}", card.id).into()))
                .flex()
                .flex_col()
                .gap(px(space::S1_5))
                .p(px(space::S3))
                .rounded(px(radius::R_XL))
                .bg(t.card())
                .border_1()
                .border_color(if card.selected {
                    t.border_hover()
                } else {
                    t.card_border()
                })
                .when(card.selected, |el| el.shadow(t.e2()))
                .cursor_pointer()
                .hover(|s| s.border_color(t.border_hover()))
                .when_some(action, |el, action| {
                    el.on_click(move |_: &ClickEvent, _, cx| {
                        cb(&action, JsonValue::from(id.clone()), cx);
                    })
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .mono_text(t, TextRole::Xs)
                                .text_color(t.fg_subtlest())
                                .child(SharedString::from(card.id.clone())),
                        )
                        .when_some(card.priority.clone(), |el, p| {
                            el.child(badge(t, card.tone, p))
                        }),
                )
                .child(
                    div()
                        .text_role(t, TextRole::Base)
                        .line_clamp(2)
                        .child(SharedString::from(card.title.clone())),
                )
                .when_some(card.detail.clone(), |el, d| {
                    let tone = if card.tone == Tone::Error {
                        t.danger_surface_fg()
                    } else {
                        t.fg_subtle()
                    };
                    el.child(
                        div()
                            .text_role(t, TextRole::Sm)
                            .text_color(tone)
                            .child(SharedString::from(d)),
                    )
                })
        }))
        .into_any_element()
}

/// Renders a recognised board: toolbar line, lanes (horizontal scroll), detail of the selected
/// task with its move buttons.
pub fn board(
    o: &mut Ous<'_>,
    board: &Board,
    show_backlog: bool,
    key: &str,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = o.t;
    let select = board.select_action.clone();
    let lanes: Vec<AnyElement> = board
        .lanes
        .iter()
        .filter(|l| visible(l, show_backlog))
        .map(|l| lane(&t, l, select.as_deref(), &o.on_action))
        .collect();
    let detail = board
        .detail
        .as_ref()
        .map(|d| render(d, &format!("{key}/detail"), o, window, cx));
    div()
        .flex()
        .flex_col()
        .size_full()
        .gap(px(space::S3))
        .child(
            div()
                .text_role(&t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(SharedString::from(board.header.clone())),
        )
        .child(
            div()
                .id(ElementId::Name(format!("{key}/lanes").into()))
                .flex()
                .flex_row()
                .gap(px(space::S3))
                .overflow_x_scroll()
                .children(lanes),
        )
        .when_some(detail, |el, d| {
            el.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S2))
                    .p(px(space::S4))
                    .rounded(px(radius::R_XL))
                    .border_1()
                    .border_color(t.border())
                    .child(d),
            )
        })
        .into_any_element()
}
