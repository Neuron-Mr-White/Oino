//! M2 Conversation (design.md §6.2): app bar (back, title + "workspace · model" sub-line,
//! status dot, side drawer, actions), the status capsule row (mini status panel; tap → Status
//! sheet) and the shared [`SessionPane`] at touch density (timeline gutters 12, tool rows 32,
//! compressed composer, meta row top-2). Blocking cards dock above the composer (M3); the
//! ones that prefer a sheet are rendered by `sheets`.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::components::badge::{Capsule, DotKind, capsule, dot};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::mobile::shell::{MobileShell, Sheet};
use crate::model::sessions::{LiveStatus, workspace_label};
use crate::views::session_pane::{SessionPane, SideTab};

impl MobileShell {
    fn status_row(
        &self,
        pane: &Entity<SessionPane>,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let p = pane.read(cx);
        let live = p.live();
        let status = p.status().clone();
        let retrying = p.model.is_busy() && live == LiveStatus::Running;
        if status.is_empty() && live != LiveStatus::Attention && !retrying {
            return None;
        }
        let (progress, running, changed) = status.mini();
        let chip = |glyph: IconName, text: String, c| {
            div()
                .flex()
                .items_center()
                .gap(px(space::S1))
                .h(px(size::CAPSULE))
                .px(px(space::S2))
                .rounded(px(radius::R_FULL))
                .bg(t.surface())
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(icon(glyph, size::ICON_SM, c))
                .child(SharedString::from(text))
        };
        let lead = match live {
            LiveStatus::Attention => Some(capsule(Capsule::Waiting, cx).into_any_element()),
            LiveStatus::Running => Some(capsule(Capsule::Running, cx).into_any_element()),
            LiveStatus::Idle | LiveStatus::Unread => None,
        };
        Some(
            div()
                .id("m-status-row")
                .flex()
                .items_center()
                .gap(px(space::S2))
                .w_full()
                .min_h(px(crate::design::tokens::touch::TARGET))
                .px(px(space::S3))
                .flex_none()
                .cursor_pointer()
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.open_sheet(Sheet::Status, cx)),
                )
                .children(lead)
                .when_some(progress, |el, p| {
                    el.child(chip(IconName::CircleDot, p, t.fg_subtle()))
                })
                .when(running > 0, |el| {
                    el.child(chip(IconName::Play, running.to_string(), t.working()))
                })
                .when(status.branch.is_some(), |el| {
                    el.child(chip(
                        IconName::GitBranch,
                        changed.to_string(),
                        t.fg_subtle(),
                    ))
                })
                .child(div().flex_1())
                .child(icon(IconName::ChevronDown, size::ICON_MD, t.fg_subtlest()))
                .into_any_element(),
        )
    }

    pub(crate) fn render_conversation(
        &mut self,
        pane: &Entity<SessionPane>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let (title, sub, live, busy) = {
            let p = pane.read(cx);
            (
                p.model.title(),
                format!(
                    "{} · {}",
                    workspace_label(&p.model.cwd),
                    p.model.model_label()
                ),
                p.live(),
                p.model.is_busy(),
            )
        };
        let heading = div()
            .id("m-title")
            .flex()
            .items_center()
            .gap(px(space::S2))
            .min_w_0()
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_sheet(Sheet::Model, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_role(&t, TextRole::Base)
                            .weight_medium()
                            .text_color(t.fg())
                            .child(SharedString::from(if title.is_empty() {
                                "New session".to_owned()
                            } else {
                                title
                            })),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_role(&t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(sub)),
                    ),
            )
            .children(match live {
                LiveStatus::Running => Some(dot(&t, DotKind::Running)),
                LiveStatus::Attention => Some(dot(&t, DotKind::Waiting)),
                LiveStatus::Unread => Some(dot(&t, DotKind::Unread)),
                LiveStatus::Idle => None,
            })
            .into_any_element();
        let mut actions = Vec::new();
        if busy {
            actions.push(Self::icon_btn(
                "m-stop",
                IconName::Square,
                "Stop",
                cx.listener(|this, _, _, cx| this.with_pane(cx, SessionPane::abort)),
            ));
        }
        actions.push(Self::icon_btn(
            "m-drawer",
            IconName::PanelRight,
            "Side panel",
            cx.listener(|this, _, _, cx| {
                this.with_pane(cx, |p, cx| {
                    if !p.side.is_open() {
                        p.side.open(SideTab::Review);
                    }
                    cx.notify();
                });
                this.drawer = true;
                cx.notify();
            }),
        ));
        actions.push(Self::icon_btn(
            "m-more",
            IconName::Ellipsis,
            "Actions",
            cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Actions, cx)),
        ));
        let bar = Self::app_bar(&t, Some(Self::back_btn(cx)), heading, actions);
        let status = self.status_row(pane, &t, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .children(status)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .border_t_1()
                    .border_color(t.border())
                    .child(pane.clone()),
            )
            .into_any_element()
    }
}
