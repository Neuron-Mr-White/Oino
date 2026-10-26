//! The dock above the composer (design.md §4.16–§4.17, §4.9 RunStateLine): banners and
//! `aboveEditor` widgets (`DockStrip`), unipi hints with ‹ › cycling, the `QueuePanel`, the docked
//! interaction card and the run-state line. Stack order top → bottom as specified.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_client::PiCommand;

use crate::components::badge::{kbd, pill};
use crate::components::button::{Btn, BtnSize};
use crate::components::ous::{self, Ous};
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::model::ui_state::WidgetPlacement;
use crate::views::session_pane::SessionPane;

fn strip(t: &OinoTheme) -> gpui_kit::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap(px(space::S2))
        .px(px(space::S3))
        .py(px(space::S1_5))
        .max_h(px(size::CONTROL_LG * 3.0))
        .overflow_hidden()
        .rounded(px(radius::R_LG))
        .bg(t.surface())
        .text_role(t, TextRole::Sm)
        .text_color(t.fg_subtle())
}

impl SessionPane {
    /// Widgets at `placement` as `DockStrips` (hints get ‹ › cycling).
    pub(crate) fn render_widgets(
        &mut self,
        placement: WidgetPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = OinoTheme::get(cx);
        let expanded_status = self.status_expanded_now();
        let widgets: Vec<(String, crate::model::ui_state::Widget)> = self
            .model
            .ui
            .widgets_keyed()
            .filter(|(_, w)| w.placement == placement)
            .map(|(k, w)| (k.to_owned(), w.clone()))
            .collect();
        let mut out = Vec::new();
        for (key, w) in widgets {
            // The footer process line duplicates the StatusPanel's Run section.
            if expanded_status && key.starts_with("unipi.footer/") && key.contains("process") {
                continue;
            }
            let hint = key.starts_with("unipi.core/") && key.contains("hint");
            let body = match &w.spec {
                Some(spec) => {
                    let mut o = Ous {
                        t,
                        on_action: ous::no_action(),
                        inputs: None,
                        cols: crate::components::ous_cols(self.width),
                        roomy: false,
                    };
                    ous::render(spec, &key, &mut o, window, cx)
                }
                None if hint => {
                    let n = w.lines.len().max(1);
                    let line = w
                        .lines
                        .get(self.hint_index % n)
                        .cloned()
                        .unwrap_or_default();
                    div()
                        .truncate()
                        .child(SharedString::from(line))
                        .into_any_element()
                }
                None => div()
                    .flex()
                    .flex_col()
                    .children(
                        w.lines
                            .iter()
                            .map(|l| div().truncate().child(SharedString::from(l.clone()))),
                    )
                    .into_any_element(),
            };
            out.push(
                strip(&t)
                    .when(hint, |el| {
                        el.child(icon(IconName::Lightbulb, size::ICON_MD, t.fg_subtle()))
                    })
                    .child(div().flex_1().min_w_0().child(body))
                    .when(hint && w.lines.len() > 1, |el| {
                        el.child(
                            Btn::icon_only(
                                ElementId::Name(format!("{key}-prev").into()),
                                IconName::ChevronLeft,
                                "Previous tip",
                            )
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hint_index = this.hint_index.wrapping_sub(1);
                                cx.notify();
                            })),
                        )
                        .child(
                            Btn::icon_only(
                                ElementId::Name(format!("{key}-next").into()),
                                IconName::ChevronRight,
                                "Next tip",
                            )
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hint_index = this.hint_index.wrapping_add(1);
                                cx.notify();
                            })),
                        )
                    })
                    .into_any_element(),
            );
        }
        out
    }

    /// The `QueuePanel` (steer / follow-up rows with send-now, edit, remove).
    pub(crate) fn render_queue(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.model.steering.len() + self.model.follow_up.len();
        if n == 0 {
            return None;
        }
        let rows = self
            .model
            .steering
            .iter()
            .map(|s| (true, s))
            .chain(self.model.follow_up.iter().map(|s| (false, s)))
            .enumerate()
            .map(|(i, (steer, text))| {
                let edit = text.clone();
                div()
                    .id(ElementId::NamedInteger(
                        "queue".into(),
                        u64::try_from(i).unwrap_or(0),
                    ))
                    .group(SharedString::from(format!("q{i}")))
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .h(px(size::CONTROL))
                    .px(px(space::S2))
                    .rounded(px(radius::R_MD))
                    .hover(|s| s.bg(t.hover()))
                    .child(if steer {
                        pill(t, t.warning_surface(), t.warning_surface_fg(), "Steer")
                    } else {
                        pill(t, t.idle_task_surface(), t.idle_task(), "Follow-up")
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Base)
                            .text_color(t.fg())
                            .child(SharedString::from(text.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .invisible()
                            .group_hover(
                                SharedString::from(format!("q{i}")),
                                gpui_kit::Styled::visible,
                            )
                            .child(
                                Btn::icon_only(
                                    ElementId::NamedInteger(
                                        "q-edit".into(),
                                        u64::try_from(i).unwrap_or(0),
                                    ),
                                    IconName::Pencil,
                                    "Edit",
                                )
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.composer.read(cx).value().is_empty() {
                                            this.set_composer_text(&edit, cx);
                                            this.send(PiCommand::ClearQueue.into(), cx);
                                        } else {
                                            this.toast("The composer has a draft", cx);
                                        }
                                    },
                                )),
                            ),
                    )
            });
        Some(
            div()
                .flex()
                .flex_col()
                .w_full()
                .p(px(space::S1))
                .rounded(px(radius::R_LG))
                .bg(t.surface())
                .child(
                    div()
                        .id("queue-head")
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .h(px(size::CONTROL))
                        .px(px(space::S2))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.queue_open = !this.queue_open;
                            cx.notify();
                        }))
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(icon(IconName::ListOrdered, size::ICON_MD, t.fg_subtle()))
                        .child(format!("{n} queued"))
                        .child(div().flex_1())
                        .child(
                            Btn::new("queue-clear")
                                .label("Clear")
                                .size(BtnSize::Xs)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send(PiCommand::ClearQueue.into(), cx);
                                })),
                        )
                        .child(icon(
                            if self.queue_open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            size::ICON_SM,
                            t.fg_subtlest(),
                        )),
                )
                .when(self.queue_open, |el| el.children(rows))
                .into_any_element(),
        )
    }

    /// The `RunStateLine` (spinner, working message, elapsed, esc hint).
    pub(crate) fn render_run_state(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.model.is_busy() {
            return None;
        }
        let frame = Ticker::frame(cx);
        let message =
            self.model
                .ui
                .working_message
                .clone()
                .unwrap_or_else(|| match self.model.run {
                    oino_core::events::RunState::Compacting => "Compacting…".to_owned(),
                    oino_core::events::RunState::Retrying => "Retrying…".to_owned(),
                    _ => "Working…".to_owned(),
                });
        let elapsed = self.run_started.map(|s| {
            let secs = s.elapsed().as_secs();
            format!("{}:{:02}", secs / 60, secs % 60)
        });
        let shimmer = if t.reduce_motion {
            1.0
        } else {
            #[allow(clippy::cast_precision_loss, reason = "small frame counter")]
            let phase = (frame % 19) as f32 / 19.0;
            0.6 + 0.4 * (phase * std::f32::consts::TAU).sin().abs()
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .h(px(size::CONTROL))
                .px(px(space::S2))
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(spinner(frame, size::ICON_MD, t.fg_subtle()))
                .child(div().opacity(shimmer).child(SharedString::from(message)))
                .when_some(elapsed, |el, e| {
                    el.child(
                        div()
                            .mono_text(t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(e),
                    )
                })
                .child(div().flex_1())
                .child(kbd(t, "esc"))
                .child(div().text_color(t.fg_subtlest()).child("to stop"))
                .into_any_element(),
        )
    }
}
