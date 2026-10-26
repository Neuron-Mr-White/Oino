//! M9 Board (design.md §6.9, unipi-app §3.9 phone pattern): the `unipi.kanboard/board` panel
//! recognised as lanes (`model::kanban`) and shown as full-width pages in a horizontal snap
//! pager with a lane pill strip (glyph, name, count; active pill `selected`); cards full
//! width (tone rail, id, priority, claim chip); New task pill bottom-right; long-press (or
//! tap) a card → its sheet (detail, allowed moves); pull down to refresh.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, ScrollEvent, SharedString, StatefulInteractiveElement as _,
    Styled as _, TouchPhase, Window, div, point, px,
};
use oino_ai::JsonValue;

use crate::components::badge::badge;
use crate::components::button::Variant;
use crate::components::ous::{self, Ous};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::tokens::touch;
use crate::design::{TextRole, radius, size, space};
use crate::mobile::gesture::long_press;
use crate::mobile::shell::{MobileShell, Sheet, pill_btn};
use crate::model::kanban::{self as kb, BOARD_PANEL, Board, Card, Lane};
use crate::model::ous::Tone;
use crate::model::sessions::workspace_label;
use crate::views::session_pane::SessionPane;

fn lane_glyph(id: &str) -> IconName {
    match id {
        "backlog" => IconName::Archive,
        "todo" | "ready" => IconName::CircleDashed,
        "in_progress" | "doing" => IconName::Play,
        "review" | "in_review" => IconName::Eye,
        "blocked" => IconName::CircleAlert,
        "done" => IconName::CircleCheck,
        _ => IconName::CircleDot,
    }
}

impl MobileShell {
    fn board_of(pane: &Entity<SessionPane>, cx: &Context<Self>) -> Option<Board> {
        let p = pane.read(cx);
        kb::recognise(&p.model.ui.panel(BOARD_PANEL)?.spec)
    }

    fn board_action(&self, action: &str, value: JsonValue, cx: &mut Context<Self>) {
        let action = action.to_owned();
        self.with_pane(cx, |p, cx| p.panel_action(BOARD_PANEL, &action, value, cx));
    }

    fn goto_lane(&mut self, i: usize, width: Pixels, cx: &mut Context<Self>) {
        self.board_lane = i;
        self.board_scroll
            .set_offset(point(-(width * i as f32), Pixels::ZERO));
        cx.notify();
    }

    fn board_card(
        &self,
        t: &OinoTheme,
        card: &Card,
        select: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let claimed = card
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("claimed by"));
        let rail = match card.tone {
            Tone::Error => t.danger(),
            Tone::Warning => t.warning(),
            Tone::Success => t.success(),
            Tone::Info => t.info(),
            _ => t.border(),
        };
        let (id, menu_id) = (card.id.clone(), card.id.clone());
        let entity = cx.entity().downgrade();
        let press = long_press(
            Rc::clone(&self.press),
            Rc::new(move |_, _, cx| {
                let id = menu_id.clone();
                let _ = entity.update(cx, |this, cx| {
                    (crate::mobile::MobilePlatform::get(cx).haptic)();
                    this.open_sheet(Sheet::CardMenu(id), cx);
                });
            }),
        );
        div()
            .id(ElementId::Name(format!("m-card-{}", card.id).into()))
            .relative()
            .flex()
            .flex_row()
            .w_full()
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(if card.selected {
                t.border_hover()
            } else {
                t.card_border()
            })
            .overflow_hidden()
            .cursor_pointer()
            .active(|s| s.bg(t.hover()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(a) = &select {
                    this.board_action(a, JsonValue::from(id.clone()), cx);
                }
                this.open_sheet(Sheet::CardMenu(id.clone()), cx);
            }))
            .child(div().w(px(size::RAIL)).flex_shrink_0().bg(rail))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(space::S1_5))
                    .p(px(space::S3))
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
                            .text_color(t.fg())
                            .line_clamp(3)
                            .child(SharedString::from(card.title.clone())),
                    )
                    .when_some(card.detail.clone(), |el, d| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .text_role(t, TextRole::Sm)
                                .text_color(if claimed {
                                    t.progress_surface_fg()
                                } else if card.tone == Tone::Error {
                                    t.danger_surface_fg()
                                } else {
                                    t.fg_subtle()
                                })
                                .when(claimed, |el| {
                                    el.child(icon(IconName::Bot, size::ICON_SM, t.working()))
                                })
                                .child(SharedString::from(d)),
                        )
                    }),
            )
            .child(press)
            .into_any_element()
    }

    fn lane_page(
        &self,
        t: &OinoTheme,
        lane: &Lane,
        select: Option<&str>,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards: Vec<AnyElement> = lane
            .cards
            .iter()
            .map(|c| self.board_card(t, c, select.map(str::to_owned), cx))
            .collect();
        let empty = cards.is_empty();
        div()
            .id(ElementId::Name(format!("m-lane-{}", lane.id).into()))
            .flex_none()
            .w(width)
            .h_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S2))
                    .px(px(space::S4))
                    .pt(px(space::S2))
                    .pb(px(touch::FAB + space::S8))
                    .children(cards)
                    .when(empty, |el| {
                        el.child(crate::components::empty::empty_compact(
                            t,
                            lane_glyph(&lane.id),
                            "No tasks",
                        ))
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn render_board(
        &mut self,
        pane: &Entity<SessionPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let board = Self::board_of(pane, cx);
        let cwd = pane.read(cx).model.cwd.clone();
        let width = window.viewport_size().width;
        let lanes: Vec<Lane> = board
            .as_ref()
            .map(|b| {
                b.lanes
                    .iter()
                    .filter(|l| kb::visible(l, true))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if self.board_lane >= lanes.len() {
            self.board_lane = 0;
        }
        let ready = board.as_ref().map_or(0, kb::ready_count);
        let header = Self::title(
            &t,
            format!("Board · {}", workspace_label(&cwd)),
            board.as_ref().map(|b| {
                if ready > 0 {
                    format!("{} · {ready} ready", b.header)
                } else {
                    b.header.clone()
                }
            }),
        );
        let bar = Self::app_bar(
            &t,
            Some(Self::back_btn(cx)),
            header,
            vec![Self::icon_btn(
                "m-board-refresh",
                IconName::RefreshCw,
                "Refresh",
                cx.listener(|this, _, _, cx| this.refresh_board(cx)),
            )],
        );
        let active = self.board_lane;
        let pills = div()
            .id("m-lane-pills")
            .flex()
            .flex_row()
            .gap(px(space::S2))
            .px(px(space::S4))
            .py(px(space::S2))
            .flex_none()
            .overflow_x_scroll()
            .children(lanes.iter().enumerate().map(|(i, l)| {
                let on = i == active;
                div()
                    .id(ElementId::NamedInteger("m-pill".into(), i as u64))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(space::S1_5))
                    .h(px(touch::PILL))
                    .px(px(space::S3))
                    .rounded(px(radius::R_FULL))
                    .cursor_pointer()
                    .text_role(&t, TextRole::Sm)
                    .weight_medium()
                    .when(on, |el| el.bg(t.selected()).text_color(t.fg()))
                    .when(!on, |el| {
                        el.border_1()
                            .border_color(t.border())
                            .text_color(t.fg_subtle())
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.goto_lane(i, width, cx);
                    }))
                    .child(icon(
                        lane_glyph(&l.id),
                        size::ICON_SM,
                        if on { t.fg() } else { t.fg_subtle() },
                    ))
                    .child(SharedString::from(l.title.clone()))
                    .child(
                        div()
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(l.cards.len().to_string())),
                    )
            }));
        let select = board.as_ref().and_then(|b| b.select_action.clone());
        let pages: Vec<AnyElement> = lanes
            .iter()
            .map(|l| self.lane_page(&t, l, select.as_deref(), width, cx))
            .collect();
        let n = lanes.len();
        let snap = self.board_scroll.clone();
        let body: AnyElement = if board.is_none() {
            crate::components::empty::empty_state(
                &t,
                IconName::Kanban,
                "Opening the board…",
                Some("Runs /unipi:kanboard in this session.".into()),
                Vec::new(),
            )
            .into_any_element()
        } else {
            div()
                .id("m-lanes")
                .flex()
                .flex_row()
                .size_full()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(&snap)
                .on_scroll(cx.listener(move |this, e: &ScrollEvent, _, cx| {
                    // Snap to the nearest lane when the finger lifts.
                    if matches!(e.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) && n > 0 {
                        let x = -f32::from(this.board_scroll.offset().x);
                        let w = f32::from(width).max(1.0);
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "lane index"
                        )]
                        let i = ((x / w).round().max(0.0) as usize).min(n - 1);
                        this.goto_lane(i, width, cx);
                    }
                }))
                .children(pages)
                .into_any_element()
        };
        let fab = div()
            .absolute()
            .right(px(space::S4))
            .bottom(px(space::S4))
            .rounded(px(radius::R_FULL))
            .shadow(t.e3())
            .child(
                pill_btn(
                    "m-new-task",
                    "New task",
                    Some(IconName::Plus),
                    Variant::Primary,
                )
                .on_click(cx.listener(|this, _, _, cx| this.board_new_task(cx))),
            );
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(pills)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(t.border())
                    .child(body)
                    .child(fab),
            )
            .into_any_element()
    }

    fn refresh_board(&mut self, cx: &mut Context<Self>) {
        let has_refresh = self.active_pane().is_some_and(|p| {
            p.read(cx)
                .model
                .ui
                .panel(BOARD_PANEL)
                .is_some_and(|panel| panel.keys.iter().any(|k| k.action_id == "refresh"))
        });
        if has_refresh {
            self.board_action("refresh", JsonValue::Null, cx);
        } else {
            self.with_pane(cx, |p, cx| p.run_prompt("/unipi:kanboard", cx));
        }
        (crate::mobile::MobilePlatform::get(cx).haptic)();
    }

    fn board_new_task(&mut self, cx: &mut Context<Self>) {
        let has_add = self.active_pane().is_some_and(|p| {
            p.read(cx)
                .model
                .ui
                .panel(BOARD_PANEL)
                .is_some_and(|panel| panel.keys.iter().any(|k| k.action_id == "add"))
        });
        if has_add {
            self.board_action("add", JsonValue::Null, cx);
        } else {
            self.with_pane(cx, |p, cx| p.set_composer_text("/unipi:kanboard add ", cx));
            if let Some(key) = self.active_key() {
                self.show_session(key, cx);
            }
        }
    }

    /// The card sheet: detail (when the extension selected it) and allowed moves.
    pub(crate) fn card_menu(
        &mut self,
        t: &OinoTheme,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(pane) = self.active_pane().cloned() else {
            return div().into_any_element();
        };
        let board = Self::board_of(&pane, cx);
        let card = board.as_ref().and_then(|b| {
            b.lanes
                .iter()
                .flat_map(|l| &l.cards)
                .find(|c| c.id == id)
                .cloned()
        });
        let selected = card.as_ref().is_some_and(|c| c.selected);
        let detail = board
            .as_ref()
            .filter(|_| selected)
            .and_then(|b| b.detail.clone())
            .map(|d| {
                pane.update(cx, |p, cx| {
                    let on_action = p.action_fn(BOARD_PANEL, cx);
                    let mut o = Ous {
                        t: *t,
                        on_action,
                        inputs: Some(&mut p.inputs),
                        cols: crate::components::ous_cols(p.width),
                        roomy: false,
                    };
                    ous::render(&d, "m-board-detail", &mut o, window, cx)
                })
            });
        let moves = board.map(|b| b.moves).unwrap_or_default();
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(d) = detail {
            rows.push(
                div()
                    .px(px(space::S4))
                    .pb(px(space::S2))
                    .child(d)
                    .into_any_element(),
            );
        }
        if selected && !moves.is_empty() {
            rows.push(Self::section(t, "Move to"));
            for (label, action) in moves {
                rows.push(
                    Self::list_row(
                        t,
                        ElementId::Name(format!("m-move-{action}").into()),
                        Some(IconName::ArrowRight),
                        label,
                        None,
                        None,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.board_action(&action, JsonValue::Null, cx);
                        this.close_sheet(cx);
                    }))
                    .into_any_element(),
                );
            }
        } else if !selected {
            rows.push(crate::components::empty::empty_compact(
                t,
                IconName::LoaderCircle,
                "Loading task…",
            ));
        }
        let open_id = id.to_owned();
        rows.push(Self::section(t, "Task"));
        rows.push(
            Self::list_row(
                t,
                "m-card-chat",
                Some(IconName::MessageSquareDashed),
                "Ask Oino about it",
                None,
                None,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let text = format!("Work on kanboard task {open_id}: ");
                this.close_sheet(cx);
                this.with_pane(cx, |p, cx| p.set_composer_text(&text, cx));
                if let Some(key) = this.active_key() {
                    this.show_session(key, cx);
                }
            }))
            .into_any_element(),
        );
        let title: SharedString = card.map_or_else(
            || id.to_owned().into(),
            |c| format!("{} · {}", c.id, c.title).into(),
        );
        let body = div().flex().flex_col().children(rows).into_any_element();
        self.sheet_frame(t, "m-card-menu", Some(title), body, None, true, false, cx)
    }
}
