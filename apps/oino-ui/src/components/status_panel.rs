//! `StatusPanel` (design.md §4.19): floating Goal / Progress / Run / Git sections, expanded at
//! ≥ 1280 px (mode Auto) or as a mini capsule.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, Div, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::components::button::{Btn, BtnSize};
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::tokens::breakpoint;
use crate::design::{TextRole, radius, size, space};
use crate::host::StatusPanelMode;
use crate::model::status::RunKind;
use crate::views::session_pane::{SessionPane, SideTab};

fn section(t: &OinoTheme, title: &str, state: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::S1_5))
        .h(px(size::CONTROL_SM))
        .text_role(t, TextRole::Sm)
        .child(
            div()
                .weight_medium()
                .text_color(t.fg())
                .child(SharedString::from(title.to_owned())),
        )
        .child(icon(IconName::ChevronDown, size::ICON_SM, t.fg_subtlest()))
        .child(div().text_color(t.fg_subtle()).child(state))
}

impl SessionPane {
    /// Whether the status shows as the mini capsule strip (above the timeline, not floating).
    pub(crate) fn status_is_mini(&self) -> bool {
        !self.status.is_empty() && !self.status_expanded_now()
    }

    /// Whether the panel renders expanded now.
    pub(crate) fn status_expanded_now(&self) -> bool {
        if let Some(e) = self.status_expanded {
            return e;
        }
        match self.settings.status_panel {
            StatusPanelMode::Expanded => true,
            StatusPanelMode::Mini => false,
            StatusPanelMode::Auto => self.width >= px(breakpoint::WIDE - size::SIDEBAR - space::S6),
        }
    }

    /// The floating panel (top-right of the timeline), when it has data.
    #[allow(clippy::too_many_lines, reason = "four sections")]
    pub(crate) fn render_status_panel(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let s = &self.status;
        if s.is_empty() {
            return None;
        }
        let toggle = cx.listener(
            |this: &mut SessionPane, _: &ClickEvent, _, cx: &mut Context<SessionPane>| {
                let now = this.status_expanded_now();
                this.status_expanded = Some(!now);
                cx.notify();
            },
        );
        if !self.status_expanded_now() {
            let (progress, running, changed) = s.mini();
            return Some(
                // Not floating: a right-aligned strip above the timeline, so the capsule never
                // covers rows (design.md §3.2; review P1-7).
                div()
                    .id("status-mini")
                    .flex()
                    .items_center()
                    .gap(px(space::S3))
                    .h(px(size::MINI_STATUS))
                    .px(px(space::S3))
                    .rounded(px(radius::R_FULL))
                    .bg(t.popover())
                    .border_1()
                    .border_color(t.popover_border())
                    .shadow(t.e2())
                    .cursor_pointer()
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .on_click(toggle)
                    .when_some(progress, |el, p| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .child(icon(IconName::CircleDot, size::ICON_SM, t.fg_subtle()))
                                .child(p),
                        )
                    })
                    .when(running > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .child(icon(IconName::Play, size::ICON_SM, t.working()))
                                .child(running.to_string()),
                        )
                    })
                    .when(s.branch.is_some(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .child(icon(IconName::GitBranch, size::ICON_SM, t.fg_subtle()))
                                .child(changed.to_string()),
                        )
                    })
                    .into_any_element(),
            )
            .map(|capsule| {
                div()
                    .flex()
                    .justify_end()
                    .flex_shrink_0()
                    .w_full()
                    .px(px(space::S4))
                    .pt(px(space::S2))
                    .pb(px(space::S1))
                    .child(capsule)
                    .into_any_element()
            });
        }
        let frame = Ticker::peek(cx);
        let (done, total) = s.progress();
        Some(
            div()
                .absolute()
                .top(px(space::S3))
                .right(px(space::S4))
                .w(px(size::STATUS_PANEL))
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .p(px(space::S3))
                .rounded(px(radius::R_2XL))
                .bg(t.popover())
                .border_1()
                .border_color(t.popover_border())
                .shadow(t.e2())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_role(t, TextRole::Sm)
                                .weight_medium()
                                .text_color(t.fg())
                                .child("Status"),
                        )
                        .child(
                            Btn::icon_only("status-collapse", IconName::Minimize2, "Collapse")
                                .size(BtnSize::Xs)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.status_expanded = Some(false);
                                    cx.notify();
                                })),
                        ),
                )
                .when_some(s.goal.clone(), |el, g| {
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .child(section(
                                t,
                                "Goal",
                                if g.paused {
                                    "paused".into()
                                } else {
                                    g.detail.clone().unwrap_or_default()
                                },
                            ))
                            .child(
                                div()
                                    .text_role(t, TextRole::Sm)
                                    .text_color(t.fg())
                                    .line_clamp(2)
                                    .child(SharedString::from(g.text)),
                            ),
                    )
                })
                .when(total > 0, |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::HALF))
                            .child(section(t, "Progress", format!("{done}/{total}")))
                            .children(s.steps.iter().map(|st| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2))
                                    .text_role(t, TextRole::Sm)
                                    .text_color(if st.done { t.fg_subtlest() } else { t.fg() })
                                    .when(st.done, gpui_kit::Styled::line_through)
                                    .child(icon(
                                        if st.done {
                                            IconName::CircleCheck
                                        } else {
                                            IconName::CircleDashed
                                        },
                                        size::ICON_SM,
                                        if st.done { t.success() } else { t.fg_subtle() },
                                    ))
                                    .child(
                                        div().truncate().child(SharedString::from(st.text.clone())),
                                    )
                            })),
                    )
                })
                .when(!s.run.is_empty(), |el| {
                    let tasks = s.run.iter().filter(|r| r.kind == RunKind::Task).count();
                    let agents = s.run.len() - tasks;
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::HALF))
                            .child(section(
                                t,
                                "Run",
                                format!(
                                    "{tasks} task{} · {agents} agent{}",
                                    if tasks == 1 { "" } else { "s" },
                                    if agents == 1 { "" } else { "s" }
                                ),
                            ))
                            .children(s.run.iter().enumerate().map(|(i, r)| {
                                let glyph = match r.kind {
                                    RunKind::Task => IconName::Terminal,
                                    RunKind::Agent => IconName::Bot,
                                    RunKind::Sidekick => IconName::Users,
                                    RunKind::Dream => IconName::Moon,
                                };
                                let call = r.id.clone();
                                let title = r.label.clone();
                                let agent = matches!(r.kind, RunKind::Agent | RunKind::Sidekick);
                                div()
                                    .id(ElementId::NamedInteger(
                                        "status-run".into(),
                                        u64::try_from(i).unwrap_or(0),
                                    ))
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2))
                                    .h(px(size::CONTROL_SM))
                                    .px(px(space::S1))
                                    .rounded(px(radius::R_MD))
                                    .cursor_pointer()
                                    .hover(|st| st.bg(t.hover()))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        if agent {
                                            this.open_side(
                                                SideTab::Agent {
                                                    call_id: call.clone(),
                                                    title: title.clone(),
                                                },
                                                cx,
                                            );
                                        } else {
                                            this.open_bottom_tasks(cx);
                                        }
                                    }))
                                    .child(spinner(frame, size::ICON_SM, t.working()))
                                    .child(icon(glyph, size::ICON_SM, t.fg_subtle()))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_role(t, TextRole::Sm)
                                            .child(SharedString::from(r.label.clone())),
                                    )
                                    .when_some(r.elapsed.clone(), |el, e| {
                                        el.child(
                                            div()
                                                .mono_text(t, TextRole::Xs)
                                                .text_color(t.fg_subtlest())
                                                .child(SharedString::from(e)),
                                        )
                                    })
                            })),
                    )
                })
                .when_some(s.branch.clone(), |el, b| {
                    el.child(section(t, "Git", format!("{b} · {} changed", s.changed)))
                })
                .into_any_element(),
        )
    }
}
