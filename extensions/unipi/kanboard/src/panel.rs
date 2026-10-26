//! The board panel (unipi-port §3.16; replaces the daemon's web UI, D22): a `sidePane` panel
//! with one `list` column per lane in a horizontal `split`, a detail section for the selected
//! task with one button per allowed move, refreshed after every action and from the daemon's
//! SSE stream.

use serde_json::{Value, json};
use unipi_kit::ous;

use crate::render::claim_order;
use crate::shapes::Task;

/// Panel id (scoped by the host as `unipi.kanboard/board`).
pub const PANEL_ID: &str = "board";
/// Action: a task was selected (`value` = task id).
pub const ACTION_SELECT: &str = "select";
/// Action prefix: move the selected task (`move:<status>`).
pub const ACTION_MOVE: &str = "move:";
/// Action: reload the board.
pub const ACTION_REFRESH: &str = "refresh";
/// Action: add a Backlog task.
pub const ACTION_ADD: &str = "add";
/// Action: post the board in chat (`/unipi:kanboard show`).
pub const ACTION_SHOW: &str = "show";

/// Board columns, in board order.
pub const COLUMNS: &[(&str, &str)] = &[
    ("backlog", "Backlog"),
    ("todo", "Todo"),
    ("in_progress", "In Progress"),
    ("blocked", "Blocked"),
    ("in_review", "In Review"),
    ("done", "Done"),
];

fn status_label(status: &str) -> String {
    COLUMNS
        .iter()
        .chain(&[("cancelled", "Cancelled"), ("archived", "Archived")])
        .find(|(id, _)| *id == status)
        .map_or_else(|| status.to_owned(), |(_, l)| (*l).to_owned())
}

fn prio_badge(priority: Option<&str>) -> Option<(&'static str, &'static str)> {
    match priority? {
        "urgent" => Some(("urgent", "error")),
        "high" => Some(("high", "warning")),
        "medium" => Some(("medium", "info")),
        "low" => Some(("low", "muted")),
        _ => None,
    }
}

fn item(task: &Task, session: &str) -> Value {
    let mut it = json!({"id": task.id, "label": [ous::span(&task.id, Some("muted")), {"text": format!(" {}", task.shown_title())}]});
    let detail = if task.claimed_by(session) {
        Some("claimed by this session".to_owned())
    } else if let Some(reason) = task.blocked_reason() {
        Some(reason.to_owned())
    } else if !task.waiting_for.is_empty() {
        Some(format!("waits for {}", task.waiting_for.join(", ")))
    } else {
        task.run
            .as_ref()
            .and_then(|r| r.session.as_deref())
            .map(|s| format!("· {s}"))
    };
    if let Some(d) = detail {
        it["detail"] = json!(d);
    }
    if let Some((label, tone)) = prio_badge(task.priority.as_deref()) {
        it["badge"] = json!(label);
        it["tone"] = json!(tone);
    }
    if task.status == "blocked" {
        it["tone"] = json!("error");
    }
    it
}

/// The detail section of the selected task.
fn detail(task: &Task) -> Value {
    let mut kv = vec![
        json!({"key": "Status", "value": status_label(&task.status)}),
        json!({"key": "Priority", "value": task.priority.clone().unwrap_or_else(|| "none".into())}),
    ];
    if !task.deps.is_empty() {
        kv.push(json!({"key": "After", "value": task.deps.join(", ")}));
    }
    if let Some(s) = task.run.as_ref().and_then(|r| r.session.as_deref()) {
        kv.push(json!({"key": "Claimed by", "value": s}));
    }
    let mut children = vec![json!({"t": "kv", "items": kv})];
    if let Some(body) = task
        .extra
        .get("body")
        .and_then(Value::as_str)
        .filter(|b| !b.trim().is_empty())
    {
        children.push(json!({"t": "markdown", "value": body}));
    }
    if let Some(last) = task.activity.last() {
        children.push(ous::text(vec![ous::dim(&format!(
            "{} [{}] {}",
            last.at, last.actor, last.text
        ))]));
    }
    let buttons: Vec<Value> = task
        .allowed_moves
        .iter()
        .map(|to| json!({"t": "button", "key": format!("mv-{to}"), "label": format!("→ {}", status_label(to)), "actionId": format!("{ACTION_MOVE}{to}")}))
        .collect();
    if !buttons.is_empty() {
        children.push(ous::stack("horizontal", 1, &buttons));
    }
    json!({"t": "section", "key": "detail", "title": format!("{} {}", task.id, task.shown_title()), "body": ous::stack("vertical", 0, &children)})
}

/// The panel body.
#[must_use]
pub fn spec(
    slug: &str,
    tasks: &[Task],
    selected: Option<&str>,
    session: &str,
    live: bool,
) -> Value {
    let columns: Vec<Value> = COLUMNS
        .iter()
        .map(|(id, label)| {
            let lane: Vec<Task> = tasks.iter().filter(|t| t.status == *id).cloned().collect();
            let lane = if *id == "todo" { claim_order(&lane) } else { lane };
            let body = if lane.is_empty() {
                ous::empty("empty")
            } else {
                let mut list = json!({"t": "list", "key": format!("lane-{id}"), "items": lane.iter().map(|t| item(t, session)).collect::<Vec<_>>(), "selectable": ACTION_SELECT});
                if let Some(sel) = selected.filter(|s| lane.iter().any(|t| t.id == *s)) {
                    list["selected"] = json!(sel);
                }
                list
            };
            json!({"t": "section", "key": format!("col-{id}"), "title": format!("{label} ({})", lane.len()), "body": body})
        })
        .collect();
    let mut children = vec![
        ous::text_nowrap(vec![
            json!({"text": slug, "bold": true}),
            ous::dim(&format!(
                " · {} tasks{}",
                tasks.len(),
                if live { " · live" } else { "" }
            )),
        ]),
        json!({"t": "split", "key": "lanes", "direction": "horizontal", "children": columns}),
    ];
    match selected.and_then(|s| tasks.iter().find(|t| t.id == s)) {
        Some(task) => children.push(detail(task)),
        None => children.push(ous::text(vec![ous::dim(
            "Select a task to see it and move it.",
        )])),
    }
    ous::stack("vertical", 1, &children)
}

/// Panel-level keys.
#[must_use]
pub fn keys() -> Value {
    json!([
        {"key": "r", "actionId": ACTION_REFRESH, "label": "Refresh"},
        {"key": "a", "actionId": ACTION_ADD, "label": "Add to Backlog"},
        {"key": "s", "actionId": ACTION_SHOW, "label": "Show in chat"},
    ])
}

/// The `openPanel` request.
#[must_use]
pub fn open_request(slug: &str, body: &Value) -> Value {
    json!({
        "panelId": PANEL_ID,
        "placement": "sidePane",
        "title": format!("Kanboard · {slug}"),
        "spec": body,
        "keys": keys(),
        "focus": true,
        "closeOnEscape": true,
    })
}

/// The comment prompt a move needs (the binary's rule text names `--comment`).
#[must_use]
pub fn needs_comment(message: &str) -> bool {
    message.contains("--comment")
        || message.contains("requires a comment")
        || message.contains("comment required")
}

/// Pulls the revision lines out of an SSE buffer: returns complete `data:` payloads and
/// leaves an incomplete trailing line in `buffer`.
pub fn drain_sse(buffer: &mut String) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(nl) = buffer.find('\n') {
        let line: String = buffer.drain(..=nl).collect();
        if let Some(data) = line.trim_end().strip_prefix("data:") {
            out.push(data.trim().to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Run;

    fn t(id: &str, status: &str) -> Task {
        Task {
            id: id.into(),
            title: format!("t{id}"),
            status: status.into(),
            ..Task::default()
        }
    }

    #[test]
    fn builds_lanes_and_detail() {
        let mut a = t("A-1", "todo");
        a.allowed_moves = vec!["backlog".into(), "cancelled".into()];
        a.priority = Some("urgent".into());
        let mut b = t("A-2", "in_progress");
        b.run = Some(Run {
            session: Some("me".into()),
            ..Run::default()
        });
        let s = spec("proj", &[a, b], Some("A-1"), "me", true);
        let split = &s["children"][1];
        assert_eq!(split["t"], "split");
        assert_eq!(split["children"].as_array().unwrap().len(), 6);
        let todo = &split["children"][1];
        assert_eq!(todo["title"], "Todo (1)");
        assert_eq!(todo["body"]["selected"], "A-1");
        assert_eq!(todo["body"]["items"][0]["badge"], "urgent");
        assert_eq!(
            split["children"][2]["body"]["items"][0]["detail"],
            "claimed by this session"
        );
        assert_eq!(split["children"][0]["body"]["t"], "empty");
        let detail = &s["children"][2];
        assert_eq!(detail["title"], "A-1 tA-1");
        let buttons = &detail["body"]["children"][1]["children"];
        assert_eq!(buttons[0]["actionId"], "move:backlog");
        assert_eq!(buttons[1]["label"], "→ Cancelled");
        let req = open_request("proj", &s);
        assert_eq!(req["placement"], "sidePane");
        assert_eq!(req["keys"][0]["key"], "r");
    }

    #[test]
    fn sse_and_comment_detection() {
        let mut buf = "data: 1\n\ndata: 2\nda".to_owned();
        assert_eq!(drain_sse(&mut buf), ["1", "2"]);
        assert_eq!(buf, "da");
        assert!(needs_comment(
            "in_review → todo requires --comment (rework note)"
        ));
        assert!(!needs_comment("unknown task"));
    }
}
