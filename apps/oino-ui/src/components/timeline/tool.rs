//! `ToolRow`, `InlineDiff`, `BashTail`, `AgentRow` (design.md §4.9).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::row_id;
use crate::components::badge::{Capsule, badge, capsule};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::log_view;
use crate::components::ous::diff_lines;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::host::ToolDensity;
use crate::model::ous::Tone;
use crate::model::timeline::{self, Subject, agent_line, tool_line};
use crate::model::tools::{self, INLINE_DIFF_LINES, TAIL_LINES, ToolKind};
use crate::model::transcript::{BashRow, ToolRow, ToolStatus};
use crate::views::session_pane::{SessionPane, SideTab};

fn tool_icon(name: &str) -> IconName {
    match ToolKind::of(name) {
        ToolKind::Read => IconName::FileText,
        ToolKind::Write | ToolKind::Edit => IconName::FilePen,
        ToolKind::Shell => IconName::SquareTerminal,
        ToolKind::Search if name == "ls" => IconName::FolderOpen,
        ToolKind::Search => IconName::Search,
        ToolKind::Agent => IconName::Bot,
        ToolKind::Other if name.starts_with("mcp__") => IconName::Plug,
        ToolKind::Other if name.contains("web") || name.contains("fetch") => IconName::Globe,
        ToolKind::Other => IconName::Zap,
    }
}

fn status_icon(t: &OinoTheme, status: ToolStatus, frame: u32) -> AnyElement {
    match status {
        ToolStatus::Preparing | ToolStatus::Running => {
            spinner(frame, size::ICON_MD, t.fg_subtle()).into_any_element()
        }
        ToolStatus::Done => {
            icon(IconName::Check, size::ICON_MD, t.fg_subtlest()).into_any_element()
        }
        ToolStatus::Error => icon(IconName::X, size::ICON_MD, t.danger()).into_any_element(),
    }
}

/// Detail block under an expanded tool line (indent 22, `surface`, `R_LG`, max 320).
fn detail(t: &OinoTheme, ix: usize) -> gpui_kit::Stateful<Div> {
    div()
        .id(row_id("tool-detail", ix))
        .ml(px(space::S5 + space::HALF))
        .flex()
        .flex_col()
        .rounded(px(radius::R_LG))
        .bg(t.surface())
        .border_1()
        .border_color(t.border())
        .max_h(px(size::DETAIL_MAX))
        .overflow_y_scroll()
}

impl SessionPane {
    pub(super) fn render_tool(
        &self,
        id: &str,
        ix: usize,
        tool: &ToolRow,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
        let frame = if matches!(tool.status, ToolStatus::Running | ToolStatus::Preparing) {
            Ticker::frame(cx)
        } else {
            0
        };
        if let Some(agent) = agent_line(tool) {
            return self.render_agent(ix, tool, &agent, t, frame, cx);
        }
        let kind = ToolKind::of(&tool.name);
        let line = tool_line(tool);
        let density = self.settings.tool_density;
        let expanded =
            tool.expanded ^ (density == ToolDensity::Advanced || self.settings.expand_tools);
        let toggle = id.to_owned();
        let group = SharedString::from(format!("tr{ix}"));
        let subject: AnyElement = match &line.subject {
            Subject::Path { name, dir } => div()
                .flex()
                .items_center()
                .gap(px(space::S1))
                .min_w_0()
                .child(icon(tool_icon(&tool.name), size::ICON_MD, t.fg_subtle()))
                .child(
                    div()
                        .text_color(t.fg())
                        .child(SharedString::from(name.clone())),
                )
                .child(
                    div()
                        .truncate()
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(dir.clone())),
                )
                .into_any_element(),
            Subject::Command(c) => div()
                .truncate()
                .mono_text(t, TextRole::Caption)
                .text_color(t.fg())
                .child(SharedString::from(format!("$ {c}")))
                .into_any_element(),
            Subject::Text(text) => div()
                .truncate()
                .text_color(t.fg())
                .child(SharedString::from(text.clone()))
                .into_any_element(),
            Subject::None => div().into_any_element(),
        };
        let head = div()
            .id(row_id("tool", ix))
            .group(group.clone())
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::TOOL_ROW))
            .min_w_0()
            .cursor_pointer()
            .text_role(t, TextRole::Base)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_row(&toggle, cx)))
            .child(status_icon(t, tool.status, frame))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(line.verb.clone())),
            )
            .child(div().flex().min_w_0().overflow_hidden().child(subject))
            .child(div().flex_1())
            .when_some(line.meta.clone(), |el, m| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(m)),
                )
            })
            .child(
                div()
                    .invisible()
                    .group_hover(group, gpui_kit::Styled::visible)
                    .child(icon(
                        if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        size::ICON_SM,
                        t.fg_subtlest(),
                    )),
            );
        let patch = tool.patch();
        let running = tool.status == ToolStatus::Running;
        let body: Option<AnyElement> =
            if let Some(p) = patch.filter(|_| expanded || density == ToolDensity::Regular) {
                let title = tool.title();
                let open = p.clone();
                Some(
                    detail(t, ix)
                        .child(diff_lines(t, &p, INLINE_DIFF_LINES))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .px(px(space::S2))
                                .py(px(space::S1))
                                .child(
                                    Btn::new(row_id("view-diff", ix))
                                        .label("View diff")
                                        .variant(Variant::Link)
                                        .size(BtnSize::Xs)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.open_side(
                                                SideTab::Diff {
                                                    title: title.clone(),
                                                    patch: open.clone(),
                                                },
                                                cx,
                                            );
                                        })),
                                ),
                        )
                        .into_any_element(),
                )
            } else if kind == ToolKind::Shell
                && (running || expanded || density == ToolDensity::Regular)
                && !tool.output.is_empty()
            {
                Some(self.bash_tail(ix, &tool.title(), &tool.output, tool.status, None, t, cx))
            } else if expanded {
                let text = if tool.output.is_empty() {
                    serde_json::to_string_pretty(&tool.args).unwrap_or_default()
                } else {
                    tool.output.clone()
                };
                let lines: Vec<String> = tools::tail(&text, 40);
                Some(
                    detail(t, ix)
                        .p(px(space::S2))
                        .code_text(t)
                        .text_color(if tool.status == ToolStatus::Error {
                            t.danger_surface_fg()
                        } else {
                            t.fg()
                        })
                        .children(lines.iter().map(|l| log_view::line(t, l)))
                        .into_any_element(),
                )
            } else {
                None
            };
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S1))
            .when(tool.parent.is_some(), |el| el.pl(px(space::S5)))
            .child(head)
            .children(body)
            .into_any_element()
    }

    /// Last lines of a shell command with an exit chip and "Open output".
    #[allow(clippy::too_many_arguments, reason = "row parts")]
    fn bash_tail(
        &self,
        ix: usize,
        title: &str,
        output: &str,
        status: ToolStatus,
        exit: Option<i32>,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lines = tools::tail(output, TAIL_LINES);
        let (title, text) = (title.to_owned(), output.to_owned());
        let failed = status == ToolStatus::Error || exit.is_some_and(|c| c != 0);
        detail(t, ix)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(space::S2))
                    .code_text(t)
                    .text_color(t.fg())
                    .children(lines.iter().map(|l| log_view::line(t, l))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .px(px(space::S2))
                    .pb(px(space::S1))
                    .when(status != ToolStatus::Running, |el| {
                        el.child(if failed {
                            badge(
                                t,
                                Tone::Error,
                                exit.map_or_else(|| "failed".to_owned(), |c| format!("exit {c}")),
                            )
                        } else {
                            badge(t, Tone::Success, "exit 0")
                        })
                    })
                    .child(div().flex_1())
                    .child(
                        Btn::new(row_id("open-output", ix))
                            .label("Open output")
                            .variant(Variant::Link)
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_side(
                                    SideTab::Output {
                                        title: title.clone(),
                                        text: text.clone(),
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_agent(
        &self,
        ix: usize,
        tool: &ToolRow,
        agent: &timeline::AgentLine,
        t: &OinoTheme,
        frame: u32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let call = tool.call_id.clone();
        let title = format!("Agent: {}", agent.name);
        let running = matches!(tool.status, ToolStatus::Running | ToolStatus::Preparing);
        let status: AnyElement = if running {
            capsule(Capsule::Running, cx).into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(space::S1))
                .mono_text(t, TextRole::Sm)
                .text_color(t.fg_subtlest())
                .child(status_icon(t, tool.status, frame))
                .when_some(tool.duration_ms, |el, ms| el.child(timeline::duration(ms)))
                .into_any_element()
        };
        div()
            .id(row_id("agent", ix))
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::CONTROL))
            .px(px(space::S2))
            .rounded(px(radius::R_LG))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_side(
                    SideTab::Agent {
                        call_id: call.clone(),
                        title: title.clone(),
                    },
                    cx,
                );
            }))
            .child(icon(
                if agent.sidekick {
                    IconName::Users
                } else {
                    IconName::Bot
                },
                size::ICON,
                t.agent_node_fg(),
            ))
            .child(
                div()
                    .flex_shrink_0()
                    .text_role(t, TextRole::Base)
                    .weight_medium()
                    .text_color(t.fg())
                    .child(SharedString::from(agent.name.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_role(t, TextRole::Base)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(agent.task.clone())),
            )
            .child(status)
            .into_any_element()
    }

    pub(super) fn render_bash_row(
        &self,
        ix: usize,
        bash: &BashRow,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = if bash.exit_code.is_some_and(|c| c != 0) {
            ToolStatus::Error
        } else {
            ToolStatus::Done
        };
        div()
            .flex()
            .flex_col()
            .gap(px(space::S1))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .h(px(size::TOOL_ROW))
                    .child(icon(IconName::SquareTerminal, size::ICON_MD, t.fg_subtle()))
                    .child(
                        div()
                            .text_role(t, TextRole::Base)
                            .text_color(t.fg_subtle())
                            .child("Ran"),
                    )
                    .child(
                        div()
                            .mono_text(t, TextRole::Caption)
                            .truncate()
                            .child(SharedString::from(format!("$ {}", bash.command))),
                    ),
            )
            .child(self.bash_tail(
                ix,
                &bash.command,
                &bash.output,
                status,
                bash.exit_code,
                t,
                cx,
            ))
            .into_any_element()
    }
}
