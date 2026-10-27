//! The work tray's Dream tab (unipi `src/tray-pane.ts`) as OUS: a run list, a run detail
//! (report summary, proposals 1–9, live trajectory, log tail) and the full report view.
//! Actions are `dream:<verb>` (the tray strips the prefix before forwarding).

use serde_json::{Value, json};

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

fn button(label: &str, action: &str) -> Value {
    json!({"t": "button", "label": label, "actionId": format!("dream:{action}")})
}

fn flash(view: &View) -> Option<Value> {
    view.flash
        .as_ref()
        .map(|f| json!({"t": "text", "value": [{"text": f, "tone": "warning"}]}))
}

/// The list body.
#[must_use]
pub fn list_body(runs: &[Run], enabled: bool, view: &View, now: i64) -> Value {
    let mut children = Vec::new();
    if runs.is_empty() {
        children.push(json!({"t": "empty", "message": if enabled {
            "No dreams yet — one starts on its own when enough new sessions pile up. r runs one now."
        } else {
            "Background dreaming is off (turn it on in /settings → Dream). r runs one now."
        }}));
    } else {
        let items: Vec<Value> = runs
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "label": [
                        {"text": format!("{} ", glyph(r.status)), "tone": tone(r.status)},
                        {"text": "Dream ", "tone": "muted"},
                        {"text": day(r.started_at, now), "bold": true},
                        {"text": format!("  {}", if r.manual { "manual" } else { "scheduled" }), "dim": true},
                    ],
                    "detail": run_tags(r, now).join(" · "),
                    "badge": r.status.as_str(),
                    "tone": tone(r.status),
                })
            })
            .collect();
        children.push(json!({"t": "list", "items": items, "selectable": "dream:open"}));
    }
    if let Some(f) = flash(view) {
        children.push(f);
    }
    children.push(json!({"t": "stack", "direction": "horizontal", "gap": 2, "children": [button("r Run now", "run")]}));
    children.push(json!({"t": "keyHints", "hints": [{"keys": "", "label": LIST_HINT}]}));
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": children})
}

fn step_line(s: &Step) -> Value {
    let (g, t) = match s.kind.as_str() {
        "tool" => ("▸", "tool"),
        "error" => ("✗", "error"),
        "text" => ("·", "default"),
        _ => ("›", "muted"),
    };
    json!({"t": "text", "wrap": false, "value": [{"text": format!("  {g} {}", s.text), "tone": t}]})
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
) -> Value {
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
    let header = json!({"t": "text", "value": [
        {"text": format!("{} ", glyph(run.status)), "tone": tone(run.status)},
        {"text": "Dream › ", "tone": "muted"},
        {"text": day(run.started_at, now), "bold": true},
        {"text": format!("   {} · {}", elapsed(run.ended_at.unwrap_or(now) - run.started_at), plural(run.sessions as usize, "session")), "dim": true},
    ]});
    let mut children = vec![
        header,
        json!({"t": "text", "value": [{"text": meta.join(" · "), "dim": true}]}),
    ];
    if view.report {
        children.push(if report.trim().is_empty() {
            json!({"t": "empty", "message": if run.status == Status::Running { "The report is written at the end of the dream." } else { "This dream left no report." }})
        } else {
            json!({"t": "markdown", "value": report})
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
        children.push(
            json!({"t": "section", "title": "Report", "body": {"t": "text", "value": summary}}),
        );
        let proposals = if run.proposals.is_empty() {
            json!({"t": "empty", "message": if run.status == Status::Running { "none yet" } else { "none — memory edits apply on their own" }})
        } else {
            json!({"t": "list", "selectable": "dream:pick", "selected": run.proposals.get(view.pick).map(|p| p.id.clone()),
                "items": run.proposals.iter().enumerate().map(|(i, p)| json!({
                    "id": p.id,
                    "label": format!("{}. [{}] {}", i + 1, p.kind, p.name),
                    "badge": p.decision,
                    "tone": match p.decision.as_str() { "approved" => "success", "rejected" => "muted", _ => "warning" },
                })).collect::<Vec<_>>()})
        };
        let title = if run.proposals.is_empty() {
            "Proposals".to_owned()
        } else {
            format!("Proposals ({} pending)", run.pending)
        };
        children.push(json!({"t": "section", "title": title, "body": proposals}));
        let traj: Vec<Value> = if steps.is_empty() {
            vec![
                json!({"t": "text", "value": [{"text": if run.status == Status::Running { "  starting…" } else { "  no steps recorded" }, "dim": true}]}),
            ]
        } else {
            steps.iter().map(step_line).collect()
        };
        children.push(json!({"t": "section", "title": "Trajectory", "body": {"t": "stack", "direction": "vertical", "children": traj}}));
        if !log.is_empty() {
            let tail = &log[log.len().saturating_sub(20)..];
            children.push(json!({"t": "section", "title": "Log", "body": {"t": "log", "lines": tail, "tail": true}}));
        }
        if run.status != Status::Running {
            children.push(match run.status {
                Status::Finished => json!({"t": "text", "value": [{"text": "✓ Finished", "tone": "success"}]}),
                Status::Stopped => json!({"t": "text", "value": [{"text": "⊘ Stopped", "tone": "warning"}]}),
                _ => json!({"t": "text", "value": [{"text": format!("✗ Failed{}", run.error.as_ref().map(|e| format!(" · {e}")).unwrap_or_default()), "tone": "error"}]}),
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
    children.push(json!({"t": "stack", "direction": "horizontal", "gap": 2, "children": buttons}));
    children.push(json!({"t": "keyHints", "hints": [{"keys": "", "label": VIEW_HINT}]}));
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": children})
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
        let body = list_body(&[], false, &View::default(), 0).to_string();
        assert!(body.contains("Background dreaming is off"));
        let body = list_body(&[run(Status::Failed)], true, &View::default(), 0);
        assert_eq!(body["children"][0]["selectable"], "dream:open");
        let d = detail_body(
            &run(Status::Running),
            "",
            &[],
            &[],
            &View::default(),
            80_000,
        )
        .to_string();
        assert!(d.contains("never wakes the agent") && d.contains("dream:stop"));
        let v = View {
            report: true,
            ..View::default()
        };
        assert!(
            detail_body(&run(Status::Failed), "", &[], &[], &v, 0)
                .to_string()
                .contains("This dream left no report.")
        );
    }
}
