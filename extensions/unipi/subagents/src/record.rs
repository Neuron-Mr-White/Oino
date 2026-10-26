//! Run records, their persisted index and the model-facing texts (unipi `manager.ts`,
//! `index.ts` result texts).

use serde::{Deserialize, Serialize};

use crate::child::{Report, ReportStatus, Usage};

/// unipi `MAX_CONCURRENT` (default; settings cap 16).
pub const MAX_CONCURRENT: usize = 8;
/// Hard cap of `maxConcurrent`.
pub const MAX_CONCURRENT_CAP: usize = 16;
/// unipi `MAX_TASK_CHARS`.
pub const MAX_TASK_CHARS: usize = 20_000;
/// unipi `CANCEL_GRACE_MS`.
pub const CANCEL_GRACE_MS: u32 = 5000;
/// Foreground `run_subagent` wait (45 min).
pub const FOREGROUND_WAIT_MS: i64 = 2_700_000;

/// unipi `SubagentStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Running.
    #[default]
    Running,
    /// Done.
    Completed,
    /// Failed.
    Failed,
    /// Cancelled.
    Cancelled,
}

impl Status {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// unipi `STATUS_LABEL`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    /// unipi `STATE_COLOR`.
    #[must_use]
    pub fn tone(self) -> &'static str {
        match self {
            Self::Running => "accent",
            Self::Completed => "success",
            Self::Failed => "error",
            Self::Cancelled => "warning",
        }
    }

    /// unipi `STATE_BADGE`.
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Running => "RUN ",
            Self::Completed => "DONE",
            Self::Failed => "FAIL",
            Self::Cancelled => "STOP",
        }
    }
}

/// Who cancelled a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CancelledBy {
    /// Tray `x` / Esc.
    User,
    /// Session shutdown.
    Session,
}

/// unipi `SubagentRecord` (persisted in `index.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// Agent id (8 hex).
    pub id: String,
    /// Title.
    pub title: String,
    /// Profile id.
    pub profile: String,
    /// `provider/id`.
    pub model: String,
    /// Thinking level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Status.
    pub status: Status,
    /// Background run.
    pub background: bool,
    /// Start (ms).
    pub started_at: i64,
    /// End (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// Tool calls.
    pub tool_calls: u32,
    /// Last activity (ms).
    pub last_activity: i64,
    /// Task of the latest run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Final report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// Error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Who cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_by: Option<CancelledBy>,
    /// Usage totals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Child session file (host path).
    pub session_file: String,
    /// Nesting depth.
    pub depth: u8,
}

/// unipi `recordStatusFor`: a user/session cancel wins over the error a forced kill produces.
#[must_use]
pub fn status_for(report: ReportStatus, cancelled_by: Option<CancelledBy>) -> Status {
    match report {
        ReportStatus::Completed => Status::Completed,
        ReportStatus::Aborted | ReportStatus::Interrupted => Status::Cancelled,
        ReportStatus::Error if cancelled_by.is_some() => Status::Cancelled,
        ReportStatus::Error => Status::Failed,
    }
}

/// Restored index: running entries become failed (unipi `restore`).
#[must_use]
pub fn restore(list: Vec<Record>) -> Vec<Record> {
    list.into_iter()
        .map(|r| {
            if r.status == Status::Running {
                Record {
                    status: Status::Failed,
                    error: Some("interrupted — Oino exited while it ran".into()),
                    ended_at: Some(r.last_activity),
                    ..r
                }
            } else {
                r
            }
        })
        .collect()
}

/// unipi `backgroundRunningReason`.
#[must_use]
pub fn background_running_reason(all: &[Record]) -> Option<String> {
    let n = all
        .iter()
        .filter(|r| r.background && r.status == Status::Running)
        .count();
    match n {
        0 => None,
        1 => Some("subagent working".into()),
        n => Some(format!("{n} subagents working")),
    }
}

/// Truncates the task to [`MAX_TASK_CHARS`].
#[must_use]
pub fn clip_task(task: &str) -> String {
    if task.chars().count() > MAX_TASK_CHARS {
        format!("{}…", task.chars().take(MAX_TASK_CHARS).collect::<String>())
    } else {
        task.to_owned()
    }
}

/// unipi admission errors (resume running, capacity, unknown resume id).
///
/// # Errors
///
/// unipi's refusal text.
pub fn admit(
    running: usize,
    max: usize,
    resume: Option<&str>,
    resume_running: bool,
    resume_known: bool,
) -> Result<(), String> {
    if let Some(id) = resume
        && resume_running
    {
        return Err(format!(
            "Subagent {id} is still running — use read_subagent to wait for it."
        ));
    }
    if running >= max {
        return Err(format!(
            "Maximum {max} concurrent subagents running — wait for one to finish (read_subagent) before starting another."
        ));
    }
    if let Some(id) = resume
        && !resume_known
    {
        return Err(format!("No subagent found for {id} — can't resume."));
    }
    Ok(())
}

/// unipi `trayPlural`.
#[must_use]
pub fn plural(n: u32, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// unipi `trayElapsed`.
#[must_use]
pub fn elapsed(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m{:02}s", s % 60);
    }
    format!("{}h{:02}m", m / 60, m % 60)
}

/// unipi `profileLabel`: `subagent_explore` → `Explore`.
#[must_use]
pub fn profile_label(profile: &str) -> String {
    let base = profile.strip_prefix("subagent_").unwrap_or(profile);
    let mut c = base.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// unipi `usageTail`: ` · 10k in · 0.2k out · $0.11` (empty when nothing).
#[must_use]
pub fn usage_tail(usage: Option<&Usage>) -> String {
    let Some(u) = usage else {
        return String::new();
    };
    let mut parts = Vec::new();
    if u.input > 0 || u.output > 0 {
        parts.push(format!("{} in", unipi_kit::fmt::format_tokens(u.input)));
        parts.push(format!("{} out", unipi_kit::fmt::format_tokens(u.output)));
    }
    if u.cost > 0.0 {
        parts.push(format!("${:.2}", u.cost));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

/// unipi `statLine`.
#[must_use]
pub fn stat_line(rec: &Record, tool_calls: u32, usage: Option<&Usage>, now: i64) -> String {
    format!(
        "{} · {} · {} · {}{}",
        profile_label(&rec.profile),
        rec.model,
        elapsed(rec.ended_at.unwrap_or(now) - rec.started_at),
        plural(tool_calls, "tool call"),
        usage_tail(usage)
    )
}

/// unipi `reportText` (the foreground/`read_subagent` result).
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "seconds display")]
pub fn report_text(rec: &Record, report: &Report, status: Status) -> String {
    let head = if status == Status::Cancelled {
        format!(
            "Subagent {} was cancelled{}. Partial output:\n\n",
            rec.id,
            if rec.cancelled_by == Some(CancelledBy::User) {
                " by the user"
            } else {
                ""
            }
        )
    } else {
        String::new()
    };
    format!(
        "{head}{}\n\n--- subagent {} · {} · {} · {:.1}s",
        report.text,
        rec.id,
        status.as_str(),
        plural(report.tool_calls, "tool call"),
        report.duration_ms as f64 / 1000.0
    )
}

/// The `<subagent_completion_notification>` content (unipi `deliverCompletion`).
#[must_use]
pub fn completion_content(rec: Option<&Record>, report: &Report, status: Status) -> String {
    let head = match status {
        Status::Cancelled => {
            if rec.and_then(|r| r.cancelled_by) == Some(CancelledBy::User) {
                "Cancelled by the user.".to_owned()
            } else {
                "Cancelled.".to_owned()
            }
        }
        Status::Failed => match &report.error {
            Some(e) => format!("Failed: {e}"),
            None => "Failed.".to_owned(),
        },
        _ => String::new(),
    };
    let body = [head.as_str(), report.text.as_str()]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "<subagent_completion_notification agent_id=\"{}\" status=\"{}\">\n{body}\n</subagent_completion_notification>",
        report.id,
        status.as_str()
    )
}

/// `read_subagent` text for a finished record.
#[must_use]
pub fn finished_read_text(rec: &Record) -> String {
    match &rec.report {
        Some(report) => format!(
            "{}{report}\n\n--- subagent {} · {} · {}",
            if rec.status == Status::Cancelled {
                format!(
                    "Cancelled{}. Partial output:\n\n",
                    if rec.cancelled_by == Some(CancelledBy::User) {
                        " by the user"
                    } else {
                        ""
                    }
                )
            } else {
                String::new()
            },
            rec.id,
            rec.status.as_str(),
            plural(rec.tool_calls, "tool call")
        ),
        None => format!(
            "Subagent {} {}{}",
            rec.id,
            rec.status.as_str(),
            rec.error
                .as_ref()
                .map(|e| format!(": {e}"))
                .unwrap_or_default()
        ),
    }
}

/// unipi `run_subagent` description.
pub const RUN_DESCRIPTION: &str = "Launch an independent subagent for a self-contained task. It has its own context and does not see this conversation, so put everything it needs in `task`. Foreground (default) waits and returns the subagent's report. With is_background:true it returns immediately and you receive a <subagent_completion_notification> when it finishes; use read_subagent to wait for it — never poll in a loop. Background subagents cannot ask for approval: tool calls that would need approval are denied. Use resume:<agent_id> to continue an earlier subagent with a follow-up task.";
/// unipi `read_subagent` description.
pub const READ_DESCRIPTION: &str = "Read a subagent's or the sidekick's result by agent_id (omit for the most recent). block:true waits for completion up to timeout seconds (default 30, max 600); block:false returns the current status immediately.";
/// unipi `WORKING` message.
pub const WORKING: &str = "Subagent running · Ctrl+B to run in background";

/// The system-prompt section (unipi `before_agent_start`).
#[must_use]
pub fn prompt_section(profiles: &[crate::profiles::Profile], fusion_active: bool) -> String {
    let mut out = vec![
        "You can delegate self-contained subtasks to subagents with `run_subagent`. A subagent is an independent agent with its own context: it does not see this conversation, so put everything it needs in `task` — the goal, relevant paths, constraints, and exactly what to report back.".to_owned(),
        String::new(),
        "Profiles:".to_owned(),
    ];
    out.extend(
        profiles
            .iter()
            .map(|p| format!("- {}: {}", p.id, p.description)),
    );
    out.extend([
        "subagent_explore is read-only and runs on a cheaper model; subagent_general can make changes and runs on your own model, so it costs like a full extra session.".to_owned(),
        String::new(),
        "- Use subagents for broad or parallelizable work: independent searches, investigations across several areas, or implementation slices that don't touch the same files. Skip them when a direct read or search answers the question.".to_owned(),
        "- Launch independent subagents in the same response so they run in parallel, and keep their work disjoint.".to_owned(),
        "- Foreground (the default) waits and returns the report. Set is_background:true only when you have other work to do meanwhile; you will receive a <subagent_completion_notification> when it finishes. Don't poll read_subagent in a loop.".to_owned(),
        "- Background subagents cannot ask for approval: tool calls that would need approval are denied. Run approval-needing work in the foreground.".to_owned(),
        "- The user can move a foreground subagent to the background (Ctrl+B) or cancel any subagent. When that happens, continue accordingly; don't restart a cancelled subagent unless asked.".to_owned(),
        "- Resume a finished, failed, or cancelled subagent with resume:<agent_id> and a follow-up task; it keeps its context.".to_owned(),
        "- The user does not see subagent output directly: summarize what matters from the report, and verify anything critical before relying on it.".to_owned(),
    ]);
    if fusion_active {
        out.push("While Fusion is active, do not use subagents other than the sidekick unless the user explicitly asks you to.".into());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(status: Status, background: bool) -> Record {
        Record {
            id: "abcd1234".into(),
            title: "Map auth".into(),
            profile: "subagent_explore".into(),
            model: "ds/flash".into(),
            status,
            background,
            started_at: 0,
            ..Record::default()
        }
    }

    fn report(status: ReportStatus, text: &str) -> Report {
        Report {
            id: "abcd1234".into(),
            status,
            text: text.into(),
            usage: Usage::default(),
            tool_calls: 2,
            duration_ms: 7300,
            events: vec![],
            error: None,
        }
    }

    #[test]
    fn statuses_and_reasons() {
        assert_eq!(
            status_for(ReportStatus::Error, Some(CancelledBy::User)),
            Status::Cancelled
        );
        assert_eq!(status_for(ReportStatus::Error, None), Status::Failed);
        assert_eq!(
            status_for(ReportStatus::Interrupted, None),
            Status::Cancelled
        );
        assert_eq!(background_running_reason(&[]), None);
        assert_eq!(
            background_running_reason(&[rec(Status::Running, true)]).as_deref(),
            Some("subagent working")
        );
        assert_eq!(
            background_running_reason(&[
                rec(Status::Running, true),
                rec(Status::Running, true),
                rec(Status::Running, false)
            ])
            .as_deref(),
            Some("2 subagents working")
        );
        let r = restore(vec![rec(Status::Running, false)]);
        assert_eq!(r[0].status, Status::Failed);
        assert!(
            r[0].error
                .as_deref()
                .is_some_and(|e| e.starts_with("interrupted"))
        );
    }

    #[test]
    fn admission() {
        assert!(
            admit(8, 8, None, false, false)
                .unwrap_err()
                .starts_with("Maximum 8 concurrent")
        );
        assert!(
            admit(0, 8, Some("x"), true, true)
                .unwrap_err()
                .contains("still running")
        );
        assert_eq!(
            admit(0, 8, Some("x"), false, false).unwrap_err(),
            "No subagent found for x — can't resume."
        );
        assert!(admit(1, 8, None, false, false).is_ok());
    }

    #[test]
    fn texts() {
        assert_eq!(profile_label("subagent_explore"), "Explore");
        assert_eq!(profile_label("reviewer"), "Reviewer");
        assert_eq!(elapsed(169_000), "2m49s");
        let u = Usage {
            input: 10_000,
            output: 200,
            cost: 0.111,
            ..Usage::default()
        };
        assert_eq!(usage_tail(Some(&u)), " · 10k in · 200 out · $0.11");
        assert_eq!(usage_tail(None), "");
        let r = rec(Status::Completed, false);
        assert_eq!(
            stat_line(&r, 1, None, 32_000),
            "Explore · ds/flash · 32s · 1 tool call"
        );
        assert_eq!(
            report_text(
                &r,
                &report(ReportStatus::Completed, "Done."),
                Status::Completed
            ),
            "Done.\n\n--- subagent abcd1234 · completed · 2 tool calls · 7.3s"
        );
        let mut c = rec(Status::Cancelled, true);
        c.cancelled_by = Some(CancelledBy::User);
        assert!(
            report_text(&c, &report(ReportStatus::Aborted, "x"), Status::Cancelled)
                .starts_with("Subagent abcd1234 was cancelled by the user. Partial output:")
        );
        assert_eq!(
            completion_content(
                Some(&c),
                &report(ReportStatus::Aborted, "partial"),
                Status::Cancelled
            ),
            "<subagent_completion_notification agent_id=\"abcd1234\" status=\"cancelled\">\nCancelled by the user.\n\npartial\n</subagent_completion_notification>"
        );
        let mut f = report(ReportStatus::Error, "");
        f.error = Some("boom".into());
        assert!(completion_content(None, &f, Status::Failed).contains("\nFailed: boom\n"));
        let mut done = rec(Status::Failed, false);
        done.error = Some("x".into());
        assert_eq!(finished_read_text(&done), "Subagent abcd1234 failed: x");
        assert!(
            prompt_section(&crate::profiles::builtins(), true)
                .ends_with("unless the user explicitly asks you to.")
        );
        assert_eq!(clip_task(&"a".repeat(20_001)).chars().count(), 20_001);
    }
}
