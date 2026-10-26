//! Tool call rows (pi `ToolExecutionComponent` + built-in tool renderers).

use oino_ai::{JsonObject, JsonValue};
use oino_view::format::{read_line_range, shorten_path};
use oino_view::transcript::ToolRow;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::RenderCtx;
use super::highlight::{highlight, language_from_path};
use super::text::{OLine, Run, blank, boxed, wrap, wrap_plain};

const COLLAPSED_ARGS_CHARS: usize = 100;

fn str_arg<'a>(args: &'a JsonObject, key: &str) -> Option<&'a str> {
    args.get(key).and_then(JsonValue::as_str)
}

fn path_arg(args: &JsonObject) -> Option<&str> {
    str_arg(args, "file_path").or_else(|| str_arg(args, "path"))
}

/// pi `formatToolDuration`.
pub(crate) fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if ms < 60_000 {
        #[allow(clippy::cast_precision_loss, reason = "display only")]
        let s = ms as f64 / 1000.0;
        return format!("{s:.1}s");
    }
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else {
        format!("{m}m {s}s")
    }
}

/// Renders one tool row inside its coloured box.
pub(crate) fn render(tool: &ToolRow, cx: &RenderCtx<'_>) -> Vec<OLine> {
    let theme = cx.theme;
    let inner = cx.width.saturating_sub(2).max(10);
    let bg = if tool.output.is_none() || tool.partial {
        theme.tool_pending_bg
    } else if tool.output.as_ref().is_some_and(|o| o.is_error) {
        theme.tool_error_bg
    } else {
        theme.tool_success_bg
    };
    let mut body = call_lines(tool, cx, inner);
    body.extend(result_lines(tool, cx, inner));
    if let Some(output) = &tool.output {
        for image in output.images() {
            body.push(Line::from(Span::styled(
                format!("[image: {}]", image.mime_type),
                Style::default().fg(theme.tool_output),
            )));
        }
    }
    let mut out = vec![blank(cx.width, Some(bg))];
    out.extend(boxed(body, cx.width, 1, Some(bg)));
    out.push(blank(cx.width, Some(bg)));
    out
}

fn title(name: &str, cx: &RenderCtx<'_>) -> Run {
    Run::new(
        name,
        Style::default()
            .fg(cx.theme.tool_title)
            .add_modifier(Modifier::BOLD),
    )
}

fn path_run(path: Option<&str>, cx: &RenderCtx<'_>) -> Run {
    match path {
        None => Run::new("[invalid arg]", Style::default().fg(cx.theme.error)),
        Some("") => Run::new("...", Style::default().fg(cx.theme.tool_output)),
        Some(p) => Run::new(
            shorten_path(p, cx.home.as_deref()),
            Style::default().fg(cx.theme.accent),
        ),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one call header per pi built-in tool renderer"
)]
fn call_lines(tool: &ToolRow, cx: &RenderCtx<'_>, width: usize) -> Vec<OLine> {
    let theme = cx.theme;
    let args = &tool.args;
    let out_style = Style::default().fg(theme.tool_output);
    let runs: Vec<Run> = match tool.name.as_str() {
        "bash" | "powershell" => {
            let prompt = if tool.name == "bash" { "$" } else { "PS>" };
            let command = str_arg(args, "command").unwrap_or("...");
            let mut runs = vec![title(&format!("{prompt} {command}"), cx)];
            if let Some(timeout) = args.get("timeout").and_then(JsonValue::as_u64) {
                runs.push(Run::new(
                    format!(" (timeout {timeout}s)"),
                    Style::default().fg(theme.muted),
                ));
            }
            runs
        }
        "read" => {
            let range = read_line_range(
                args.get("offset").and_then(JsonValue::as_u64),
                args.get("limit").and_then(JsonValue::as_u64),
            );
            let path = path_arg(args);
            let skill = path.filter(|p| p.ends_with("/SKILL.md") && !cx.expanded);
            if let Some(skill) = skill {
                let name = skill
                    .trim_end_matches("/SKILL.md")
                    .rsplit('/')
                    .next()
                    .unwrap_or("");
                vec![
                    Run::new(
                        "[skill] ",
                        Style::default()
                            .fg(theme.custom_message_label)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Run::new(
                        name.to_owned(),
                        Style::default().fg(theme.custom_message_text),
                    ),
                    Run::new(
                        format!(" ({} to expand)", cx.expand_key),
                        Style::default().fg(theme.dim),
                    ),
                ]
            } else {
                vec![
                    title("read", cx),
                    Run::new(" ", Style::default()),
                    path_run(path, cx),
                    Run::new(range, Style::default().fg(theme.warning)),
                ]
            }
        }
        "write" | "edit" => vec![
            title(&tool.name, cx),
            Run::new(" ", Style::default()),
            path_run(path_arg(args), cx),
        ],
        "grep" => {
            let mut runs = vec![
                title("grep", cx),
                Run::new(
                    format!(" /{}/", str_arg(args, "pattern").unwrap_or("")),
                    Style::default().fg(theme.accent),
                ),
                Run::new(
                    format!(
                        " in {}",
                        shorten_path(
                            str_arg(args, "path")
                                .filter(|p| !p.is_empty())
                                .unwrap_or("."),
                            cx.home.as_deref()
                        )
                    ),
                    out_style,
                ),
            ];
            if let Some(glob) = str_arg(args, "glob") {
                runs.push(Run::new(format!(" ({glob})"), out_style));
            }
            if let Some(limit) = args.get("limit") {
                runs.push(Run::new(format!(" limit {limit}"), out_style));
            }
            runs
        }
        "find" => {
            let mut runs = vec![
                title("find", cx),
                Run::new(
                    format!(" {}", str_arg(args, "pattern").unwrap_or("")),
                    Style::default().fg(theme.accent),
                ),
                Run::new(
                    format!(
                        " in {}",
                        shorten_path(
                            str_arg(args, "path")
                                .filter(|p| !p.is_empty())
                                .unwrap_or("."),
                            cx.home.as_deref()
                        )
                    ),
                    out_style,
                ),
            ];
            if let Some(limit) = args.get("limit") {
                runs.push(Run::new(format!(" (limit {limit})"), out_style));
            }
            runs
        }
        "ls" => vec![
            title("ls", cx),
            Run::new(" ", Style::default()),
            path_run(
                Some(
                    str_arg(args, "path")
                        .filter(|p| !p.is_empty())
                        .unwrap_or("."),
                ),
                cx,
            ),
        ],
        other => {
            // pi `formatToolCallWithArgs`.
            let mut runs = vec![title(other, cx)];
            if !args.is_empty() {
                if cx.expanded {
                    let lines: Vec<String> = args
                        .iter()
                        .map(|(k, v)| {
                            let text = v.as_str().map_or_else(
                                || serde_json::to_string_pretty(v).unwrap_or_default(),
                                str::to_owned,
                            );
                            format!(
                                "  {k}: {}",
                                text.replace('\t', "   ")
                                    .replace('\r', "")
                                    .replace('\n', "\n    ")
                            )
                        })
                        .collect();
                    runs.push(Run::new(
                        format!("\n{}", lines.join("\n")),
                        Style::default().fg(theme.muted),
                    ));
                } else {
                    let pairs: Vec<String> = args
                        .iter()
                        .map(|(k, v)| {
                            format!("{k}={}", serde_json::to_string(v).unwrap_or_default())
                        })
                        .collect();
                    let pairs = pairs.join(" ");
                    let preview = if pairs.chars().count() > COLLAPSED_ARGS_CHARS {
                        format!(
                            "{}...",
                            pairs
                                .chars()
                                .take(COLLAPSED_ARGS_CHARS - 3)
                                .collect::<String>()
                        )
                    } else {
                        pairs
                    };
                    runs.push(Run::new(
                        format!(" {preview}"),
                        Style::default().fg(theme.muted),
                    ));
                }
            }
            runs
        }
    };
    let mut lines = wrap(&runs, width, &[], &[]);
    if tool.name == "write"
        && let Some(content) = str_arg(args, "content")
        && !content.is_empty()
    {
        lines.push(Line::default());
        lines.extend(code_preview(content, path_arg(args), 10, cx, width));
    }
    if tool.name == "edit" {
        let diff = tool
            .output
            .as_ref()
            .filter(|o| !o.is_error)
            .and_then(|o| o.details.as_ref())
            .and_then(|d| d.get("diff"))
            .and_then(JsonValue::as_str);
        if let Some(diff) = diff {
            lines.push(Line::default());
            lines.extend(diff_lines(diff, cx, width));
        }
    }
    lines
}

fn code_preview(
    content: &str,
    path: Option<&str>,
    max: usize,
    cx: &RenderCtx<'_>,
    width: usize,
) -> Vec<OLine> {
    let theme = cx.theme;
    let lang = path.and_then(language_from_path);
    let content = content.replace('\t', "   ");
    let trimmed = content.trim_end_matches('\n');
    let highlighted = highlight(
        trimmed,
        lang.as_deref(),
        theme,
        Style::default().fg(theme.tool_output),
    );
    let total = highlighted.len();
    let shown = if cx.expanded { total } else { total.min(max) };
    let mut out: Vec<OLine> = highlighted
        .into_iter()
        .take(shown)
        .map(|spans| super::text::truncate_line(Line::from(spans), width))
        .collect();
    if total > shown {
        out.push(Line::from(vec![
            Span::styled(
                format!("... ({} more lines, {total} total, ", total - shown),
                Style::default().fg(theme.muted),
            ),
            Span::styled(cx.expand_key.clone(), Style::default().fg(theme.dim)),
            Span::styled(" to expand)", Style::default().fg(theme.muted)),
        ]));
    }
    out
}

/// pi `renderDiff` (line colours; intra-line inverse highlighting is omitted).
pub(crate) fn diff_lines(diff: &str, cx: &RenderCtx<'_>, width: usize) -> Vec<OLine> {
    let theme = cx.theme;
    diff.lines()
        .flat_map(|line| {
            let color = match line.chars().next() {
                Some('+') => theme.diff_added,
                Some('-') => theme.diff_removed,
                _ => theme.diff_context,
            };
            wrap_plain(
                &line.replace('\t', "   "),
                Style::default().fg(color),
                width,
            )
        })
        .collect()
}

fn hint_line(hidden: usize, earlier: bool, cx: &RenderCtx<'_>) -> OLine {
    let theme = cx.theme;
    let text = if earlier {
        format!("... ({hidden} earlier lines, ")
    } else {
        format!("... ({hidden} more lines, ")
    };
    Line::from(vec![
        Span::styled(text, Style::default().fg(theme.muted)),
        Span::styled(cx.expand_key.clone(), Style::default().fg(theme.dim)),
        Span::styled(" to expand)", Style::default().fg(theme.muted)),
    ])
}

fn truncation_warning(details: Option<&JsonValue>, cx: &RenderCtx<'_>) -> Option<OLine> {
    let details = details?;
    let truncation = details.get("truncation");
    let truncated = truncation
        .and_then(|t| t.get("truncated"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let full = details.get("fullOutputPath").and_then(JsonValue::as_str);
    if !truncated && full.is_none() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(full) = full {
        parts.push(format!("Full output: {full}"));
    }
    if truncated && let Some(t) = truncation {
        let n = |k: &str| t.get(k).and_then(JsonValue::as_u64).unwrap_or(0);
        if t.get("truncatedBy").and_then(JsonValue::as_str) == Some("lines") {
            parts.push(format!(
                "Truncated: showing {} of {} lines",
                n("outputLines"),
                n("totalLines")
            ));
        } else {
            parts.push(format!("Truncated: {} lines shown", n("outputLines")));
        }
    }
    Some(Line::from(Span::styled(
        format!("[{}]", parts.join(". ")),
        Style::default().fg(cx.theme.warning),
    )))
}

#[allow(
    clippy::too_many_lines,
    reason = "one result layout per pi built-in tool renderer"
)]
fn result_lines(tool: &ToolRow, cx: &RenderCtx<'_>, width: usize) -> Vec<OLine> {
    let Some(output) = &tool.output else {
        return Vec::new();
    };
    let theme = cx.theme;
    let out_style = Style::default().fg(theme.tool_output);
    let text = output.text();
    let mut lines = Vec::new();
    match tool.name.as_str() {
        "bash" | "powershell" => {
            let mut text = text.trim().to_owned();
            if let Some(full) = output
                .details
                .as_ref()
                .and_then(|d| d.get("fullOutputPath"))
                .and_then(JsonValue::as_str)
                && !tool.partial
                && text.ends_with(']')
                && let Some(start) = text.rfind("\n\n[")
                && text[start..].contains(full)
            {
                text.truncate(start);
            }
            if !text.is_empty() {
                lines.push(Line::default());
                let all: Vec<OLine> = text
                    .lines()
                    .flat_map(|l| wrap_plain(l, out_style, width))
                    .collect();
                if cx.expanded || all.len() <= 5 {
                    lines.extend(all);
                } else {
                    lines.push(hint_line(all.len() - 5, true, cx));
                    lines.extend(all[all.len() - 5..].iter().cloned());
                }
            }
            if let Some(warning) = truncation_warning(output.details.as_ref(), cx) {
                lines.push(Line::default());
                lines.push(warning);
            }
        }
        "read" => {
            if !cx.expanded && !output.is_error {
                return lines;
            }
            if output.is_error {
                lines.push(Line::default());
                lines.extend(wrap_plain(&text, Style::default().fg(theme.error), width));
            } else {
                lines.push(Line::default());
                lines.extend(code_preview(
                    &text,
                    path_arg(&tool.args),
                    usize::MAX,
                    cx,
                    width,
                ));
            }
        }
        "write" | "edit" => {
            if output.is_error && !text.is_empty() {
                lines.push(Line::default());
                lines.extend(wrap_plain(&text, Style::default().fg(theme.error), width));
            }
        }
        name => {
            let max = match name {
                "grep" => 15,
                "find" | "ls" => 20,
                _ => 10,
            };
            let text = text.trim_end();
            if !text.is_empty() {
                lines.push(Line::default());
                let style = if output.is_error {
                    Style::default().fg(theme.error)
                } else {
                    out_style
                };
                let all: Vec<&str> = text.lines().collect();
                let shown = if cx.expanded {
                    all.len()
                } else {
                    all.len().min(max)
                };
                for line in &all[..shown] {
                    lines.extend(wrap_plain(line, style, width));
                }
                if all.len() > shown {
                    lines.push(hint_line(all.len() - shown, false, cx));
                }
            }
            if let Some(warning) = truncation_warning(output.details.as_ref(), cx) {
                lines.push(Line::default());
                lines.push(warning);
            }
        }
    }
    if !tool.partial
        && let Some(ms) = tool.duration_ms
        && matches!(tool.name.as_str(), "bash" | "powershell")
    {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!("Took {}", format_duration(ms)),
            Style::default().fg(theme.muted),
        )));
    }
    lines
}
