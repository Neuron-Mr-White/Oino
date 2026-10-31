//! Job, run and settings types for the cron scheduler.

use serde::{Deserialize, Serialize};

/// Default `time_limit_s` for a job that doesn't set one.
pub const DEFAULT_TIME_LIMIT_S: u64 = 1800;

/// What happens when a job fires while its previous run is still going.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlapPolicy {
    /// Record the fire as `skipped` ("previous run still going") — the default.
    #[default]
    Skip,
    /// Start it anyway (subject to the global concurrency limit).
    Allow,
}

impl OverlapPolicy {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Allow => "allow",
        }
    }
}

/// What happens to fires missed while no scheduler was running (host asleep, daemon down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MissedPolicy {
    /// Run the single latest missed fire if it is still inside the missed window — the
    /// default. Never catch up on every missed occurrence.
    #[default]
    RunLatest,
    /// Drop missed fires silently.
    Skip,
}

impl MissedPolicy {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunLatest => "run-latest",
            Self::Skip => "skip",
        }
    }
}

/// A job's notification settings. Absent = the user's enabled notify channels,
/// events `["failed"]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifySpec {
    /// `"ntfy"` / `"gotify"` / `"telegram"`. Absent = whatever the user's notify
    /// config has enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    /// Run statuses to notify on (`"done"` / `"failed"`, `"skipped"`). Absent =
    /// `["failed"]` (or the default from settings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
}

/// A scheduled job (`<agent dir>/cron/jobs/<id>.json`, schema `"v": 2`, camelCase —
/// ext-abi §6.2; the legacy `snake_case` keys stay readable for the `jobs.json` migration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// A short slug, unique among jobs (must equal the file stem).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The prompt every run sends.
    pub prompt: String,
    /// A 6-field cron (seconds first) or `at:<RFC3339>`.
    pub schedule: String,
    /// An IANA zone name; defaults to the system zone at creation time.
    pub tz: String,
    /// The job's working directory. Must exist.
    pub cwd: String,
    /// Model override (`provider/id`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Thinking level override, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Enabled jobs fire; disabled ones don't.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-job wall-clock limit for one run.
    #[serde(default = "default_time_limit", alias = "time_limit_s")]
    pub time_limit_s: u64,
    /// A run that costs more than this is recorded `failed` (`None` = no cap).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "cost_cap_usd"
    )]
    pub cost_cap_usd: Option<f64>,
    /// What to do when a fire overlaps the previous run.
    #[serde(default)]
    pub overlap: OverlapPolicy,
    /// What to do with missed fires.
    #[serde(default)]
    pub missed: MissedPolicy,
    /// Per-job notification settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<NotifySpec>,
    /// The chat a finished run wakes (host.md §9.8): the host opens this session in its hub
    /// (takeover semantics) and prompts [`wake_text`]. Absent = no wake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_link: Option<SessionLink>,
    /// Unix ms.
    #[serde(alias = "created")]
    pub created: u64,
    /// Unix ms.
    #[serde(alias = "updated")]
    pub updated: u64,
}

impl Job {
    /// A job with the given identity and defaults for everything else (tests, builders).
    #[must_use]
    pub fn new(id: &str, name: &str, prompt: &str, schedule: &str, tz: &str, cwd: &str) -> Self {
        let now = 0;
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            prompt: prompt.to_owned(),
            schedule: schedule.to_owned(),
            tz: tz.to_owned(),
            cwd: cwd.to_owned(),
            model: None,
            thinking: None,
            enabled: true,
            time_limit_s: DEFAULT_TIME_LIMIT_S,
            cost_cap_usd: None,
            overlap: OverlapPolicy::default(),
            missed: MissedPolicy::default(),
            notify: None,
            session_link: None,
            created: now,
            updated: now,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_time_limit() -> u64 {
    DEFAULT_TIME_LIMIT_S
}

/// The chat a finished run wakes (host.md §9.8; `wake_session`/`wake_here` on the CLI): the
/// session file (most precise) or its id, resolved by the host's session hub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLink {
    /// The session file (`~/.oino/sessions/…/<ts>_<id>.jsonl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// The session id (when no file was given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl SessionLink {
    /// The hub target: the file when one was recorded, else the id.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.session_file
            .as_deref()
            .filter(|v| !v.is_empty())
            .or(self.session_id.as_deref().filter(|v| !v.is_empty()))
    }
}

/// The wake prompt a finished run sends to its linked chat (host.md §9.8): the job, its
/// outcome and the run id, in one short message.
#[must_use]
pub fn wake_text(job: &Job, run: &Run) -> String {
    let summary = run
        .summary
        .as_deref()
        .or(run.reason.as_deref())
        .unwrap_or("(no summary)");
    let clipped: String = summary.chars().take(300).collect();
    format!(
        "[cron] {} finished: {}. {}. Run {} — details: unipi-host cron log {}",
        job.name,
        run.status.as_str(),
        clipped,
        run.id,
        run.id,
    )
}

/// What a run is doing or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Running right now.
    Running,
    /// Finished successfully.
    Ok,
    /// Failed (non-zero exit, spawn error, cost cap).
    Failed,
    /// Killed at its time limit.
    Timeout,
    /// Not run (overlap policy, or validation).
    Skipped,
    /// Waiting for a concurrency slot.
    Queued,
}

impl RunStatus {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Skipped => "skipped",
            Self::Queued => "queued",
        }
    }

    /// Whether this status ends a run (not `running`/`queued`).
    #[must_use]
    pub fn is_finished(self) -> bool {
        !matches!(self, Self::Running | Self::Queued)
    }
}

/// One run record (`<agent dir>/cron/runs/<job id>/<run id>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Run id (also the run's Oino session id).
    pub id: String,
    /// Owning job id.
    pub job_id: String,
    /// Current status.
    pub status: RunStatus,
    /// Unix ms when this fire was recorded (the scheduled time, or now for `run`).
    pub fired_at: u64,
    /// Unix ms when execution started (`None` while queued).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Unix ms when the run ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// The child's exit code, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// The assistant's final summary text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Why a run was skipped or failed before execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The run's Oino session file, when one was written (browse it with `oino --session`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_path: Option<String>,
    /// The run's reported cost in USD, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Whether this fire was a catch-up ("missed") run.
    #[serde(default)]
    pub missed: bool,
    /// The child's pid while it runs (adoption / liveness checks after a restart).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Per-channel notification failures; never fails the run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notify_errors: Vec<String>,
    /// What happened waking the job's linked session (`"sent"`, `"session not found: …"`,
    /// …); absent when the job links none (host.md §9.8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<String>,
}

/// The scheduler's own settings (`<agent dir>/cron/settings.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronSettings {
    /// How many runs may execute at once (min 1).
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// Seconds after its scheduled time within which a missed fire still runs late.
    #[serde(default = "default_missed_window")]
    pub missed_window_s: u64,
    /// Past runs kept per job (newest first).
    #[serde(default = "default_retention_runs")]
    pub retention_runs: u32,
    /// Days of past runs kept.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    /// Paused: no new runs start; running ones finish.
    #[serde(default)]
    pub paused: bool,
    /// `add` falls back to this when a job sets no `time_limit_s` of its own.
    #[serde(default = "default_time_limit")]
    pub default_time_limit_s: u64,
    /// Which run outcomes notify by default (`done`, `failed`, `skipped`).
    #[serde(default)]
    pub notify_default_events: Vec<String>,
}

impl Default for CronSettings {
    fn default() -> Self {
        Self {
            concurrency: default_concurrency(),
            missed_window_s: default_missed_window(),
            retention_runs: default_retention_runs(),
            retention_days: default_retention_days(),
            paused: false,
            default_time_limit_s: default_time_limit(),
            notify_default_events: Vec::new(),
        }
    }
}

fn default_concurrency() -> u32 {
    2
}
fn default_missed_window() -> u64 {
    21_600
}
fn default_retention_runs() -> u32 {
    50
}
fn default_retention_days() -> u32 {
    30
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_round_trips_with_defaults_for_new_fields() {
        // A job written before overlap/missed existed loads with the defaults.
        let raw = r#"{
            "id": "digest", "name": "Digest", "prompt": "p", "schedule": "0 0 9 * * *",
            "tz": "UTC", "cwd": "/tmp", "created": 1, "updated": 2
        }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.overlap, OverlapPolicy::Skip);
        assert_eq!(job.missed, MissedPolicy::RunLatest);
        assert!(job.enabled);
        assert_eq!(job.time_limit_s, DEFAULT_TIME_LIMIT_S);
    }

    #[test]
    fn job_file_fields_are_camel_case_and_legacy_snake_case_loads() {
        let job = Job::new("j", "n", "p", "0 0 * * * *", "UTC", "/tmp");
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["timeLimitS"], DEFAULT_TIME_LIMIT_S);
        assert!(value.get("time_limit_s").is_none());
        assert!(value.get("costCapUsd").is_none());
        // The pre-v2 `jobs.json` wrote snake_case with `created`/`updated`.
        let legacy = r#"{
            "id": "digest", "name": "Digest", "prompt": "p", "schedule": "0 0 9 * * *",
            "tz": "UTC", "cwd": "/tmp", "time_limit_s": 600, "cost_cap_usd": 1.5,
            "created": 1, "updated": 2
        }"#;
        let job: Job = serde_json::from_str(legacy).unwrap();
        assert_eq!(job.time_limit_s, 600);
        assert_eq!(job.cost_cap_usd, Some(1.5));
        assert_eq!(job.created, 1);
        // Re-serialising writes the v2 camelCase form.
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["timeLimitS"], 600);
        assert_eq!(value["costCapUsd"], 1.5);
    }

    #[test]
    fn policies_use_kebab_case_on_the_wire() {
        let job = Job {
            overlap: OverlapPolicy::Allow,
            missed: MissedPolicy::Skip,
            ..Job::new("j", "n", "p", "0 0 * * * *", "UTC", "/tmp")
        };
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["overlap"], "allow");
        assert_eq!(value["missed"], "skip");
    }

    #[test]
    fn run_status_wire_names() {
        let run: Run =
            serde_json::from_str(r#"{"id":"r","job_id":"j","status":"ok","fired_at":0}"#).unwrap();
        assert_eq!(run.status, RunStatus::Ok);
        assert_eq!(RunStatus::Queued.as_str(), "queued");
    }

    #[test]
    fn session_link_round_trips_and_targets_the_file_first() {
        let mut job = Job::new("j", "n", "p", "0 0 * * * *", "UTC", "/tmp");
        assert!(job.session_link.is_none());
        job.session_link = Some(SessionLink {
            session_file: Some("/s.jsonl".into()),
            session_id: Some("abc".into()),
        });
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["sessionLink"]["sessionFile"], "/s.jsonl");
        let back: Job = serde_json::from_value(value).unwrap();
        assert_eq!(back.session_link, job.session_link);
        assert_eq!(
            back.session_link.as_ref().unwrap().target(),
            Some("/s.jsonl")
        );
        // An id-only link resolves to the id; an empty link has no target.
        let id_only = SessionLink {
            session_file: Some(String::new()),
            session_id: Some("abc".into()),
        };
        assert_eq!(id_only.target(), Some("abc"));
        assert_eq!(SessionLink::default().target(), None);
    }

    #[test]
    fn wake_text_names_the_job_status_summary_and_run() {
        let job = Job::new("digest", "Daily digest", "p", "0 0 9 * * *", "UTC", "/tmp");
        let run: Run = serde_json::from_str(
            r#"{"id":"r7","job_id":"digest","status":"ok","fired_at":0,
                "summary":"Summarized 12 emails"}"#,
        )
        .unwrap();
        let text = wake_text(&job, &run);
        assert!(
            text.starts_with("[cron] Daily digest finished: ok."),
            "{text}"
        );
        assert!(text.contains("Summarized 12 emails"), "{text}");
        assert!(text.contains("Run r7"), "{text}");
        // No summary: the reason stands in; no reason either: a placeholder.
        let mut failed = run.clone();
        failed.summary = None;
        failed.reason = Some("pi exited with status 1".into());
        assert!(wake_text(&job, &failed).contains("pi exited with status 1"));
        failed.reason = None;
        assert!(wake_text(&job, &failed).contains("(no summary)"));
    }

    #[test]
    fn settings_defaults() {
        let settings: CronSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, CronSettings::default());
        assert_eq!(settings.concurrency, 2);
        assert_eq!(settings.missed_window_s, 21_600);
        assert!(!settings.paused);
    }
}
