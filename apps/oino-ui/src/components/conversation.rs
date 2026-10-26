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
use oino_ai::JsonValue;

use crate::components::button::{Btn, BtnSize};
use crate::components::dialog::modal_with_toolbar;
use crate::components::ous::{self, Ous};
use crate::components::unicrab::unicrab;
use crate::design::brand::CrabState;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::composer::greeting;
use crate::model::ui_state::{PanelPlacement, WidgetPlacement};
use crate::views::session_pane::SessionPane;

/// Local hour (0–23); UTC offset from `TZ`-aware `date` is not available without a crate, so
/// the shell passes `OINO_LOCAL_HOUR` in screenshots and falls back to UTC.
fn local_hour() -> u32 {
    if let Some(h) = std::env::var("OINO_LOCAL_HOUR")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        return h % 24;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    u32::try_from((secs / 3600) % 24).unwrap_or(12)
}

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

    fn render_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let widgets = self.render_widgets(WidgetPlacement::AboveEditor, window, cx);
        let queue = self.render_queue(&t, cx);
        let card = self
            .interaction
            .as_ref()
            .filter(|v| !v.read(cx).is_modal())
            .cloned();
        if let Some(card) = &card {
            let h = self.width.max(px(size::CONVERSATION_MIN)) * 0.3;
            card.update(cx, |v, _| v.set_max_height(h));
        }
        let run = self.render_run_state(&t, cx);
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S2))
            .children(widgets)
            .children(queue)
            .when_some(card, gpui_kit::ParentElement::child)
            .children(run)
            .into_any_element()
    }

    fn render_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let headers: Vec<_> = self.model.ui.headers().cloned().collect();
        let hero: AnyElement = if headers.is_empty() {
            unicrab(CrabState::Idle, size::CRAB.2, cx)
        } else {
            let mut o = Ous {
                t,
                on_action: ous::no_action(),
                inputs: None,
                cols: crate::components::ous_cols(self.width.min(px(size::WELCOME_COMPOSER))),
                roomy: false,
            };
            let rendered: Vec<AnyElement> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| ous::render(h, &format!("hdr{i}"), &mut o, window, cx))
                .collect();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(space::S4))
                .w_full()
                .max_w(px(size::WELCOME_COMPOSER))
                .child(unicrab(CrabState::Idle, size::CRAB.1, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(space::S2))
                        .children(rendered),
                )
                .into_any_element()
        };
        let dock = self.render_dock(window, cx);
        let composer = self.render_composer(true, window, cx);
        let meta = self.render_meta_row(&t, cx);
        let suggestions = self.render_suggestions(&t, cx);
        div()
            .id("welcome")
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .size_full()
            .overflow_y_scroll()
            .px(px(space::S4))
            .child(div().flex_1().min_h(px(space::S12)))
            .child(
                // Ember glow behind the welcome header (unipi §1.2 `glow`, A1.2).
                div()
                    .relative()
                    .flex()
                    .justify_center()
                    .w_full()
                    .child(
                        div()
                            .absolute()
                            .top(px(space::S8))
                            .size(px(size::CRAB.2))
                            .rounded(px(radius::R_FULL))
                            .shadow(vec![gpui_kit::BoxShadow {
                                color: t.glow(),
                                offset: gpui_kit::point(px(space::S0), px(space::S0)),
                                blur_radius: px(size::CRAB.2),
                                spread_radius: px(size::CRAB.1),
                                inset: false,
                            }]),
                    )
                    .child(hero),
            )
            .child(
                div()
                    .pt(px(space::S8))
                    .pb(px(space::S6))
                    .text_role(&t, TextRole::Display)
                    .weight_semibold()
                    .text_color(t.fg())
                    .child(greeting(local_hour())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S2))
                    .w_full()
                    .max_w(px(size::WELCOME_COMPOSER))
                    .child(dock)
                    .child(composer)
                    .children(meta),
            )
            .child(suggestions)
            .child(div().flex_1().min_h(px(space::S12)))
            .child(self.measure(cx))
            .into_any_element()
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
        if self.has_command("unipi:kanboard") {
            cards.push((
                IconName::Kanban,
                ready.map_or_else(|| "Board".to_owned(), |n| format!("Board: {n} ready")),
                "Open the kanboard for this project".to_owned(),
                Rc::new(|p, cx| p.run_prompt("/unipi:kanboard", cx)),
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
        if cards.len() < 3 && self.has_command("unipi:info") {
            cards.push((
                IconName::Info,
                "Session dashboard".to_owned(),
                "Usage, modules and tools at a glance".to_owned(),
                Rc::new(|p, cx| p.run_prompt("/unipi:info", cx)),
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
        let toolbar: Vec<AnyElement> = panel
            .keys
            .iter()
            .map(|k| {
                let a = Rc::clone(&on_action);
                let action = k.action_id.clone();
                Btn::new(ElementId::Name(format!("ov-key-{}", k.action_id).into()))
                    .label(SharedString::from(k.label.clone()))
                    .kbd(SharedString::from(k.key.clone()))
                    .size(BtnSize::Sm)
                    .on_click(move |_, _, cx| a(&action, JsonValue::Null, cx))
                    .into_any_element()
            })
            .collect();
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
        Some(modal_with_toolbar(
            &t,
            "overlay-panel",
            SharedString::from(panel.title.clone()),
            width,
            toolbar,
            body,
            Vec::new(),
            Rc::new(move |_, cx: &mut App| {
                let _ = entity.update(cx, |p, cx| p.close_panel(&close_id, cx));
            }),
        ))
    }

    fn jump_pill(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let max = self.list.max_offset_for_scrollbar().y;
        let at = -self.list.scroll_px_offset_for_scrollbar().y;
        let viewport = self.list.viewport_bounds().size.height;
        if max - at <= viewport {
            return None;
        }
        Some(
            div()
                .absolute()
                .bottom(px(space::S3))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("jump-latest")
                        .flex()
                        .items_center()
                        .gap(px(space::S1))
                        .h(px(size::CONTROL))
                        .px(px(space::S3))
                        .rounded(px(radius::R_FULL))
                        .bg(t.popover())
                        .border_1()
                        .border_color(t.popover_border())
                        .shadow(t.e2())
                        .cursor_pointer()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.list.scroll_to_end();
                            cx.notify();
                        }))
                        .child(icon(IconName::ArrowDown, size::ICON_MD, t.fg()))
                        .child("Jump to latest")
                        .when(self.model.is_busy(), |el| {
                            el.child(crate::components::badge::dot(
                                t,
                                crate::components::badge::DotKind::Unread,
                            ))
                        }),
                )
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
        let header = self.render_header(state, &t, cx);
        if self.model.transcript.is_empty() {
            let welcome = self.render_welcome(window, cx);
            return div()
                .flex()
                .flex_col()
                .size_full()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_pane_key))
                .child(header)
                .child(div().flex_1().min_h_0().child(welcome))
                .into_any_element();
        }
        let header_widgets: Vec<AnyElement> = {
            let mut o = Ous {
                t,
                on_action: ous::no_action(),
                inputs: None,
                cols: crate::components::ous_cols(self.width),
                roomy: false,
            };
            let headers: Vec<_> = self.model.ui.headers().cloned().collect();
            headers
                .iter()
                .enumerate()
                .map(|(i, h)| ous::render(h, &format!("hdr{i}"), &mut o, window, cx))
                .collect()
        };
        let header_widgets: Vec<AnyElement> = header_widgets
            .into_iter()
            .chain(self.render_widgets(WidgetPlacement::Header, window, cx))
            .collect();
        let timeline = list(self.list.clone(), cx.processor(SessionPane::render_row)).size_full();
        let rail = self.render_rail(&t, cx);
        let mut status = self.render_status_panel(&t, cx);
        let mini_status = self.status_is_mini();
        let find = self.render_find(&t, cx);
        let pill = self.jump_pill(&t, cx);
        let dock = self.render_dock(window, cx);
        let composer = self.render_composer(false, window, cx);
        let meta = self.render_meta_row(&t, cx);
        let below = self.render_widgets(WidgetPlacement::BelowEditor, window, cx);
        let measure = self.measure(cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_pane_key))
            .child(header)
            .when(!header_widgets.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::S1))
                        .px(px(space::S4))
                        .pt(px(space::S2))
                        .child(
                            div()
                                .w_full()
                                .max_w(px(size::READING))
                                .mx_auto()
                                .flex()
                                .flex_col()
                                .gap(px(space::S1))
                                .children(header_widgets),
                        ),
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
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .px(px(space::S4))
                    .pb(px(space::S2))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::S2))
                            .w_full()
                            .max_w(px(size::READING))
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
