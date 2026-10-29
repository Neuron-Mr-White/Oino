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

use crate::automation::Automatable as _;
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent display toggles, not states"
)]
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
    /// Show the dashed "Add task" row under the first lane (the board page).
    pub add_row: bool,
    /// Status id of the card being dragged (round-5 #8): refused lanes dim,
    /// accepted lanes outline.
    pub dragging: Option<String>,
    /// Lane id under the pointer during a drag (its highlight).
    pub hover_lane: Option<String>,
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

fn lane_noun(id: &str) -> &'static str {
    match id {
        "backlog" => "in the backlog",
        "todo" => "to do",
        "in_progress" => "in progress",
        "blocked" => "blocked",
        "in_review" => "to review",
        "done" => "done yet",
        "cancelled" => "cancelled",
        _ => "here",
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "lane head, cards, empty lane, add row"
)]
/// A card dragged between lanes (round-5 #8): `on_drag` carries it, lanes
/// accept it in `can_drop`/`on_drop` and report hover through `on_drag_move`.
pub struct BoardDrag {
    /// Task id.
    pub id: String,
    /// Source lane's status id.
    pub from: String,
}

/// Whether a user move from `from` to `to` is a legal transition (the same
/// rule the board's `move` buttons use — dimmed lanes refuse the drop too).
fn accepts(from: &str, to: &str) -> bool {
    let (Ok(from), Ok(to)) = (
        from.parse::<oino_kanboard::Status>(),
        to.parse::<oino_kanboard::Status>(),
    ) else {
        return false;
    };
    oino_kanboard::transitions::allowed_targets(from, oino_kanboard::Actor::User).contains(&to)
}

/// The drag preview (a small card ghost following the pointer).
struct CardGhost {
    id: String,
    title: String,
}

impl gpui_kit::Render for CardGhost {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(space::S1))
            .w(px(size::LANE))
            .p(px(space::S3))
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.focus())
            .shadow(t.e_toast())
            .child(
                div()
                    .mono_text(&t, TextRole::Xs)
                    .text_color(t.fg_subtlest())
                    .child(SharedString::from(self.id.clone())),
            )
            .child(
                div()
                    .text_role(&t, TextRole::Prose)
                    .weight_semibold()
                    .text_color(t.fg())
                    .line_clamp(2)
                    .child(SharedString::from(self.title.clone())),
            )
            .into_any_element()
    }
}

/// Per-lane drag state (round-5 #8): a drag is live, the lane accepts the
/// dragged card (`accept`), and the pointer hovers it (`hover`).
#[derive(Clone, Copy, Default)]
pub struct DropHint {
    /// A board card drag is live.
    pub dragging: bool,
    /// The dragged card's status may move to this lane.
    pub accept: bool,
    /// The pointer is over this lane.
    pub hover: bool,
}

#[allow(clippy::too_many_lines, reason = "one card builder")]
fn lane(
    t: &OinoTheme,
    lane: &Lane,
    select: Option<&str>,
    on_action: &ActionFn,
    cursor: Option<usize>,
    add: bool,
    drop: Option<DropHint>,
) -> AnyElement {
    let c = lane_color(t, &lane.id);
    let n = lane.cards.len();
    let head = format!("{}, {n} task{}", lane.title, if n == 1 { "" } else { "s" }).to_uppercase();
    let empty = lane.cards.is_empty().then(|| {
        // A2.17: a dashed card with the lane glyph ("Nothing blocked").
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::S2))
            .h(px(size::LANE * 0.3))
            .rounded(px(radius::R_CARD))
            .border_1()
            .border_dashed()
            .border_color(t.border())
            .text_role(t, TextRole::Caption)
            .text_color(t.fg_subtlest())
            .child(icon(lane_icon(&lane.id), size::ICON_LG, c))
            .child(SharedString::from(format!(
                "Nothing {}",
                lane_noun(&lane.id)
            )))
            .into_any_element()
    });
    let lane_id = lane.id.clone();
    let add_row = add.then(|| {
        let cb = Rc::clone(on_action);
        div()
            .id(ElementId::Name(format!("lane-add-{}", lane.id).into()))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(space::S1_5))
            .h(px(size::BUTTON_SM + space::S1))
            .rounded(px(radius::R_FULL))
            .border_1()
            .border_dashed()
            .border_color(t.border())
            .text_role(t, TextRole::Caption)
            .text_color(t.fg_subtle())
            .cursor_pointer()
            .hover(|s| s.bg(t.hover()).text_color(t.fg()))
            .on_click(move |_: &ClickEvent, _, cx| cb("add", JsonValue::Null, cx))
            .child(icon(IconName::Plus, size::ICON_SM, t.fg_subtle()))
            .child("Add task")
            .automation(crate::automation::Meta::labelled(
                crate::automation::Role::Button,
                format!("Add task ({})", lane.title),
            ))
            .into_any_element()
    });
    div()
        .id(ElementId::Name(format!("lane-{}", lane.id).into()))
        .flex()
        .flex_col()
        .w(px(size::LANE))
        .flex_shrink_0()
        .gap(px(space::S2_5))
        .rounded(px(radius::R_CARD))
        // round-5 #8: the column is the drop target. `can_drop` refuses illegal
        // transitions (the OS no-drop cursor); refused lanes also dim while a
        // drag is live, the hovered accepted lane takes the focus border.
        .border_1()
        .border_color(drop.map_or_else(gpui_kit::transparent_black, |d| {
            if d.hover && d.accept {
                t.focus()
            } else if d.dragging && d.accept {
                t.fg_subtlest()
            } else {
                gpui_kit::transparent_black()
            }
        }))
        .when(drop.is_some_and(|d| d.dragging && !d.accept), |el| {
            el.opacity(0.45)
        })
        .can_drop({
            let to = lane_id.clone();
            move |v: &dyn std::any::Any, _, _| {
                v.downcast_ref::<BoardDrag>()
                    .is_some_and(|d| accepts(&d.from, &to))
            }
        })
        .on_drop::<BoardDrag>({
            let cb = Rc::clone(on_action);
            let to = lane_id.clone();
            move |d, _, cx| cb(&format!("move:{to}"), JsonValue::from(d.id.clone()), cx)
        })
        .on_drag_move::<BoardDrag>({
            let cb = Rc::clone(on_action);
            let to = lane_id.clone();
            move |_: &gpui_kit::DragMoveEvent<BoardDrag>, _, cx| {
                cb(&format!("hover:{to}"), JsonValue::Null, cx);
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::S1_5))
                .h(px(size::BADGE + space::S1))
                .text_role(t, TextRole::Sm)
                .weight_medium()
                .text_color(t.fg_subtle())
                .child(
                    div()
                        .size(px(space::S1_5))
                        .rounded(px(radius::R_FULL))
                        .bg(c),
                )
                .child(SharedString::from(head)),
        )
        .children(empty)
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
                .gap(px(space::S1))
                .p(px(space::S3))
                .rounded(px(radius::R_CARD))
                .bg(t.card())
                .border_1()
                .border_color(if card.selected || focused {
                    t.chip_active_border()
                } else {
                    t.card_border()
                })
                .when(card.selected, |el| el.bg(t.hover()))
                .when(focused, |el| el.border_color(t.focus()))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover()).border_color(t.fg_subtlest().opacity(0.55)))
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
                        .gap(px(space::S2))
                        .child(
                            div()
                                .mono_text(t, TextRole::Xs)
                                .text_color(t.fg_subtlest())
                                .child(SharedString::from(card.id.clone())),
                        )
                        .when_some(card.priority.clone(), |el, p| {
                            el.child(crate::components::chip::tag(t, t.tone_fg(card.tone), p))
                        }),
                )
                .child(
                    div()
                        .text_role(t, TextRole::Prose)
                        .weight_semibold()
                        .text_color(t.fg())
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
                            .text_role(t, TextRole::Caption)
                            .text_color(tone)
                            .line_clamp(2)
                            .child(SharedString::from(d)),
                    )
                })
                // Claimed by an agent: a `shell` pill at 14 % ("● agent …").
                .when(claimed, |el| {
                    el.child(
                        div().flex().child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .h(px(size::BADGE))
                                .px(px(space::S2))
                                .rounded(px(radius::R_FULL))
                                .bg(t.brand().opacity(0.14))
                                .border_1()
                                .border_color(t.brand().opacity(0.38))
                                .text_color(t.brand())
                                .text_role(t, TextRole::Xs)
                                .weight_medium()
                                .child(
                                    div()
                                        .size(px(space::S1_5))
                                        .rounded(px(radius::R_FULL))
                                        .bg(t.brand()),
                                )
                                .child(SharedString::from(
                                    card.detail
                                        .clone()
                                        .unwrap_or_default()
                                        .trim_start_matches("claimed by ")
                                        .to_owned(),
                                )),
                        ),
                    )
                })
                // round-5 #8: press-and-drag the card; the ghost follows the
                // pointer and `drag-start` lets the view dim refused lanes.
                // (`on_drag` needs the stateful div, so it sits before
                // `.automation`'s Probe wrapper.)
                .on_drag(
                    BoardDrag {
                        id: card.id.clone(),
                        from: lane_id.clone(),
                    },
                    {
                        let cb = Rc::clone(on_action);
                        let title = card.title.clone();
                        move |d: &BoardDrag, _point, _window, cx| {
                            cb(&format!("drag-start:{}", d.from), JsonValue::Null, cx);
                            gpui_kit::AppContext::new(cx, |_| CardGhost {
                                id: d.id.clone(),
                                title: title.clone(),
                            })
                        }
                    },
                )
                .automation(
                    crate::automation::Meta::labelled(
                        crate::automation::Role::Card,
                        format!("{} {}", card.id, card.title),
                    )
                    .selected(card.selected)
                    .disabled(select.is_none()),
                )
        }))
        .children(add_row)
        // The drop target the driver drags to (`to_id: "lane-<status>"` /
        // `to_label: "lane In Progress"`); last in the chain (`Probe` wraps
        // the finished element).
        .automation(crate::automation::Meta::labelled(
            crate::automation::Role::Row,
            format!("lane {}", lane.title),
        ))
        .into_any_element()
}

/// The lane strip (A2.17): one 38 px pill per visible lane with its glyph, name and count;
/// the pill of the cursor's lane takes the lane colour at 13 %. Clicking moves the cursor.
pub fn lane_strip(
    t: &OinoTheme,
    lanes: &[Lane],
    active: Option<usize>,
    key: &str,
    on_pick: Rc<dyn Fn(usize, &mut Window, &mut App)>,
) -> AnyElement {
    div()
        .id(ElementId::Name(format!("{key}/strip").into()))
        .flex()
        .flex_row()
        .gap(px(space::S2))
        .overflow_x_scroll()
        .children(lanes.iter().enumerate().map(|(i, l)| {
            let c = lane_color(t, &l.id);
            let on = active == Some(i);
            let pick = Rc::clone(&on_pick);
            div()
                .id(ElementId::NamedInteger(
                    SharedString::from(format!("{key}/strip")),
                    u64::try_from(i).unwrap_or(0),
                ))
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(space::S1_5))
                .h(px(size::CHIP))
                .px(px(space::S3_5))
                .rounded(px(radius::R_FULL))
                .bg(if on { c.opacity(0.13) } else { t.chip() })
                .when(on, |el| el.border_1().border_color(c.opacity(0.45)))
                .text_role(t, TextRole::Base)
                .weight_medium()
                .text_color(if on { t.fg() } else { t.fg_subtle() })
                .cursor_pointer()
                .hover(|s| s.bg(t.bubble()))
                .on_click(move |_: &ClickEvent, w, cx| pick(i, w, cx))
                .child(icon(lane_icon(&l.id), size::ICON_MD, c))
                .child(SharedString::from(l.title.clone()))
                .child(crate::components::chip::count_badge(
                    t,
                    l.cards.len(),
                    false,
                ))
                .automation(
                    crate::automation::Meta::labelled(
                        crate::automation::Role::Tab,
                        l.title.clone(),
                    )
                    .selected(on),
                )
        }))
        .into_any_element()
}

/// The phone-width board (round-5 #10): a lane pill strip on top, then the
/// active lane's cards as one scrollable column, then the selected task's
/// detail inline. The side pane's `unipi.kanboard/board` tab renders this.
#[allow(clippy::too_many_arguments, reason = "view plumbing, not data")]
pub fn phone_board(
    o: &mut Ous<'_>,
    board: &Board,
    active: usize,
    key: &str,
    on_lane: Rc<dyn Fn(usize, &mut Window, &mut App)>,
    dragging: Option<&str>,
    hover: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = o.t;
    let active = active.min(board.lanes.len().saturating_sub(1));
    let strip = lane_strip(&t, &board.lanes, Some(active), key, on_lane);
    let cards = board.lanes.get(active).map(|l| {
        lane(
            &t,
            l,
            board.select_action.as_deref(),
            &o.on_action,
            None,
            false,
            dragging.map(|d| DropHint {
                dragging: true,
                accept: accepts(d, &l.id),
                hover: hover == Some(l.id.as_str()),
            }),
        )
    });
    let detail = board
        .detail
        .as_ref()
        .map(|d| render(d, &format!("{key}/detail"), o, window, cx));
    div()
        .flex()
        .flex_col()
        .size_full()
        .gap(px(space::S3))
        .child(strip)
        .child(
            div()
                .id(ElementId::Name(format!("{key}/lane").into()))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(space::S3))
                .children(cards)
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
                }),
        )
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
            lane(
                &t,
                l,
                select.as_deref(),
                &o.on_action,
                cur,
                li == 0 && opts.add_row,
                opts.dragging.as_deref().map(|d| DropHint {
                    dragging: true,
                    accept: accepts(d, &l.id),
                    hover: opts.hover_lane.as_deref() == Some(l.id.as_str()),
                }),
            )
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
        .when(!opts.add_row, |el| {
            el.child(
                div()
                    .text_role(&t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(board.header.clone())),
            )
        })
        .child(
            div()
                .id(ElementId::Name(format!("{key}/lanes").into()))
                .flex_1()
                .min_h_0()
                .items_start()
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
