//! Cron for the app (WP-H8, host.md §9.8; ext-abi §6): the `cron_*` and `cron_settings_*`
//! wire ops as a thin adapter over [`unipi_cron`] — the job-file store under `<agent
//! dir>/cron` and the embedded scheduler `unipi-host serve` owns.
//!
//! Gating (UNI-248) is kept: read-only ops (`cron_list`, `cron_show`, `cron_runs`,
//! `cron_log`, `cron_status`, `cron_parse`) and every write op alike need the `cron`
//! sharing feature once the caller isn't this PC's own app. The sharing block itself
//! (`enabled`/`devices`) stays in the host's `config.json`; the scheduler's own settings
//! live in `<agent dir>/cron/settings.json` (the library's file, shared with the CLI and
//! the cron extension).
//!
//! [`CronHost`] is what the protocol handler and the CLI's `control.sock` carry: the same
//! store paths plus the live scheduler when this `serve` holds the owner lock.

use std::path::Path;
use std::sync::Arc;

use unipi_cron::control::{self, CronControl, EditJob, NewJob, SettingsPatch};
use unipi_cron::model::{CronSettings, Job, NotifySpec, Run, RunStatus, SessionLink};
use unipi_cron::{schedule, store, CronPaths, Scheduler};

use unipi_proto::{
    AppErrorCode, AppErrorDto, Control, CronAddRequest, CronEditRequest, CronJob, CronJobStatus,
    CronJobsData, CronLogData, CronNotify, CronParseData, CronRun, CronRunStartedData,
    CronRunStatus, CronRunsData, CronSettingsData, CronSettingsSetRequest, CronShowData,
    CronStatusData, CronStoppedData, CronWake, Request,
};

use crate::data_dir::HostPaths;
use crate::protocol::write_control;

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false)
}

fn bad_request(e: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, e.into(), false)
}

fn not_found(e: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_NOT_FOUND, e.to_string(), false)
}

/// Gating error: the `cron` sharing feature is off for this device.
pub fn not_allowed() -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_UNAUTHORIZED, "Scheduled cron jobs are not shared with this phone", false)
        .with_hint("Ask the PC from the phone (feature_request{feature:\"cron\"}), or run `unipi-host cron share --device <name>` on the PC.")
}

/// The host's cron: the library's job files at `<agent dir>/cron` plus the embedded
/// scheduler when this `serve` process owns it (`scheduler.lock`).
#[derive(Debug, Clone)]
pub struct CronHost {
    host: HostPaths,
    paths: CronPaths,
    scheduler: Option<Arc<Scheduler>>,
}

impl CronHost {
    /// The store of `host`, without a live scheduler (the CLI's storage ops, tests).
    #[must_use]
    pub fn new(host: &HostPaths) -> Self {
        Self {
            host: host.clone(),
            paths: cron_paths_for_host(&host.root),
            scheduler: None,
        }
    }

    /// The same, with the scheduler this `serve` embeds ([`unipi_cron::embedded`]).
    #[must_use]
    pub fn with_scheduler(host: &HostPaths, scheduler: Arc<Scheduler>) -> Self {
        Self {
            host: host.clone(),
            paths: cron_paths_for_host(&host.root),
            scheduler: Some(scheduler),
        }
    }

    /// The host data dir (notify key, app endpoints — what the notification channels need).
    #[must_use]
    pub fn host(&self) -> &HostPaths {
        &self.host
    }

    /// The store paths.
    #[must_use]
    pub fn paths(&self) -> &CronPaths {
        &self.paths
    }

    /// The live scheduler, when this process owns the tick loop.
    #[must_use]
    pub fn scheduler(&self) -> Option<&Scheduler> {
        self.scheduler.as_deref()
    }

    /// Runs one control request against the files (and the live scheduler, when present).
    ///
    /// # Errors
    /// The library's user-facing message.
    pub async fn control(&self, request: CronControl) -> Result<serde_json::Value, String> {
        // A test notification must go through the same channels a real run does (§9.5):
        // the library's own sender is plain HTTP, the host's is the E2E push channel.
        if let CronControl::TestNotify { id } = &request {
            return self.test_notify(id).await;
        }
        control::handle_control(&self.paths, self.scheduler.as_deref(), request)
            .await
            .map_err(|e| e.to_string())
    }

    async fn test_notify(&self, id: &str) -> Result<serde_json::Value, String> {
        let jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
        let job = jobs
            .iter()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("no job {id:?}"))?;
        let settings = store::load_settings(&self.paths).unwrap_or_default();
        let channels = crate::cron_notify::Channels::load(&self.host, &settings);
        let selected = crate::cron_notify::resolve_channels(job, &channels.default_channels());
        if selected.is_empty() {
            return Err(
                "no notify channels configured for this job (and none of the user's channels are enabled)"
                    .to_owned(),
            );
        }
        let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default();
        let run = Run {
            id: format!("test-{now}"),
            job_id: job.id.clone(),
            status: RunStatus::Ok,
            fired_at: now,
            started_at: Some(now),
            ended_at: Some(now),
            exit_code: Some(0),
            summary: Some(
                "This is a test notification from `unipi-host cron test-notify`.".to_owned(),
            ),
            reason: None,
            session_path: None,
            cost_usd: None,
            missed: false,
            pid: None,
            notify_errors: Vec::new(),
            wake: None,
        };
        let errors = crate::cron_notify::dispatch_forced(job, &run, &channels).await;
        Ok(serde_json::json!({ "id": id, "channels": selected, "errors": errors }))
    }
}

fn run_status(s: RunStatus) -> CronRunStatus {
    match s {
        RunStatus::Running => CronRunStatus::Running,
        RunStatus::Ok => CronRunStatus::Ok,
        RunStatus::Failed => CronRunStatus::Failed,
        RunStatus::Timeout => CronRunStatus::Timeout,
        RunStatus::Skipped => CronRunStatus::Skipped,
        RunStatus::Queued => CronRunStatus::Queued,
    }
}

fn run_to_wire(run: &Run) -> CronRun {
    CronRun {
        id: run.id.clone(),
        job_id: run.job_id.clone(),
        status: run_status(run.status),
        fired_at: run.fired_at,
        started_at: run.started_at,
        ended_at: run.ended_at,
        exit_code: run.exit_code,
        summary: run.summary.clone(),
        reason: run.reason.clone(),
        session_path: run.session_path.clone(),
        cost_usd: run.cost_usd,
        missed: run.missed,
    }
}

fn next_fire_for(job: &Job, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    if !job.enabled {
        return None;
    }
    let tz = schedule::resolve_tz(&job.tz).ok()?;
    let sched = schedule::Schedule::parse(&job.schedule).ok()?;
    sched.next_after(tz, now, false).map(|dt| dt.to_rfc3339())
}

fn job_to_wire(
    job: &Job,
    running: bool,
    last_run_status: Option<RunStatus>,
    now: chrono::DateTime<chrono::Utc>,
) -> CronJob {
    CronJob {
        id: job.id.clone(),
        name: job.name.clone(),
        prompt: job.prompt.clone(),
        schedule: job.schedule.clone(),
        tz: job.tz.clone(),
        cwd: job.cwd.clone(),
        model: job.model.clone(),
        thinking: job.thinking.clone(),
        enabled: job.enabled,
        time_limit_s: job.time_limit_s,
        cost_cap_usd: job.cost_cap_usd,
        created: job.created,
        updated: job.updated,
        next_fire: next_fire_for(job, now),
        running,
        last_run_status: last_run_status.map(run_status),
        notify: job.notify.as_ref().map(|n| CronNotify {
            channels: n.channels.clone(),
            events: n.events.clone(),
        }),
        // v2: wake links live in the job file (`sessionLink`, written by the CLI's
        // `--wake-session`/`--wake-here` or the extension); the app cannot resolve a live
        // bridge pid any more, so `wake_pid` is rejected on add/edit.
        wake: job.session_link.as_ref().map(wake_to_wire),
    }
}

fn wake_to_wire(link: &SessionLink) -> CronWake {
    CronWake {
        session_file: link.session_file.clone(),
        session_id: link.session_id.clone(),
        pid: None,
        session_name: None,
    }
}

/// `notify` off the wire: both halves absent = the defaults (`None`).
fn notify_from_wire(n: CronNotify) -> Option<NotifySpec> {
    let clean = |v: Option<Vec<String>>| {
        v.map(|v| {
            v.into_iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
    };
    let spec = NotifySpec {
        channels: clean(n.channels),
        events: clean(n.events),
    };
    (spec.channels.is_some() || spec.events.is_some()).then_some(spec)
}

fn last_run_status(paths: &CronPaths, job_id: &str) -> Option<RunStatus> {
    store::load_runs(paths, job_id)
        .ok()?
        .first()
        .map(|r| r.status)
}

/// The form's schedule as typed — plain language ("every day at 9:00", "in 2 hours",
/// "tomorrow at 9"), a 6-field cron or `at:` — resolved the way the CLI does, against the
/// job's zone (else this PC's).
fn resolve_schedule(input: &str, tz: Option<&str>) -> Result<String, AppErrorDto> {
    control::resolve_schedule_input(input, false, tz).map_err(bad_request)
}

fn new_job_from_add(mut req: CronAddRequest) -> Result<NewJob, AppErrorDto> {
    req.schedule = resolve_schedule(&req.schedule, req.tz.as_deref())?;
    // v2: the app bridge is gone, so a pid can no longer be resolved to a session link
    // (wake links carry the session file/id in the job file instead).
    if req.wake_pid.is_some() {
        return Err(bad_request(
            "wake_pid is gone in v2: link the session file/id instead",
        ));
    }
    Ok(NewJob {
        id: req.id.map(|i| i.as_str().to_string()),
        name: req.name.as_str().to_string(),
        prompt: req.prompt.as_str().to_string(),
        schedule: req.schedule,
        tz: req.tz,
        cwd: Some(req.cwd),
        model: req.model,
        thinking: req.thinking,
        enabled: req.enabled,
        time_limit_s: req.time_limit_s,
        cost_cap_usd: req.cost_cap_usd,
        overlap: None,
        missed: None,
        notify: req.notify.and_then(notify_from_wire),
        session_link: None,
    })
}

fn edit_job_from_patch(
    mut req: CronEditRequest,
    current_tz: Option<&str>,
) -> Result<EditJob, AppErrorDto> {
    if let Some(input) = req.schedule.take() {
        req.schedule = Some(resolve_schedule(&input, req.tz.as_deref().or(current_tz))?);
    }
    if req.wake_pid.is_some() {
        return Err(bad_request(
            "wake_pid is gone in v2: link the session file/id instead",
        ));
    }
    Ok(EditJob {
        name: req.name.map(|n| n.as_str().to_string()),
        prompt: req.prompt.map(|p| p.as_str().to_string()),
        schedule: req.schedule,
        tz: req.tz,
        cwd: req.cwd,
        model: req.model.map(Some),
        thinking: req.thinking.map(Some),
        enabled: req.enabled,
        time_limit_s: req.time_limit_s,
        cost_cap_usd: req.cost_cap_usd.map(Some),
        overlap: None,
        missed: None,
        notify: req.notify.map(notify_from_wire),
        // The app only clears a link (`wake_clear`); setting one is the CLI's or the
        // extension's job-file write.
        session_link: req.wake_clear.and_then(|clear| clear.then_some(None)),
    })
}

fn job_err(e: String) -> AppErrorDto {
    if e.starts_with("no job") || e.starts_with("no run") {
        not_found(e)
    } else {
        bad_request(e)
    }
}

/// Handles one `cron_*`/`cron_settings_*` request. Gating: `Peer::Local` (this PC's own
/// app) always; any other device needs the `cron` sharing feature — checked here,
/// uniformly, for every op (read-only included).
pub async fn serve_request<W>(
    paths: &HostPaths,
    cron: &CronHost,
    req: Request,
    device_id: &str,
    device_name: Option<&str>,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if device_id != crate::protocol::LOCAL_DEVICE_ID
        && !crate::sharing::allowed_for(
            paths,
            unipi_proto::FeatureKind::Cron,
            device_id,
            device_name,
        )
    {
        return Err(not_allowed());
    }
    let store_paths = cron.paths();
    let reply = |data: serde_json::Value| Control::Ok { data: Some(data) };
    let empty = || Control::Ok { data: None };
    let now = chrono::Utc::now();
    let running = match cron.scheduler() {
        Some(scheduler) => scheduler.running_job_ids().await,
        None => Vec::new(),
    };
    match req {
        Request::CronList => {
            let jobs = store::load_jobs(store_paths).map_err(internal)?;
            let settings = store::load_settings(store_paths).map_err(internal)?;
            let jobs: Vec<CronJob> = jobs
                .iter()
                .map(|j| {
                    job_to_wire(
                        j,
                        running.contains(&j.id),
                        last_run_status(store_paths, &j.id),
                        now,
                    )
                })
                .collect();
            let data = CronJobsData {
                jobs,
                paused: settings.paused,
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronShow { id } => {
            let jobs = store::load_jobs(store_paths).map_err(internal)?;
            let job = jobs
                .iter()
                .find(|j| j.id == id.as_str())
                .ok_or_else(|| not_found(format!("no job {:?}", id.as_str())))?;
            let runs = store::load_runs(store_paths, id.as_str()).map_err(internal)?;
            let data = CronShowData {
                job: job_to_wire(
                    job,
                    running.contains(&job.id),
                    runs.first().map(|r| r.status),
                    now,
                ),
                runs: runs.iter().map(run_to_wire).collect(),
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronRuns { id, limit } => {
            let mut runs = match &id {
                Some(id) => store::load_runs(store_paths, id.as_str()).map_err(internal)?,
                None => store::load_all_runs(store_paths).map_err(internal)?,
            };
            if let Some(limit) = limit {
                runs.truncate(limit.value());
            }
            let data = CronRunsData {
                runs: runs.iter().map(run_to_wire).collect(),
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronLog { run_id } => {
            let Some((job_id, run)) =
                store::find_run(store_paths, run_id.as_str()).map_err(internal)?
            else {
                return Err(not_found(format!("no run {:?}", run_id.as_str())));
            };
            let log = store::read_log(store_paths, &job_id, run_id.as_str()).unwrap_or_default();
            let data = CronLogData {
                job_id,
                run: run_to_wire(&run),
                log: String::from_utf8_lossy(&log).into_owned(),
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronAdd(add) => {
            let value = cron
                .control(CronControl::Add(Box::new(new_job_from_add(add)?)))
                .await
                .map_err(job_err)?;
            let job: Job = serde_json::from_value(value).map_err(internal)?;
            let data = job_to_wire(
                &job,
                running.contains(&job.id),
                last_run_status(store_paths, &job.id),
                now,
            );
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronEdit { id, patch } => {
            let current_tz = store::load_jobs(store_paths)
                .ok()
                .and_then(|jobs| jobs.into_iter().find(|j| j.id == id.as_str()).map(|j| j.tz));
            let patch = edit_job_from_patch(patch, current_tz.as_deref())?;
            let value = cron
                .control(CronControl::Edit {
                    id: id.as_str().to_string(),
                    patch: Box::new(patch),
                })
                .await
                .map_err(job_err)?;
            let job: Job = serde_json::from_value(value).map_err(internal)?;
            let data = job_to_wire(
                &job,
                running.contains(&job.id),
                last_run_status(store_paths, &job.id),
                now,
            );
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronRm { id } => {
            cron.control(CronControl::Remove {
                id: id.as_str().to_string(),
            })
            .await
            .map_err(job_err)?;
            write_control(write, &empty()).await.map_err(internal)?;
        }
        Request::CronEnable { id } => {
            cron.control(CronControl::Enable {
                id: id.as_str().to_string(),
            })
            .await
            .map_err(job_err)?;
            write_control(write, &empty()).await.map_err(internal)?;
        }
        Request::CronDisable { id } => {
            cron.control(CronControl::Disable {
                id: id.as_str().to_string(),
            })
            .await
            .map_err(job_err)?;
            write_control(write, &empty()).await.map_err(internal)?;
        }
        Request::CronRun { id } => {
            let value = cron
                .control(CronControl::RunNow {
                    id: id.as_str().to_string(),
                })
                .await
                .map_err(job_err)?;
            let data: CronRunStartedData = serde_json::from_value(value).map_err(internal)?;
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronStop { id } => {
            let value = cron
                .control(CronControl::Stop {
                    id: id.as_str().to_string(),
                })
                .await
                .map_err(job_err)?;
            let data: CronStoppedData = serde_json::from_value(value).map_err(internal)?;
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronPauseAll => {
            cron.control(CronControl::PauseAll).await.map_err(job_err)?;
            write_control(write, &empty()).await.map_err(internal)?;
        }
        Request::CronResumeAll => {
            cron.control(CronControl::ResumeAll)
                .await
                .map_err(job_err)?;
            write_control(write, &empty()).await.map_err(internal)?;
        }
        Request::CronStatus => {
            let value = cron.control(CronControl::Status).await.map_err(job_err)?;
            let jobs: Vec<CronJobStatus> = value["jobs"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|j| CronJobStatus {
                    id: j["id"].as_str().unwrap_or_default().to_string(),
                    enabled: j["enabled"].as_bool().unwrap_or(false),
                    next_fire: j["next_fire"].as_str().map(str::to_string),
                    running: j["running"].as_bool().unwrap_or(false),
                })
                .collect();
            let data = CronStatusData {
                paused: value["paused"].as_bool().unwrap_or(false),
                concurrency: value["concurrency"].as_u64().unwrap_or(0) as u32,
                jobs,
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronParse {
            schedule: sched_input,
            at,
            tz,
        } => {
            let tz_zone = schedule::resolve_tz(tz.as_deref().unwrap_or_default())
                .map_err(|e| bad_request(e.0))?;
            let for_at = at.is_some();
            let input = sched_input
                .or(at)
                .ok_or_else(|| bad_request("cron_parse needs `schedule` or `at`"))?;
            let resolved = control::resolve_schedule_input(&input, for_at, Some(tz_zone.name()))
                .map_err(bad_request)?;
            let parsed = schedule::Schedule::parse(&resolved).map_err(|e| bad_request(e.0))?;
            let fires =
                schedule::next_fires(&parsed, tz_zone, now, unipi_proto::CRON_PARSE_PREVIEW)
                    .into_iter()
                    .map(|dt| dt.to_rfc3339())
                    .collect();
            let data = CronParseData {
                expression: resolved,
                one_time: parsed.is_one_time(),
                tz: tz_zone.name().to_string(),
                next_fires: fires,
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronSettingsGet => {
            let settings = store::load_settings(store_paths).map_err(internal)?;
            write_control(
                write,
                &reply(serde_json::to_value(settings_to_wire(paths, &settings)).map_err(internal)?),
            )
            .await
            .map_err(internal)?;
        }
        Request::CronSettingsSet(patch) => {
            let settings = apply_settings_patch(store_paths, patch).map_err(bad_request)?;
            write_control(
                write,
                &reply(serde_json::to_value(settings_to_wire(paths, &settings)).map_err(internal)?),
            )
            .await
            .map_err(internal)?;
        }
        _ => unreachable!("cron_serve::serve_request called with a non-cron request"),
    }
    Ok(())
}

fn notify_channels() -> Vec<String> {
    let path = crate::notify::notify_config_json_path();
    crate::notify::default_channels(
        &crate::notify::read_ntfy_config(&path),
        &crate::notify::read_gotify_config(&path),
        &crate::notify::read_telegram_config(&path),
    )
}

fn settings_to_wire(paths: &HostPaths, settings: &CronSettings) -> CronSettingsData {
    let share = crate::sharing::cron_share(paths);
    CronSettingsData {
        concurrency: settings.concurrency,
        missed_window_s: settings.missed_window_s,
        retention_runs: settings.retention_runs,
        retention_days: settings.retention_days,
        default_time_limit_s: settings.default_time_limit_s,
        paused: settings.paused,
        enabled: share.enabled,
        devices: share.devices,
        ntfy_topic: crate::notify::read_ntfy_config(&crate::notify::notify_config_json_path())
            .topic,
        notify_default_events: settings.notify_default_events.clone(),
        notify_channels: notify_channels(),
    }
}

/// Applies a validated patch to the scheduler's settings (`<agent dir>/cron/settings.json`).
/// Shared by the `cron_settings_set` op and the CLI (`unipi-host cron settings`).
///
/// # Errors
/// The library's user-facing message (out-of-range values, unreadable file).
pub fn apply_settings_patch(
    paths: &CronPaths,
    patch: CronSettingsSetRequest,
) -> Result<CronSettings, String> {
    if patch.ntfy_topic.is_some() {
        return Err(
            "the ntfy topic moved to the notify settings (extensionSettings.notify.ntfy.topic); `cron_settings_set` no longer stores one"
                .to_owned(),
        );
    }
    let mut settings = store::load_settings(paths).map_err(|e| e.to_string())?;
    control::apply_settings_patch(
        &mut settings,
        SettingsPatch {
            concurrency: patch.concurrency,
            missed_window_s: patch.missed_window_s,
            retention_runs: patch.retention_runs,
            retention_days: patch.retention_days,
            default_time_limit_s: patch.default_time_limit_s,
            paused: patch.paused,
            notify_default_events: patch.notify_default_events,
        },
    )?;
    store::save_settings(paths, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

/// The store paths of a host data dir's agent dir (what `serve` and the CLI both use).
#[must_use]
pub fn cron_paths_for_host(data_dir: &Path) -> CronPaths {
    CronPaths::new(crate::sessions::services::agent_dir(data_dir))
}
