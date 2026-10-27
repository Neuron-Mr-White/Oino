//! The conversation frame of a session (design.md §5.1–§5.3): header, timeline with query
//! rail, status panel, find bar and jump-to-latest pill, the dock and the composer; the
//! welcome layout for empty sessions; overlay panels and modal dialogs for the shell.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, list, px,
};

use crate::components::interaction::InteractionView;
use crate::components::ous::{self, Ous};
use crate::components::unicrab::unicrab;
use crate::design::brand::CrabState;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::composer::greeting;
use crate::model::ui_state::{PanelPlacement, WidgetPlacement};
use crate::views::session_pane::SessionPane;

/// The local UTC offset in seconds, read once (`date +%z`; UTC when unavailable). Warmed
/// off the UI thread when a pane opens.
pub(crate) fn utc_offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let out = std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let z = out.trim();
        let (sign, digits) = match z.split_at_checked(1) {
            Some(("-", d)) => (-1, d),
            Some(("+", d)) => (1, d),
            _ => return 0,
        };
        let (h, m) = digits.split_at_checked(2).unwrap_or(("0", "0"));
        sign * (h.parse::<i64>().unwrap_or(0) * 3600 + m.parse::<i64>().unwrap_or(0) * 60)
    })
}

/// Local hour (0–23) for the greeting; `OINO_LOCAL_HOUR` pins it for screenshots.
fn local_hour() -> u32 {
    if let Some(h) = std::env::var("OINO_LOCAL_HOUR")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        return h % 24;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    u32::try_from((secs + utc_offset()).rem_euclid(86_400) / 3600).unwrap_or(12)
}

/// Interaction card height cap (A2.11: min(64 %, 560)).
const CARD_MAX_H: f32 = 560.0;
/// `cards_open` key of the start header card.
const HEADER_CARD: &str = "$header";

impl SessionPane {
    fn measure(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        canvas(
            move |bounds, _, cx| {
                let _ = this.update(cx, |p, _| {
                    if (p.width - bounds.size.width).abs() > px(space::HALF) {
                        p.width = bounds.size.width;
                    }
                });
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    /// The docked interaction card when one is pending and not modal (touch: unless it
    /// prefers a bottom sheet, which the mobile shell draws).
    fn docked_card(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Entity<InteractionView>> {
        let touch = OinoTheme::get(cx).touch();
        self.interaction
            .as_ref()
            .filter(|v| {
                let v = v.read(cx);
                !v.is_modal() && (!touch || !v.touch_sheet())
            })
            .cloned()
    }

    /// The dock above the composer slot: `aboveEditor` widgets and hints, the queue strip and
    /// the run-state line (A2.11). The interaction card is not here: it takes the composer
    /// slot (see [`Self::render_composer_slot`]).
    fn render_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let widgets = self.render_widgets(WidgetPlacement::AboveEditor, window, cx);
        let queue = self.render_queue(&t, cx);
        let run = self.render_run_state(&t, cx);
        div()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S1_5))
            .children(widgets)
            .children(run)
            .children(queue)
            .into_any_element()
    }

    /// The composer slot (A2.11 L6): a pending interaction card replaces the composer until it
    /// resolves (the draft stays in the editor state); otherwise the composer.
    fn render_composer_slot(
        &mut self,
        welcome: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let Some(card) = self.docked_card(cx) else {
            return self.render_composer(welcome, window, cx);
        };
        // A2.11: the card is at most min(64 % of the frame, 560) high, then scrolls.
        // Measured against the window, not the timeline (which shrinks as the card grows).
        let frame_h = window.viewport_size().height - px(size::DETAIL_HEAD);
        let h = if t.touch() {
            window.viewport_size().height * 0.5
        } else {
            (frame_h * 0.64).min(px(CARD_MAX_H))
        };
        card.update(cx, |v, _| v.set_max_height(h));
        // Touch: a card taller than half the screen moves into a bottom sheet (design.md
        // §6.3), measured after layout.
        let promote = t.touch().then(|| {
            let weak = card.downgrade();
            canvas(
                move |bounds, window, cx| {
                    if bounds.size.height > window.viewport_size().height * 0.5 {
                        let _ = weak.update(cx, InteractionView::promote);
                    }
                },
                |_, (), _, _| {},
            )
            .absolute()
            .size_full()
        });
        div()
            .relative()
            .w_full()
            .child(card)
            .children(promote)
            .into_any_element()
    }

    fn render_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        // A2.17: the splash is an EmptyState: the crab once (88) + one build line. The
        // extension start header (setHeader) is reachable from the card in the header slot
        // once the session has messages; it is never drawn as a second crab (GA-5).
        let build = self.build_line();
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(space::S2_5))
            .child(unicrab(CrabState::Idle, size::CRAB_EMPTY, cx))
            .child(
                div()
                    .text_role(&t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .child(SharedString::from(build)),
            )
            .child(
                div()
                    .pt(px(space::S2))
                    .text_role(&t, TextRole::Title)
                    .weight_semibold()
                    .text_color(t.fg())
                    .child(greeting(local_hour())),
            );
        let dock = self.render_dock(window, cx);
        let composer = self.render_composer_slot(true, window, cx);
        let meta = self.render_meta_row(&t, cx);
        let suggestions = self.render_suggestions(&t, cx);
        if t.touch() {
            // Touch (design.md §6.2): the composer stays docked at the bottom so the soft
            // keyboard lifts it; the hero and suggestions scroll above.
            return div()
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    div()
                        .id("welcome")
                        .flex()
                        .flex_col()
                        .items_center()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(t.gutter())
                        .child(div().flex_1().min_h(px(space::S6)))
                        .child(hero)
                        .child(suggestions)
                        .child(div().flex_1().min_h(px(space::S4))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .gap(px(space::S2))
                        .w_full()
                        .px(t.gutter())
                        .pb(px(space::S2))
                        .child(dock)
                        .child(composer)
                        .children(meta),
                )
                .child(self.measure(cx))
                .into_any_element();
        }
        div()
            .id("welcome")
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .overflow_y_scroll()
            .px(px(size::READING_PAD))
            .child(div().flex_1().min_h(px(size::EMPTY_TOP)))
            .child(hero)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S2))
                    .pt(px(space::S6))
                    .w_full()
                    .max_w(px(size::COMPOSER_MAX))
                    .child(dock)
                    .child(composer)
                    .children(meta),
            )
            .child(suggestions)
            .child(div().flex_1().min_h(px(space::S12)))
            .child(self.measure(cx))
            .into_any_element()
    }

    /// The welcome build line (A2.17): Oino version · model.
    fn build_line(&self) -> String {
        let mut line = format!("Oino {}", env!("CARGO_PKG_VERSION"));
        let model = self.model.model_label();
        if !model.is_empty() {
            line.push_str(" · ");
            line.push_str(&model);
        }
        line
    }

    /// The extension start header (`setHeader`) in a non-empty session: one collapsed
    /// harness card that opens the full header (A2.10, GA-5). Shown only while the timeline
    /// is scrolled to the top, so it scrolls away with the first rows instead of staying
    /// pinned.
    fn render_header_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let headers: Vec<_> = self.model.ui.headers().cloned().collect();
        if headers.is_empty() {
            return None;
        }
        let top = self.list.logical_scroll_top();
        if top.item_ix > 0 || top.offset_in_item > px(space::S1) {
            return None;
        }
        let t = OinoTheme::get(cx);
        let open = self.cards_open.contains(HEADER_CARD);
        let body = open.then(|| {
            let mut o = Ous {
                t,
                on_action: ous::no_action(),
                inputs: None,
                cols: crate::components::ous_cols(self.width.min(px(size::READING))),
                roomy: false,
            };
            let rendered: Vec<AnyElement> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| ous::render(h, &format!("hdr{i}"), &mut o, window, cx))
                .collect();
            div()
                .flex()
                .flex_col()
                .gap(px(space::S1))
                .px(px(space::S3))
                .pb(px(space::S2))
                .children(rendered)
        });
        let pad = if t.touch() {
            t.gutter()
        } else {
            px(size::READING_PAD)
        };
        Some(
            div()
                .w_full()
                .px(pad)
                .pt(px(space::S3))
                .child(
                    div()
                        .w_full()
                        .max_w(px(size::READING))
                        .mx_auto()
                        .rounded(px(radius::R_HARNESS))
                        .border_1()
                        .border_color(t.border().opacity(0.75))
                        .child(
                            div()
                                .id("header-card")
                                .flex()
                                .items_center()
                                .gap(px(space::S2))
                                .min_h(px(size::TOOL_ROW))
                                .px(px(space::S3))
                                .rounded(px(radius::R_HARNESS))
                                .cursor_pointer()
                                .hover(|s| s.bg(t.card()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if !this.cards_open.remove(HEADER_CARD) {
                                        this.cards_open.insert(HEADER_CARD.to_owned());
                                    }
                                    cx.notify();
                                }))
                                .child(unicrab(CrabState::Idle, size::CRAB.0, cx))
                                .child(
                                    div()
                                        .text_role(&t, TextRole::Caption)
                                        .weight_semibold()
                                        .text_color(t.fg())
                                        .child("Unicrab"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_role(&t, TextRole::Caption)
                                        .text_color(t.fg_subtle())
                                        .child(SharedString::from(self.build_line())),
                                )
                                .child(icon(
                                    if open {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    },
                                    size::ICON_MD,
                                    t.fg_subtlest(),
                                )),
                        )
                        .children(body),
                )
                .into_any_element(),
        )
    }

    fn render_suggestions(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut cards: Vec<(
            IconName,
            String,
            String,
            Rc<dyn Fn(&mut SessionPane, &mut Context<SessionPane>)>,
        )> = Vec::new();
        let ready = self
            .model
            .ui
            .panel(crate::model::kanban::BOARD_PANEL)
            .and_then(|p| crate::model::kanban::recognise(&p.spec))
            .map(|b| crate::model::kanban::ready_count(&b));
        if self.has_command("kanboard") {
            cards.push((
                IconName::Kanban,
                ready.map_or_else(|| "Board".to_owned(), |n| format!("Board: {n} ready")),
                "Open the kanboard for this project".to_owned(),
                Rc::new(|p, cx| p.run_prompt("/kanboard open", cx)),
            ));
        }
        for (name, description) in self
            .commands
            .iter()
            .filter(|(n, _)| n.starts_with("skill:") || n.starts_with("prompt:"))
            .take(2)
        {
            let cmd = format!("/{name} ");
            cards.push((
                IconName::Zap,
                name.trim_start_matches("skill:")
                    .trim_start_matches("prompt:")
                    .to_owned(),
                description.clone().unwrap_or_default(),
                Rc::new(move |p, cx| p.set_composer_text(&cmd, cx)),
            ));
        }
        if cards.len() < 3 && self.has_command("info") {
            cards.push((
                IconName::Info,
                "Session dashboard".to_owned(),
                "Usage, modules and tools at a glance".to_owned(),
                Rc::new(|p, cx| p.run_prompt("/info", cx)),
            ));
        }
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .justify_center()
            .gap(px(space::S3))
            .pt(px(space::S6))
            .w_full()
            .max_w(px(size::WELCOME_COMPOSER + space::S12))
            .children(
                cards
                    .into_iter()
                    .take(3)
                    .enumerate()
                    .map(|(i, (glyph, title, line, f))| {
                        div()
                            .id(ElementId::NamedInteger("suggest".into(), i as u64))
                            .flex()
                            .flex_col()
                            .gap(px(space::S2))
                            .w(px(size::SUGGESTION))
                            .p(px(space::S4))
                            .rounded(px(radius::R_CARD))
                            .bg(t.card())
                            .border_1()
                            .border_color(t.card_border())
                            .cursor_pointer()
                            .hover(|s| s.border_color(t.border_hover()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| f(this, cx)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2))
                                    .child(icon(glyph, size::ICON, t.fg_subtle()))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_role(t, TextRole::Base)
                                            .weight_medium()
                                            .child(SharedString::from(title)),
                                    ),
                            )
                            .child(
                                div()
                                    .line_clamp(2)
                                    .text_role(t, TextRole::Sm)
                                    .text_color(t.fg_subtle())
                                    .child(SharedString::from(line)),
                            )
                    }),
            )
            .into_any_element()
    }

    /// The overlay / fullscreen panel or modal dialog, rendered by the shell at window level.
    pub fn render_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some(view) = self.interaction.as_ref().filter(|v| v.read(cx).is_modal()) {
            return Some(view.clone().into_any_element());
        }
        let panel = self
            .model
            .ui
            .panels()
            .find(|p| {
                matches!(
                    p.placement,
                    PanelPlacement::Overlay | PanelPlacement::Fullscreen
                )
            })?
            .clone();
        let t = OinoTheme::get(cx);
        let id = panel.id.clone();
        let on_action = self.action_fn(&id, cx);
        let saved = self.width;
        self.width = px(size::DIALOG_LG);
        let body = self.render_panel_body(&id, true, window, cx);
        self.width = saved;
        let entity = cx.entity().downgrade();
        let close_id = id.clone();
        let width = if panel.placement == PanelPlacement::Fullscreen {
            f32::from(window.viewport_size().width) - space::S8 * 2.0
        } else {
            size::DIALOG_LG
        };
        // A2.12 dialog look (gui-fix-unipi U3): panel `keys` are one footer legend and are
        // bound while the dialog has focus — never a row of buttons (GA-22, GA-23).
        Some(ous::panel_dialog(
            &t,
            &panel,
            width,
            body,
            on_action,
            Rc::new(move |_, cx: &mut App| {
                let _ = entity.update(cx, |p, cx| p.close_panel(&close_id, cx));
            }),
            window,
            cx,
        ))
    }

    /// Jump to latest (A2.10): a 38 circle, surface-3, bottom-right 16 above the composer.
    fn jump_pill(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let max = self.list.max_offset_for_scrollbar().y;
        let at = -self.list.scroll_px_offset_for_scrollbar().y;
        let viewport = self.list.viewport_bounds().size.height;
        if max - at <= viewport {
            return None;
        }
        Some(
            div()
                .id("jump-latest")
                .absolute()
                .bottom(px(space::S4))
                .right(px(space::S4))
                .flex()
                .items_center()
                .justify_center()
                .size(px(size::JUMP))
                .rounded(px(radius::R_FULL))
                .bg(t.bubble())
                .shadow(t.e_fab())
                .cursor_pointer()
                .tooltip(crate::components::tooltip::tip("Jump to latest", None))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.list.scroll_to_end();
                    cx.notify();
                }))
                .child(icon(IconName::ArrowDown, size::ICON, t.fg()))
                .when(self.model.is_busy(), |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(px(space::HALF))
                            .right(px(space::HALF))
                            .child(crate::components::badge::dot(
                                t,
                                crate::components::badge::DotKind::Unread,
                            )),
                    )
                })
                .into_any_element(),
        )
    }
}

impl Render for SessionPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Spinners in this pane tick only this pane (not every split pane).
        crate::design::ticker::Ticker::scoped(cx.entity_id(), || self.render_inner(window, cx))
    }
}

impl SessionPane {
    fn render_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.flush_composer(window, cx);
        self.sync_interaction(window, cx);
        let t = OinoTheme::get(cx);
        let kb = self.focus.is_focused(window);
        if kb != self.timeline_kb {
            self.timeline_kb = kb;
            if let Some(c) = self.cursor.filter(|c| *c < self.model.transcript.len()) {
                self.list.remeasure_items(c..c + 1);
            }
        }
        let state = self.header_state;
        // Touch: the mobile shell draws the app bar and the status capsule row (design.md §6.2).
        let touch = t.touch();
        let header = (!touch).then(|| self.render_header(state, &t, cx));
        if self.model.transcript.is_empty() {
            let welcome = self.render_welcome(window, cx);
            return div()
                .flex()
                .flex_col()
                .size_full()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_pane_key))
                .children(header)
                .child(div().flex_1().min_h_0().child(welcome))
                .into_any_element();
        }
        // Extension `header` widgets stay; the start header (setHeader) is one collapsed card
        // that scrolls away (GA-5).
        let header_widgets: Vec<AnyElement> = self
            .render_header_card(window, cx)
            .into_iter()
            .chain(self.render_widgets(WidgetPlacement::Header, window, cx))
            .collect();
        let timeline = list(self.list.clone(), cx.processor(SessionPane::render_row)).size_full();
        let rail = if touch {
            None
        } else {
            self.render_rail(&t, cx)
        };
        let mut status = if touch {
            None
        } else {
            self.render_status_panel(&t, cx)
        };
        let mini_status = self.status_is_mini();
        let find = self.render_find(&t, cx);
        let pill = self.jump_pill(&t, cx);
        let dock = self.render_dock(window, cx);
        let composer = self.render_composer_slot(false, window, cx);
        let meta = self.render_meta_row(&t, cx);
        let below = self.render_widgets(WidgetPlacement::BelowEditor, window, cx);
        let measure = self.measure(cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_pane_key))
            .children(header)
            .when(!header_widgets.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::S1))
                        .children(header_widgets),
                )
            })
            .when(mini_status, |el| el.children(status.take()))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(timeline)
                    .children(rail)
                    .children(status)
                    .children(find)
                    .children(pill)
                    .child(measure),
            )
            .child(
                // A2.11: the bottom area has padding 0 24 14, max 884, centred.
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .px(if touch { t.gutter() } else { px(space::S6) })
                    .pb(px(space::S3_5))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::S2))
                            .w_full()
                            .max_w(px(size::COMPOSER_MAX))
                            .mx_auto()
                            .child(dock)
                            .child(composer)
                            .children(meta)
                            .children(below),
                    ),
            )
            .into_any_element()
    }
}
