//! Chat cards as v2 nodes (ui-protocol §12.1; unipi `cards.ts`): a four-letter state chip, the
//! bold task name and a dim right side; expanded adds the command, error, output tail, output
//! path and id.

use oino_extension::ui::widgets::prelude::*;
use serde_json::Value;

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

/// The `bg` sibling of a tone token (`<tone>Bg`, §5); `None` when the token has none.
fn bg(tone: &Color) -> Option<Color> {
    Some(match tone {
        Color::Accent => Color::AccentBg,
        Color::Info => Color::InfoBg,
        Color::Success => Color::SuccessBg,
        Color::Warning => Color::WarningBg,
        Color::Error => Color::ErrorBg,
        _ => return None,
    })
}

/// A badge span: `{content, style:{fg:tone, bg:<tone>Bg}}` (§12.1 `badge` → `Span`).
fn badge(text: &str, tone: &str) -> Span {
    let color = Color::parse(tone);
    match bg(&color) {
        Some(fill) => Span::from_string(text).fg(color).bg(fill),
        None => Span::from_string(text).fg(color),
    }
}

/// A nowrap paragraph from spans (`wrap:false`).
fn text(spans: Vec<Span>) -> Paragraph {
    Paragraph::new(Line::from(spans)).wrap(Wrap::Disabled(false))
}

/// The rule between the name and the right side (`divider`, §12.1: a top-border `Block`).
fn divider() -> Block {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
}

fn head(chip_text: &str, chip_tone: &str, name: &str, right: &str) -> Node {
    let children: Vec<Node> = vec![
        text(vec![badge(chip_text, chip_tone)]).into(),
        text(vec![Span::from(name).bold()]).into(),
        divider().into(),
        text(vec![Span::from(right).dim()]).into(),
    ];
    Layout::horizontal([
        Constraint::Min(0),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .children(children)
    .into()
}

fn expanded_lines(task: &Snapshot) -> Vec<Node> {
    let cmd = if task.command.chars().count() > 120 {
        format!("{}…", task.command.chars().take(120).collect::<String>())
    } else {
        task.command.clone()
    };
    let mut out: Vec<Node> = vec![text(vec![Span::from(format!("  $ {cmd}")).dim()]).into()];
    if let Some(e) = task.error.as_deref().filter(|e| !e.is_empty()) {
        out.push(text(vec![Span::from(format!("  {e}")).fg(Color::Error)]).into());
    }
    for line in task.output_tail.iter().flatten() {
        out.push(text(vec![Span::from(format!("  {line}")).fg(Color::Tool)]).into());
    }
    out.push(text(vec![Span::from(format!("  {}", task.output_path)).dim()]).into());
    out.push(text(vec![Span::from(format!("  ({})", task.id)).dim()]).into());
    out
}

fn card(head_node: Node, task: &Snapshot, expanded: bool) -> Node {
    if !expanded {
        return head_node;
    }
    let mut children = vec![head_node];
    children.extend(expanded_lines(task));
    Layout::default().children(children).into()
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
    node(card(h, task, expanded))
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
        return node(head("DONE", tone(Status::Completed), "bg task", ""));
    };
    let h = head(
        chip(task.status),
        tone(task.status),
        &task.display_name(),
        &completion_meta(task),
    );
    node(card(h, task, expanded))
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
        Span::from(task.display_name()).fg(Color::Accent),
        Span::from(format!(" ({})", task.id)).dim(),
        Span::from(format!(
            " {} {bytes_read}",
            if tail { "tail" } else { "head" }
        ))
        .fg(Color::Muted),
    ];
    if truncated {
        spans.push(Span::from(" (truncated)").fg(Color::Warning));
    }
    let mut children: Vec<Node> = vec![
        text(spans).into(),
        text(vec![Span::from(format!("Full output: {path}")).dim()]).into(),
    ];
    if let Some(out) = output {
        children.push(Log::new(out.split('\n').take(30)).into());
    }
    node(Layout::default().children(children))
}

/// `bg_kill` result card.
#[must_use]
pub fn kill_card(task: &Snapshot) -> Value {
    let line = text(vec![
        Span::from("■ killed").fg(Color::Warning),
        Span::from(" "),
        Span::from(task.display_name()).fg(Color::Accent),
        Span::from(format!(" ({})", task.id)).dim(),
    ]);
    let path = text(vec![
        Span::from(format!("Output: {}", task.output_path)).dim(),
    ]);
    node(Layout::default().children([line.into(), path.into()]))
}

/// A built node as the wire JSON the renderer callback returns.
fn node(node: impl Into<Node>) -> Value {
    serde_json::to_value(node.into()).unwrap_or(Value::Null)
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
        assert_eq!(c["children"][0]["text"][0][0]["content"], "BG  ");
        assert_eq!(c["children"][0]["text"][0][0]["style"]["bg"], "accentBg");

        assert_eq!(
            c["children"][3]["text"][0][0]["content"],
            "started · wakes agent"
        );
        let e = launch_card(&task(Status::Running), true);
        assert_eq!(e["children"][1]["text"][0][0]["content"], "  $ npm test");
        assert_eq!(e["children"][2]["text"][0][0]["content"], "  ok");
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
        assert_eq!(completion_card(None, false)["children"][2]["t"], "Block");
        assert_eq!(
            completion_card(Some(&f), false)["children"][0]["text"][0][0]["content"],
            "FAIL"
        );
    }
}
