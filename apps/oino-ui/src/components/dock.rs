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

use gpui_kit::component::input::Input;

use crate::automation::Automatable as _;
use crate::components::badge::kbd;
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
        .rounded(px(radius::R_INPUT))
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

    /// The queue strip (A2.11): surface-1, radius 16, rows 13 px with a kind tag (Steer
    /// amber / Queue gold) and ↑ send now, ✎ edit in place, ✕ remove (`_oino/queue_edit`).
    #[allow(clippy::too_many_lines, reason = "row layout with three actions")]
    pub(crate) fn render_queue(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.model.steering.len() + self.model.follow_up.len();
        if n == 0 {
            return None;
        }
        let editing = self.queue_edit.as_ref().map(|(q, i, input)| {
            let row = match q {
                oino_client::protocol::QueueKind::Steering => *i,
                oino_client::protocol::QueueKind::FollowUp => self.model.steering.len() + *i,
            };
            (row, input.clone())
        });
        let rows = self
            .model
            .steering
            .iter()
            .map(|s| (true, s))
            .chain(self.model.follow_up.iter().map(|s| (false, s)))
            .enumerate()
            .map(|(i, (steer, text))| {
                let id = |name: &'static str| {
                    ElementId::NamedInteger(name.into(), u64::try_from(i).unwrap_or(0))
                };
                let (tag_color, tag) = if steer {
                    (t.amber(), "Steer")
                } else {
                    (t.gold(), "Queue")
                };
                let field = editing
                    .as_ref()
                    .filter(|(row, _)| *row == i)
                    .map(|(_, input)| input.clone());
                let (up, down) = self.queue_can_move(i);
                div()
                    .id(id("queue"))
                    .group(SharedString::from(format!("q{i}")))
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .min_h(px(size::CONTROL_LG))
                    .px(px(space::S2))
                    .rounded(px(radius::R_ROW))
                    .hover(|s| s.bg(t.hover()))
                    .child(crate::components::chip::tag(t, tag_color, tag))
                    .child(match field {
                        Some(input) => div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input).appearance(false))
                            .into_any_element(),
                        None => div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Caption)
                            .text_color(t.fg())
                            .child(SharedString::from(text.clone()))
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(space::HALF))
                            .child(
                                Btn::icon_only(id("q-up"), IconName::ChevronUp, "Move up")
                                    .size(BtnSize::Sm)
                                    .disabled(!up)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.queue_up(i, cx);
                                    })),
                            )
                            .child(
                                Btn::icon_only(id("q-down"), IconName::ChevronDown, "Move down")
                                    .size(BtnSize::Sm)
                                    .disabled(!down)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.queue_down(i, cx);
                                    })),
                            )
                            .child(
                                Btn::icon_only(id("q-now"), IconName::ArrowUp, "Send now")
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.queue_send_now(i, cx);
                                    })),
                            )
                            .child(
                                Btn::icon_only(id("q-edit"), IconName::Pencil, "Edit")
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.queue_edit.is_some() {
                                            this.queue_commit_edit(cx);
                                        } else {
                                            this.queue_begin_edit(i, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                Btn::icon_only(
                                    id("q-recall"),
                                    IconName::CornerDownLeft,
                                    "Recall to composer",
                                )
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.queue_recall(i, cx);
                                    },
                                )),
                            )
                            .child(
                                Btn::icon_only(id("q-remove"), IconName::X, "Remove")
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.queue_remove(i, cx);
                                    })),
                            ),
                    )
                    // Read-back for the driver: the row text in queue order.
                    .automation(crate::automation::Meta::labelled(
                        crate::automation::Role::Row,
                        format!("{tag} {text}"),
                    ))
            });
        Some(
            div()
                .flex()
                .flex_col()
                .w_full()
                .p(px(space::S1))
                .rounded_t(px(radius::R_FLOAT))
                .bg(t.card())
                .border_1()
                .border_color(t.border())
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
                        .child(icon(IconName::ListOrdered, size::ICON_MD, t.fg_subtlest()))
                        .child(format!("{n} queued"))
                        .child(div().flex_1())
                        .child(
                            Btn::new("queue-clear")
                                .label("Clear")
                                .variant(crate::components::button::Variant::Ghost)
                                .size(BtnSize::Xs)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.queue_edit = None;
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
                .when(self.queue_open, |el| {
                    el.child(
                        div()
                            .id("queue-rows")
                            .flex()
                            .flex_col()
                            .max_h(px(size::QUEUE_MAX))
                            .overflow_y_scroll()
                            .children(rows),
                    )
                })
                .into_any_element(),
        )
    }

    /// The `RunStateLine` (spinner, working message, elapsed, esc hint). After a run it stays
    /// mounted for `EXIT_MS` and fades out (`design::motion::exit`).
    pub(crate) fn render_run_state(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.model.is_busy() {
            let (_, label) = self.run_exit.as_ref()?;
            let line = div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .h(px(size::CONTROL))
                .px(px(space::S2))
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(div().child(SharedString::from(label.clone())));
            return Some(crate::design::motion::exit(
                t,
                ElementId::Name(format!("run-exit-{}", self.model.id).into()),
                line,
            ));
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
                .child(spinner(frame, size::ICON_MD, t.working()))
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
