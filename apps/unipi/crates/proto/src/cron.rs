//! Scheduled pi runs from the app (UNI-317) and their settings (UNI-318).
//! Mirrors `packages/protocol/src/cron.ts`. The host owns the real
//! `Job`/`Run`/`CronSettings` types (`crates/host/src/cron/model.rs`,
//! documented in `docs/m7/CRON.md`); these are the wire shapes, built from
//! (and read back into) the same JSON the CLI already prints with
//! `--json`.
//!
//! One request/reply op per action below (`hello{req:{op:"cron_*", …}}` →
//! `ok{data}` or `err`). **Who may use these:** this PC's own app
//! (`Peer::Local`) always; a phone needs the `cron` sharing feature in its
//! per-device allow list first (UNI-248 pattern — `feature_request{feature:
//! "cron"}`), for every op below including the read-only ones (`cron_list`,
//! `cron_show`, `cron_runs`, `cron_log`, `cron_status`, `cron_parse`).
//!
//! - `cron_list` → [`CronJobsData`].
//! - `cron_show{id}` → [`CronShowData`] (the job + its runs).
//! - `cron_runs{id?, limit?}` → [`CronRunsData`] (all jobs' runs when `id`
//!   is omitted).
//! - `cron_log{run_id}` → [`CronLogData`] (one run's captured output).
//! - `cron_add{…}` → [`CronJob`] (the job as stored, including the
//!   resolved `tz`/`id`).
//! - `cron_edit{id, …}` → [`CronJob`] (only given fields change).
//! - `cron_rm{id}` → `ok{data:null}`.
//! - `cron_enable`/`cron_disable{id}` → `ok{data:null}`.
//! - `cron_run{id}` → [`CronRunStartedData`] (starts now, bypassing the
//!   schedule).
//! - `cron_stop{id}` → [`CronStoppedData`].
//! - `cron_pause`/`cron_resume` → `ok{data:null}` (every job; running ones
//!   finish).
//! - `cron_status` → [`CronStatusData`].
//! - `cron_parse{schedule|at, tz?}` → [`CronParseData`]: the plain-language
//!   or machine schedule resolved to its 6-field cron (or `at:`) form, plus
//!   the next 3 fires — the add form's live preview.
//! - `cron_settings_get` → [`CronSettingsData`].
//! - `cron_settings_set{…}` → [`CronSettingsData`] (only given fields
//!   change; validated).
//!
//! Hosts older than this proto version answer every `cron_*` op with
//! `E_SCHEMA` (unknown op).

use serde::{Deserialize, Serialize};

/// Longest `name`/`prompt` accepted from the wire (the host's own limits
/// are generous; this just keeps a phone from sending something absurd).
pub const CRON_NAME_MAX: usize = 200;
pub const CRON_PROMPT_MAX: usize = 20_000;
/// `cron_runs`/`cron_parse` bounds.
pub const CRON_RUNS_LIMIT_MAX: usize = 500;
pub const CRON_PARSE_PREVIEW: usize = 3;

/// A job id: 1..=64 of `[A-Za-z0-9_-]` (the host's slugs fit this; it also
/// double-checks when creating one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct CronJobId(String);

impl CronJobId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CronJobId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 64 {
            return Err(format!("id must be 1..=64 characters, got {}", value.len()));
        }
        if !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("id must be [A-Za-z0-9_-]".into());
        }
        Ok(CronJobId(value))
    }
}

impl<'de> Deserialize<'de> for CronJobId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        CronJobId::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A run id: 1..=64 of `[A-Za-z0-9_-]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct CronRunId(String);

impl CronRunId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CronRunId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 64 {
            return Err(format!("run id must be 1..=64 characters, got {}", value.len()));
        }
        if !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("run id must be [A-Za-z0-9_-]".into());
        }
        Ok(CronRunId(value))
    }
}

impl<'de> Deserialize<'de> for CronRunId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        CronRunId::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A job name or prompt: non-empty, within [`CRON_NAME_MAX`]/[`CRON_PROMPT_MAX`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CronName(String);

impl CronName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CronName {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if !(1..=CRON_NAME_MAX).contains(&len) {
            return Err(format!("name must be 1..={CRON_NAME_MAX} characters, got {len}"));
        }
        Ok(CronName(value))
    }
}

impl<'de> Deserialize<'de> for CronName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        CronName::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CronPrompt(String);

impl CronPrompt {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CronPrompt {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if !(1..=CRON_PROMPT_MAX).contains(&len) {
            return Err(format!("prompt must be 1..={CRON_PROMPT_MAX} characters, got {len}"));
        }
        Ok(CronPrompt(value))
    }
}

impl<'de> Deserialize<'de> for CronPrompt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        CronPrompt::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A `--limit`-style bound: 1..=[`CRON_RUNS_LIMIT_MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CronRunsLimit(usize);

impl CronRunsLimit {
    pub fn value(self) -> usize {
        self.0
    }
}

impl TryFrom<usize> for CronRunsLimit {
    type Error = String;
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if (1..=CRON_RUNS_LIMIT_MAX).contains(&value) {
            Ok(CronRunsLimit(value))
        } else {
            Err(format!("limit must be 1..={CRON_RUNS_LIMIT_MAX}, got {value}"))
        }
    }
}

impl<'de> Deserialize<'de> for CronRunsLimit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        CronRunsLimit::try_from(usize::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// What a run is doing or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronRunStatus {
    Running,
    Ok,
    Failed,
    Timeout,
    Skipped,
    Queued,
}

/// One job, as the wire sees it (built from the host's `Job`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// A 6-field cron (seconds first) or `at:<RFC3339>`.
    pub schedule: String,
    pub tz: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    pub enabled: bool,
    pub time_limit_s: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_usd: Option<f64>,
    pub created: u64,
    pub updated: u64,
    /// The job's next scheduled fire (RFC3339, UTC), when enabled and the
    /// schedule has one; absent for a one-time `at:` already run, or a
    /// disabled job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire: Option<String>,
    /// Is a run of this job in progress right now?
    #[serde(default)]
    pub running: bool,
    /// The most recent run's status, if any (for the list's status chip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<CronRunStatus>,
    /// UNI-319: the job's own notify choice; absent = the PC's defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<CronNotify>,
    /// UNI-319: the chat this job wakes when a run ends, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<CronWake>,
}

/// A job's notify choice (UNI-316/319). Each half absent = the default:
/// every channel the PC has set up / the PC's default outcomes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronNotify {
    /// `"ntfy"` / `"gotify"` / `"telegram"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    /// `"done"` / `"failed"` / `"skipped"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
}

/// The chat a job wakes (UNI-319), as the host sees it right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronWake {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The live pi on that session (absent = the session isn't open now;
    /// the wake is skipped until it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
}

/// `ok.data` for `cron_open_session` (UNI-319): the chat now on the run's
/// session — one that already had it open, or one just started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronOpenSessionData {
    pub pid: u32,
    /// A pi already had this session open; nothing was started.
    #[serde(default)]
    pub already_open: bool,
    /// Where the new pi runs (absent when `already_open`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<crate::chat::ChatNewData>,
}

/// One run record, as the wire sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub status: CronRunStatus,
    pub fired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub missed: bool,
}

/// `ok.data` for `cron_list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronJobsData {
    pub jobs: Vec<CronJob>,
    /// `cron pause`/`cron resume`: no new runs while true.
    pub paused: bool,
}

/// `ok.data` for `cron_show`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronShowData {
    pub job: CronJob,
    pub runs: Vec<CronRun>,
}

/// `ok.data` for `cron_runs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronRunsData {
    pub runs: Vec<CronRun>,
}

/// `ok.data` for `cron_log`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronLogData {
    pub job_id: String,
    pub run: CronRun,
    pub log: String,
}

/// `ok.data` for `cron_run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronRunStartedData {
    pub id: String,
    pub run_id: String,
}

/// `ok.data` for `cron_stop`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronStoppedData {
    pub id: String,
    pub stopped: bool,
}

/// One job's status line in [`CronStatusData`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronJobStatus {
    pub id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire: Option<String>,
    pub running: bool,
}

/// `ok.data` for `cron_status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronStatusData {
    pub paused: bool,
    pub concurrency: u32,
    pub jobs: Vec<CronJobStatus>,
}

/// `ok.data` for `cron_parse`: the resolved schedule plus a live preview
/// of its next fires, for the add/edit form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronParseData {
    /// The 6-field cron (or `at:<RFC3339>`) form `input` resolved to.
    pub expression: String,
    /// Is this a one-time (`at:`) schedule?
    pub one_time: bool,
    /// The resolved IANA zone (after `local`/`""` falls back to the system zone).
    pub tz: String,
    /// The next (up to) [`CRON_PARSE_PREVIEW`] fires, RFC3339 UTC. Empty
    /// for an `at:` schedule already in the past.
    pub next_fires: Vec<String>,
}

/// The `cron` block of `config.json` (UNI-318), as the wire sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronSettingsData {
    pub concurrency: u32,
    pub missed_window_s: u64,
    pub retention_runs: u32,
    pub retention_days: u32,
    pub default_time_limit_s: u64,
    pub paused: bool,
    /// UNI-317/UNI-253: is the `cron` sharing feature on for any phone?
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
    /// UNI-318/C3: stored and shown here; C3 wires the actual sending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntfy_topic: Option<String>,
    #[serde(default)]
    pub notify_default_events: Vec<String>,
    /// UNI-319: the notify channels set up on this PC (what a job can pick).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notify_channels: Vec<String>,
}

/// Fields for `cron_add`; `id` is optional (a slug is made from `name`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronAddRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<CronJobId>,
    pub name: CronName,
    pub prompt: CronPrompt,
    pub schedule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_limit_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_usd: Option<f64>,
    /// UNI-319; absent = the PC's defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<CronNotify>,
    /// UNI-319: wake this live chat (its pi's pid) when a run ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_pid: Option<std::num::NonZeroU32>,
}

/// Fields for `cron_edit`; every field left out keeps the current value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronEditRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<CronName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<CronPrompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_limit_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_usd: Option<f64>,
    /// UNI-319: replaces the notify choice; a `CronNotify` with both halves
    /// absent goes back to the PC's defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<CronNotify>,
    /// UNI-319: wake this live chat instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_pid: Option<std::num::NonZeroU32>,
    /// UNI-319: stop waking any chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_clear: Option<bool>,
}

/// Fields for `cron_settings_set`; every field left out keeps the current
/// value. Validated on the host (`concurrency`/`retention_*` ≥ 1, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronSettingsSetRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_window_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_runs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_time_limit_s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    /// `Some("")` clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntfy_topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_default_events: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_bounds() {
        assert!(CronJobId::try_from(String::new()).is_err());
        assert!(CronJobId::try_from("x".repeat(65)).is_err());
        assert!(CronJobId::try_from("a/b".to_string()).is_err());
        assert!(CronJobId::try_from("daily-digest".to_string()).is_ok());
        assert!(CronRunId::try_from("r1234".to_string()).is_ok());
    }

    #[test]
    fn name_prompt_bounds() {
        assert!(CronName::try_from(String::new()).is_err());
        assert!(CronName::try_from("x".repeat(CRON_NAME_MAX + 1)).is_err());
        assert!(CronName::try_from("daily digest".to_string()).is_ok());
        assert!(CronPrompt::try_from(String::new()).is_err());
        assert!(CronPrompt::try_from("Summarize today's inbox".to_string()).is_ok());
    }

    #[test]
    fn runs_limit_bounds() {
        assert!(CronRunsLimit::try_from(0).is_err());
        assert!(CronRunsLimit::try_from(CRON_RUNS_LIMIT_MAX + 1).is_err());
        assert_eq!(CronRunsLimit::try_from(10).unwrap().value(), 10);
    }

    #[test]
    fn job_round_trips() {
        let job = CronJob {
            id: "daily".into(),
            name: "Daily digest".into(),
            prompt: "Summarize".into(),
            schedule: "0 0 9 * * *".into(),
            tz: "Europe/Berlin".into(),
            cwd: "/home/me/jobs/digest".into(),
            model: None,
            thinking: None,
            enabled: true,
            time_limit_s: 1800,
            cost_cap_usd: Some(1.5),
            created: 1,
            updated: 2,
            next_fire: Some("2025-01-01T09:00:00Z".into()),
            running: false,
            last_run_status: Some(CronRunStatus::Ok),
            notify: Some(CronNotify { channels: None, events: Some(vec!["done".into()]) }),
            wake: Some(CronWake { session_file: Some("/s.jsonl".into()), session_id: None, pid: Some(7), session_name: None }),
        };
        let json = serde_json::to_value(&job).unwrap();
        assert_eq!(json["last_run_status"], "ok");
        let back: CronJob = serde_json::from_value(json).unwrap();
        assert_eq!(back, job);
    }

    #[test]
    fn settings_round_trip_and_defaults() {
        let json = serde_json::json!({
            "concurrency": 2,
            "missed_window_s": 21600,
            "retention_runs": 50,
            "retention_days": 30,
            "default_time_limit_s": 1800,
            "paused": false,
            "enabled": false,
        });
        let settings: CronSettingsData = serde_json::from_value(json).unwrap();
        assert!(settings.devices.is_empty());
        assert!(settings.ntfy_topic.is_none());
        assert!(settings.notify_default_events.is_empty());
    }
}
