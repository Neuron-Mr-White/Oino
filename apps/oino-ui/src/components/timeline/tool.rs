//! Tool rows, tool bodies and agent rows (design.md A2.10).
//!
//! Tool row: min 32, radius 10; icon 18 · name 13.5/500 · dot · subject mono 12 dim · stat
//! (`+7` green, `−3` red, `3.4s`). The body is a surface-1 card (radius 12, inset line):
//! bash = `$` in claw + command, rule, output 11.5/17 muted (max 240); edits = the diff.
//! Agent row: fold-line style with `bot` in claw, the agent name, the task and the child's
//! live action from the `_oino/agents` feed (GA-15).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::{ICON_TOOL, row_id, shimmer};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::log_view;
use crate::components::ous::diff_lines;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::model::timeline::{self, Subject, agent_line, tool_line};
use crate::model::tools::{self, INLINE_DIFF_LINES, TAIL_LINES, ToolKind};
use crate::model::transcript::{BashRow, RowKind, ToolRow, ToolStatus};
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

fn row_icon(t: &OinoTheme, name: &str, status: ToolStatus, frame: u32) -> AnyElement {
    match status {
        ToolStatus::Preparing | ToolStatus::Running => {
            spinner(frame, ICON_TOOL, t.working()).into_any_element()
        }
        ToolStatus::Error => icon(IconName::CircleX, ICON_TOOL, t.danger()).into_any_element(),
        ToolStatus::Done => icon(tool_icon(name), ICON_TOOL, t.fg_subtlest()).into_any_element(),
    }
}

/// Tool body card (A2.10: surface-1, radius 12, inset line, max 240 output).
fn body_card(t: &OinoTheme, ix: usize) -> gpui_kit::Stateful<Div> {
    div()
        .id(row_id("tool-detail", ix))
        .ml(px(ICON_TOOL + space::S1_5 + 1.0 + space::S1))
        .mb(px(space::S1))
        .flex()
        .flex_col()
        .rounded(px(radius::R_TOOL))
        .bg(t.card())
        .border_1()
        .border_color(t.border())
        .max_h(px(size::TOOL_OUT_MAX + size::TOOL_ROW))
        .overflow_y_scroll()
}

/// The stat of a tool line: `+7 −3` toned, else muted.
fn stat(t: &OinoTheme, meta: &str) -> AnyElement {
    if let Some((add, del)) = meta.split_once(' ')
        && add.starts_with('+')
        && del.starts_with('−')
    {
        return div()
            .flex()
            .flex_shrink_0()
            .gap(px(space::S1))
            .mono_text(t, TextRole::Xs)
            .child(
                div()
                    .text_color(t.diff_added())
                    .child(SharedString::from(add.to_owned())),
            )
            .child(
                div()
                    .text_color(t.diff_removed())
                    .child(SharedString::from(del.to_owned())),
            )
            .into_any_element();
    }
    div()
        .flex_shrink_0()
        .mono_text(t, TextRole::Xs)
        .text_color(t.fg_subtlest())
        .child(SharedString::from(meta.to_owned()))
        .into_any_element()
}

impl SessionPane {
    #[allow(clippy::too_many_lines, reason = "row head + three body kinds")]
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
        let expanded = tool.expanded;
        let toggle = id.to_owned();
        let failed = tool.status == ToolStatus::Error;
        // The first error line replaces the subject (A2.10).
        let error_line = failed
            .then(|| tool.output.lines().find(|l| !l.trim().is_empty()))
            .flatten()
            .map(str::to_owned);
        let subject: Option<AnyElement> = match (&error_line, &line.subject) {
            (Some(e), _) => Some(
                div()
                    .truncate()
                    .text_role(t, TextRole::Caption)
                    .text_color(t.danger_surface_fg())
                    .child(SharedString::from(e.clone()))
                    .into_any_element(),
            ),
            (None, Subject::Path { name, dir }) => Some(
                div()
                    .truncate()
                    .mono_text(t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .child(SharedString::from(format!("{dir}{name}")))
                    .into_any_element(),
            ),
            (None, Subject::Command(c) | Subject::Text(c)) => Some(
                div()
                    .truncate()
                    .mono_text(t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .child(SharedString::from(c.clone()))
                    .into_any_element(),
            ),
            (None, Subject::None) => None,
        };
        let live = matches!(tool.status, ToolStatus::Running | ToolStatus::Preparing);
        let group = SharedString::from(format!("tr{ix}"));
        let head = div()
            .id(row_id("tool", ix))
            .group(group.clone())
            .flex()
            .items_center()
            .gap(px(space::S1_5 + 1.0))
            .min_h(px(size::TOOL_ROW))
            .px(px(space::S1))
            .min_w_0()
            .rounded(px(radius::R_ROW))
            .cursor_pointer()
            .hover(|s| s.bg(t.hover()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_row(&toggle, cx)))
            .child(row_icon(t, &tool.name, tool.status, frame))
            .child(
                div()
                    .flex_shrink_0()
                    .text_role(t, TextRole::Caption)
                    .weight_medium()
                    .text_color(t.fg())
                    .when(live, |el| el.opacity(shimmer(t, frame)))
                    .child(SharedString::from(line.verb.clone())),
            )
            .when_some(subject, |el, s| {
                el.child(
                    div()
                        .size(px(space::HALF + 1.0))
                        .flex_shrink_0()
                        .rounded(px(radius::R_FULL))
                        .bg(t.fg_subtlest()),
                )
                .child(div().flex_1().min_w_0().overflow_hidden().child(s))
            })
            .child(div().flex_1())
            .when_some(line.meta.clone(), |el, m| el.child(stat(t, &m)))
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
                        size::ICON_MD,
                        t.fg_subtlest(),
                    )),
            );
        let patch = tool.patch();
        let body: Option<AnyElement> = if !expanded {
            None
        } else if let Some(p) = patch {
            let title = tool.title();
            let open = p.clone();
            Some(
                body_card(t, ix)
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
        } else if kind == ToolKind::Shell {
            let command = match &line.subject {
                Subject::Command(c) => c.clone(),
                _ => tool.title(),
            };
            Some(self.bash_body(ix, &command, &tool.output, tool.status, None, t, cx))
        } else {
            let text = if tool.output.is_empty() {
                serde_json::to_string_pretty(&tool.args).unwrap_or_default()
            } else {
                tool.output.clone()
            };
            let lines: Vec<String> = tools::tail(&text, 40);
            Some(
                body_card(t, ix)
                    .px(px(space::S3))
                    .py(px(space::S2))
                    .mono_text(t, TextRole::Xs)
                    .text_color(if failed {
                        t.danger_surface_fg()
                    } else {
                        t.fg_subtle()
                    })
                    .children(lines.iter().map(|l| log_view::line(t, l)))
                    .into_any_element(),
            )
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .child(head)
            .children(body)
            .into_any_element()
    }

    /// Bash body: `$` (claw) + command, a rule, the output tail and "Open output".
    #[allow(clippy::too_many_arguments, reason = "row parts")]
    fn bash_body(
        &self,
        ix: usize,
        command: &str,
        output: &str,
        status: ToolStatus,
        exit: Option<i32>,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lines = tools::tail(output, TAIL_LINES);
        let (title, text) = (command.to_owned(), output.to_owned());
        let failed = status == ToolStatus::Error || exit.is_some_and(|c| c != 0);
        body_card(t, ix)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(space::S1_5))
                    .px(px(space::S3))
                    .py(px(space::S2))
                    .mono_text(t, TextRole::Sm)
                    .child(div().text_color(t.working()).child("$"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(t.fg())
                            .child(SharedString::from(command.to_owned())),
                    ),
            )
            .when(!lines.is_empty(), |el| {
                el.child(div().h(px(size::HAIRLINE)).bg(t.border())).child(
                    div()
                        .flex()
                        .flex_col()
                        .px(px(space::S3))
                        .py(px(space::S2))
                        .mono_text(t, TextRole::Xs)
                        .text_color(t.fg_subtle())
                        .children(lines.iter().map(|l| log_view::line(t, l))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .px(px(space::S3))
                    .pb(px(space::S1_5))
                    .text_role(t, TextRole::Xs)
                    .when(status != ToolStatus::Running, |el| {
                        el.child(
                            div()
                                .mono_text(t, TextRole::Xs)
                                .text_color(if failed { t.danger() } else { t.fg_subtlest() })
                                .child(SharedString::from(if failed {
                                    exit.map_or_else(
                                        || "failed".to_owned(),
                                        |c| format!("exit {c}"),
                                    )
                                } else {
                                    "exit 0".to_owned()
                                })),
                        )
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

    /// The nth running-or-finished agent row before `ix` (maps rows to feed agents).
    fn agent_ordinal(&self, ix: usize) -> usize {
        self.model.transcript.rows()[..ix]
            .iter()
            .filter(|r| matches!(&r.kind, RowKind::Tool(t) if agent_line(t).is_some()))
            .count()
    }

    #[allow(clippy::too_many_lines, reason = "row layout")]
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
        let live = self
            .agents
            .for_row(agent.child.as_deref(), self.agent_ordinal(ix));
        let latest = live.and_then(timeline::AgentLive::latest);
        let child_id = live.map(|a| a.info.id.clone());
        let elapsed = if running {
            live.map(|a| a.started.elapsed().as_millis())
                .and_then(|ms| u64::try_from(ms).ok())
        } else {
            tool.duration_ms
        };
        let steps = live.map_or(0, timeline::AgentLive::steps);
        let detail = {
            let mut parts = Vec::new();
            if steps > 0 {
                parts.push(format!("{steps} step{}", if steps == 1 { "" } else { "s" }));
            }
            if let Some(ms) = elapsed.filter(|ms| *ms >= 1000) {
                parts.push(timeline::duration(ms));
            }
            parts.join(" · ")
        };
        let task = if running {
            latest.unwrap_or_else(|| agent.task.clone())
        } else {
            agent.task.clone()
        };
        div()
            .id(row_id("agent", ix))
            .group(SharedString::from(format!("ag{ix}")))
            .flex()
            .items_center()
            .gap(px(space::S1_5 + 1.0))
            .min_h(px(size::TOOL_ROW))
            .px(px(space::S1))
            .rounded(px(radius::R_ROW))
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
            .child(if running {
                spinner(frame, ICON_TOOL, t.working()).into_any_element()
            } else if tool.status == ToolStatus::Error {
                icon(IconName::CircleX, ICON_TOOL, t.danger()).into_any_element()
            } else {
                icon(
                    if agent.sidekick {
                        IconName::Users
                    } else {
                        IconName::Bot
                    },
                    ICON_TOOL,
                    t.working(),
                )
                .into_any_element()
            })
            .child(
                div()
                    .flex_shrink_0()
                    .text_role(t, TextRole::Caption)
                    .weight_medium()
                    .text_color(t.fg())
                    .child(SharedString::from(agent.name.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtle())
                    .when(running, |el| el.opacity(shimmer(t, frame)))
                    .child(SharedString::from(task)),
            )
            .when(!detail.is_empty(), |el| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .mono_text(t, TextRole::Xs)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(detail)),
                )
            })
            .when_some(child_id.filter(|_| running), |el, id| {
                el.child(
                    div()
                        .invisible()
                        .group_hover(
                            SharedString::from(format!("ag{ix}")),
                            gpui_kit::Styled::visible,
                        )
                        .child(
                            Btn::icon_only(
                                row_id("agent-stop", ix),
                                IconName::Square,
                                "Stop agent",
                            )
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.frontend(cx, move |b| async move {
                                        oino_client::FrontendApi::agent_abort(b.as_ref(), &id).await
                                    });
                                },
                            )),
                        ),
                )
            })
            .child(icon(IconName::ChevronRight, size::ICON_MD, t.fg_subtlest()))
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
        self.bash_body(
            ix,
            &bash.command,
            &bash.output,
            status,
            bash.exit_code,
            t,
            cx,
        )
    }
}
