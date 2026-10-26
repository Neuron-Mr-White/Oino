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
        "in_progress" => t.working(),
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

/// How the board is shown (D9 toolbar state; the side-pane copy uses the defaults).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoardOptions {
    /// Show the Backlog lane.
    pub show_backlog: bool,
    /// Hide Done / Cancelled lanes.
    pub hide_done: bool,
    /// Search text (id or title, case-insensitive).
    pub query: String,
    /// Filter: `None` all, `Some("blocked")`, `Some("high")` (high / urgent), `Some("claimed")`.
    pub filter: Option<String>,
    /// Keyboard cursor `(lane, card)` among the visible lanes.
    pub cursor: Option<(usize, usize)>,
    /// Render the selected task's detail inline under the lanes (side pane); the D9 board
    /// shows it as an overlay instead.
    pub inline_detail: bool,
}

impl BoardOptions {
    /// Whether `card` passes the search and filter.
    #[must_use]
    pub fn keeps(&self, card: &crate::model::kanban::Card) -> bool {
        let q = self.query.trim().to_lowercase();
        let text_ok = q.is_empty()
            || card.id.to_lowercase().contains(&q)
            || card.title.to_lowercase().contains(&q);
        let filter_ok = match self.filter.as_deref() {
            Some("blocked") => card.tone == Tone::Error && !is_claim(card),
            Some("high") => card
                .priority
                .as_deref()
                .is_some_and(|p| matches!(p, "high" | "urgent")),
            Some("claimed") => is_claim(card),
            _ => true,
        };
        text_ok && filter_ok
    }

    /// Whether `lane` is shown.
    #[must_use]
    pub fn shows(&self, lane: &Lane) -> bool {
        visible(lane, self.show_backlog)
            && !(self.hide_done && matches!(lane.id.as_str(), "done" | "cancelled"))
    }
}

/// A card claimed by an agent (`detail` "claimed by …").
fn is_claim(card: &crate::model::kanban::Card) -> bool {
    card.detail
        .as_deref()
        .is_some_and(|d| d.starts_with("claimed"))
}

/// Visible lanes and their filtered cards.
#[must_use]
pub fn filtered(board: &Board, opts: &BoardOptions) -> Vec<Lane> {
    board
        .lanes
        .iter()
        .filter(|l| opts.shows(l))
        .map(|l| Lane {
            id: l.id.clone(),
            title: l.title.clone(),
            cards: l.cards.iter().filter(|c| opts.keeps(c)).cloned().collect(),
        })
        .collect()
}

fn lane(
    t: &OinoTheme,
    lane: &Lane,
    select: Option<&str>,
    on_action: &ActionFn,
    cursor: Option<usize>,
) -> AnyElement {
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
        .children(lane.cards.iter().enumerate().map(|(ci, card)| {
            let focused = cursor == Some(ci);
            let claimed = is_claim(card);
            let cb = Rc::clone(on_action);
            let action = select.map(str::to_owned);
            let id = card.id.clone();
            div()
                .id(ElementId::Name(format!("card-{}", card.id).into()))
                .flex()
                .flex_col()
                .gap(px(space::S1_5))
                .p(px(space::S3))
                .rounded(px(radius::R_CARD))
                .bg(t.card())
                .border_1()
                .border_color(if card.selected {
                    t.border_hover()
                } else {
                    t.card_border()
                })
                .when(card.selected, |el| el.shadow(t.e2()))
                .when(focused, |el| el.border_2().border_color(t.focus()))
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
                .when_some(card.detail.clone().filter(|_| !claimed), |el, d| {
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
                // Claimed by an agent: `bot` chip in the working colour (not an error).
                .when(claimed, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .h(px(size::BADGE))
                            .px(px(space::S1_5))
                            .rounded(px(radius::R_FULL))
                            .bg(t.progress_surface())
                            .text_color(t.progress_surface_fg())
                            .text_role(t, TextRole::Xs)
                            .child(icon(IconName::Bot, size::ICON_SM, t.progress_surface_fg()))
                            .child(SharedString::from(
                                card.detail
                                    .clone()
                                    .unwrap_or_default()
                                    .trim_start_matches("claimed by ")
                                    .to_owned(),
                            )),
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
    opts: &BoardOptions,
    key: &str,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = o.t;
    let select = board.select_action.clone();
    let lanes: Vec<AnyElement> = filtered(board, opts)
        .iter()
        .enumerate()
        .map(|(li, l)| {
            let cur = opts.cursor.filter(|(cl, _)| *cl == li).map(|(_, cc)| cc);
            lane(&t, l, select.as_deref(), &o.on_action, cur)
        })
        .collect();
    let detail = board
        .detail
        .as_ref()
        .filter(|_| opts.inline_detail)
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
                    .rounded(px(radius::R_CARD))
                    .border_1()
                    .border_color(t.border())
                    .child(d),
            )
        })
        .into_any_element()
}
