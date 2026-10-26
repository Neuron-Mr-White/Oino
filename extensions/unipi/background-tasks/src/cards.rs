//! Chat cards as OUS nodes (unipi `cards.ts`): a four-letter state chip, the bold task name and
//! a dim right side; expanded adds the command, error, output tail, output path and id.

use serde_json::{Value, json};
use unipi_kit::ous::{span, text};

use crate::snapshot::{Snapshot, Status};
use crate::text::format_duration;

/// unipi `CHIP`.
#[must_use]
pub fn chip(status: Status) -> &'static str {
    match status {
        Status::Running => "BG  ",
        Status::Completed => "DONE",
        Status::Failed => "FAIL",
        Status::Killed => "STOP",
    }
}

/// unipi `STATE_COLOR` for a task status.
#[must_use]
pub fn tone(status: Status) -> &'static str {
    match status {
        Status::Running => "accent",
        Status::Completed => "success",
        Status::Failed => "error",
        Status::Killed => "warning",
    }
}

fn head(chip_text: &str, chip_tone: &str, name: &str, right: &str) -> Value {
    json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
        {"t": "badge", "value": chip_text, "tone": chip_tone},
        text(json!([{"text": name, "bold": true}])),
        {"t": "divider"},
        text(json!([{"text": right, "dim": true}])),
    ]})
}

fn expanded_lines(task: &Snapshot) -> Vec<Value> {
    let cmd = if task.command.chars().count() > 120 {
        format!("{}…", task.command.chars().take(120).collect::<String>())
    } else {
        task.command.clone()
    };
    let mut out = vec![text(json!([{"text": format!("  $ {cmd}"), "dim": true}]))];
    if let Some(e) = task.error.as_deref().filter(|e| !e.is_empty()) {
        out.push(text(json!([span(&format!("  {e}"), Some("error"))])));
    }
    for line in task.output_tail.iter().flatten() {
        out.push(text(json!([span(&format!("  {line}"), Some("tool"))])));
    }
    out.push(text(
        json!([{"text": format!("  {}", task.output_path), "dim": true}]),
    ));
    out.push(text(
        json!([{"text": format!("  ({})", task.id), "dim": true}]),
    ));
    out
}

fn card(head_node: Value, task: &Snapshot, expanded: bool) -> Value {
    if !expanded {
        return head_node;
    }
    let mut children = vec![head_node];
    children.extend(expanded_lines(task));
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
}

/// `bg_run` result card: `BG   <name> ···· started · wakes agent`.
#[must_use]
pub fn launch_card(task: &Snapshot, expanded: bool) -> Value {
    let wake = if task.trigger_on_completion {
        "wakes agent"
    } else if task.notify_on_completion {
        "notifies"
    } else {
        "silent"
    };
    let h = head(
        chip(task.status),
        tone(task.status),
        &task.display_name(),
        &format!("started · {wake}"),
    );
    card(h, task, expanded)
}

/// Right side of the completion card (`exit 0 · 25s · agent woken`, `killed · 4s`).
#[must_use]
pub fn completion_meta(task: &Snapshot) -> String {
    let mut meta: Vec<String> = Vec::new();
    let elapsed = task
        .end_time
        .map(|end| format_duration(end - task.start_time));
    if task.status == Status::Killed {
        meta.push("killed".into());
        meta.extend(elapsed);
    } else {
        match task.exit_code {
            Some(Some(c)) => meta.push(format!("exit {c}")),
            _ if task.status == Status::Failed => meta.push(
                if task.error.is_some() {
                    "error"
                } else {
                    "failed"
                }
                .into(),
            ),
            _ => {}
        }
        meta.extend(elapsed);
        if task.trigger_on_completion {
            meta.push("agent woken".into());
        }
    }
    meta.join(" · ")
}

/// Completion notification card (`DONE <name> ···· exit 0 · 25s · agent woken` + tail).
#[must_use]
pub fn completion_card(task: Option<&Snapshot>, expanded: bool) -> Value {
    let Some(task) = task else {
        return head("DONE", tone(Status::Completed), "bg task", "");
    };
    let h = head(
        chip(task.status),
        tone(task.status),
        &task.display_name(),
        &completion_meta(task),
    );
    card(h, task, expanded)
}

/// `bg_logs` result line(s).
#[must_use]
pub fn logs_card(
    task: &Snapshot,
    tail: bool,
    bytes_read: u64,
    truncated: bool,
    path: &str,
    output: Option<&str>,
) -> Value {
    let mut spans = vec![
        span(&task.display_name(), Some("accent")),
        json!({"text": format!(" ({})", task.id), "dim": true}),
        span(
            &format!(" {} {bytes_read}", if tail { "tail" } else { "head" }),
            Some("muted"),
        ),
    ];
    if truncated {
        spans.push(span(" (truncated)", Some("warning")));
    }
    let mut children = vec![
        text(Value::Array(spans)),
        text(json!([{"text": format!("Full output: {path}"), "dim": true}])),
    ];
    if let Some(out) = output {
        let lines: Vec<&str> = out.split('\n').take(30).collect();
        children.push(json!({"t": "log", "lines": lines}));
    }
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
}

/// `bg_kill` result card.
#[must_use]
pub fn kill_card(task: &Snapshot) -> Value {
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": [
        text(json!([span("■ killed", Some("warning")), {"text": " "}, span(&task.display_name(), Some("accent")), {"text": format!(" ({})", task.id), "dim": true}])),
        text(json!([{"text": format!("Output: {}", task.output_path), "dim": true}])),
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: Status) -> Snapshot {
        Snapshot {
            id: "t1".into(),
            name: Some("Run full test suite".into()),
            command: "npm test".into(),
            status,
            output_path: "/o.log".into(),
            start_time: 0,
            end_time: Some(25_000),
            exit_code: Some(Some(0)),
            trigger_on_completion: true,
            notify_on_completion: true,
            output_tail: Some(vec!["ok".into()]),
            ..Snapshot::default()
        }
    }

    #[test]
    fn launch_and_completion_cards() {
        let c = launch_card(&task(Status::Running), false);
        assert_eq!(c["children"][0]["value"], "BG  ");
        assert_eq!(
            c["children"][3]["value"][0]["text"],
            "started · wakes agent"
        );
        let e = launch_card(&task(Status::Running), true);
        assert_eq!(e["children"][1]["value"][0]["text"], "  $ npm test");
        assert_eq!(e["children"][2]["value"][0]["text"], "  ok");
        assert_eq!(
            completion_meta(&task(Status::Completed)),
            "exit 0 · 25s · agent woken"
        );
        let mut k = task(Status::Killed);
        k.exit_code = Some(None);
        assert_eq!(completion_meta(&k), "killed · 25s");
        let mut f = task(Status::Failed);
        f.exit_code = Some(None);
        f.trigger_on_completion = false;
        f.error = Some("boom".into());
        assert_eq!(completion_meta(&f), "error · 25s");
        assert_eq!(completion_card(None, false)["children"][2]["t"], "divider");
        assert_eq!(
            completion_card(Some(&f), false)["children"][0]["value"],
            "FAIL"
        );
    }
}
