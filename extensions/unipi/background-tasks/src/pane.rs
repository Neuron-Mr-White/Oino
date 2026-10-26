//! The work tray's "Background tasks" tab (unipi `tray-pane.ts`) as OUS.
//!
//! List view: tasks grouped Running / Recent (running newest-started first, then finished
//! newest-ended first), each row `kind · name · command` with tags. Detail view: header, meta,
//! command/description/info, the output log (tail, or the full log), the settled line.
//! Keys become buttons; unipi's double-press confirmations (`x x` kill, `a a` stop all) are
//! kept as an armed state: the first press arms and flashes, the second runs.

use serde_json::{Value, json};
use unipi_kit::ous::{dim, key_hints, span, text};

use crate::cards::tone;
use crate::snapshot::{Snapshot, Status};
use crate::text::{compact_whitespace, format_size, tray_elapsed};

/// Tab id.
pub const TAB: &str = "bg";
/// unipi `BG_LIST_HINT`.
pub const LIST_HINT: &str =
    "↑↓ navigate · ↵ view · s stop · x kill · d dismiss · R rerun · c path · ←→ tabs · esc close";
/// unipi `BG_VIEW_HINT`.
pub const VIEW_HINT: &str = "↑↓ scroll · g/G top/end · l full log · s stop · x kill · d dismiss · R rerun · c path · ←/esc back";
/// Live tail read for the detail view.
pub const TAIL_BYTES: u32 = 128 * 1024;
/// `l full log` read.
pub const FULL_LOG_BYTES: u32 = 4 * 1024 * 1024;
const DETAIL_COMMAND_MAX: usize = 160;

/// What the first press of a confirming key armed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Armed {
    /// `x` on a task.
    Kill(String),
    /// `a` (stop all).
    StopAll,
}

/// Per-session pane state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneState {
    /// Task shown in the detail view.
    pub view: Option<String>,
    /// Full log instead of the tail.
    pub full_log: bool,
    /// Armed confirmation.
    pub armed: Option<Armed>,
    /// One-shot message line.
    pub flash: Option<String>,
}

/// A tray action decoded from `<action>[:<task id>]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneAction {
    /// Open a task (`open` with the list item id as value).
    Open(String),
    /// Back to the list.
    Back,
    /// Graceful stop.
    Stop(String),
    /// Kill now (armed).
    Kill(String),
    /// Stop all running (armed).
    StopAll,
    /// Forget a finished task.
    Dismiss(String),
    /// Forget every finished task.
    DismissAll,
    /// Re-run.
    Rerun(String),
    /// Show the output path.
    Path(String),
    /// Toggle the full log.
    FullLog(String),
}

impl PaneAction {
    /// Parses an action id (without the `bg:` prefix) and value.
    #[must_use]
    pub fn parse(action_id: &str, value: Option<&Value>) -> Option<Self> {
        let (verb, arg) = action_id
            .split_once(':')
            .map_or((action_id, None), |(v, a)| (v, Some(a.to_owned())));
        let arg_or_value = || {
            arg.clone()
                .or_else(|| value.and_then(Value::as_str).map(ToOwned::to_owned))
        };
        Some(match verb {
            "open" => Self::Open(arg_or_value()?),
            "back" => Self::Back,
            "stop" => Self::Stop(arg_or_value()?),
            "kill" => Self::Kill(arg_or_value()?),
            "stopall" => Self::StopAll,
            "dismiss" => Self::Dismiss(arg_or_value()?),
            "dismissall" => Self::DismissAll,
            "rerun" => Self::Rerun(arg_or_value()?),
            "path" => Self::Path(arg_or_value()?),
            "full" => Self::FullLog(arg_or_value()?),
            _ => return None,
        })
    }
}

fn duration_of(t: &Snapshot, now: i64) -> i64 {
    t.end_time.unwrap_or(now) - t.start_time
}

/// Shown after the name when it adds something.
#[must_use]
pub fn short_command(t: &Snapshot) -> Option<String> {
    let cmd = compact_whitespace(&t.command);
    let name = compact_whitespace(&t.display_name());
    if cmd == name || cmd.is_empty() {
        return None;
    }
    Some(if cmd.chars().count() > 48 {
        format!("{}…", cmd.chars().take(47).collect::<String>())
    } else {
        cmd
    })
}

/// unipi `taskTags`.
#[must_use]
pub fn task_tags(t: &Snapshot, now: i64) -> Vec<String> {
    let mut tags = vec![
        tray_elapsed(duration_of(t, now)),
        format_size(t.bytes_written),
    ];
    if t.status == Status::Running && t.trigger_on_completion {
        tags.push("wakes agent".into());
    }
    if let Some(Some(c)) = t.exit_code
        && t.status != Status::Running
        && c != 0
    {
        tags.push(format!("exit {c}"));
    }
    tags
}

/// unipi `orderTasks`.
#[must_use]
pub fn order_tasks(tasks: &[Snapshot]) -> Vec<Snapshot> {
    let mut sorted = tasks.to_vec();
    sorted.sort_by_key(|t| std::cmp::Reverse(t.end_time.unwrap_or(t.start_time)));
    let (mut running, rest): (Vec<Snapshot>, Vec<Snapshot>) = sorted
        .into_iter()
        .partition(|t| t.status == Status::Running);
    running.extend(rest);
    running
}

fn kind(t: &Snapshot) -> &'static str {
    if t.is_agent { "Agent" } else { "Shell" }
}

fn row(t: &Snapshot, now: i64) -> Value {
    let mut item = json!({
        "id": t.id,
        "label": [dim(kind(t)), {"text": " "}, {"text": t.display_name(), "bold": true}],
        "badge": task_tags(t, now).join(" · "),
        "tone": tone(t.status),
    });
    if let Some(d) = short_command(t) {
        item["detail"] = json!(d);
    }
    item
}

fn button(label: &str, action: &str, tone: Option<&str>) -> Value {
    let mut b = json!({"t": "button", "label": label, "actionId": format!("{TAB}:{action}")});
    if let Some(t) = tone {
        b["tone"] = json!(t);
    }
    b
}

fn flash_node(state: &PaneState) -> Option<Value> {
    state
        .flash
        .as_deref()
        .map(|f| text(json!([span(f, Some("warning"))])))
}

/// List view body.
#[must_use]
pub fn list_body(tasks: &[Snapshot], state: &PaneState, now: i64) -> Value {
    let ordered = order_tasks(tasks);
    let mut children: Vec<Value> = Vec::new();
    if ordered.is_empty() {
        children.push(json!({"t": "empty", "message": "No background tasks."}));
    } else {
        let selected = ordered
            .iter()
            .find(|t| t.status == Status::Running)
            .or_else(|| ordered.first())
            .map(|t| t.id.clone());
        for (title, running) in [("Running", true), ("Recent", false)] {
            let items: Vec<Value> = ordered
                .iter()
                .filter(|t| (t.status == Status::Running) == running)
                .map(|t| row(t, now))
                .collect();
            if items.is_empty() {
                continue;
            }
            children.push(json!({"t": "section", "title": title, "body": {
                "t": "list", "items": items, "selectable": format!("{TAB}:open"), "selected": selected,
            }}));
        }
        children.push(
            json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
                button("Stop all", "stopall", Some("warning")),
                button("Dismiss finished", "dismissall", None),
            ]}),
        );
    }
    children.extend(flash_node(state));
    children.push(key_hints(&[
        ("↑↓", "navigate"),
        ("↵", "view"),
        ("s", "stop"),
        ("x", "kill"),
        ("d", "dismiss"),
        ("R", "rerun"),
        ("c", "path"),
        ("←→", "tabs"),
        ("esc", "close"),
    ]));
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
}

/// The output read for the detail view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogView {
    /// Lines (trailing empty line dropped).
    pub lines: Vec<String>,
    /// Earlier output not shown.
    pub truncated: bool,
    /// Total output bytes.
    pub total_bytes: u64,
    /// Read error.
    pub error: Option<String>,
}

impl LogView {
    /// From a read of the last `cap` bytes of `full_len` bytes of output.
    #[must_use]
    pub fn from_read(content: &str, full_len: u64) -> Self {
        let cleaned = content.replace('\r', "");
        let mut lines: Vec<String> = cleaned.split('\n').map(ToOwned::to_owned).collect();
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        Self {
            lines,
            truncated: full_len > content.len() as u64,
            total_bytes: full_len,
            error: None,
        }
    }
}

fn info_lines(t: &Snapshot) -> Vec<String> {
    let mut info = vec![format!("cwd {}", t.cwd), format!("log {}", t.output_path)];
    if let Some(r) = &t.telemetry_unavailable_reason {
        info.push(format!("telemetry {r}"));
    }
    info
}

fn settled_line(t: &Snapshot) -> Option<Value> {
    let code = match t.exit_code {
        Some(Some(c)) => format!("exit {c}"),
        _ => String::new(),
    };
    let with = |s: &str| {
        if s.is_empty() {
            String::new()
        } else {
            format!(" · {s}")
        }
    };
    let err = t.error.clone().unwrap_or_default();
    match t.status {
        Status::Running => t
            .error
            .as_deref()
            .map(|e| text(json!([span(e, Some("warning"))]))),
        Status::Completed => Some(text(json!([span(
            &format!("✓ Completed{}", with(&code)),
            Some("success")
        )]))),
        Status::Killed => Some(text(json!([span(
            &format!("⊘ Stopped{}", with(&err)),
            Some("warning")
        )]))),
        Status::Failed => Some(text(json!([span(
            &format!("✗ Failed{}{}", with(&code), with(&err)),
            Some("error")
        )]))),
    }
}

/// Detail view body.
#[must_use]
pub fn detail_body(t: &Snapshot, log: Option<&LogView>, state: &PaneState, now: i64) -> Value {
    let status_text = match t.status {
        Status::Running if t.trigger_on_completion => "running · wakes agent".to_owned(),
        Status::Running => "running".to_owned(),
        Status::Killed => "stopped".to_owned(),
        s => s.as_str().to_owned(),
    };
    let mut meta = vec![format!("id {}", t.id)];
    if let Some(pid) = t.pid {
        meta.push(format!("pid {pid}"));
    }
    meta.push(status_text);
    meta.push(format!(
        "started {}",
        unipi_kit::fmt::iso_from_ms(t.start_time)
    ));
    if state.full_log {
        meta.push("full log".into());
    }
    let mut children = vec![
        json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": [
            if t.status == Status::Running {
                json!({"t": "spinner", "label": crate::cards::chip(t.status)})
            } else {
                json!({"t": "badge", "value": crate::cards::chip(t.status), "tone": tone(t.status)})
            },
            text(json!([dim(kind(t)), dim(" › "), {"text": t.display_name(), "bold": true}])),
            {"t": "divider"},
            text(json!([dim(&format!("{} · {}", tray_elapsed(duration_of(t, now)), format_size(t.bytes_written)))])),
        ]}),
        text(json!([dim(&meta.join(" · "))])),
    ];
    let cmd = if t.command.chars().count() > DETAIL_COMMAND_MAX * 4 {
        format!(
            "{}…",
            t.command
                .chars()
                .take(DETAIL_COMMAND_MAX * 4)
                .collect::<String>()
        )
    } else {
        t.command.clone()
    };
    children.push(json!({"t": "section", "title": "Command", "body": {"t": "code", "value": cmd, "language": "sh"}}));
    if let Some(d) = &t.description
        && compact_whitespace(d) != compact_whitespace(&t.display_name())
    {
        children.push(json!({"t": "section", "title": "Description", "body": text(d.as_str())}));
    }
    children.push(text(json!([dim(&info_lines(t).join("\n"))])));
    let output = match log {
        Some(LogView { error: Some(e), .. }) => text(json!([span(e, Some("error"))])),
        None => text(json!([dim("reading…")])),
        Some(l) if l.lines.is_empty() => text(json!([dim("no output yet")])),
        Some(l) => {
            let mut parts = Vec::new();
            if l.truncated {
                parts.push(text(json!([dim(&format!(
                    "… earlier output not shown ({} total{})",
                    format_size(l.total_bytes),
                    if state.full_log { "" } else { " · l full log" }
                ))])));
            }
            parts.push(json!({"t": "log", "lines": l.lines, "tail": true, "maxLines": 5000}));
            json!({"t": "stack", "direction": "vertical", "gap": 0, "children": parts})
        }
    };
    children.push(json!({"t": "section", "title": "Output", "body": output}));
    children.extend(settled_line(t));
    let id = &t.id;
    let mut buttons = vec![button("Back", "back", None)];
    if t.status == Status::Running {
        buttons.push(button("Stop", &format!("stop:{id}"), Some("warning")));
        buttons.push(button("Kill", &format!("kill:{id}"), Some("error")));
    } else {
        buttons.push(button("Dismiss", &format!("dismiss:{id}"), None));
    }
    buttons.push(button("Rerun", &format!("rerun:{id}"), None));
    buttons.push(button(
        if state.full_log { "Tail" } else { "Full log" },
        &format!("full:{id}"),
        None,
    ));
    buttons.push(button("Path", &format!("path:{id}"), None));
    children.push(json!({"t": "stack", "direction": "horizontal", "gap": 1, "children": buttons}));
    children.extend(flash_node(state));
    children.push(key_hints(&[
        ("↑↓", "scroll"),
        ("g/G", "top/end"),
        ("l", "full log"),
        ("s", "stop"),
        ("x", "kill"),
        ("d", "dismiss"),
        ("R", "rerun"),
        ("c", "path"),
        ("←/esc", "back"),
    ]));
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": children})
}

/// `Counts` of the tab (`total`, `running`).
#[must_use]
pub fn counts(tasks: &[Snapshot]) -> unipi_kit::tray::Counts {
    let n = |f: &dyn Fn(&Snapshot) -> bool| {
        u32::try_from(tasks.iter().filter(|t| f(t)).count()).unwrap_or(u32::MAX)
    };
    unipi_kit::tray::Counts {
        total: n(&|_| true),
        running: n(&|t| t.status == Status::Running),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, status: Status, start: i64, end: Option<i64>) -> Snapshot {
        Snapshot {
            id: id.into(),
            name: Some(format!("task {id}")),
            command: format!("run {id} --long"),
            status,
            start_time: start,
            end_time: end,
            exit_code: end.map(|_| Some(1)),
            trigger_on_completion: true,
            bytes_written: 559,
            ..Snapshot::default()
        }
    }

    #[test]
    fn orders_running_first_then_newest() {
        let tasks = [
            t("a", Status::Completed, 0, Some(10)),
            t("b", Status::Running, 5, None),
            t("c", Status::Failed, 0, Some(20)),
            t("d", Status::Running, 8, None),
        ];
        let ids: Vec<String> = order_tasks(&tasks).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["d", "b", "c", "a"]);
    }

    #[test]
    fn tags_and_rows() {
        let r = t("r", Status::Running, 0, None);
        assert_eq!(task_tags(&r, 70_000), ["1m10s", "559B", "wakes agent"]);
        let f = t("f", Status::Failed, 0, Some(1000));
        assert_eq!(task_tags(&f, 0), ["1s", "559B", "exit 1"]);
        assert_eq!(short_command(&r).as_deref(), Some("run r --long"));
        let body = list_body(&[r.clone(), f], &PaneState::default(), 70_000);
        assert_eq!(body["children"][0]["title"], "Running");
        assert_eq!(body["children"][1]["title"], "Recent");
        assert_eq!(body["children"][0]["body"]["selectable"], "bg:open");
        assert_eq!(body["children"][0]["body"]["selected"], "r");
        let empty = list_body(&[], &PaneState::default(), 0);
        assert_eq!(empty["children"][0]["message"], "No background tasks.");
        let d = detail_body(
            &r,
            Some(&LogView::from_read("x\ny\n", 4)),
            &PaneState::default(),
            0,
        );
        let s = d.to_string();
        assert!(s.contains("\"bg:stop:r\"") && s.contains("\"bg:kill:r\""));
        assert!(s.contains("running · wakes agent"));
    }

    #[test]
    fn parses_actions() {
        assert_eq!(
            PaneAction::parse("open", Some(&json!("t1"))),
            Some(PaneAction::Open("t1".into()))
        );
        assert_eq!(
            PaneAction::parse("kill:t2", None),
            Some(PaneAction::Kill("t2".into()))
        );
        assert_eq!(
            PaneAction::parse("stopall", None),
            Some(PaneAction::StopAll)
        );
        assert_eq!(PaneAction::parse("stop", None), None);
        assert_eq!(PaneAction::parse("bogus", None), None);
        let l = LogView::from_read("a\r\nb\n", 100);
        assert_eq!((l.lines.len(), l.truncated), (2, true));
    }
}
