//! The work tray's "Background tasks" tab (unipi `tray-pane.ts`) as OUS.
//!
//! List view: tasks grouped Running / Recent (running newest-started first, then finished
//! newest-ended first), each row `kind · name · command` with tags. Detail view: header, meta,
//! command/description/info, the output log (tail, or the full log), the settled line.
//! Keys become buttons; unipi's double-press confirmations (`x x` kill, `a a` stop all) are
//! kept as an armed state: the first press arms and flashes, the second runs.

use oino_extension::ui::widgets::prelude::*;
use serde_json::Value;

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

fn row(t: &Snapshot, now: i64) -> ListItem {
    let item = ListItem::new(Line::from(vec![
        Span::from(kind(t)).dim(),
        Span::from(" "),
        Span::from(t.display_name()).bold(),
    ]))
    .key(&t.id)
    .badge(Line::from(task_tags(t, now).join(" · ")))
    .style(Style::new().fg(Color::parse(tone(t.status))));
    match short_command(t) {
        Some(d) => item.detail(Line::from(d)),
        None => item,
    }
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

fn button(label: &str, action: &str, tone: Option<&str>) -> Node {
    let button = Button::new(label, format!("{TAB}:{action}"));
    match tone {
        Some("error" | "warning") => button.variant(ButtonVariant::Danger).into(),
        _ => button.into(),
    }
}

/// A dim (or toned) one-line paragraph.
fn line_spans(spans: Vec<Span>) -> Paragraph {
    Paragraph::new(Line::from(spans)).wrap(Wrap::Disabled(false))
}

fn flash_node(state: &PaneState) -> Option<Node> {
    state
        .flash
        .as_deref()
        .map(|f| line_spans(vec![Span::from(f).fg(Color::Warning)]).into())
}

/// List view body.
#[must_use]
pub fn list_body(tasks: &[Snapshot], state: &PaneState, now: i64) -> Node {
    let ordered = order_tasks(tasks);
    let mut children: Vec<Node> = Vec::new();
    if ordered.is_empty() {
        children.push(Empty::new().message("No background tasks.").into());
    } else {
        let selected = ordered
            .iter()
            .find(|t| t.status == Status::Running)
            .or_else(|| ordered.first())
            .map(|t| t.id.clone());
        for (title, running) in [("Running", true), ("Recent", false)] {
            let items: Vec<ListItem> = ordered
                .iter()
                .filter(|t| (t.status == Status::Running) == running)
                .map(|t| row(t, now))
                .collect();
            if items.is_empty() {
                continue;
            }
            let list = List::new(items).on_activate(format!("{TAB}:open")).state(
                selected
                    .clone()
                    .map_or_else(ListState::default, ListState::with_selected),
            );
            children.push(section(title, list));
        }
        children.push(
            Layout::horizontal([Constraint::Min(0), Constraint::Min(0)])
                .spacing(1)
                .children([
                    button("Stop all", "stopall", Some("warning")),
                    button("Dismiss finished", "dismissall", None),
                ])
                .into(),
        );
    }
    children.extend(flash_node(state));
    children.push(
        KeyHints::new([
            ("↑↓", "navigate"),
            ("↵", "view"),
            ("s", "stop"),
            ("x", "kill"),
            ("d", "dismiss"),
            ("R", "rerun"),
            ("c", "path"),
            ("←→", "tabs"),
            ("esc", "close"),
        ])
        .into(),
    );
    Layout::default().children(children).into()
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

fn settled_line(t: &Snapshot) -> Option<Node> {
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
    let row = |spans: Vec<Span>| -> Node { line_spans(spans).into() };
    match t.status {
        Status::Running => t
            .error
            .as_deref()
            .map(|e| row(vec![Span::from(e).fg(Color::Warning)])),
        Status::Completed => Some(row(vec![
            Span::from(format!("✓ Completed{}", with(&code))).fg(Color::Success),
        ])),
        Status::Killed => Some(row(vec![
            Span::from(format!("⊘ Stopped{}", with(&err))).fg(Color::Warning),
        ])),
        Status::Failed => Some(row(vec![
            Span::from(format!("✗ Failed{}{}", with(&code), with(&err))).fg(Color::Error),
        ])),
    }
}

/// Chip, `kind › name` and the elapsed · bytes line of the detail view.
fn header_row(t: &Snapshot, now: i64) -> Node {
    let chip_node: Node = if t.status == Status::Running {
        Spinner::new(crate::cards::chip(t.status)).into()
    } else {
        line_spans(vec![crate::cards::badge(
            crate::cards::chip(t.status),
            tone(t.status),
        )])
        .into()
    };
    Layout::horizontal([
        Constraint::Min(0),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .children([
        chip_node,
        line_spans(vec![
            Span::from(kind(t)).dim(),
            Span::from(" › ").dim(),
            Span::from(t.display_name()).bold(),
        ])
        .into(),
        Block::default()
            .borders(Borders::Sides(vec![BorderSide::TOP]))
            .border_style(Style::new().fg(Color::BorderMuted))
            .into(),
        line_spans(vec![
            Span::from(format!(
                "{} · {}",
                tray_elapsed(duration_of(t, now)),
                format_size(t.bytes_written)
            ))
            .dim(),
        ])
        .into(),
    ])
    .into()
}

/// The output block: read error, empty, or the (tailing) log behind a truncation note.
fn output_body(log: Option<&LogView>, full_log: bool) -> Node {
    match log {
        Some(LogView { error: Some(e), .. }) => {
            line_spans(vec![Span::from(e.as_str()).fg(Color::Error)]).into()
        }
        None => line_spans(vec![Span::from("reading…").dim()]).into(),
        Some(l) if l.lines.is_empty() => line_spans(vec![Span::from("no output yet").dim()]).into(),
        Some(l) => {
            let mut parts: Vec<Node> = Vec::new();
            if l.truncated {
                parts.push(
                    line_spans(vec![
                        Span::from(format!(
                            "… earlier output not shown ({} total{})",
                            format_size(l.total_bytes),
                            if full_log { "" } else { " · l full log" }
                        ))
                        .dim(),
                    ])
                    .into(),
                );
            }
            parts.push(
                Log::new(l.lines.iter().cloned())
                    .follow(true)
                    .max_lines(5000)
                    .into(),
            );
            Layout::default().children(parts).into()
        }
    }
}

/// The action row (back / stop / kill / … as buttons).
fn buttons_row(t: &Snapshot, full_log: bool) -> Node {
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
        if full_log { "Tail" } else { "Full log" },
        &format!("full:{id}"),
        None,
    ));
    buttons.push(button("Path", &format!("path:{id}"), None));
    let n = buttons.len();
    Layout::horizontal(vec![Constraint::Min(0); n])
        .spacing(1)
        .children(buttons)
        .into()
}

/// Detail view body.
#[must_use]
pub fn detail_body(t: &Snapshot, log: Option<&LogView>, state: &PaneState, now: i64) -> Node {
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
        header_row(t, now),
        line_spans(vec![Span::from(meta.join(" · ")).dim()]).into(),
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
    children.push(section("Command", Code::new(cmd).language("sh")));
    if let Some(d) = &t.description
        && compact_whitespace(d) != compact_whitespace(&t.display_name())
    {
        children.push(section("Description", Paragraph::new(d.as_str())));
    }
    children.push(line_spans(vec![Span::from(info_lines(t).join("\n")).dim()]).into());
    children.push(section("Output", output_body(log, state.full_log)));
    children.extend(settled_line(t));
    children.push(buttons_row(t, state.full_log));
    children.extend(flash_node(state));
    children.push(
        KeyHints::new([
            ("↑↓", "scroll"),
            ("g/G", "top/end"),
            ("l", "full log"),
            ("s", "stop"),
            ("x", "kill"),
            ("d", "dismiss"),
            ("R", "rerun"),
            ("c", "path"),
            ("←/esc", "back"),
        ])
        .into(),
    );
    Layout::default().children(children).into()
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
    use serde_json::json;

    /// A node as the wire JSON the tray carries.
    fn wire(node: Node) -> Value {
        serde_json::to_value(node).unwrap_or_default()
    }

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
        let body = wire(list_body(&[r.clone(), f], &PaneState::default(), 70_000));
        assert_eq!(body["children"][0]["title"], "Running");
        assert_eq!(body["children"][1]["title"], "Recent");
        assert_eq!(body["children"][0]["t"], "Block");
        assert_eq!(body["children"][0]["child"]["onActivate"], "bg:open");
        assert_eq!(body["children"][0]["child"]["state"]["selected"], "r");
        let empty = wire(list_body(&[], &PaneState::default(), 0));
        assert_eq!(empty["children"][0]["message"], "No background tasks.");
        let d = wire(detail_body(
            &r,
            Some(&LogView::from_read("x\ny\n", 4)),
            &PaneState::default(),
            0,
        ));
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
