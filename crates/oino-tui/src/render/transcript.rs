//! Transcript rows → lines (pi chat container components), with a per-row cache.

use oino_ai::{JsonValue, UserContentField};
use oino_view::transcript::{AssistantBlock, NoticeLevel, Row, RowKind};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::RenderCtx;
use super::markdown::{self, MdStyle};
use super::ous::{self, Focus};
use super::text::{OLine, blank, boxed, wrap_plain};
use crate::host::RenderRequest;

/// Renders one row (without the leading spacer).
#[allow(
    clippy::too_many_lines,
    reason = "one arm per transcript row kind (pi chat components)"
)]
pub(crate) fn row_lines(row: &Row, cx: &RenderCtx<'_>) -> Vec<OLine> {
    let theme = cx.theme;
    let width = cx.width;
    let inner = width.saturating_sub(2).max(10);
    match &row.kind {
        RowKind::User(user) => {
            let bg = theme.user_message_bg;
            let style = MdStyle {
                base: Style::default().fg(theme.user_message_text).bg(bg),
            };
            let mut body = markdown::render(&user.text, inner, style, theme);
            for image in &user.images {
                body.push(Line::from(Span::styled(
                    format!("[image: {}]", image.mime_type),
                    Style::default().fg(theme.muted).bg(bg),
                )));
            }
            let mut out = vec![blank(width, Some(bg))];
            out.extend(boxed(body, width, 1, Some(bg)));
            out.push(blank(width, Some(bg)));
            out
        }
        RowKind::Skill(skill) => {
            let bg = theme.custom_message_bg;
            let mut body = vec![Line::from(vec![
                Span::styled(
                    "[skill] ",
                    Style::default()
                        .fg(theme.custom_message_label)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    skill.name.clone(),
                    Style::default().fg(theme.custom_message_text),
                ),
            ])];
            if cx.expanded {
                body.push(Line::default());
                body.extend(markdown::render(
                    &skill.content,
                    inner,
                    MdStyle {
                        base: Style::default().fg(theme.custom_message_text),
                    },
                    theme,
                ));
            } else {
                body.push(expand_hint("", cx));
            }
            framed(body, cx, bg)
        }
        RowKind::Assistant(assistant) => {
            let mut out: Vec<OLine> = Vec::new();
            for (i, block) in assistant.blocks.iter().enumerate() {
                if i > 0 {
                    out.push(Line::default());
                }
                match block {
                    AssistantBlock::Text(text) => {
                        let body = markdown::render(
                            text,
                            inner,
                            MdStyle {
                                base: Style::default().fg(theme.text),
                            },
                            theme,
                        );
                        out.extend(boxed(body, width, 1, None));
                    }
                    AssistantBlock::Thinking(text) => {
                        let style = Style::default()
                            .fg(theme.thinking_text)
                            .add_modifier(Modifier::ITALIC);
                        let body = if cx.hide_thinking {
                            vec![Line::from(Span::styled("Thinking...", style))]
                        } else {
                            markdown::render(text, inner, MdStyle { base: style }, theme)
                        };
                        out.extend(boxed(body, width, 1, None));
                    }
                }
            }
            let error = Style::default().fg(theme.error);
            let note = match assistant.stop_reason {
                Some(oino_ai::StopReason::Length) => {
                    Some("Response was truncated before completion.".to_owned())
                }
                Some(oino_ai::StopReason::Aborted) if !assistant.has_tool_calls => Some(
                    assistant
                        .error_message
                        .clone()
                        .filter(|m| m != "Request was aborted")
                        .unwrap_or_else(|| "Operation aborted".to_owned()),
                ),
                Some(oino_ai::StopReason::Error) if !assistant.has_tool_calls => Some(format!(
                    "Error: {}",
                    assistant
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "Unknown error".into())
                )),
                _ => None,
            };
            if let Some(note) = note {
                if !out.is_empty() {
                    out.push(Line::default());
                }
                out.extend(boxed(wrap_plain(&note, error, inner), width, 1, None));
            }
            out
        }
        RowKind::Tool(tool) => {
            let request = RenderRequest {
                target: "tool",
                key: tool.name.clone(),
                entry_id: Some(tool.tool_call_id.clone()),
                expanded: cx.expanded,
                data: serde_json::json!({
                    "toolCall": {"id": tool.tool_call_id, "name": tool.name},
                    "args": tool.args,
                    "result": tool.output.as_ref().map(|o| serde_json::json!({"content": o.content, "details": o.details})),
                    "isError": tool.output.as_ref().is_some_and(|o| o.is_error),
                }),
            };
            if !is_builtin_tool(&tool.name)
                && let Some(node) = cx.render_spec(&request)
            {
                return node;
            }
            super::tools::render(tool, cx)
        }
        RowKind::Bash(bash) => bash_lines(bash, cx),
        RowKind::Custom(custom) => {
            let request = RenderRequest {
                target: "customMessage",
                key: custom.custom_type.clone(),
                entry_id: custom.entry_id.clone(),
                expanded: cx.expanded,
                data: serde_json::json!({
                    "role": "custom", "customType": custom.custom_type, "content": content_json(&custom.content),
                    "display": true, "details": custom.details,
                }),
            };
            if let Some(node) = cx.render_spec(&request) {
                return node;
            }
            let mut body = vec![Line::from(Span::styled(
                format!("[{}]", custom.custom_type),
                Style::default()
                    .fg(theme.custom_message_label)
                    .add_modifier(Modifier::BOLD),
            ))];
            body.push(Line::default());
            body.extend(markdown::render(
                &custom.text(),
                inner,
                MdStyle {
                    base: Style::default().fg(theme.custom_message_text),
                },
                theme,
            ));
            framed(body, cx, theme.custom_message_bg)
        }
        RowKind::CustomEntry(entry) => {
            let request = RenderRequest {
                target: "customEntry",
                key: entry.custom_type.clone(),
                entry_id: Some(entry.entry_id.clone()),
                expanded: cx.expanded,
                data: serde_json::json!({
                    "type": "custom", "id": entry.entry_id, "customType": entry.custom_type, "data": entry.data,
                }),
            };
            cx.render_spec(&request).unwrap_or_default()
        }
        RowKind::BranchSummary(summary) => {
            let mut body = vec![Line::from(Span::styled(
                "[branch]",
                Style::default()
                    .fg(theme.custom_message_label)
                    .add_modifier(Modifier::BOLD),
            ))];
            body.push(Line::default());
            if cx.expanded {
                body.extend(markdown::render(
                    &format!("**Branch Summary**\n\n{}", summary.summary),
                    inner,
                    MdStyle {
                        base: Style::default().fg(theme.custom_message_text),
                    },
                    theme,
                ));
            } else {
                body.push(expand_hint("Branch summary (", cx));
            }
            framed(body, cx, theme.custom_message_bg)
        }
        RowKind::Compaction(summary) => {
            let tokens = crate::app::group(summary.tokens_before);
            let mut body = vec![Line::from(Span::styled(
                "[compaction]",
                Style::default()
                    .fg(theme.custom_message_label)
                    .add_modifier(Modifier::BOLD),
            ))];
            body.push(Line::default());
            if cx.expanded {
                body.extend(markdown::render(
                    &format!("**Compacted from {tokens} tokens**\n\n{}", summary.summary),
                    inner,
                    MdStyle {
                        base: Style::default().fg(theme.custom_message_text),
                    },
                    theme,
                ));
            } else {
                body.push(expand_hint(
                    &format!("Compacted from {tokens} tokens ("),
                    cx,
                ));
            }
            framed(body, cx, theme.custom_message_bg)
        }
        RowKind::Notice(notice) => {
            let style = match notice.level {
                NoticeLevel::Info => Style::default().fg(theme.dim),
                NoticeLevel::Warning => Style::default().fg(theme.warning),
                NoticeLevel::Error => Style::default().fg(theme.error),
                NoticeLevel::Document => {
                    let body = markdown::render(
                        &notice.text,
                        inner,
                        MdStyle {
                            base: Style::default().fg(theme.text),
                        },
                        theme,
                    );
                    return boxed(body, width, 1, None);
                }
            };
            boxed(wrap_plain(&notice.text, style, inner), width, 1, None)
        }
    }
}

fn is_builtin_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "write" | "edit" | "bash" | "grep" | "find" | "ls" | "powershell"
    )
}

fn content_json(content: &UserContentField) -> JsonValue {
    serde_json::to_value(content).unwrap_or(JsonValue::Null)
}

fn framed(body: Vec<OLine>, cx: &RenderCtx<'_>, bg: ratatui::style::Color) -> Vec<OLine> {
    let mut out = vec![blank(cx.width, Some(bg))];
    out.extend(boxed(body, cx.width, 1, Some(bg)));
    out.push(blank(cx.width, Some(bg)));
    out
}

fn expand_hint(prefix: &str, cx: &RenderCtx<'_>) -> OLine {
    let theme = cx.theme;
    let text = Style::default().fg(theme.custom_message_text);
    if prefix.is_empty() {
        Line::from(vec![
            Span::styled("(", text),
            Span::styled(cx.expand_key.clone(), Style::default().fg(theme.dim)),
            Span::styled(" to expand)", text),
        ])
    } else {
        Line::from(vec![
            Span::styled(prefix.to_owned(), text),
            Span::styled(cx.expand_key.clone(), Style::default().fg(theme.dim)),
            Span::styled(" to expand)", text),
        ])
    }
}

fn bash_lines(bash: &oino_view::transcript::BashRow, cx: &RenderCtx<'_>) -> Vec<OLine> {
    let theme = cx.theme;
    let inner = cx.width.saturating_sub(2).max(10);
    let border = Style::default().fg(if bash.exclude_from_context {
        theme.dim
    } else {
        theme.bash_mode
    });
    let mut body = vec![Line::from(Span::styled(
        format!(
            "{} {}",
            if bash.exclude_from_context { "$$" } else { "$" },
            bash.command
        ),
        border.add_modifier(Modifier::BOLD),
    ))];
    let output = bash.output.trim_end();
    if !output.is_empty() {
        let out_style = Style::default().fg(theme.tool_output);
        let lines: Vec<OLine> = output
            .lines()
            .flat_map(|l| wrap_plain(l, out_style, inner))
            .collect();
        body.push(Line::default());
        if cx.expanded || lines.len() <= 20 {
            body.extend(lines);
        } else {
            body.push(Line::from(vec![
                Span::styled(
                    format!("... ({} earlier lines, ", lines.len() - 20),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(cx.expand_key.clone(), Style::default().fg(theme.dim)),
                Span::styled(" to expand)", Style::default().fg(theme.muted)),
            ]));
            body.extend(lines[lines.len() - 20..].iter().cloned());
        }
    }
    let status = if bash.running {
        Some(Span::styled(
            format!("{} Running...", cx.spinner),
            Style::default().fg(theme.muted),
        ))
    } else if bash.cancelled {
        Some(Span::styled(
            "(cancelled)",
            Style::default().fg(theme.warning),
        ))
    } else {
        match bash.exit_code {
            Some(0) | None => None,
            Some(code) => Some(Span::styled(
                format!("(exit {code})"),
                Style::default().fg(theme.error),
            )),
        }
    };
    if let Some(status) = status {
        body.push(Line::default());
        body.push(Line::from(status));
    }
    if bash.truncated || bash.full_output_path.is_some() {
        let text = bash.full_output_path.as_ref().map_or_else(
            || "[Output truncated]".to_owned(),
            |p| format!("[Output truncated. Full output: {p}]"),
        );
        body.push(Line::from(Span::styled(
            text,
            Style::default().fg(theme.warning),
        )));
    }
    let rule = Line::from(Span::styled("─".repeat(cx.width), border));
    let mut out = vec![rule.clone()];
    out.extend(boxed(body, cx.width, 1, None));
    out.push(rule);
    out
}

/// Renders an extension-supplied OUS spec (custom renderers).
pub(crate) fn spec_lines(node: &JsonValue, cx: &RenderCtx<'_>) -> Vec<OLine> {
    let node = oino_view::ous::OusNode::from_json(node);
    let inner = cx.width.saturating_sub(2).max(10);
    boxed(
        ous::render(&node, inner, cx.theme, Focus::default(), cx.tick),
        cx.width,
        1,
        None,
    )
}
