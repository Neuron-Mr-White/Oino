//! Transcript items, chat cards and the work-tray "Subagents" tab as OUS (unipi
//! `transcript.ts`, `cards.ts`, `ui.ts`).

use oino_extension::ui::widgets::prelude::*;
use serde_json::{Value, json};

use crate::child::{ChildEvent, Usage, primary_arg};
use crate::record::{Record, Status, elapsed, plural, profile_label, stat_line, usage_tail};

/// Tab id.
pub const TAB: &str = "subagents";
/// unipi `LIST_HINT`.
pub const LIST_HINT: &str = "↑↓ navigate · ↵ view · f foreground · x cancel · ←→ tabs · esc close";
/// unipi `VIEW_HINT`.
pub const VIEW_HINT: &str =
    "↑↓ scroll · g/G top/end · o output · f foreground · x cancel · ←/esc back";
const TOOL_OUTPUT_LINES: usize = 3;

/// unipi `TranscriptItem`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A task prompt.
    Task(String),
    /// Assistant text.
    Text(String),
    /// A tool call.
    Tool {
        /// Tool.
        name: String,
        /// Primary argument.
        arg: String,
        /// Output.
        output: String,
        /// Failed.
        is_error: bool,
        /// Running.
        running: bool,
        /// Duration.
        duration_ms: Option<i64>,
    },
}

/// unipi `itemsFromEvents`.
#[must_use]
pub fn items_from_events(events: &[ChildEvent]) -> Vec<Item> {
    events
        .iter()
        .filter_map(|e| match e {
            ChildEvent::Text { text, .. } => {
                (!text.trim().is_empty()).then(|| Item::Text(text.clone()))
            }
            ChildEvent::Tool {
                name,
                args,
                output,
                is_error,
                done,
                started_at,
                ended_at,
                ..
            } => Some(Item::Tool {
                name: name.clone(),
                arg: primary_arg(name, args.as_ref()),
                output: output.clone(),
                is_error: *is_error,
                running: !done,
                duration_ms: ended_at.map(|e| e - started_at),
            }),
        })
        .collect()
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// unipi `parseSession`: a pi session `.jsonl` into transcript items.
#[must_use]
pub fn items_from_session(raw: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut tools: Vec<(String, usize)> = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(msg) = entry.get("message") else {
            continue;
        };
        let content = msg.get("content").unwrap_or(&Value::Null);
        match msg.get("role").and_then(Value::as_str) {
            Some("user") => {
                let t = text_of(content).trim().to_owned();
                if !t.is_empty() {
                    items.push(Item::Task(t));
                }
            }
            Some("assistant") => {
                for part in content.as_array().into_iter().flatten() {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = part.get("text").and_then(Value::as_str)
                                && !t.trim().is_empty()
                            {
                                items.push(Item::Text(t.to_owned()));
                            }
                        }
                        Some("toolCall") => {
                            let name = part
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("tool")
                                .to_owned();
                            let id = part.get("id").and_then(Value::as_str).unwrap_or_default();
                            tools.push((id.to_owned(), items.len()));
                            items.push(Item::Tool {
                                arg: primary_arg(&name, part.get("arguments")),
                                name,
                                output: String::new(),
                                is_error: false,
                                running: true,
                                duration_ms: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            Some("toolResult") => {
                let id = msg
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some((_, idx)) = tools.iter().find(|(t, _)| t == id)
                    && let Some(Item::Tool {
                        output,
                        is_error,
                        running,
                        ..
                    }) = items.get_mut(*idx)
                {
                    let t = text_of(content);
                    let n = t.chars().count();
                    *output = t.chars().skip(n.saturating_sub(4000)).collect();
                    *is_error = msg.get("isError").and_then(Value::as_bool).unwrap_or(false);
                    *running = false;
                }
            }
            _ => {}
        }
    }
    items
}

/// unipi `buildTranscript`.
#[must_use]
pub fn build_transcript(
    from_file: Vec<Item>,
    task: Option<&str>,
    running: bool,
    events: Option<&[ChildEvent]>,
) -> Vec<Item> {
    let live = events.map(items_from_events);
    if !running {
        if !from_file.is_empty() {
            return from_file;
        }
        let mut out: Vec<Item> = task.map(|t| Item::Task(t.to_owned())).into_iter().collect();
        out.extend(live.unwrap_or_default());
        return out;
    }
    let last_task = from_file.iter().rposition(|i| matches!(i, Item::Task(_)));
    let current = last_task.is_some_and(|i| match (&from_file[i], task) {
        (Item::Task(t), Some(task)) => t == task.trim(),
        (Item::Task(_), None) => true,
        _ => false,
    });
    let mut history = if current {
        from_file[..=last_task.unwrap_or(0)].to_vec()
    } else {
        let mut h = from_file;
        h.extend(task.map(|t| Item::Task(t.to_owned())));
        h
    };
    history.extend(live.unwrap_or_default());
    history
}

/// Transcript items as v2 nodes (tools `●`, text markdown, output tails).
#[must_use]
pub fn render_items(items: &[Item], full_output: bool) -> Vec<Node> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Task(t) => {
                out.push(section(
                    "Task",
                    paragraph(vec![Span::from(t.as_str()).dim()]),
                ));
            }
            Item::Text(t) => out.push(Markdown::new(t.as_str()).into()),
            Item::Tool {
                name,
                arg,
                output,
                is_error,
                running,
                duration_ms,
            } => {
                let mut head = vec![if *running {
                    Span::from("⠋ ").fg(Color::Accent)
                } else {
                    Span::from(if *is_error { "✗ " } else { "● " }).fg(if *is_error {
                        Color::Error
                    } else {
                        Color::Accent
                    })
                }];
                head.push(Span::from(name.as_str()).bold().fg(Color::Tool));
                if !arg.is_empty() {
                    head.push(Span::from(format!(" {arg}")).fg(Color::Accent));
                }
                if let Some(d) = duration_ms.filter(|d| *d >= 1000) {
                    head.push(Span::from(format!(" {}", elapsed(d))).dim());
                }
                out.push(paragraph(head).into());
                let trimmed = output.trim_end();
                if !trimmed.is_empty() {
                    let lines: Vec<&str> = trimmed.split('\n').collect();
                    let shown = if full_output {
                        &lines[..]
                    } else {
                        &lines[lines.len().saturating_sub(TOOL_OUTPUT_LINES)..]
                    };
                    if shown.len() < lines.len() {
                        out.push(
                            paragraph(vec![
                                Span::from(format!(
                                    "  … {} earlier lines",
                                    lines.len() - shown.len()
                                ))
                                .dim(),
                            ])
                            .into(),
                        );
                    }
                    out.push(Log::new(shown.iter().map(|l| Line::from(*l))).into());
                }
            }
        }
    }
    out
}

/// A nowrap paragraph from spans.
fn paragraph(spans: Vec<Span>) -> Paragraph {
    Paragraph::new(Line::from(spans)).wrap(Wrap::Disabled(false))
}

/// A top-rule section (`section` → `Block`, §12.1).
fn section(title: &str, body: impl Into<Node>) -> Node {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
        .title(title)
        .child(body.into())
        .into()
}

/// The rule between a card's title and its right side (`divider` → top-border `Block`).
fn divider() -> Block {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
}

/// A built node as the wire JSON the renderer callback returns.
#[cfg(test)]
fn wire(node: impl Into<Node>) -> Value {
    serde_json::to_value(node.into()).unwrap_or(Value::Null)
}

/// One list row (unipi `subagentRow`).
#[must_use]
pub fn row(rec: &Record, tool_calls: u32, usage: Option<&Usage>, now: i64) -> ListItem {
    let mut tags = vec![
        elapsed(rec.ended_at.unwrap_or(now) - rec.started_at),
        plural(tool_calls, "tool"),
    ];
    if rec.status == Status::Running && rec.background {
        tags.push("bg".into());
    }
    tags.push(rec.model.clone());
    let tok = usage_tail(usage);
    if !tok.is_empty() {
        tags.push(tok[3..].to_owned());
    }
    ListItem::new(Line::from(vec![
        Span::from(rec.status.badge())
            .fg(Color::parse(rec.status.tone()))
            .bold(),
        Span::from(" "),
        Span::from(profile_label(&rec.profile)).dim(),
        Span::from(format!(" {}", rec.title)),
    ]))
    .key(&rec.id)
    .badge(Line::from(tags.join(" · ")))
    .style(Style::new().fg(Color::parse(rec.status.tone())))
}

fn button(label: &str, action: &str) -> Node {
    Button::new(label, format!("{TAB}:{action}")).into()
}

/// Running first, newest first within each group.
#[must_use]
pub fn ordered(records: &[Record]) -> Vec<Record> {
    let mut sorted = records.to_vec();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    let (mut running, rest): (Vec<Record>, Vec<Record>) = sorted
        .into_iter()
        .partition(|r| r.status == Status::Running);
    running.extend(rest);
    running
}

/// List body.
#[must_use]
pub fn list_body(rows: &[ListItem], flash: Option<&str>) -> Node {
    let mut children: Vec<Node> = if rows.is_empty() {
        vec![
            Empty::new()
                .message("No subagents in this session yet.")
                .into(),
        ]
    } else {
        vec![
            List::new(rows.iter().cloned())
                .on_activate(format!("{TAB}:open"))
                .into(),
        ]
    };
    if let Some(f) = flash {
        children.push(paragraph(vec![Span::from(f).fg(Color::Warning)]).into());
    }
    children.push(
        KeyHints::new([
            ("↑↓", "navigate"),
            ("↵", "view"),
            ("f", "foreground"),
            ("x", "cancel"),
            ("←→", "tabs"),
            ("esc", "close"),
        ])
        .into(),
    );
    Layout::default().children(children).into()
}

/// The `Model · id · phase · usage` line of the detail view.
fn detail_meta(rec: &Record, usage: Option<&Usage>) -> String {
    let mut meta = vec![
        format!(
            "Model: {}{}",
            rec.model,
            rec.thinking
                .as_ref()
                .map(|t| format!(" · {t}"))
                .unwrap_or_default()
        ),
        format!("id {}", rec.id),
        if rec.status == Status::Running {
            if rec.background {
                "background"
            } else {
                "foreground"
            }
            .to_owned()
        } else {
            rec.status.label().to_owned()
        },
    ];
    let tok = usage_tail(usage);
    if !tok.is_empty() {
        meta.push(tok[3..].to_owned());
    }
    meta.join(" · ")
}

/// Detail (transcript) body.
#[must_use]
pub fn detail_body(
    rec: &Record,
    items: &[Item],
    tool_calls: u32,
    usage: Option<&Usage>,
    full_output: bool,
    flash: Option<&str>,
    now: i64,
) -> Node {
    let mut children = vec![
        paragraph(vec![
            Span::from(rec.status.badge())
                .fg(Color::parse(rec.status.tone()))
                .bold(),
            Span::from(" "),
            Span::from(profile_label(&rec.profile)).dim(),
            Span::from(" › ").dim(),
            Span::from(rec.title.as_str()).bold(),
            Span::from(format!(
                "  {} · {}",
                elapsed(rec.ended_at.unwrap_or(now) - rec.started_at),
                plural(tool_calls, "tool")
            ))
            .dim(),
        ])
        .into(),
        paragraph(vec![Span::from(detail_meta(rec, usage)).dim()]).into(),
    ];
    children.extend(render_items(items, full_output));
    if rec.status != Status::Running {
        if let Some(e) = &rec.error {
            let (g, tone) = if rec.status == Status::Cancelled {
                ("⊘", Color::Warning)
            } else {
                ("✗", Color::Error)
            };
            children.push(paragraph(vec![Span::from(format!("{g} {e}")).fg(tone)]).into());
        } else if rec.status == Status::Cancelled {
            let by = if rec.cancelled_by == Some(crate::record::CancelledBy::User) {
                " by you"
            } else {
                ""
            };
            children.push(
                paragraph(vec![
                    Span::from(format!("⊘ Cancelled{by}")).fg(Color::Warning),
                ])
                .into(),
            );
        }
        if let Some(h) = &rec.resume_hint {
            children.push(paragraph(vec![Span::from(format!("↻ {h}")).fg(Color::Accent)]).into());
        }
    }
    let id = &rec.id;
    children.push(
        Layout::horizontal(vec![Constraint::Min(0); 4])
            .spacing(1)
            .children([
                button("Back", "back"),
                button(
                    if full_output {
                        "Tail output"
                    } else {
                        "Full output"
                    },
                    &format!("output:{id}"),
                ),
                button("Foreground", &format!("foreground:{id}")),
                button("Cancel", &format!("cancel:{id}")),
            ])
            .into(),
    );
    if let Some(f) = flash {
        children.push(paragraph(vec![Span::from(f).fg(Color::Warning)]).into());
    }
    children.push(
        KeyHints::new([
            ("↑↓", "scroll"),
            ("g/G", "top/end"),
            ("o", "output"),
            ("f", "foreground"),
            ("x", "cancel"),
            ("←/esc", "back"),
        ])
        .into(),
    );
    Layout::default().children(children).into()
}

/// `run_subagent` details (unipi `CardDetails`).
#[must_use]
pub fn details(rec: &Record, phase: Option<&str>, status: Status) -> Value {
    let mut d = json!({"owner": "subagents", "id": rec.id, "title": rec.title, "profile": rec.profile, "profileLabel": format!("{} subagent", profile_label(&rec.profile)), "model": rec.model, "status": status.as_str()});
    if let Some(p) = phase {
        d["phase"] = json!(p);
    }
    d
}

/// unipi `cardOutcome` (`└ Completed · 10s · 3 tool calls`).
#[must_use]
pub fn card_outcome(d: &Value) -> Node {
    let phase = d.get("phase").and_then(Value::as_str);
    let status = d
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let mut m: Vec<String> = Vec::new();
    if let Some(ms) = d.get("durationMs").and_then(Value::as_i64) {
        m.push(elapsed(ms));
    }
    if let Some(n) = d.get("toolCalls").and_then(Value::as_u64) {
        m.push(plural(u32::try_from(n).unwrap_or(u32::MAX), "tool call"));
    }
    let usage: Option<Usage> = d
        .get("usage")
        .and_then(|u| serde_json::from_value(u.clone()).ok());
    let tok = usage_tail(usage.as_ref());
    if !tok.is_empty() {
        m.push(tok[3..].to_owned());
    }
    let tail = if m.is_empty() {
        String::new()
    } else {
        format!(" · {}", m.join(" · "))
    };
    let line: Vec<Span> = match (phase, status) {
        (Some("started"), _) => vec![Span::from("└ Background subagent started.").dim()],
        (Some("moved"), _) => vec![Span::from("└ Moved to background · keeps working").dim()],
        (_, "running") => vec![Span::from("└ Still running in the background.").dim()],
        (_, s) => {
            let st = match s {
                "failed" => Status::Failed,
                "cancelled" => Status::Cancelled,
                _ => Status::Completed,
            };
            let label = if st == Status::Cancelled
                && d.get("cancelledBy").and_then(Value::as_str) == Some("user")
            {
                "Cancelled by you"
            } else {
                st.label()
            };
            let mut v = vec![
                Span::from("└ ").dim(),
                Span::from(label).fg(Color::parse(st.tone())),
            ];
            if st == Status::Failed
                && let Some(e) = d.get("error").and_then(Value::as_str)
            {
                v.push(Span::from(format!(": {e}")).fg(Color::Error));
            }
            v.push(Span::from(tail).dim());
            v
        }
    };
    paragraph(line).into()
}

/// Background completion card (`DONE Explore title ···· 50s · 2 tool calls`).
#[must_use]
pub fn completion_card(d: &Value, expanded: bool) -> Node {
    let status = match d.get("status").and_then(Value::as_str) {
        Some("failed") => Status::Failed,
        Some("cancelled" | "aborted" | "interrupted") => Status::Cancelled,
        Some("running") => Status::Running,
        _ => Status::Completed,
    };
    let mut right: Vec<String> = Vec::new();
    if status == Status::Cancelled && d.get("cancelledBy").and_then(Value::as_str) == Some("user") {
        right.push("cancelled by you".into());
    } else if status == Status::Failed {
        right.push("failed".into());
    }
    if let Some(m) = d.get("model").and_then(Value::as_str) {
        right.push(m.into());
    }
    if let Some(ms) = d.get("durationMs").and_then(Value::as_i64) {
        right.push(elapsed(ms));
    }
    if let Some(n) = d.get("toolCalls").and_then(Value::as_u64) {
        right.push(plural(u32::try_from(n).unwrap_or(u32::MAX), "tool call"));
    }
    let profile = d.get("profile").and_then(Value::as_str).map(profile_label);
    let color = Color::parse(status.tone());
    let head: Node = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .children([
        paragraph(vec![Span::from(status.badge()).fg(color).bold()]).into(),
        paragraph(vec![
            Span::from(profile.unwrap_or_default()).bold(),
            Span::from(format!(
                " {}",
                d.get("title").and_then(Value::as_str).unwrap_or_default()
            )),
        ])
        .into(),
        divider().into(),
        paragraph(vec![Span::from(right.join(" · ")).dim()]).into(),
    ])
    .into();
    match d.get("report").and_then(Value::as_str).filter(|_| expanded) {
        Some(r) => Layout::default()
            .children([head, Markdown::new(r).into()])
            .into(),
        None => head,
    }
}

/// `subagent-step` entry card (delegated step).
#[must_use]
pub fn step_card(data: &Value, expanded: bool) -> Node {
    let label = data
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("subagent");
    let body: Node = match data.get("kind").and_then(Value::as_str) {
        Some("tool") => {
            let err = data
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut parts: Vec<Node> = vec![
                paragraph(vec![
                    Span::from(if err { "✗ " } else { "● " }).fg(if err {
                        Color::Error
                    } else {
                        Color::Accent
                    }),
                    Span::from(data.get("name").and_then(Value::as_str).unwrap_or("tool")).bold(),
                    Span::from(format!(
                        " {}",
                        data.get("arg").and_then(Value::as_str).unwrap_or_default()
                    ))
                    .fg(Color::Accent),
                ])
                .into(),
            ];
            if expanded
                && let Some(o) = data
                    .get("output")
                    .and_then(Value::as_str)
                    .filter(|o| !o.is_empty())
            {
                parts.push(Log::new(o.split('\n').map(Line::from)).into());
            }
            Layout::default().children(parts).into()
        }
        _ => Markdown::new(data.get("text").and_then(Value::as_str).unwrap_or_default()).into(),
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .title(label)
        .rail(Color::Tool)
        .child(body)
        .into()
}

/// Running foreground card body: tail + stat line.
#[must_use]
pub fn live_card(
    rec: &Record,
    items: &[Item],
    tool_calls: u32,
    usage: Option<&Usage>,
    now: i64,
) -> Node {
    let steps: Vec<Item> = items
        .iter()
        .filter(|i| !matches!(i, Item::Task(_)))
        .cloned()
        .collect();
    let tail = &steps[steps.len().saturating_sub(4)..];
    let mut children = render_items(tail, false);
    children.push(
        paragraph(vec![
            Span::from(format!(
                "└ {} · ctrl+b background · esc cancel",
                stat_line(rec, tool_calls, usage, now)
            ))
            .dim(),
        ])
        .into(),
    );
    Layout::default().children(children).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_carry_the_resolved_profile_label() {
        // QA-8: the parent model may call run_subagent without `agent`/`profile`; the
        // streamed details must name the resolved profile so the UI's AgentRow does not
        // fall back to the generic "Run subagent" label.
        let rec = Record {
            profile: "subagent_explore".into(),
            ..Record::default()
        };
        let d = details(&rec, Some("done"), Status::Completed);
        assert_eq!(d["profileLabel"], "Explore subagent");
        assert_eq!(d["profile"], "subagent_explore");
    }

    #[test]
    fn session_and_live_transcripts() {
        let raw = [
            json!({"type": "message", "message": {"role": "user", "content": "Do X"}}),
            json!({"type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": "ok"}, {"type": "toolCall", "id": "c1", "name": "bash", "arguments": {"command": "ls\n-la"}}]}}),
            json!({"type": "message", "message": {"role": "toolResult", "toolCallId": "c1", "content": [{"type": "text", "text": "a\nb"}], "isError": true}}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n{torn";
        let items = items_from_session(&raw);
        assert_eq!(items.len(), 3);
        assert!(
            matches!(&items[2], Item::Tool { arg, is_error: true, running: false, .. } if arg == "ls")
        );
        let live = vec![ChildEvent::Text {
            text: "live".into(),
            open: true,
        }];
        let t = build_transcript(items.clone(), Some("Do X"), true, Some(&live));
        assert_eq!(t.len(), 2, "history up to the current task, then live");
        let t = build_transcript(items.clone(), Some("Do Y"), true, Some(&live));
        assert_eq!(t.len(), 5);
        assert_eq!(
            build_transcript(Vec::new(), Some("T"), false, Some(&live)).len(),
            2
        );
        assert_eq!(build_transcript(items, None, false, None).len(), 3);
    }

    #[test]
    fn cards() {
        let d = json!({"status": "completed", "durationMs": 10_000, "toolCalls": 3});
        let o = wire(card_outcome(&d));
        assert_eq!(o["text"][0][1]["content"], "Completed");
        assert_eq!(o["text"][0][2]["content"], " · 10s · 3 tool calls");
        assert_eq!(
            wire(card_outcome(&json!({"phase": "moved"})))["text"][0][0]["content"],
            "└ Moved to background · keeps working"
        );
        let c = wire(completion_card(
            &json!({"status": "aborted", "cancelledBy": "user", "profile": "subagent_explore", "title": "T"}),
            false,
        ));
        assert_eq!(c["children"][0]["text"][0][0]["content"], "STOP");
        assert_eq!(
            c["children"][3]["text"][0][0]["content"],
            "cancelled by you"
        );
        let rec = Record {
            id: "a".into(),
            title: "t".into(),
            profile: "subagent_general".into(),
            model: "m".into(),
            status: Status::Running,
            background: true,
            ..Record::default()
        };
        assert_eq!(
            wire(List::new([row(&rec, 2, None, 5000)]))["items"][0]["badge"],
            "5s · 2 tools · bg · m"
        );
        let body = wire(list_body(&[row(&rec, 0, None, 0)], Some("hi")));
        assert_eq!(body["children"][0]["onActivate"], "subagents:open");
        let det = wire(detail_body(
            &rec,
            &[Item::Task("x".into())],
            0,
            None,
            false,
            None,
            0,
        ))
        .to_string();
        assert!(det.contains("subagents:cancel:a") && det.contains("background"));
        assert_eq!(
            wire(step_card(
                &json!({"kind": "text", "text": "hi", "label": "L"}),
                false
            ))["title"],
            "L"
        );
    }
}
