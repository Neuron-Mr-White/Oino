//! `cron_*` ops from the app (UNI-317) and `cron_settings_*` (UNI-318):
//! translates the wire [`unipi_proto::Request`] variants to/from the
//! scheduler's own [`crate::cron::CronControl`]/`NewJob`/`EditJob`, and
//! applies the UNI-248 sharing gate — read-only ops (`cron_list`,
//! `cron_show`, `cron_runs`, `cron_log`, `cron_status`, `cron_parse`) and
//! every write op alike need the `cron` feature once the caller isn't
//! this PC's own app.
//!
//! The scheduler's own control ops (used by the CLI's `control.sock`) stay
//! untouched; this module is a thin shim so the merge with the C3/C4/C5
//! work in progress on other branches stays easy (see `TASK.md`).

use unipi_proto::{
    AppErrorCode, AppErrorDto, Control, CronAddRequest, CronEditRequest, CronJob, CronJobStatus,
    CronJobsData, CronLogData, CronNotify, CronParseData, CronRun, CronRunStartedData,
    CronRunStatus, CronRunsData, CronSettingsData, CronSettingsSetRequest, CronShowData,
    CronStatusData, CronStoppedData, CronWake, Request,
};

use crate::cron::model::{Job, NotifySpec, Run, RunStatus, SessionLink};
use crate::cron::scheduler::{CronControl, EditJob, NewJob};
use crate::cron::{schedule, store, Scheduler};
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
        wake: job.session_link.as_ref().map(wake_to_wire),
    }
}

/// The job's linked session (the live-pi lookup moves to the session hub
/// with WP-H8's wake rework; the wire shape stays).
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

fn last_run_status(paths: &HostPaths, job_id: &str) -> Option<RunStatus> {
    store::load_runs(paths, job_id)
        .ok()?
        .first()
        .map(|r| r.status)
}

/// The form's schedule as typed — plain language ("every day at 9:00",
/// "in 2 hours", "tomorrow at 9"), a 6-field cron or `at:` — resolved the
/// way the CLI does, against the job's zone (else this PC's).
fn resolve_schedule(input: &str, tz: Option<&str>) -> Result<String, AppErrorDto> {
    let zone = schedule::resolve_tz(tz.unwrap_or_default()).map_err(bad_request)?;
    let resolved = schedule::parse_human(input, zone, chrono::Utc::now()).map_err(bad_request)?;
    schedule::Schedule::parse(&resolved).map_err(bad_request)?;
    Ok(resolved)
}

fn new_job_from_add(mut req: CronAddRequest) -> Result<NewJob, AppErrorDto> {
    req.schedule = resolve_schedule(&req.schedule, req.tz.as_deref())?;
    // v2: the app bridge is gone, so a pid can no longer be resolved to a
    // session link (wake links carry the session file/id directly now).
    if req.wake_pid.is_some() {
        return Err(bad_request(
            "wake_pid is gone in v2: link the session file/id instead",
        ));
    }
    let session_link = None;
    Ok(NewJob {
        id: req.id.map(|i| i.as_str().to_string()),
        name: req.name.as_str().to_string(),
        prompt: req.prompt.as_str().to_string(),
        schedule: req.schedule,
        tz: req.tz,
        cwd: req.cwd,
        model: req.model,
        thinking: req.thinking,
        enabled: req.enabled,
        time_limit_s: req.time_limit_s,
        cost_cap_usd: req.cost_cap_usd,
        notify: req.notify.and_then(notify_from_wire),
        session_link,
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
    let session_link = match req.wake_clear {
        Some(true) => Some(None),
        _ => None,
    };
    Ok(EditJob {
        name: req.name.map(|n| n.as_str().to_string()),
        prompt: req.prompt.map(|p| p.as_str().to_string()),
        schedule: req.schedule,
        tz: req.tz,
        cwd: req.cwd,
        model: req.model,
        thinking: req.thinking,
        enabled: req.enabled,
        time_limit_s: req.time_limit_s,
        cost_cap_usd: req.cost_cap_usd,
        notify: req.notify.map(notify_from_wire),
        session_link,
    })
}

fn job_err(e: String) -> AppErrorDto {
    if e.starts_with("no job") || e.starts_with("no run") {
        not_found(e)
    } else {
        bad_request(e)
    }
}

/// Handles one `cron_*`/`cron_settings_*` request. Gating: `Peer::Local`
/// (this PC's own app) always; any other device needs the `cron` sharing
/// feature — checked here, uniformly, for every op (read-only included).
pub async fn serve_request<W>(
    paths: &HostPaths,
    cron: &Scheduler,
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
    let reply = |data: serde_json::Value| Control::Ok { data: Some(data) };
    let empty = || Control::Ok { data: None };
    let now = chrono::Utc::now();
    match req {
        Request::CronList => {
            let jobs = store::load_jobs(paths).map_err(internal)?;
            let settings = store::load_settings(&paths.config()).map_err(internal)?;
            let running = cron.running_job_ids().await;
            let jobs: Vec<CronJob> = jobs
                .iter()
                .map(|j| {
                    job_to_wire(
                        j,
                        running.contains(&j.id),
                        last_run_status(paths, &j.id),
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
            let jobs = store::load_jobs(paths).map_err(internal)?;
            let job = jobs
                .iter()
                .find(|j| j.id == id.as_str())
                .ok_or_else(|| not_found(format!("no job {:?}", id.as_str())))?;
            let runs = store::load_runs(paths, id.as_str()).map_err(internal)?;
            let running = cron.running_job_ids().await;
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
                Some(id) => store::load_runs(paths, id.as_str()).map_err(internal)?,
                None => store::load_all_runs(paths).map_err(internal)?,
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
            let Some((job_id, run)) = store::find_run(paths, run_id.as_str()).map_err(internal)?
            else {
                return Err(not_found(format!("no run {:?}", run_id.as_str())));
            };
            let log = store::read_log(paths, &job_id, run_id.as_str()).unwrap_or_default();
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
                .control(CronControl::Add(new_job_from_add(add)?))
                .await
                .map_err(job_err)?;
            let job: Job = serde_json::from_value(value).map_err(internal)?;
            let running = cron.running_job_ids().await;
            let data = job_to_wire(
                &job,
                running.contains(&job.id),
                last_run_status(paths, &job.id),
                now,
            );
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronEdit { id, patch } => {
            let current_tz = store::load_jobs(paths)
                .ok()
                .and_then(|jobs| jobs.into_iter().find(|j| j.id == id.as_str()).map(|j| j.tz));
            let patch = edit_job_from_patch(patch, current_tz.as_deref())?;
            let value = cron
                .control(CronControl::Edit {
                    id: id.as_str().to_string(),
                    patch,
                })
                .await
                .map_err(job_err)?;
            let job: Job = serde_json::from_value(value).map_err(internal)?;
            let running = cron.running_job_ids().await;
            let data = job_to_wire(
                &job,
                running.contains(&job.id),
                last_run_status(paths, &job.id),
                now,
            );
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronRm { id } => {
            cron.control(CronControl::Rm {
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
            let tz_zone =
                schedule::resolve_tz(tz.as_deref().unwrap_or_default()).map_err(bad_request)?;
            let for_at = at.is_some();
            let input = sched_input
                .or(at)
                .ok_or_else(|| bad_request("cron_parse needs `schedule` or `at`"))?;
            let resolved = if for_at {
                match schedule::parse_human(&input, tz_zone, now) {
                    Ok(resolved) if resolved.starts_with("at:") => resolved,
                    _ => {
                        let bare = format!("at:{input}");
                        schedule::Schedule::parse(&bare).map_err(bad_request)?;
                        bare
                    }
                }
            } else {
                schedule::parse_human(&input, tz_zone, now).map_err(bad_request)?
            };
            let parsed = schedule::Schedule::parse(&resolved).map_err(bad_request)?;
            let fires =
                schedule::next_fires(&parsed, tz_zone, now, unipi_proto::CRON_PARSE_PREVIEW)
                    .into_iter()
                    .map(|dt| dt.to_rfc3339())
                    .collect();
            let data = CronParseData {
                expression: resolved,
                one_time: parsed.is_one_time(),
                tz: tz_zone.to_string(),
                next_fires: fires,
            };
            write_control(write, &reply(serde_json::to_value(data).map_err(internal)?))
                .await
                .map_err(internal)?;
        }
        Request::CronSettingsGet => {
            let settings = store::load_settings(&paths.config()).map_err(internal)?;
            write_control(
                write,
                &reply(serde_json::to_value(settings_to_wire(&settings)).map_err(internal)?),
            )
            .await
            .map_err(internal)?;
        }
        Request::CronSettingsSet(patch) => {
            let settings = apply_settings_patch(paths, patch).map_err(bad_request)?;
            write_control(
                write,
                &reply(serde_json::to_value(settings_to_wire(&settings)).map_err(internal)?),
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

fn settings_to_wire(settings: &crate::cron::CronSettings) -> CronSettingsData {
    CronSettingsData {
        concurrency: settings.concurrency,
        missed_window_s: settings.missed_window_s,
        retention_runs: settings.retention_runs,
        retention_days: settings.retention_days,
        default_time_limit_s: settings.default_time_limit_s,
        paused: settings.paused,
        enabled: settings.enabled,
        devices: settings.devices.clone(),
        ntfy_topic: settings.ntfy_topic.clone(),
        notify_default_events: settings.notify_default_events.clone(),
        notify_channels: notify_channels(),
    }
}

/// Applies a validated patch to the `cron` settings block. Shared by the
/// `cron_settings_set` op and the CLI (`unipi-host cron settings`).
pub fn apply_settings_patch(
    paths: &HostPaths,
    patch: CronSettingsSetRequest,
) -> Result<crate::cron::CronSettings, String> {
    let path = paths.config();
    let mut settings = store::load_settings(&path).map_err(|e| e.to_string())?;
    if let Some(v) = patch.concurrency {
        if v == 0 {
            return Err("concurrency must be at least 1".into());
        }
        settings.concurrency = v;
    }
    if let Some(v) = patch.missed_window_s {
        settings.missed_window_s = v;
    }
    if let Some(v) = patch.retention_runs {
        if v == 0 {
            return Err("retention_runs must be at least 1".into());
        }
        settings.retention_runs = v;
    }
    if let Some(v) = patch.retention_days {
        if v == 0 {
            return Err("retention_days must be at least 1".into());
        }
        settings.retention_days = v;
    }
    if let Some(v) = patch.default_time_limit_s {
        if v == 0 {
            return Err("default_time_limit_s must be at least 1".into());
        }
        settings.default_time_limit_s = v;
    }
    if let Some(v) = patch.paused {
        settings.paused = v;
    }
    if let Some(v) = patch.ntfy_topic {
        settings.ntfy_topic = (!v.trim().is_empty()).then(|| v.trim().to_string());
    }
    if let Some(v) = patch.notify_default_events {
        settings.notify_default_events = v;
    }
    store::save_settings(&path, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}
