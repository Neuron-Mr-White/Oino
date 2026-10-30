//! The work tray's Dream tab (unipi `src/tray-pane.ts`) as v2 nodes: a run list, a run detail
//! (report summary, proposals 1–9, live trajectory, log tail) and the full report view.
//! Actions are `dream:<verb>` (the tray strips the prefix before forwarding).

use oino_extension::ui::widgets::prelude::*;

use crate::store::{Run, Status, Step, summarize_report};

/// List hint (unipi `DREAM_LIST_HINT`).
pub const LIST_HINT: &str = "↑↓ navigate · ↵ view · o report · s stop · r run now · a/x approve/reject · d dismiss · ←→ tabs · esc close";
/// Detail hint (unipi `DREAM_VIEW_HINT`).
pub const VIEW_HINT: &str = "↑↓ scroll · 1-9 pick · a/x approve/reject · o report · s stop · r run now · d dismiss · ←/esc back";

/// Per-session tab state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// Run shown in detail.
    pub open: Option<String>,
    /// Full report instead of the detail body.
    pub report: bool,
    /// Picked proposal index.
    pub pick: usize,
    /// First press of `a`/`x` on a proposal (`(verb, proposal id)`).
    pub armed: Option<(String, String)>,
    /// One-shot message.
    pub flash: Option<String>,
}

/// `14:02` (UTC; the host exposes no local zone).
#[must_use]
pub fn clock(ms: i64) -> String {
    let iso = unipi_kit::fmt::iso_from_ms(ms);
    iso.get(11..16).unwrap_or_default().to_owned()
}

fn day(ms: i64, now: i64) -> String {
    let (a, b) = (
        unipi_kit::fmt::iso_from_ms(ms),
        unipi_kit::fmt::iso_from_ms(now),
    );
    if a.get(..10) == b.get(..10) {
        clock(ms)
    } else {
        format!("{} {}", a.get(5..10).unwrap_or_default(), clock(ms))
    }
}

/// `1m10s` style elapsed.
#[must_use]
pub fn elapsed(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

fn plural(n: usize, w: &str) -> String {
    format!("{n} {w}{}", if n == 1 { "" } else { "s" })
}

/// Row tags (unipi `runTags`).
#[must_use]
pub fn run_tags(run: &Run, now: i64) -> Vec<String> {
    let mut tags = vec![
        elapsed(run.ended_at.unwrap_or(now) - run.started_at),
        plural(run.sessions as usize, "session"),
    ];
    if run.pending > 0 {
        tags.push(format!("{} pending", run.pending));
    } else if !run.proposals.is_empty() {
        tags.push(plural(run.proposals.len(), "proposal"));
    }
    if let Some(e) = &run.error {
        tags.push(e.clone());
    }
    tags
}

fn tone(s: Status) -> &'static str {
    match s {
        Status::Running => "accent",
        Status::Finished => "success",
        Status::Stopped => "warning",
        Status::Failed => "error",
    }
}

fn glyph(s: Status) -> &'static str {
    match s {
        Status::Running => "⠋",
        Status::Finished => "✓",
        Status::Stopped => "⊘",
        Status::Failed => "✗",
    }
}

/// Colour token of a run status.
fn color_of(s: Status) -> Color {
    Color::parse(tone(s))
}

fn button(label: &str, action: &str) -> Node {
    Button::new(label, format!("dream:{action}")).into()
}

/// A row of buttons.
fn button_row(buttons: Vec<Node>) -> Node {
    Layout::horizontal(vec![Constraint::Min(0); buttons.len()])
        .spacing(2)
        .children(buttons)
        .into()
}

/// One unwrapped text line.
fn text_line(span: Span) -> Node {
    Paragraph::new(Line::from(span))
        .wrap(Wrap::Disabled(false))
        .into()
}

/// A section: a top rule with the title above its body (§12.1 `section`).
fn section(title: &str, body: Node) -> Node {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
        .title(title)
        .child(body)
        .into()
}

fn flash(view: &View) -> Option<Node> {
    view.flash
        .as_ref()
        .map(|f| text_line(Span::from_string(f.clone()).fg(Color::Warning)))
}

/// The list body.
#[must_use]
pub fn list_body(runs: &[Run], enabled: bool, view: &View, now: i64) -> Node {
    let mut children = Vec::new();
    if runs.is_empty() {
        children.push(
            Empty::new()
                .message(if enabled {
                    "No dreams yet — one starts on its own when enough new sessions pile up. r runs one now."
                } else {
                    "Background dreaming is off (turn it on in /settings → Dream). r runs one now."
                })
                .into(),
        );
    } else {
        let items: Vec<ListItem> = runs
            .iter()
            .map(|r| {
                ListItem::new(Line::from(vec![
                    Span::from_string(format!("{} ", glyph(r.status))).fg(color_of(r.status)),
                    Span::from("Dream ").fg(Color::Muted),
                    Span::from_string(day(r.started_at, now)).bold(),
                    Span::from_string(format!(
                        "  {}",
                        if r.manual { "manual" } else { "scheduled" }
                    ))
                    .dim(),
                ]))
                .key(r.id.clone())
                .detail(Line::from(run_tags(r, now).join(" · ")))
                .badge(Line::from(r.status.as_str()).fg(color_of(r.status)))
            })
            .collect();
        children.push(List::new(items).on_activate("dream:open").into());
    }
    if let Some(f) = flash(view) {
        children.push(f);
    }
    children.push(button_row(vec![button("r Run now", "run")]));
    children.push(KeyHints::new([("", LIST_HINT)]).into());
    Layout::default().spacing(1).children(children).into()
}

fn step_line(s: &Step) -> Node {
    let (g, t) = match s.kind.as_str() {
        "tool" => ("▸", "tool"),
        "error" => ("✗", "error"),
        "text" => ("·", "default"),
        _ => ("›", "muted"),
    };
    text_line(Span::from_string(format!("  {g} {}", s.text)).fg(Color::parse(t)))
}

/// The detail body (or the full report when `view.report`).
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "report, proposals, trajectory, log, actions"
)]
pub fn detail_body(
    run: &Run,
    report: &str,
    steps: &[Step],
    log: &[String],
    view: &View,
    now: i64,
) -> Node {
    let mut meta = vec![run.id.clone()];
    meta.push(if run.status == Status::Running {
        "running · never wakes the agent".into()
    } else {
        run.status.as_str().into()
    });
    meta.push(if run.manual { "manual" } else { "scheduled" }.into());
    if view.report {
        meta.push("report".into());
    }
    let header =
        text_line(Span::from_string(format!("{} ", glyph(run.status))).fg(color_of(run.status)));
    let mut children = vec![
        Paragraph::new(Line::from(vec![
            Span::from_string(format!("{} ", glyph(run.status))).fg(color_of(run.status)),
            Span::from("Dream › ").fg(Color::Muted),
            Span::from_string(day(run.started_at, now)).bold(),
            Span::from_string(format!(
                "   {} · {}",
                elapsed(run.ended_at.unwrap_or(now) - run.started_at),
                plural(run.sessions as usize, "session")
            ))
            .dim(),
        ]))
        .into(),
        text_line(Span::from_string(meta.join(" · ")).dim()),
    ];
    let _ = header;
    if view.report {
        children.push(if report.trim().is_empty() {
            Empty::new()
                .message(if run.status == Status::Running {
                    "The report is written at the end of the dream."
                } else {
                    "This dream left no report."
                })
                .into()
        } else {
            Markdown::new(report).into()
        });
    } else {
        let summary = if run.has_report {
            let s = summarize_report(report);
            if s.is_empty() {
                "ready — o to read it".to_owned()
            } else {
                s.join(" · ")
            }
        } else if run.status == Status::Running {
            "not yet — written at the end".to_owned()
        } else {
            "none".to_owned()
        };
        children.push(section("Report", Paragraph::new(summary).into()));
        let proposals: Node = if run.proposals.is_empty() {
            Empty::new()
                .message(if run.status == Status::Running {
                    "none yet"
                } else {
                    "none — memory edits apply on their own"
                })
                .into()
        } else {
            let items: Vec<ListItem> = run
                .proposals
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let tone = match p.decision.as_str() {
                        "approved" => Color::Success,
                        "rejected" => Color::Muted,
                        _ => Color::Warning,
                    };
                    ListItem::new(format!("{}. [{}] {}", i + 1, p.kind, p.name))
                        .key(p.id.clone())
                        .badge(Line::from(p.decision.clone()).fg(tone))
                })
                .collect();
            let list = List::new(items).on_activate("dream:pick");
            match run.proposals.get(view.pick) {
                Some(p) => list.state(ListState::with_selected(p.id.clone())).into(),
                None => list.into(),
            }
        };
        let title = if run.proposals.is_empty() {
            "Proposals".to_owned()
        } else {
            format!("Proposals ({} pending)", run.pending)
        };
        children.push(section(&title, proposals));
        let traj: Vec<Node> = if steps.is_empty() {
            vec![text_line(
                Span::from_string(if run.status == Status::Running {
                    "  starting…"
                } else {
                    "  no steps recorded"
                })
                .dim(),
            )]
        } else {
            steps.iter().map(step_line).collect()
        };
        children.push(section(
            "Trajectory",
            Layout::default().children(traj).into(),
        ));
        if !log.is_empty() {
            let tail = &log[log.len().saturating_sub(20)..];
            children.push(section(
                "Log",
                Log::new(tail.iter().cloned()).follow(true).into(),
            ));
        }
        if run.status != Status::Running {
            children.push(match run.status {
                Status::Finished => text_line(Span::from("✓ Finished").fg(Color::Success)),
                Status::Stopped => text_line(Span::from("⊘ Stopped").fg(Color::Warning)),
                _ => text_line(
                    Span::from_string(format!(
                        "✗ Failed{}",
                        run.error
                            .as_ref()
                            .map(|e| format!(" · {e}"))
                            .unwrap_or_default()
                    ))
                    .fg(Color::Error),
                ),
            });
        }
    }
    if let Some(f) = flash(view) {
        children.push(f);
    }
    let mut buttons = vec![button("← Back", "back")];
    buttons.push(button(
        if view.report { "Details" } else { "o Report" },
        "report",
    ));
    if run.status == Status::Running {
        buttons.push(button("s Stop", "stop"));
    } else {
        buttons.push(button("r Run now", "run"));
        buttons.push(button("d Dismiss", "dismiss"));
    }
    if run
        .proposals
        .get(view.pick)
        .is_some_and(|p| p.decision == "pending")
    {
        buttons.push(button("a Approve", "approve"));
        buttons.push(button("x Reject", "reject"));
    }
    children.push(button_row(buttons));
    children.push(KeyHints::new([("", VIEW_HINT)]).into());
    Layout::default().spacing(1).children(children).into()
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn run(status: Status) -> Run {
        Run {
            id: "staging-1".into(),
            staging: PathBuf::from("/x"),
            status,
            started_at: 0,
            ended_at: Some(70_000),
            manual: true,
            sessions: 6,
            events: 3,
            has_report: false,
            proposals: Vec::new(),
            pending: 0,
            error: None,
            interrupted: false,
        }
    }

    #[test]
    fn rendering() {
        assert_eq!(elapsed(70_000), "1m10s");
        assert_eq!(elapsed(5_000), "5s");
        assert_eq!(clock(0), "00:00");
        assert_eq!(run_tags(&run(Status::Finished), 0), ["1m10s", "6 sessions"]);
        let body =
            serde_json::to_string(&list_body(&[], false, &View::default(), 0)).unwrap_or_default();
        assert!(body.contains("Background dreaming is off"));
        let body: serde_json::Value =
            serde_json::to_value(list_body(&[run(Status::Failed)], true, &View::default(), 0))
                .unwrap_or_default();
        assert_eq!(body["children"][0]["onActivate"], "dream:open");
        let d = serde_json::to_string(&detail_body(
            &run(Status::Running),
            "",
            &[],
            &[],
            &View::default(),
            80_000,
        ))
        .unwrap_or_default();
        assert!(d.contains("never wakes the agent") && d.contains("dream:stop"));
        let v = View {
            report: true,
            ..View::default()
        };
        assert!(
            serde_json::to_string(&detail_body(&run(Status::Failed), "", &[], &[], &v, 0))
                .unwrap_or_default()
                .contains("This dream left no report.")
        );
    }
}
