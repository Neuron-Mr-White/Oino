//! Transcript renderings: the in-chat board (`show`), badges, notices and the board progress
//! bar (unipi `commands.ts` show renderer, `badges.ts`, `notice-buffer.ts`, `progress.ts` +
//! core `progress.ts`). Plain text stays the stored content; renderers return OUS nodes.

use serde_json::{Value, json};
use unipi_kit::ous;

use crate::shapes::Task;
use crate::shell::invocations;

/// Badge entry type.
pub const BADGE_ENTRY: &str = "unipi:kanboard-badge";
/// Notice entry type.
pub const NOTICE_ENTRY: &str = "unipi:kanboard-notice";
/// Progress entry type (core `PROGRESS_ENTRY`).
pub const PROGRESS_ENTRY: &str = "unipi-progress";

// ── show ────────────────────────────────────────────────────────────────

const SHOW_LANES: &[(&str, &str)] = &[
    ("in_progress", "In Progress"),
    ("blocked", "Blocked"),
    ("todo", "Todo"),
    ("in_review", "In Review"),
    ("backlog", "Backlog"),
    ("done", "Done"),
];
const SHOW_EXTRA_LANES: &[(&str, &str)] = &[("cancelled", "Cancelled"), ("archived", "Archived")];

/// Lanes `show` lists.
#[must_use]
pub fn show_lanes(all: bool, filter: Option<&str>) -> Vec<(&'static str, &'static str)> {
    let lanes = SHOW_LANES.iter().chain(SHOW_EXTRA_LANES).copied();
    match filter {
        Some(f) => lanes.filter(|(id, _)| *id == f).collect(),
        None if all => lanes.collect(),
        None => SHOW_LANES.to_vec(),
    }
}

fn prio_glyph(p: Option<&str>) -> &'static str {
    match p.unwrap_or("none") {
        "urgent" => "⇈",
        "high" => "↑",
        "medium" => "·",
        "low" => "↓",
        _ => " ",
    }
}

fn prio_rank(p: Option<&str>) -> u8 {
    match p.unwrap_or("none") {
        "urgent" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

/// The CLI's claim order: priority, then lane order, then id.
#[must_use]
pub fn claim_order(tasks: &[Task]) -> Vec<Task> {
    let mut out = tasks.to_vec();
    out.sort_by(|a, b| {
        prio_rank(a.priority.as_deref())
            .cmp(&prio_rank(b.priority.as_deref()))
            .then(a.order.unwrap_or(0).cmp(&b.order.unwrap_or(0)))
            .then(a.id.cmp(&b.id))
    });
    out
}

/// One row of a lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Tree prefix (`""`, `"├ "`, `"└ "`).
    pub branch: &'static str,
    /// `1.` numbering (todo lane).
    pub number: Option<String>,
    /// Task id.
    pub id: String,
    /// Priority glyph.
    pub prio: &'static str,
    /// Title.
    pub title: String,
    /// `after …`, `· session`, blocked reason, `ready`/`waits for …`.
    pub extra: Option<String>,
    /// `dim` or `error`.
    pub tone: Option<&'static str>,
}

fn emit_todo(
    task: &Task,
    branch: &'static str,
    number: usize,
    kids: &std::collections::HashMap<String, Vec<Task>>,
    emitted: &mut Vec<String>,
    rows: &mut Vec<Row>,
) {
    emitted.push(task.id.clone());
    let waits: Vec<&str> = task
        .waiting_for
        .iter()
        .map(String::as_str)
        .filter(|w| !w.is_empty())
        .collect();
    let waiting = task.ready == Some(false) || !waits.is_empty();
    rows.push(Row {
        branch,
        number: Some(format!("{number}.")),
        id: task.id.clone(),
        prio: prio_glyph(task.priority.as_deref()),
        title: task.shown_title().to_owned(),
        extra: Some(if waiting {
            format!("waits for {}", waits.join(", "))
        } else {
            "ready".into()
        }),
        tone: waiting.then_some("dim"),
    });
    let children = kids.get(&task.id).cloned().unwrap_or_default();
    let last = children.len().saturating_sub(1);
    for (i, child) in children.iter().enumerate() {
        emit_todo(
            child,
            if i == last { "└ " } else { "├ " },
            number,
            kids,
            emitted,
            rows,
        );
    }
}

/// Rows of one lane.
#[must_use]
pub fn lane_rows(tasks: &[Task], lane: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    if lane == "todo" {
        let ordered = claim_order(tasks);
        let mut kids: std::collections::HashMap<String, Vec<Task>> =
            std::collections::HashMap::new();
        for task in &ordered {
            for dep in &task.deps {
                if ordered.iter().any(|t| &t.id == dep) {
                    kids.entry(dep.clone()).or_default().push(task.clone());
                }
            }
        }
        let mut emitted: Vec<String> = Vec::new();
        let mut n = 0;
        for task in &ordered {
            if emitted.contains(&task.id) {
                continue;
            }
            n += 1;
            emit_todo(task, "", n, &kids, &mut emitted, &mut rows);
        }
        return rows;
    }
    for task in tasks {
        let session = task.run.as_ref().and_then(|r| r.session.as_deref());
        let (extra, tone) = if lane == "in_progress" && session.is_some() {
            (Some(format!("· {}", session.unwrap_or_default())), None)
        } else if lane == "blocked" && task.blocked_reason().is_some() {
            (task.blocked_reason().map(ToOwned::to_owned), Some("error"))
        } else if !task.deps.is_empty() {
            (Some(format!("after {}", task.deps.join(", "))), None)
        } else {
            (None, None)
        };
        rows.push(Row {
            branch: "",
            number: None,
            id: task.id.clone(),
            prio: prio_glyph(task.priority.as_deref()),
            title: task.shown_title().to_owned(),
            extra,
            tone,
        });
    }
    rows
}

/// Plain-text board (the stored message content).
#[must_use]
pub fn show_plain(project: &str, tasks: &[Task], all: bool, filter: Option<&str>) -> String {
    let tasks: Vec<Task> = tasks
        .iter()
        .filter(|t| filter.is_none_or(|f| t.status == f))
        .cloned()
        .collect();
    let mut lines = vec![format!("{project} · {} tasks", tasks.len())];
    for (id, label) in show_lanes(all, filter) {
        let lane: Vec<Task> = tasks.iter().filter(|t| t.status == id).cloned().collect();
        if lane.is_empty() {
            lines.push(format!("{label} — empty"));
            continue;
        }
        lines.push(label.to_owned());
        for row in lane_rows(&lane, id) {
            lines.push(format!(
                "  {}{}{} {} {}{}",
                row.branch,
                row.number.map(|n| format!("{n} ")).unwrap_or_default(),
                row.id,
                row.prio,
                row.title,
                row.extra.map(|e| format!("  {e}")).unwrap_or_default()
            ));
        }
    }
    lines.join("\n")
}

/// OUS rendering of the show message (`details = {project, tasks, all, lane}`). Frontends
/// truncate rows to the width (unipi's width budget is a frontend concern).
#[must_use]
pub fn show_node(details: &Value) -> Option<Value> {
    let tasks: Vec<Task> = serde_json::from_value(details.get("tasks")?.clone()).ok()?;
    let all = details.get("all").and_then(Value::as_bool).unwrap_or(false);
    let filter = details.get("lane").and_then(Value::as_str);
    let project = details
        .get("project")
        .and_then(Value::as_str)
        .unwrap_or("board");
    let tasks: Vec<Task> = tasks
        .into_iter()
        .filter(|t| filter.is_none_or(|f| t.status == f))
        .collect();
    let mut children = vec![ous::text_nowrap(vec![
        json!({"text": format!("{project} · {} tasks", tasks.len()), "bold": true}),
    ])];
    for (id, label) in show_lanes(all, filter) {
        let lane: Vec<Task> = tasks.iter().filter(|t| t.status == id).cloned().collect();
        if lane.is_empty() {
            children.push(ous::text_nowrap(vec![ous::dim(&format!(
                "  {label} — empty"
            ))]));
            continue;
        }
        children.push(ous::text_nowrap(vec![ous::span(label, Some("accent"))]));
        for row in lane_rows(&lane, id) {
            let mut spans = vec![
                json!({"text": format!("  {}{}", row.branch, row.number.map(|n| format!("{n} ")).unwrap_or_default())}),
                ous::span(&row.id, Some("muted")),
                json!({"text": format!(" {} {}", row.prio, row.title)}),
            ];
            if let Some(extra) = row.extra {
                spans.push(if row.tone == Some("error") {
                    ous::span(&format!("  {extra}"), Some("error"))
                } else {
                    ous::dim(&format!("  {extra}"))
                });
            }
            children.push(ous::text_nowrap(spans));
        }
    }
    Some(ous::stack("vertical", 0, &children))
}

// ── badges ──────────────────────────────────────────────────────────────

/// A badge for a successful lead `start`/`finish`/`move <ID> blocked`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    /// `started`, `finished` or `blocked`.
    pub kind: &'static str,
    /// Task id.
    pub id: String,
    /// What a blocked task needs.
    pub text: Option<String>,
}

impl Badge {
    /// Entry data.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut v = json!({"kind": self.kind, "id": self.id});
        if let Some(t) = &self.text {
            v["text"] = json!(t);
        }
        v
    }
}

/// The badge a successful kanboard bash call produces.
#[must_use]
pub fn badge_from_tool_call(tool_name: &str, input: &Value, is_error: bool) -> Option<Badge> {
    if is_error || (tool_name != "bash" && tool_name != "powershell") {
        return None;
    }
    let command = input.get("command").and_then(Value::as_str).unwrap_or("");
    for inv in invocations(command) {
        let kind = match inv.sub.as_str() {
            "start" => "started",
            "finish" => "finished",
            "move" if inv.args.iter().any(|a| a == "blocked") => "blocked",
            _ => continue,
        };
        let Some(id) = inv.first_positional() else {
            continue;
        };
        let text = (kind == "blocked")
            .then(|| inv.args.iter().position(|a| a == "--comment"))
            .flatten()
            .map(|i| {
                inv.args
                    .get(i + 1)
                    .map(|c| {
                        c.trim_start_matches(['"', '\''])
                            .trim_end_matches(['"', '\''])
                            .to_owned()
                    })
                    .unwrap_or_default()
            })
            .filter(|t| !t.is_empty());
        return Some(Badge {
            kind,
            id: id.to_owned(),
            text,
        });
    }
    None
}

/// Badge text (`▣ UNI-30 started`, `✓ UNI-30 → In Review`, `⊘ UNI-30 blocked: …`).
#[must_use]
pub fn badge_text(kind: &str, id: &str, text: Option<&str>) -> String {
    match kind {
        "started" => format!("▣ {id} started"),
        "finished" => format!("✓ {id} → In Review"),
        _ => {
            let what = text
                .map(|t| format!(": {}", t.chars().take(80).collect::<String>()))
                .unwrap_or_default();
            format!("⊘ {id} blocked{what}")
        }
    }
}

/// OUS for a badge entry.
#[must_use]
pub fn badge_node(data: &Value) -> Option<Value> {
    let id = data.get("id")?.as_str()?;
    let kind = data
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("started");
    let tone = match kind {
        "started" => "accent",
        "finished" => "success",
        _ => "warning",
    };
    let text = badge_text(kind, id, data.get("text").and_then(Value::as_str));
    Some(ous::text_nowrap(vec![ous::bold(&text, tone)]))
}

/// OUS for a notice entry (`{text, level}`).
#[must_use]
pub fn notice_node(data: &Value) -> Option<Value> {
    let text = data.get("text")?.as_str()?;
    let tone = if data.get("level").and_then(Value::as_str) == Some("warning") {
        "warning"
    } else {
        "muted"
    };
    Some(ous::text(vec![ous::span(text, Some(tone))]))
}

// ── progress ────────────────────────────────────────────────────────────

/// Board progress bar data (`unipi-progress` entry): solid = in review + done, shade = in
/// progress, light = todo + blocked; backlog/cancelled/archived are left out.
#[must_use]
pub fn board_progress(tasks: &[Task], slug: &str) -> Option<Value> {
    const COUNTED: &[&str] = &["todo", "in_progress", "in_review", "done", "blocked"];
    let counted: Vec<&Task> = tasks
        .iter()
        .filter(|t| COUNTED.contains(&t.status.as_str()))
        .collect();
    if counted.is_empty() {
        return None;
    }
    let n = |s: &str| counted.iter().filter(|t| t.status == s).count();
    let blocked = n("blocked");
    let mut v = json!({
        "icon": "▣",
        "label": if slug.is_empty() { "Board".to_owned() } else { format!("Board · {slug}") },
        "done": n("in_review") + n("done"),
        "active": n("in_progress"),
        "total": counted.len(),
        "unit": "tasks",
        "color": "success",
    });
    if blocked > 0 {
        v["detail"] = json!(format!("{blocked} blocked"));
    }
    Some(v)
}

/// A 20-cell bar: `█` done, `▒` active, `░` rest.
#[must_use]
pub fn progress_bar(done: u64, active: u64, total: u64, width: u64) -> String {
    if total == 0 {
        return "░".repeat(usize::try_from(width).unwrap_or(0));
    }
    let cells = |n: u64| (n.min(total) * width).div_ceil(total).min(width);
    let solid = cells(done);
    let shade = cells(done + active).saturating_sub(solid);
    let light = width.saturating_sub(solid + shade);
    let rep = |c: &str, n: u64| c.repeat(usize::try_from(n).unwrap_or(0));
    format!("{}{}{}", rep("█", solid), rep("▒", shade), rep("░", light))
}

/// OUS for a `unipi-progress` entry (core `progressLines`).
#[must_use]
pub fn progress_node(d: &Value) -> Option<Value> {
    let label = d.get("label")?.as_str()?;
    let u = |k: &str| d.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (done, active, total) = (u("done"), u("active"), u("total"));
    let tone = d.get("color").and_then(Value::as_str).unwrap_or("accent");
    let count = if let Some(unit) = d.get("unit").and_then(Value::as_str) {
        format!("{done}/{total} {unit}")
    } else {
        let pct = (done * 100 + total / 2).checked_div(total).unwrap_or(0);
        let est = if d.get("estimated").and_then(Value::as_bool) == Some(true) {
            "~"
        } else {
            ""
        };
        format!("{est}{pct}%")
    };
    let mut spans = vec![
        ous::span(
            d.get("icon").and_then(Value::as_str).unwrap_or("◎"),
            Some(tone),
        ),
        json!({"text": format!(" {label}"), "bold": true}),
        json!({"text": "  "}),
        ous::span(&progress_bar(done, active, total, 20), Some(tone)),
        json!({"text": format!("  {count}")}),
    ];
    if let Some(detail) = d.get("detail").and_then(Value::as_str) {
        spans.push(ous::dim(&format!("  {detail}")));
    }
    let mut children = vec![ous::text_nowrap(spans)];
    if let Some(summary) = d.get("summary").and_then(Value::as_str) {
        children.push(ous::text(vec![ous::span(
            &format!("  {}", summary.trim()),
            Some("muted"),
        )]));
    }
    Some(ous::stack("vertical", 0, &children))
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
    fn orders_lanes_numbers_todo_and_marks_waits() {
        let mut a = t("A-1", "todo");
        a.priority = Some("high".into());
        let mut b = t("A-2", "todo");
        b.deps = vec!["A-1".into()];
        b.ready = Some(false);
        b.waiting_for = vec!["A-1".into()];
        let c = t("A-3", "todo");
        let mut ip = t("A-4", "in_progress");
        ip.run = Some(Run {
            session: Some("oino-s".into()),
            ..Run::default()
        });
        let mut bl = t("A-5", "blocked");
        bl.extra
            .insert("blockedReason".into(), json!({"text": "need creds"}));
        let text = show_plain(
            "proj",
            &[c, b, a, ip, bl, t("A-6", "cancelled")],
            false,
            None,
        );
        assert_eq!(
            text,
            "proj · 6 tasks\nIn Progress\n  A-4   tA-4  · oino-s\nBlocked\n  A-5   tA-5  need creds\nTodo\n  1. A-1 ↑ tA-1  ready\n  └ 1. A-2   tA-2  waits for A-1\n  2. A-3   tA-3  ready\nIn Review — empty\nBacklog — empty\nDone — empty"
        );
        assert!(
            show_plain("p", &[t("A-6", "cancelled")], true, None)
                .contains("Cancelled\n  A-6   tA-6")
        );
        assert_eq!(
            show_plain(
                "p",
                &[t("A-1", "todo"), t("A-2", "done")],
                false,
                Some("done")
            ),
            "p · 1 tasks\nDone\n  A-2   tA-2"
        );
    }

    #[test]
    fn show_node_renders_lanes() {
        let details = json!({"project": "p", "tasks": [t("A-1", "todo")], "all": false});
        let node = show_node(&details).unwrap();
        assert_eq!(node["t"], "stack");
        assert_eq!(node["children"].as_array().unwrap().len(), 1 + 5 + 2);
        assert!(show_node(&json!({})).is_none());
    }

    #[test]
    fn badges() {
        let b = badge_from_tool_call("bash", &json!({"command": "oino-kanboard --actor agent move A-1 blocked --comment 'need creds'"}), false).unwrap();
        assert_eq!(
            b,
            Badge {
                kind: "blocked",
                id: "A-1".into(),
                text: Some("need creds".into())
            }
        );
        assert_eq!(
            badge_text("blocked", "A-1", Some("need creds")),
            "⊘ A-1 blocked: need creds"
        );
        assert_eq!(badge_text("started", "A-1", None), "▣ A-1 started");
        assert_eq!(badge_text("finished", "A-1", None), "✓ A-1 → In Review");
        assert!(
            badge_from_tool_call("bash", &json!({"command": "oino-kanboard start A-1"}), true)
                .is_none()
        );
        assert!(badge_from_tool_call("read", &json!({}), false).is_none());
        assert!(
            badge_from_tool_call("bash", &json!({"command": "oino-kanboard list"}), false)
                .is_none()
        );
        assert_eq!(
            badge_node(&b.to_json()).unwrap()["value"][0]["tone"],
            "warning"
        );
        assert_eq!(
            notice_node(&json!({"text": "x", "level": "warning"})).unwrap()["value"][0]["tone"],
            "warning"
        );
    }

    #[test]
    fn progress() {
        let tasks = [
            t("1", "done"),
            t("2", "in_review"),
            t("3", "in_progress"),
            t("4", "todo"),
            t("5", "blocked"),
            t("6", "backlog"),
        ];
        let p = board_progress(&tasks, "proj").unwrap();
        assert_eq!(p["done"], 2);
        assert_eq!(p["active"], 1);
        assert_eq!(p["total"], 5);
        assert_eq!(p["detail"], "1 blocked");
        assert_eq!(p["label"], "Board · proj");
        assert!(board_progress(&[t("6", "backlog")], "x").is_none());
        assert_eq!(
            progress_bar(2, 1, 5, 20),
            format!("{}{}{}", "█".repeat(8), "▒".repeat(4), "░".repeat(8))
        );
        assert!(progress_node(&p).is_some());
    }
}
