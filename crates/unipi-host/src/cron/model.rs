//! Job, run and settings types for `unipi-host`'s cron scheduler (UNI-314).
//!
//! See `crates/host/src/cron/mod.rs` for the module overview and
//! `docs/m7/CRON.md` for the storage layout and rules.

use serde::{Deserialize, Serialize};

/// Default `time_limit_s` for a job that doesn't set one.
pub const DEFAULT_TIME_LIMIT_S: u64 = 1800;

/// A scheduled job (`<host dir>/cron/jobs.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// A short slug, unique among jobs.
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// A 6-field cron (seconds first) or `at:<RFC3339>`.
    pub schedule: String,
    /// An IANA zone name; defaults to the system zone at creation time.
    pub tz: String,
    /// The job's working directory. Must exist.
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_time_limit")]
    pub time_limit_s: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_usd: Option<f64>,
    /// Per-job notification settings (UNI-316, Cron C3). Absent = the
    /// user's enabled notify channels, events `["failed"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<NotifySpec>,
    /// A link back to a live pi session to wake once this job's run ends
    /// (UNI-316, Cron C3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_link: Option<SessionLink>,
    /// Unix ms.
    pub created: u64,
    pub updated: u64,
}

/// A job's `notify` field (UNI-316): which channels to use and which run
/// statuses trigger a notification. `None` fields fall back to the
/// defaults described on [`Job::notify`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotifySpec {
    /// `"ntfy"` / `"gotify"` / `"telegram"`. Absent = whatever the user's
    /// notify config has enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    /// Run statuses to notify on (`"ok"`/`"done"`, `"failed"`,
    /// `"skipped"`, `"timeout"`). Absent = `["failed"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
}

/// A job's `session_link` field (UNI-316): identifies a live bridge by
/// its session file (preferred) or session id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_time_limit() -> u64 {
    DEFAULT_TIME_LIMIT_S
}

/// What a run is doing or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Ok,
    Failed,
    Timeout,
    Skipped,
    /// Queued (waiting for a concurrency slot) but not started yet.
    Queued,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Ok => "ok",
            RunStatus::Failed => "failed",
            RunStatus::Timeout => "timeout",
            RunStatus::Skipped => "skipped",
            RunStatus::Queued => "queued",
        }
    }
}

/// One run record (`<host dir>/cron/runs/<job id>/<run id>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub job_id: String,
    pub status: RunStatus,
    /// Unix ms when this fire was recorded (scheduled time, or now for run_now).
    pub fired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Why a run was skipped (overlap) or failed before exec (validation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// pi's session path, when we could find it. Null otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_path: Option<String>,
    /// pi's reported cost in USD, when we could find it. Null otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Whether this fire was a catch-up ("missed") run.
    #[serde(default)]
    pub missed: bool,
    /// Per-channel delivery failures (UNI-316); never fails the run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notify_errors: Vec<String>,
    /// What happened trying to wake a linked session (UNI-316):
    /// `"sent"`, `"session closed"`, or absent when no `session_link` was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<String>,
}

/// The `cron` block of `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronSettings {
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    #[serde(default = "default_missed_window")]
    pub missed_window_s: u64,
    #[serde(default = "default_retention_runs")]
    pub retention_runs: u32,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default)]
    pub paused: bool,
    /// UNI-318: `cron add`/`cron edit` fall back to this when a job sets no
    /// `time_limit_s` of its own. Separate from each job's own field so
    /// changing it doesn't need touching every job.
    #[serde(default = "default_time_limit")]
    pub default_time_limit_s: u64,
    /// UNI-317/UNI-253: phones need this feature in their per-device allow
    /// list (same pattern as forwarding/browser/push); this PC's own app
    /// (`Peer::Local`) always has access regardless of this flag.
    #[serde(default)]
    pub enabled: bool,
    /// Paired device ids / id prefixes (≥ 8 chars) / names allowed to use
    /// cron ops from a phone; empty = every paired device (once `enabled`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
    /// UNI-318/C3: ntfy topic cron notifications are sent to (stored/shown
    /// here; C3 wires the sending).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntfy_topic: Option<String>,
    /// UNI-318/C3: which run outcomes notify by default for a job that
    /// doesn't set its own `notify` (e.g. `["failed", "timeout"]`).
    #[serde(default)]
    pub notify_default_events: Vec<String>,
}

impl Default for CronSettings {
    fn default() -> Self {
        CronSettings {
            concurrency: default_concurrency(),
            missed_window_s: default_missed_window(),
            retention_runs: default_retention_runs(),
            retention_days: default_retention_days(),
            paused: false,
            default_time_limit_s: default_time_limit(),
            enabled: false,
            devices: Vec::new(),
            ntfy_topic: None,
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
