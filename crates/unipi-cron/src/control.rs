//! Every cron control operation ([`CronControl`]), handled identically by the daemon
//! (over its TCP socket) and by any process reading the store directly (CLI without a
//! daemon, the desktop). Ops that need a live scheduler (`run`, `stop`, `status`)
//! fail with a clear message when none is attached.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{CronSettings, Job, MissedPolicy, NotifySpec, OverlapPolicy, Run, RunStatus};
use crate::paths::CronPaths;
use crate::schedule::{self, Schedule};
use crate::scheduler::{Scheduler, job_json, run_json, slugify, unique_id, validate_job};
use crate::store;

/// One cron control request (the daemon wire format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum CronControl {
    /// Every job.
    List,
    /// One job and its recent runs.
    Show {
        /// Job id.
        id: String,
    },
    /// Add a job ([`NewJob`]).
    Add(Box<NewJob>),
    /// Edit a job ([`EditJob`]); every field `None` keeps the current value.
    Edit {
        /// Job id.
        id: String,
        /// The patch.
        patch: EditJob,
    },
    /// Remove a job and its run history.
    Remove {
        /// Job id.
        id: String,
    },
    /// Enable a job.
    Enable {
        /// Job id.
        id: String,
    },
    /// Disable a job.
    Disable {
        /// Job id.
        id: String,
    },
    /// Fire a job now (needs a live scheduler).
    RunNow {
        /// Job id.
        id: String,
    },
    /// Kill a job's running runs (needs a live scheduler).
    Stop {
        /// Job id.
        id: String,
    },
    /// Run history, one job's or all, newest first.
    Runs {
        /// Job id (`None` = every job).
        #[serde(default)]
        job_id: Option<String>,
        /// Truncate to the newest N.
        #[serde(default)]
        limit: Option<usize>,
    },
    /// A run's record and captured output.
    Log {
        /// Run id.
        run_id: String,
    },
    /// Pause every job (running ones finish).
    PauseAll,
    /// Resume after a pause.
    ResumeAll,
    /// Scheduler status: paused, concurrency, per-job next fire and running flag (live
    /// running flags need a scheduler; without one the store's latest runs are used).
    Status,
    /// Read the scheduler settings.
    SettingsGet,
    /// Patch the scheduler settings ([`SettingsPatch`]).
    SettingsSet(SettingsPatch),
    /// Send one real test notification on `id`'s channels (or the user's default set).
    TestNotify {
        /// Job id.
        id: String,
    },
    /// Resolve a schedule as typed (plain language or machine form) and preview the next
    /// fires.
    Parse {
        /// The schedule text.
        schedule: String,
        /// IANA zone (default: system).
        #[serde(default)]
        tz: Option<String>,
        /// How many fires to preview.
        #[serde(default = "default_preview")]
        count: usize,
    },
}

fn default_preview() -> usize {
    3
}

/// Fields for `add`; `id` is optional (a slug is made from `name`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewJob {
    /// A short slug (default: made from `name`).
    #[serde(default)]
    pub id: Option<String>,
    /// Job name.
    pub name: String,
    /// The prompt every run sends.
    pub prompt: String,
    /// A recurring schedule (plain language or 6-field cron) or a one-time
    /// `at:<RFC3339>` / plain-language one-time form.
    pub schedule: String,
    /// IANA zone (default: this machine's).
    #[serde(default)]
    pub tz: Option<String>,
    /// Working directory (must exist).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Model override.
    #[serde(default)]
    pub model: Option<String>,
    /// Thinking level override.
    #[serde(default)]
    pub thinking: Option<String>,
    /// Start enabled (default).
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Wall-clock limit for one run.
    #[serde(default)]
    pub time_limit_s: Option<u64>,
    /// Cost cap.
    #[serde(default)]
    pub cost_cap_usd: Option<f64>,
    /// Overlap policy.
    #[serde(default)]
    pub overlap: Option<OverlapPolicy>,
    /// Missed-fire policy.
    #[serde(default)]
    pub missed: Option<MissedPolicy>,
    /// Notification settings.
    #[serde(default)]
    pub notify: Option<NotifySpec>,
}

/// Fields for `edit`; every field `None` keeps the current value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditJob {
    /// New name.
    #[serde(default)]
    pub name: Option<String>,
    /// New prompt.
    #[serde(default)]
    pub prompt: Option<String>,
    /// New schedule (plain language or machine form).
    #[serde(default)]
    pub schedule: Option<String>,
    /// New zone.
    #[serde(default)]
    pub tz: Option<String>,
    /// New working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// New model (`Some(None)` clears).
    #[serde(default)]
    pub model: Option<Option<String>>,
    /// New thinking level (`Some(None)` clears).
    #[serde(default)]
    pub thinking: Option<Option<String>>,
    /// Enable/disable.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// New wall-clock limit.
    #[serde(default)]
    pub time_limit_s: Option<u64>,
    /// New cost cap (`Some(None)` clears).
    #[serde(default)]
    pub cost_cap_usd: Option<Option<f64>>,
    /// New overlap policy.
    #[serde(default)]
    pub overlap: Option<OverlapPolicy>,
    /// New missed policy.
    #[serde(default)]
    pub missed: Option<MissedPolicy>,
    /// Notification settings (`Some(None)` clears).
    #[serde(default)]
    pub notify: Option<Option<NotifySpec>>,
}

/// A settings patch (unipi's `cron settings --key value`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    /// Concurrency (min 1).
    #[serde(default)]
    pub concurrency: Option<u32>,
    /// Missed window seconds.
    #[serde(default)]
    pub missed_window_s: Option<u64>,
    /// Runs kept per job (min 1).
    #[serde(default)]
    pub retention_runs: Option<u32>,
    /// Days of runs kept (min 1).
    #[serde(default)]
    pub retention_days: Option<u32>,
    /// Default time limit seconds.
    #[serde(default)]
    pub default_time_limit_s: Option<u64>,
    /// Pause/resume everything.
    #[serde(default)]
    pub paused: Option<bool>,
    /// Default notify events.
    #[serde(default)]
    pub notify_default_events: Option<Vec<String>>,
}

/// Resolves a schedule as typed (plain language or machine form) into the machine form
/// the scheduler stores. `for_at` requires a one-time result (a bare RFC3339 instant is
/// accepted too).
///
/// # Errors
/// A user-facing message when nothing matches.
pub fn resolve_schedule_input(
    input: &str,
    for_at: bool,
    tz_input: Option<&str>,
) -> Result<String, String> {
    let tz = schedule::resolve_tz(tz_input.unwrap_or_default()).map_err(|e| e.0)?;
    let now = Utc::now();
    if for_at {
        if let Ok(resolved) = schedule::parse_human(input, tz, now)
            && resolved.starts_with("at:")
        {
            return Ok(resolved);
        }
        let bare = format!("at:{input}");
        if Schedule::parse(&bare).is_ok() {
            return Ok(bare);
        }
        return Err(format!(
            "didn't understand the one-time schedule {input:?}. Try: \"in 2 hours\", \"tomorrow at 9\", or an exact instant like 2025-12-01T09:00:00Z"
        ));
    }
    schedule::parse_human(input, tz, now).map_err(|e| e.0)
}

/// Applies a [`NewJob`] onto `jobs`, validating it. Returns the stored job.
///
/// # Errors
/// A user-facing message on validation failures.
pub fn add_job(
    paths: &CronPaths,
    jobs: &mut Vec<Job>,
    new: NewJob,
    now_ms: u64,
) -> Result<Job, String> {
    let tz = new.tz.clone().unwrap_or_else(schedule::system_tz_name);
    let resolved = resolve_schedule_input(&new.schedule, false, Some(&tz))?;
    let cwd = match new.cwd.clone() {
        Some(cwd) => cwd,
        None => std::env::current_dir()
            .map_err(|e| format!("no --cwd given and the current directory is unreadable: {e}"))?
            .to_string_lossy()
            .into_owned(),
    };
    let id = match new.id.clone() {
        Some(id) => {
            if jobs.iter().any(|j| j.id == id) {
                return Err(format!("a job {id:?} already exists"));
            }
            id
        }
        None => unique_id(jobs, &slugify(&new.name)),
    };
    let settings = store::load_settings(paths).unwrap_or_default();
    let job = Job {
        id,
        name: new.name,
        prompt: new.prompt,
        schedule: resolved,
        tz,
        cwd,
        model: new.model.filter(|m| !m.is_empty()),
        thinking: new.thinking.filter(|t| !t.is_empty()),
        enabled: new.enabled.unwrap_or(true),
        time_limit_s: new
            .time_limit_s
            .unwrap_or(settings.default_time_limit_s.max(1)),
        cost_cap_usd: new.cost_cap_usd,
        overlap: new.overlap.unwrap_or_default(),
        missed: new.missed.unwrap_or_default(),
        notify: new.notify,
        created: now_ms,
        updated: now_ms,
    };
    validate_job(&job)?;
    jobs.push(job.clone());
    Ok(job)
}

/// Applies an [`EditJob`] onto `job` in place, validating the result.
///
/// # Errors
/// A user-facing message on validation failures.
pub fn edit_job(job: &mut Job, patch: EditJob, now_ms: u64) -> Result<(), String> {
    if let Some(v) = patch.name {
        job.name = v;
    }
    if let Some(v) = patch.prompt {
        job.prompt = v;
    }
    if let Some(v) = patch.schedule {
        job.schedule = resolve_schedule_input(&v, false, Some(&job.tz))?;
    }
    if let Some(v) = patch.tz {
        job.tz = v;
    }
    if let Some(v) = patch.cwd {
        job.cwd = v;
    }
    if let Some(v) = patch.model {
        job.model = v.filter(|m| !m.is_empty());
    }
    if let Some(v) = patch.thinking {
        job.thinking = v.filter(|t| !t.is_empty());
    }
    if let Some(v) = patch.enabled {
        job.enabled = v;
    }
    if let Some(v) = patch.time_limit_s {
        job.time_limit_s = v;
    }
    if let Some(v) = patch.cost_cap_usd {
        job.cost_cap_usd = v;
    }
    if let Some(v) = patch.overlap {
        job.overlap = v;
    }
    if let Some(v) = patch.missed {
        job.missed = v;
    }
    if let Some(v) = patch.notify {
        job.notify = v;
    }
    validate_job(job)?;
    job.updated = now_ms;
    Ok(())
}

/// Applies a settings patch to `settings`.
///
/// # Errors
/// A user-facing message for out-of-range values.
pub fn apply_settings_patch(
    settings: &mut CronSettings,
    patch: SettingsPatch,
) -> Result<(), String> {
    if let Some(v) = patch.concurrency {
        if v == 0 {
            return Err("concurrency must be at least 1".to_owned());
        }
        settings.concurrency = v;
    }
    if let Some(v) = patch.missed_window_s {
        settings.missed_window_s = v;
    }
    if let Some(v) = patch.retention_runs {
        if v == 0 {
            return Err("retention_runs must be at least 1".to_owned());
        }
        settings.retention_runs = v;
    }
    if let Some(v) = patch.retention_days {
        if v == 0 {
            return Err("retention_days must be at least 1".to_owned());
        }
        settings.retention_days = v;
    }
    if let Some(v) = patch.default_time_limit_s {
        if v == 0 {
            return Err("default_time_limit_s must be at least 1".to_owned());
        }
        settings.default_time_limit_s = v;
    }
    if let Some(v) = patch.paused {
        settings.paused = v;
    }
    if let Some(v) = patch.notify_default_events {
        settings.notify_default_events = v;
    }
    Ok(())
}

/// The `next_fire` / `running` extras the status and list views add to a job.
#[must_use]
pub fn job_runtime(job: &Job, running: bool, now: DateTime<Utc>) -> Value {
    let next = if job.enabled {
        schedule::resolve_tz(&job.tz)
            .ok()
            .zip(Schedule::parse(&job.schedule).ok())
            .and_then(|(tz, sched)| sched.next_after(tz, now, false))
            .map(|dt| dt.to_rfc3339())
    } else {
        None
    };
    json!({ "next_fire": next, "running": running })
}

/// Handles one control request. `scheduler` is the live owner (daemon / desktop embed);
/// without it, `run-now`, `stop` and live `status` running flags fail gracefully.
///
/// # Errors
/// [`crate::error::CronError::Control`] with a user-facing message.
#[allow(
    clippy::too_many_lines,
    reason = "one flat match over CronControl; each arm is 1–10 lines"
)]
pub async fn handle_control(
    paths: &CronPaths,
    scheduler: Option<&Scheduler>,
    request: CronControl,
) -> crate::error::Result<Value> {
    let now_ms = scheduler.map_or_else(now_ms, Scheduler::clock_now_ms);
    match request {
        CronControl::List => {
            let jobs = store::load_jobs(paths)?;
            let running = running_set(scheduler).await;
            let now = dt_now(scheduler);
            let jobs: Vec<Value> = jobs
                .iter()
                .map(|j| {
                    let mut value = job_json(j);
                    if let Some(obj) = value.as_object_mut()
                        && let Some(extra) =
                            job_runtime(j, running.contains(&j.id), now).as_object()
                    {
                        for (k, v) in extra {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                    value
                })
                .collect();
            Ok(json!({ "jobs": jobs }))
        }
        CronControl::Show { id } => {
            let jobs = store::load_jobs(paths)?;
            let job = jobs
                .iter()
                .find(|j| j.id == id)
                .ok_or_else(|| not_found(&format!("no job {id:?}")))?;
            let runs = store::load_runs(paths, &id)?;
            let running = running_set(scheduler).await;
            let mut value = job_json(job);
            if let Some(obj) = value.as_object_mut()
                && let Some(extra) =
                    job_runtime(job, running.contains(&job.id), dt_now(scheduler)).as_object()
            {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }
            Ok(json!({
                "job": value,
                "runs": runs.iter().map(run_json).collect::<Vec<_>>(),
            }))
        }
        CronControl::Add(new) => {
            let mut jobs = store::load_jobs(paths)?;
            let job = add_job(paths, &mut jobs, *new, now_ms)
                .map_err(crate::error::CronError::control)?;
            store::save_jobs(paths, &jobs)?;
            Ok(job_json(&job))
        }
        CronControl::Edit { id, patch } => {
            let mut jobs = store::load_jobs(paths)?;
            let job = jobs
                .iter_mut()
                .find(|j| j.id == id)
                .ok_or_else(|| not_found(&format!("no job {id:?}")))?;
            edit_job(job, patch, now_ms).map_err(crate::error::CronError::control)?;
            let result = job_json(job);
            store::save_jobs(paths, &jobs)?;
            Ok(result)
        }
        CronControl::Remove { id } => {
            let mut jobs = store::load_jobs(paths)?;
            let before = jobs.len();
            jobs.retain(|j| j.id != id);
            if jobs.len() == before {
                return Err(not_found(&format!("no job {id:?}")));
            }
            store::save_jobs(paths, &jobs)?;
            store::remove_runs(paths, &id);
            Ok(json!({ "removed": id }))
        }
        CronControl::Enable { id } => {
            set_enabled(paths, scheduler, &id, true)?;
            Ok(json!({ "id": id, "enabled": true }))
        }
        CronControl::Disable { id } => {
            set_enabled(paths, scheduler, &id, false)?;
            Ok(json!({ "id": id, "enabled": false }))
        }
        CronControl::RunNow { id } => {
            let Some(scheduler) = scheduler else {
                return Err(crate::error::CronError::NotRunning);
            };
            let jobs = store::load_jobs(paths)?;
            let job = jobs
                .iter()
                .find(|j| j.id == id)
                .ok_or_else(|| not_found(&format!("no job {id:?}")))?;
            let run_id = scheduler
                .run_now(job)
                .await
                .map_err(crate::error::CronError::control)?;
            Ok(json!({ "id": id, "run_id": run_id }))
        }
        CronControl::Stop { id } => {
            let Some(scheduler) = scheduler else {
                return Err(crate::error::CronError::NotRunning);
            };
            let stopped = scheduler.stop(&id).await;
            Ok(json!({ "id": id, "stopped": stopped }))
        }
        CronControl::Runs { job_id, limit } => {
            let mut runs = match &job_id {
                Some(id) => store::load_runs(paths, id)?,
                None => store::load_all_runs(paths)?,
            };
            if let Some(limit) = limit {
                runs.truncate(limit);
            }
            Ok(json!({ "runs": runs.iter().map(run_json).collect::<Vec<_>>() }))
        }
        CronControl::Log { run_id } => {
            let Some((job_id, run)) = store::find_run(paths, &run_id)?
                .filter(|(_, run)| run.status != RunStatus::Queued || run.started_at.is_some())
            else {
                return Err(not_found(&format!("no run {run_id:?}")));
            };
            let log = store::read_log(paths, &job_id, &run_id).unwrap_or_default();
            Ok(json!({
                "job_id": job_id,
                "run": run_json(&run),
                "log": String::from_utf8_lossy(&log),
            }))
        }
        CronControl::PauseAll => {
            let mut settings = store::load_settings(paths)?;
            settings.paused = true;
            store::save_settings(paths, &settings)?;
            Ok(json!({ "paused": true }))
        }
        CronControl::ResumeAll => {
            let mut settings = store::load_settings(paths)?;
            settings.paused = false;
            store::save_settings(paths, &settings)?;
            Ok(json!({ "paused": false }))
        }
        CronControl::Status => {
            let jobs = store::load_jobs(paths)?;
            let settings = store::load_settings(paths)?;
            let running = running_set(scheduler).await;
            let now = dt_now(scheduler);
            let jobs: Vec<Value> = jobs
                .iter()
                .map(|job| {
                    let mut value = json!({
                        "id": job.id,
                        "enabled": job.enabled,
                    });
                    let extra = job_runtime(job, running.contains(&job.id), now);
                    value["next_fire"] = extra["next_fire"].clone();
                    value["running"] = extra["running"].clone();
                    // The newest recorded run, so a UI can show "last ok 09:00" without
                    // asking for every job's history.
                    if let Ok(Some(last)) =
                        store::load_runs(paths, &job.id).map(|runs| runs.into_iter().next())
                    {
                        value["last_run_id"] = json!(last.id);
                        value["last_status"] = json!(last.status);
                        value["last_fire"] = json!(last.fired_at);
                    }
                    value
                })
                .collect();
            Ok(json!({
                "paused": settings.paused,
                "concurrency": settings.concurrency,
                "owner": scheduler.map(|_| scheduler_owner()),
                "jobs": jobs,
            }))
        }
        CronControl::SettingsGet => {
            Ok(serde_json::to_value(store::load_settings(paths)?).unwrap_or_else(|_| json!({})))
        }
        CronControl::SettingsSet(patch) => {
            let mut settings = store::load_settings(paths)?;
            apply_settings_patch(&mut settings, patch).map_err(crate::error::CronError::control)?;
            store::save_settings(paths, &settings)?;
            Ok(serde_json::to_value(settings).unwrap_or_else(|_| json!({})))
        }
        CronControl::TestNotify { id } => {
            let jobs = store::load_jobs(paths)?;
            let job = jobs
                .iter()
                .find(|j| j.id == id)
                .ok_or_else(|| not_found(&format!("no job {id:?}")))?;
            let settings = store::load_settings(paths)?;
            let channels = crate::notify::Channels::load(paths, &settings);
            let selected = crate::notify::resolve_channels(job, &channels.default_channels());
            if selected.is_empty() {
                return Err(crate::error::CronError::control(
                    "no notify channels configured for this job (and none of the user's channels are enabled)",
                ));
            }
            let now = dt_now(scheduler);
            let stamp = now.timestamp_millis().max(0).cast_unsigned();
            let test_run = Run {
                id: format!("test-{stamp}"),
                job_id: job.id.clone(),
                status: RunStatus::Ok,
                fired_at: stamp,
                started_at: Some(stamp),
                ended_at: Some(stamp),
                exit_code: Some(0),
                summary: Some(
                    "This is a test notification from `oino cron test-notify`.".to_owned(),
                ),
                reason: None,
                session_path: None,
                cost_usd: None,
                missed: false,
                pid: None,
                notify_errors: Vec::new(),
            };
            // Bypass the events filter (a test always sends) by dispatching directly.
            let errors = crate::notify::dispatch_forced(job, &test_run, &channels).await;
            Ok(json!({ "id": id, "channels": selected, "errors": errors }))
        }
        CronControl::Parse {
            schedule,
            tz,
            count,
        } => {
            let zone = schedule::resolve_tz(tz.as_deref().unwrap_or_default())
                .map_err(|e| crate::error::CronError::control(e.0))?;
            let resolved = schedule::parse_human(&schedule, zone, Utc::now())
                .map_err(|e| crate::error::CronError::control(e.0))?;
            let parsed =
                Schedule::parse(&resolved).map_err(|e| crate::error::CronError::control(e.0))?;
            let fires = schedule::next_fires(&parsed, zone, Utc::now(), count.max(1));
            Ok(json!({
                "expression": resolved,
                "one_time": parsed.is_one_time(),
                "tz": zone.to_string(),
                "next_fires": fires.iter().map(DateTime::to_rfc3339).collect::<Vec<_>>(),
            }))
        }
    }
}

fn scheduler_owner() -> Value {
    json!({ "pid": std::process::id() })
}

fn dt_now(scheduler: Option<&Scheduler>) -> DateTime<Utc> {
    scheduler.map_or_else(Utc::now, Scheduler::clock_now)
}

fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis().max(0)).unwrap_or(0)
}

fn not_found(message: &str) -> crate::error::CronError {
    crate::error::CronError::control(message.to_owned())
}

async fn running_set(scheduler: Option<&Scheduler>) -> Vec<String> {
    match scheduler {
        Some(s) => s.running_job_ids().await,
        None => {
            // Without a live scheduler, a run still marked running in the store counts.
            match CronPaths::from_agent_dir() {
                Ok(paths) => {
                    let mut ids: Vec<String> = store::load_all_runs(&paths)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|r| !r.status.is_finished())
                        .map(|r| r.job_id)
                        .collect();
                    ids.sort();
                    ids.dedup();
                    ids
                }
                Err(_) => Vec::new(),
            }
        }
    }
}

fn set_enabled(
    paths: &CronPaths,
    scheduler: Option<&Scheduler>,
    id: &str,
    enabled: bool,
) -> crate::error::Result<()> {
    if let Some(s) = scheduler {
        s.set_enabled(id, enabled)
    } else {
        let mut jobs = store::load_jobs(paths)?;
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else {
            return Err(not_found(&format!("no job {id:?}")));
        };
        job.enabled = enabled;
        job.updated = now_ms();
        store::save_jobs(paths, &jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DEFAULT_TIME_LIMIT_S;

    fn paths() -> (tempfile::TempDir, CronPaths) {
        let tmp = tempfile::tempdir().unwrap();
        let p = CronPaths::new(tmp.path());
        (tmp, p)
    }

    fn new_job(name: &str, schedule: &str, cwd: &str) -> NewJob {
        NewJob {
            name: name.to_owned(),
            prompt: "p".to_owned(),
            schedule: schedule.to_owned(),
            cwd: Some(cwd.to_owned()),
            ..NewJob::default()
        }
    }

    #[tokio::test]
    async fn add_edit_remove_round_trip_without_a_scheduler() {
        let (tmp, paths) = paths();
        let cwd = tmp.path().to_string_lossy().into_owned();
        // add
        let value = handle_control(
            &paths,
            None,
            CronControl::Add(Box::new(new_job("Daily Digest", "every day at 9:00", &cwd))),
        )
        .await
        .unwrap();
        assert_eq!(value["id"], "daily-digest");
        assert_eq!(value["schedule"], "0 0 9 * * *");
        assert_eq!(value["time_limit_s"], DEFAULT_TIME_LIMIT_S);
        // edit: human schedule resolved against the job's zone
        let value = handle_control(
            &paths,
            None,
            CronControl::Edit {
                id: "daily-digest".to_owned(),
                patch: EditJob {
                    schedule: Some("weekdays at 18:30".to_owned()),
                    model: Some(Some("oino/deepseek/deepseek-flash".to_owned())),
                    ..EditJob::default()
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(value["schedule"], "0 30 18 * * MON-FRI");
        assert_eq!(value["model"], "oino/deepseek/deepseek-flash");
        // show
        let value = handle_control(
            &paths,
            None,
            CronControl::Show {
                id: "daily-digest".to_owned(),
            },
        )
        .await
        .unwrap();
        assert_eq!(value["job"]["id"], "daily-digest");
        // remove
        handle_control(
            &paths,
            None,
            CronControl::Remove {
                id: "daily-digest".to_owned(),
            },
        )
        .await
        .unwrap();
        assert!(
            handle_control(
                &paths,
                None,
                CronControl::Show {
                    id: "daily-digest".to_owned()
                }
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn add_validates_cwd_and_duplicate_ids() {
        let (tmp, paths) = paths();
        let cwd = tmp.path().to_string_lossy().into_owned();
        let err = handle_control(
            &paths,
            None,
            CronControl::Add(Box::new(new_job("x", "hourly", &format!("{cwd}/missing")))),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("doesn't exist"), "{err}");
        handle_control(
            &paths,
            None,
            CronControl::Add(Box::new(NewJob {
                id: Some("fixed".to_owned()),
                ..new_job("x", "hourly", &cwd)
            })),
        )
        .await
        .unwrap();
        let err = handle_control(
            &paths,
            None,
            CronControl::Add(Box::new(NewJob {
                id: Some("fixed".to_owned()),
                ..new_job("x", "hourly", &cwd)
            })),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[tokio::test]
    async fn run_now_and_stop_need_a_scheduler() {
        let (tmp, paths) = paths();
        let cwd = tmp.path().to_string_lossy().into_owned();
        handle_control(
            &paths,
            None,
            CronControl::Add(Box::new(new_job("x", "hourly", &cwd))),
        )
        .await
        .unwrap();
        let err = handle_control(&paths, None, CronControl::RunNow { id: "x".to_owned() })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not running"), "{err}");
    }

    #[tokio::test]
    async fn pause_resume_and_settings_round_trip() {
        let (_tmp, paths) = paths();
        handle_control(&paths, None, CronControl::PauseAll)
            .await
            .unwrap();
        let value = handle_control(&paths, None, CronControl::Status)
            .await
            .unwrap();
        assert_eq!(value["paused"], true);
        handle_control(
            &paths,
            None,
            CronControl::SettingsSet(SettingsPatch {
                concurrency: Some(4),
                ..SettingsPatch::default()
            }),
        )
        .await
        .unwrap();
        let value = handle_control(&paths, None, CronControl::SettingsGet)
            .await
            .unwrap();
        assert_eq!(value["concurrency"], 4);
        let err = handle_control(
            &paths,
            None,
            CronControl::SettingsSet(SettingsPatch {
                concurrency: Some(0),
                ..SettingsPatch::default()
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("at least 1"));
        handle_control(&paths, None, CronControl::ResumeAll)
            .await
            .unwrap();
        assert_eq!(
            handle_control(&paths, None, CronControl::Status)
                .await
                .unwrap()["paused"],
            false
        );
    }

    #[tokio::test]
    async fn parse_resolves_human_forms_and_previews_fires() {
        let (_tmp, paths) = paths();
        let value = handle_control(
            &paths,
            None,
            CronControl::Parse {
                schedule: "every 5 minutes".to_owned(),
                tz: Some("UTC".to_owned()),
                count: 2,
            },
        )
        .await
        .unwrap();
        assert_eq!(value["expression"], "0 */5 * * * *");
        assert_eq!(value["one_time"], false);
        assert_eq!(value["next_fires"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn resolve_schedule_input_for_at_accepts_bare_instants() {
        assert_eq!(
            resolve_schedule_input("at:2030-01-01T00:00:00Z", true, Some("UTC")).unwrap(),
            "at:2030-01-01T00:00:00Z"
        );
        assert_eq!(
            resolve_schedule_input("2030-01-01T00:00:00Z", true, Some("UTC")).unwrap(),
            "at:2030-01-01T00:00:00Z"
        );
        assert!(resolve_schedule_input("whenever", true, Some("UTC")).is_err());
    }
}
