//! Transcript items, chat cards and the work-tray "Subagents" tab as OUS (unipi
//! `transcript.ts`, `cards.ts`, `ui.ts`).

use serde_json::{Value, json};
use unipi_kit::ous::{dim, key_hints, span, text};

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

/// Transcript items as OUS (tools `●`, text markdown, output tails).
#[must_use]
pub fn render_items(items: &[Item], full_output: bool) -> Vec<Value> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Task(t) => {
                out.push(json!({"t": "section", "title": "Task", "body": text(json!([dim(t)]))}));
            }
            Item::Text(t) => out.push(json!({"t": "markdown", "value": t})),
            Item::Tool {
                name,
                arg,
                output,
                is_error,
                running,
                duration_ms,
            } => {
                let mut head = vec![if *running {
                    json!({"text": "⠋ ", "tone": "accent"})
                } else {
                    span(
                        if *is_error { "✗ " } else { "● " },
                        Some(if *is_error { "error" } else { "accent" }),
                    )
                }];
                head.push(json!({"text": name, "bold": true, "tone": "tool"}));
                if !arg.is_empty() {
                    head.push(span(&format!(" {arg}"), Some("accent")));
                }
                if let Some(d) = duration_ms.filter(|d| *d >= 1000) {
                    head.push(dim(&format!(" {}", elapsed(d))));
                }
                out.push(text(Value::Array(head)));
                let trimmed = output.trim_end();
                if !trimmed.is_empty() {
                    let lines: Vec<&str> = trimmed.split('\n').collect();
                    let shown = if full_output {
                        &lines[..]
                    } else {
                        &lines[lines.len().saturating_sub(TOOL_OUTPUT_LINES)..]
                    };
                    if shown.len() < lines.len() {
                        out.push(text(json!([dim(&format!(
                            "  … {} earlier lines",
                            lines.len() - shown.len()
                        ))])));
                    }
                    out.push(json!({"t": "log", "lines": shown}));
                }
            }
        }
    }
    out
}

/// One list row (unipi `subagentRow`).
#[must_use]
pub fn row(rec: &Record, tool_calls: u32, usage: Option<&Usage>, now: i64) -> Value {
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
    json!({
        "id": rec.id,
        "label": [{"text": rec.status.badge(), "tone": rec.status.tone(), "bold": true}, {"text": " "}, dim(&profile_label(&rec.profile)), {"text": format!(" {}", rec.title)}],
        "badge": tags.join(" · "),
        "tone": rec.status.tone(),
    })
}

fn button(label: &str, action: &str) -> Value {
    json!({"t": "button", "label": label, "actionId": format!("{TAB}:{action}")})
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
pub fn list_body(rows: &[Value], flash: Option<&str>) -> Value {
    let mut children = if rows.is_empty() {
        vec![json!({"t": "empty", "message": "No subagents in this session yet."})]
    } else {
        vec![json!({"t": "list", "items": rows, "selectable": format!("{TAB}:open")})]
    };
    if let Some(f) = flash {
        children.push(text(json!([span(f, Some("warning"))])));
    }
    children.push(key_hints(&[
        ("↑↓", "navigate"),
        ("↵", "view"),
        ("f", "foreground"),
        ("x", "cancel"),
        ("←→", "tabs"),
        ("esc", "close"),
    ]));
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
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
) -> Value {
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
    let mut children = vec![
        text(json!([
            {"text": rec.status.badge(), "tone": rec.status.tone(), "bold": true},
            {"text": " "},
            dim(&profile_label(&rec.profile)),
            dim(" › "),
            {"text": rec.title, "bold": true},
            dim(&format!("  {} · {}", elapsed(rec.ended_at.unwrap_or(now) - rec.started_at), plural(tool_calls, "tool"))),
        ])),
        text(json!([dim(&meta.join(" · "))])),
    ];
    children.extend(render_items(items, full_output));
    if rec.status != Status::Running {
        if let Some(e) = &rec.error {
            let (g, tone) = if rec.status == Status::Cancelled {
                ("⊘", "warning")
            } else {
                ("✗", "error")
            };
            children.push(text(json!([span(&format!("{g} {e}"), Some(tone))])));
        } else if rec.status == Status::Cancelled {
            let by = if rec.cancelled_by == Some(crate::record::CancelledBy::User) {
                " by you"
            } else {
                ""
            };
            children.push(text(json!([span(
                &format!("⊘ Cancelled{by}"),
                Some("warning")
            )])));
        }
    }
    let id = &rec.id;
    children.push(json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
        button("Back", "back"),
        button(if full_output { "Tail output" } else { "Full output" }, &format!("output:{id}")),
        button("Foreground", &format!("foreground:{id}")),
        button("Cancel", &format!("cancel:{id}")),
    ]}));
    if let Some(f) = flash {
        children.push(text(json!([span(f, Some("warning"))])));
    }
    children.push(key_hints(&[
        ("↑↓", "scroll"),
        ("g/G", "top/end"),
        ("o", "output"),
        ("f", "foreground"),
        ("x", "cancel"),
        ("←/esc", "back"),
    ]));
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
}

/// `run_subagent` details (unipi `CardDetails`).
#[must_use]
pub fn details(rec: &Record, phase: Option<&str>, status: Status) -> Value {
    let mut d = json!({"owner": "subagents", "id": rec.id, "title": rec.title, "profile": rec.profile, "model": rec.model, "status": status.as_str()});
    if let Some(p) = phase {
        d["phase"] = json!(p);
    }
    d
}

/// unipi `cardOutcome` (`└ Completed · 10s · 3 tool calls`).
#[must_use]
pub fn card_outcome(d: &Value) -> Value {
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
    let line = match (phase, status) {
        (Some("started"), _) => vec![dim("└ Background subagent started.")],
        (Some("moved"), _) => vec![dim("└ Moved to background · keeps working")],
        (_, "running") => vec![dim("└ Still running in the background.")],
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
            let mut v = vec![dim("└ "), span(label, Some(st.tone()))];
            if st == Status::Failed
                && let Some(e) = d.get("error").and_then(Value::as_str)
            {
                v.push(span(&format!(": {e}"), Some("error")));
            }
            v.push(dim(&tail));
            v
        }
    };
    text(Value::Array(line))
}

/// Background completion card (`DONE Explore title ···· 50s · 2 tool calls`).
#[must_use]
pub fn completion_card(d: &Value, expanded: bool) -> Value {
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
    let head = json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
        {"t": "badge", "value": status.badge(), "tone": status.tone()},
        text(json!([{"text": profile.unwrap_or_default(), "bold": true}, {"text": format!(" {}", d.get("title").and_then(Value::as_str).unwrap_or_default())}])),
        {"t": "divider"},
        text(json!([dim(&right.join(" · "))])),
    ]});
    match d.get("report").and_then(Value::as_str).filter(|_| expanded) {
        Some(r) => {
            json!({"t": "stack", "direction": "vertical", "gap": 0, "children": [head, {"t": "markdown", "value": r}]})
        }
        None => head,
    }
}

/// `subagent-step` entry card (delegated step).
#[must_use]
pub fn step_card(data: &Value, expanded: bool) -> Value {
    let label = data
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("subagent");
    let body = match data.get("kind").and_then(Value::as_str) {
        Some("tool") => {
            let err = data
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut parts = vec![text(json!([
                span(if err { "✗ " } else { "● " }, Some(if err { "error" } else { "accent" })),
                {"text": data.get("name").and_then(Value::as_str).unwrap_or("tool"), "bold": true},
                span(&format!(" {}", data.get("arg").and_then(Value::as_str).unwrap_or_default()), Some("accent")),
            ]))];
            if expanded
                && let Some(o) = data
                    .get("output")
                    .and_then(Value::as_str)
                    .filter(|o| !o.is_empty())
            {
                parts.push(json!({"t": "log", "lines": o.split('\n').collect::<Vec<_>>()}));
            }
            json!({"t": "stack", "direction": "vertical", "gap": 0, "children": parts})
        }
        _ => {
            json!({"t": "markdown", "value": data.get("text").and_then(Value::as_str).unwrap_or_default()})
        }
    };
    json!({"t": "card", "title": label, "rail": "tool", "body": body})
}

/// Running foreground card body: tail + stat line.
#[must_use]
pub fn live_card(
    rec: &Record,
    items: &[Item],
    tool_calls: u32,
    usage: Option<&Usage>,
    now: i64,
) -> Value {
    let steps: Vec<Item> = items
        .iter()
        .filter(|i| !matches!(i, Item::Task(_)))
        .cloned()
        .collect();
    let tail = &steps[steps.len().saturating_sub(4)..];
    let mut children = render_items(tail, false);
    children.push(text(json!([dim(&format!(
        "└ {} · ctrl+b background · esc cancel",
        stat_line(rec, tool_calls, usage, now)
    ))])));
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let o = card_outcome(&d);
        assert_eq!(o["value"][1]["text"], "Completed");
        assert_eq!(o["value"][2]["text"], " · 10s · 3 tool calls");
        assert_eq!(
            card_outcome(&json!({"phase": "moved"}))["value"][0]["text"],
            "└ Moved to background · keeps working"
        );
        let c = completion_card(
            &json!({"status": "aborted", "cancelledBy": "user", "profile": "subagent_explore", "title": "T"}),
            false,
        );
        assert_eq!(c["children"][0]["value"], "STOP");
        assert_eq!(c["children"][3]["value"][0]["text"], "cancelled by you");
        let rec = Record {
            id: "a".into(),
            title: "t".into(),
            profile: "subagent_general".into(),
            model: "m".into(),
            status: Status::Running,
            background: true,
            ..Record::default()
        };
        assert_eq!(row(&rec, 2, None, 5000)["badge"], "5s · 2 tools · bg · m");
        let body = list_body(&[row(&rec, 0, None, 0)], Some("hi"));
        assert_eq!(body["children"][0]["selectable"], "subagents:open");
        let det = detail_body(&rec, &[Item::Task("x".into())], 0, None, false, None, 0).to_string();
        assert!(det.contains("subagents:cancel:a") && det.contains("background"));
        assert_eq!(
            step_card(&json!({"kind": "text", "text": "hi", "label": "L"}), false)["title"],
            "L"
        );
    }
}
