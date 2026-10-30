//! The cron scheduler owner: ticks, decides fires, runs jobs, and the
//! control ops for the CLI (UNI-314).
//!
//! Pure decision logic ([`next_due_fire`], [`ConcurrencyGate`]) is kept
//! separate from the async machinery so it's unit-testable without a
//! tick loop or real processes.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::model::{CronSettings, Job, Run, RunStatus};
use super::schedule::{resolve_tz, validate, Schedule};
use super::{exec, store};
use crate::data_dir::HostPaths;

// ------------------------------------------------------------- pure logic

/// One fire that is due now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueFire {
    pub at_ms: u64,
    pub missed: bool,
}

/// Decides whether a job is due to fire, collapsing several missed
/// occurrences into the single latest one and dropping it silently if
/// that one is older than `missed_window_s` (UNI-314: "never catch up
/// several"). A fire counts as `missed` once it's more than
/// `tolerance_s` in the past by the time we notice it — ordinary tick
/// cadence keeps on-time fires under that, so only a real gap (host
/// down, system sleep) is flagged.
pub fn next_due_fire(
    schedule: &Schedule,
    tz: chrono_tz::Tz,
    last_known_ms: u64,
    now_ms: u64,
    missed_window_s: u64,
    tolerance_s: u64,
) -> Option<DueFire> {
    let last_known = ms_to_dt(last_known_ms);
    let now = ms_to_dt(now_ms);
    // Walk backward from `now` (inclusive) to find the latest occurrence
    // at or before it -- the single fire to run, however many were missed
    // in between. Walking forward from `last_known` instead would mean
    // iterating every occurrence since then, which is unbounded when
    // `last_known` is old (e.g. a job's `created` time at epoch 0).
    let due = schedule.last_at_or_before(tz, now)?;
    if due <= last_known {
        return None; // already ran (or scheduled before it ever existed)
    }
    let age_s = (now - due).num_seconds().max(0) as u64;
    let missed = age_s > tolerance_s;
    if missed && age_s > missed_window_s {
        return None;
    }
    Some(DueFire {
        at_ms: dt_to_ms(due),
        missed,
    })
}

fn ms_to_dt(ms: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms as i64).unwrap_or_else(Utc::now)
}
fn dt_to_ms(dt: DateTime<Utc>) -> u64 {
    dt.timestamp_millis().max(0) as u64
}

/// A FIFO concurrency limiter: at most `capacity` running at once, the
/// rest wait in order.
#[derive(Debug, Clone)]
pub struct ConcurrencyGate {
    capacity: usize,
    in_flight: usize,
    queue: VecDeque<String>,
}

impl ConcurrencyGate {
    pub fn new(capacity: usize) -> Self {
        ConcurrencyGate {
            capacity: capacity.max(1),
            in_flight: 0,
            queue: VecDeque::new(),
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
    }

    /// Tries to start `id` now. `true` = go ahead; `false` = queued
    /// (FIFO; call [`finish`] to pop the next one once a slot frees up).
    pub fn try_start(&mut self, id: &str) -> bool {
        if self.in_flight < self.capacity {
            self.in_flight += 1;
            true
        } else {
            self.queue.push_back(id.to_string());
            false
        }
    }

    /// A running job ended; returns the next queued id to start, if any
    /// (the caller must actually start it; `in_flight` stays accounted
    /// for that slot).
    pub fn finish(&mut self) -> Option<String> {
        self.in_flight = self.in_flight.saturating_sub(1);
        if let Some(next) = self.queue.pop_front() {
            self.in_flight += 1;
            Some(next)
        } else {
            None
        }
    }

    pub fn queued(&self) -> &VecDeque<String> {
        &self.queue
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight
    }
}

// ------------------------------------------------------------- scheduler

/// An injectable clock (system by default; fixed in tests).
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>);

impl Clock {
    pub fn system() -> Self {
        Clock(Arc::new(Utc::now))
    }

    pub fn fixed(shared: Arc<Mutex<DateTime<Utc>>>) -> Self {
        Clock(Arc::new(move || {
            // `try_lock`: the clock is read far more often than set in
            // tests, and this is never called from inside the lock.
            match shared.try_lock() {
                Ok(guard) => *guard,
                Err(_) => Utc::now(),
            }
        }))
    }

    pub fn now(&self) -> DateTime<Utc> {
        (self.0)()
    }

    pub fn now_ms(&self) -> u64 {
        dt_to_ms(self.now())
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::system()
    }
}

/// One job's in-flight reservation (queued or running).
struct Active {
    run_id: String,
    pid: Option<u32>,
}

struct SchedulerState {
    active: HashMap<String, Active>,
    gate: ConcurrencyGate,
}

/// Owns the cron tick loop and the control ops.
#[derive(Clone)]
pub struct Scheduler {
    paths: HostPaths,
    clock: Clock,
    state: Arc<Mutex<SchedulerState>>,
    run_seq: Arc<AtomicU32>,
    /// How often [`Scheduler::tick`] is called by the owner (`serve`);
    /// used as the on-time tolerance for [`next_due_fire`].
    pub tick_interval: Duration,
}

impl std::fmt::Debug for Scheduler {
    // `Clock` wraps an `Arc<dyn Fn>`, which isn't `Debug`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    pub fn new(paths: HostPaths) -> Self {
        Scheduler::with_clock(paths, Clock::system())
    }

    pub fn with_clock(paths: HostPaths, clock: Clock) -> Self {
        Scheduler {
            paths,
            clock,
            state: Arc::new(Mutex::new(SchedulerState {
                active: HashMap::new(),
                gate: ConcurrencyGate::new(2),
            })),
            run_seq: Arc::new(AtomicU32::new(0)),
            tick_interval: Duration::from_secs(1),
        }
    }

    fn new_run_id(&self) -> String {
        let n = self.run_seq.fetch_add(1, Ordering::Relaxed);
        format!("r{:x}{:04x}", self.clock.now_ms(), n)
    }

    fn settings(&self) -> CronSettings {
        store::load_settings(&self.paths.config()).unwrap_or_default()
    }

    /// One scheduling pass: checks every enabled job, starts due fires
    /// (respecting overlap/pause/concurrency), and prunes nothing itself
    /// (pruning happens right after each run finishes). Called on a
    /// timer by `serve`; also harmless to call back-to-back in tests.
    pub async fn tick(&self) {
        let settings = self.settings();
        if settings.paused {
            return;
        }
        let Ok(jobs) = store::load_jobs(&self.paths) else {
            return;
        };
        let now_ms = self.clock.now_ms();
        for job in jobs {
            if !job.enabled {
                continue;
            }
            let Ok(tz) = resolve_tz(&job.tz) else {
                continue;
            };
            let Ok(schedule) = Schedule::parse(&job.schedule) else {
                continue;
            };
            let last_known = store::load_runs(&self.paths, &job.id)
                .ok()
                .and_then(|r| r.first().map(|r| r.fired_at))
                .unwrap_or(job.created);
            let tolerance_s = self.tick_interval.as_secs().max(1) * 3;
            let Some(due) = next_due_fire(
                &schedule,
                tz,
                last_known,
                now_ms,
                settings.missed_window_s,
                tolerance_s,
            ) else {
                continue;
            };
            self.consider_fire(&job, due.at_ms, due.missed, settings.concurrency as usize)
                .await;
        }
    }

    /// Reserves `job` for one fire (or records it `skipped` on overlap),
    /// then starts it or queues it per concurrency.
    async fn consider_fire(&self, job: &Job, fired_at: u64, missed: bool, concurrency: usize) {
        let mut state = self.state.lock().await;
        state.gate.set_capacity(concurrency);
        if state.active.contains_key(&job.id) {
            drop(state);
            let run = Run {
                id: self.new_run_id(),
                job_id: job.id.clone(),
                status: RunStatus::Skipped,
                fired_at,
                started_at: None,
                ended_at: Some(self.clock.now_ms()),
                exit_code: None,
                summary: None,
                reason: Some("previous run still going".into()),
                session_path: None,
                cost_usd: None,
                missed,
                notify_errors: Vec::new(),
                wake: None,
            };
            let _ = store::save_run(&self.paths, &run);
            tracing::info!(
                op = "cron_fire",
                job = job.id,
                run = run.id,
                status = "skipped",
                "overlap"
            );
            return;
        }
        let run_id = self.new_run_id();
        state.active.insert(
            job.id.clone(),
            Active {
                run_id: run_id.clone(),
                pid: None,
            },
        );
        let go = state.gate.try_start(&job.id);
        drop(state);
        if go {
            self.start_run(job.clone(), run_id, fired_at, missed).await;
        } else {
            let run = Run {
                id: run_id,
                job_id: job.id.clone(),
                status: RunStatus::Queued,
                fired_at,
                started_at: None,
                ended_at: None,
                exit_code: None,
                summary: None,
                reason: None,
                session_path: None,
                cost_usd: None,
                missed,
                notify_errors: Vec::new(),
                wake: None,
            };
            let _ = store::save_run(&self.paths, &run);
        }
    }

    /// Actually runs `job` now (used by both scheduled fires and
    /// `run_now`, which bypasses the schedule check but still goes
    /// through overlap + concurrency).
    ///
    /// Boxed: this is part of a `start_run` → `release` → `start_queued` →
    /// `start_run` cycle (the next queued job starts as soon as a slot
    /// frees up), which would otherwise make an infinitely-sized future.
    fn start_run<'a>(
        &'a self,
        job: Job,
        run_id: String,
        fired_at: u64,
        missed: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(self.start_run_inner(job, run_id, fired_at, missed))
    }

    async fn start_run_inner(&self, job: Job, run_id: String, fired_at: u64, missed: bool) {
        let started_at = self.clock.now_ms();
        let run = Run {
            id: run_id.clone(),
            job_id: job.id.clone(),
            status: RunStatus::Running,
            fired_at,
            started_at: Some(started_at),
            ended_at: None,
            exit_code: None,
            summary: None,
            reason: None,
            session_path: None,
            cost_usd: None,
            missed,
            notify_errors: Vec::new(),
            wake: None,
        };
        if store::save_run(&self.paths, &run).is_err() {
            tracing::warn!(
                job = job.id,
                run = run_id,
                "couldn't record the run starting"
            );
        }
        tracing::info!(
            op = "cron_fire",
            job = job.id,
            run = run_id,
            status = "running",
            missed,
            "start"
        );

        let this = self.clone();
        let job2 = job.clone();
        let run_id2 = run_id.clone();
        tokio::spawn(async move {
            let spec = exec::RunSpec {
                prompt: &job2.prompt,
                cwd: &job2.cwd,
                model: job2.model.as_deref(),
                thinking: job2.thinking.as_deref(),
                time_limit_s: job2.time_limit_s,
                cost_cap_usd: job2.cost_cap_usd,
            };
            let pid_recorder = this.clone();
            let pid_job_id = job2.id.clone();
            let outcome = exec::run(&this.paths, &job2.id, &run_id2, &spec, move |pid| {
                let pid_recorder = pid_recorder.clone();
                let pid_job_id = pid_job_id.clone();
                tokio::spawn(async move { pid_recorder.record_pid(&pid_job_id, pid).await });
            })
            .await;
            let ended_at = this.clock.now_ms();
            let mut run = store::load_run(&this.paths, &job2.id, &run_id2)
                .unwrap_or(None)
                .unwrap_or(run);
            exec::finish(&mut run, outcome, started_at, ended_at);
            let _ = store::save_run(&this.paths, &run);
            tracing::info!(
                op = "cron_fire",
                job = job2.id,
                run = run_id2,
                status = run.status.as_str(),
                "finished"
            );

            // A one-time `at` job disables itself once it has run.
            if let Ok(schedule) = Schedule::parse(&job2.schedule) {
                if schedule.is_one_time() {
                    let _ = this.set_enabled(&job2.id, false).await;
                }
            }

            let settings = this.settings();

            // UNI-316: best-effort notify + session wake, after the run
            // record already exists -- neither ever turns a run itself
            // into a failure, both are just recorded on it.
            let channels = super::notify::Channels::load(&this.paths, &settings);
            run.notify_errors = super::notify::dispatch(&job2, &run, &channels).await;
            // v2: bridge-poking wake is gone; the hub wake link lands with
            // WP-H8 (host.md §9.8). `run.wake` stays in the wire model.
            let _ = store::save_run(&this.paths, &run);

            let _ = store::prune_runs(
                &this.paths,
                &job2.id,
                settings.retention_runs,
                settings.retention_days,
                ended_at,
            );

            this.release(&job2.id).await;
        });
    }

    /// A run finished (or was never started): frees its slot and starts
    /// the next queued job for it, if any.
    async fn release(&self, job_id: &str) {
        let mut state = self.state.lock().await;
        state.active.remove(job_id);
        let next = state.gate.finish();
        drop(state);
        if let Some(next_job_id) = next {
            self.start_queued(&next_job_id).await;
        }
    }

    async fn start_queued(&self, job_id: &str) {
        let Ok(jobs) = store::load_jobs(&self.paths) else {
            return;
        };
        let Some(job) = jobs.into_iter().find(|j| j.id == job_id) else {
            return;
        };
        let state = self.state.lock().await;
        let Some(run_id) = state.active.get(job_id).map(|a| a.run_id.clone()) else {
            return;
        };
        drop(state);
        let fired_at = store::load_run(&self.paths, job_id, &run_id)
            .ok()
            .flatten()
            .map(|r| r.fired_at)
            .unwrap_or_else(|| self.clock.now_ms());
        let missed = store::load_run(&self.paths, job_id, &run_id)
            .ok()
            .flatten()
            .map(|r| r.missed)
            .unwrap_or(false);
        self.start_run(job, run_id, fired_at, missed).await;
    }

    async fn set_enabled(&self, job_id: &str, enabled: bool) -> anyhow::Result<()> {
        let mut jobs = store::load_jobs(&self.paths)?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
            job.enabled = enabled;
            job.updated = self.clock.now_ms();
        }
        store::save_jobs(&self.paths, &jobs)
    }

    /// Kills a running job's process group (UNIX) or process (Windows).
    pub async fn stop(&self, job_id: &str) -> bool {
        let state = self.state.lock().await;
        let Some(pid) = state.active.get(job_id).and_then(|a| a.pid) else {
            return false;
        };
        drop(state);
        #[cfg(unix)]
        {
            let pgid = nix::unistd::Pid::from_raw(pid as i32);
            let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
        }
        #[cfg(not(unix))]
        {
            super::exec::kill_tree(pid);
        }
        true
    }

    /// Records the pid of a just-started run so `stop` can kill it.
    pub async fn record_pid(&self, job_id: &str, pid: u32) {
        let mut state = self.state.lock().await;
        if let Some(active) = state.active.get_mut(job_id) {
            active.pid = Some(pid);
        }
    }

    pub async fn running_job_ids(&self) -> Vec<String> {
        self.state.lock().await.active.keys().cloned().collect()
    }

    /// Runs `job` right now, bypassing the schedule (still subject to
    /// overlap + concurrency). Returns the new run id, or an error if
    /// the job is already running/queued.
    pub async fn run_now(&self, job: &Job) -> Result<String, String> {
        let concurrency = self.settings().concurrency as usize;
        let mut state = self.state.lock().await;
        state.gate.set_capacity(concurrency);
        if state.active.contains_key(&job.id) {
            return Err("already running".into());
        }
        let run_id = self.new_run_id();
        state.active.insert(
            job.id.clone(),
            Active {
                run_id: run_id.clone(),
                pid: None,
            },
        );
        let go = state.gate.try_start(&job.id);
        drop(state);
        let fired_at = self.clock.now_ms();
        if go {
            self.start_run(job.clone(), run_id.clone(), fired_at, false)
                .await;
        } else {
            let run = Run {
                id: run_id.clone(),
                job_id: job.id.clone(),
                status: RunStatus::Queued,
                fired_at,
                started_at: None,
                ended_at: None,
                exit_code: None,
                summary: None,
                reason: None,
                session_path: None,
                cost_usd: None,
                missed: false,
                notify_errors: Vec::new(),
                wake: None,
            };
            let _ = store::save_run(&self.paths, &run);
        }
        Ok(run_id)
    }
}

// ------------------------------------------------------------- control ops

/// One `cron` request over `control.sock` (mirrors `browser::BrowserControl`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CronControl {
    List,
    Show {
        id: String,
    },
    Add(NewJob),
    Edit {
        id: String,
        patch: EditJob,
    },
    Rm {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    RunNow {
        id: String,
    },
    Stop {
        id: String,
    },
    Runs {
        #[serde(default)]
        job_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// A run's captured output (stdout+stderr, capped at 64 KB) — the run
    /// id alone is enough; its job is found by scanning run records.
    Log {
        run_id: String,
    },
    PauseAll,
    ResumeAll,
    Status,
    /// UNI-316: sends one real test notification for `id`'s channels
    /// (`notify.channels`, or the user's default set).
    TestNotify {
        id: String,
    },
}

/// Fields for `cron add`; `id` is optional (a slug is made from `name`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewJob {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    #[serde(default)]
    pub tz: Option<String>,
    pub cwd: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub time_limit_s: Option<u64>,
    #[serde(default)]
    pub cost_cap_usd: Option<f64>,
    /// UNI-316.
    #[serde(default)]
    pub notify: Option<super::model::NotifySpec>,
    /// UNI-316.
    #[serde(default)]
    pub session_link: Option<super::model::SessionLink>,
}

/// Fields for `cron edit`; every field `None` keeps the current value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditJob {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub time_limit_s: Option<u64>,
    #[serde(default)]
    pub cost_cap_usd: Option<f64>,
    /// UNI-316; `Some(None)` clears it (set `--notify none`/`--notify-on
    /// none`), `None` leaves whatever is stored.
    #[serde(default)]
    pub notify: Option<Option<super::model::NotifySpec>>,
    /// UNI-316; same `Some(None)` clearing convention.
    #[serde(default)]
    pub session_link: Option<Option<super::model::SessionLink>>,
}

/// Event sent back on `control.sock` for a `cron` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CronEvent {
    Data { data: serde_json::Value },
    Error { message: String },
}

fn slugify(name: &str) -> String {
    let mut slug: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug = slug.trim_matches('-').to_string();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    if slug.is_empty() {
        slug = "job".to_string();
    }
    slug.chars().take(40).collect()
}

fn unique_id(jobs: &[Job], base: &str) -> String {
    if !jobs.iter().any(|j| j.id == base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !jobs.iter().any(|j| j.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", rand::random::<u16>())
}

fn job_json(job: &Job) -> serde_json::Value {
    serde_json::to_value(job).unwrap_or_default()
}

fn run_json(run: &Run) -> serde_json::Value {
    serde_json::to_value(run).unwrap_or_default()
}

impl Scheduler {
    /// Runs one `cron` control request; `Ok` is the JSON for the CLI.
    pub async fn control(&self, request: CronControl) -> Result<serde_json::Value, String> {
        match request {
            CronControl::List => {
                let jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "jobs": jobs.iter().map(job_json).collect::<Vec<_>>() }))
            }
            CronControl::Show { id } => {
                let jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let job = jobs
                    .iter()
                    .find(|j| j.id == id)
                    .ok_or_else(|| format!("no job {id:?}"))?;
                let runs = store::load_runs(&self.paths, &id).map_err(|e| e.to_string())?;
                Ok(
                    serde_json::json!({ "job": job_json(job), "runs": runs.iter().map(run_json).collect::<Vec<_>>() }),
                )
            }
            CronControl::Add(new) => {
                let mut jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let tz = new.tz.unwrap_or_else(super::schedule::system_tz_name);
                validate(&new.schedule, &tz, &new.cwd)?;
                let id = match new.id {
                    Some(id) => {
                        if jobs.iter().any(|j| j.id == id) {
                            return Err(format!("a job {id:?} already exists"));
                        }
                        id
                    }
                    None => unique_id(&jobs, &slugify(&new.name)),
                };
                let now = self.clock.now_ms();
                let job = Job {
                    id,
                    name: new.name,
                    prompt: new.prompt,
                    schedule: new.schedule,
                    tz,
                    cwd: new.cwd,
                    model: new.model,
                    thinking: new.thinking,
                    enabled: new.enabled.unwrap_or(true),
                    time_limit_s: new
                        .time_limit_s
                        .unwrap_or(super::model::DEFAULT_TIME_LIMIT_S),
                    cost_cap_usd: new.cost_cap_usd,
                    notify: new.notify,
                    session_link: new.session_link,
                    created: now,
                    updated: now,
                };
                let result = job_json(&job);
                jobs.push(job);
                store::save_jobs(&self.paths, &jobs).map_err(|e| e.to_string())?;
                Ok(result)
            }
            CronControl::Edit { id, patch } => {
                let mut jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let job = jobs
                    .iter_mut()
                    .find(|j| j.id == id)
                    .ok_or_else(|| format!("no job {id:?}"))?;
                if let Some(v) = patch.name {
                    job.name = v;
                }
                if let Some(v) = patch.prompt {
                    job.prompt = v;
                }
                if let Some(v) = patch.schedule {
                    job.schedule = v;
                }
                if let Some(v) = patch.tz {
                    job.tz = v;
                }
                if let Some(v) = patch.cwd {
                    job.cwd = v;
                }
                if patch.model.is_some() {
                    job.model = patch.model;
                }
                if patch.thinking.is_some() {
                    job.thinking = patch.thinking;
                }
                if let Some(v) = patch.enabled {
                    job.enabled = v;
                }
                if let Some(v) = patch.time_limit_s {
                    job.time_limit_s = v;
                }
                if patch.cost_cap_usd.is_some() {
                    job.cost_cap_usd = patch.cost_cap_usd;
                }
                if let Some(v) = patch.notify {
                    job.notify = v;
                }
                if let Some(v) = patch.session_link {
                    job.session_link = v;
                }
                validate(&job.schedule, &job.tz, &job.cwd)?;
                job.updated = self.clock.now_ms();
                let result = job_json(job);
                store::save_jobs(&self.paths, &jobs).map_err(|e| e.to_string())?;
                Ok(result)
            }
            CronControl::Rm { id } => {
                let mut jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let before = jobs.len();
                jobs.retain(|j| j.id != id);
                if jobs.len() == before {
                    return Err(format!("no job {id:?}"));
                }
                store::save_jobs(&self.paths, &jobs).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_dir_all(store::runs_dir(&self.paths, &id));
                Ok(serde_json::json!({ "removed": id }))
            }
            CronControl::Enable { id } => {
                self.set_enabled(&id, true)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "id": id, "enabled": true }))
            }
            CronControl::Disable { id } => {
                self.set_enabled(&id, false)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "id": id, "enabled": false }))
            }
            CronControl::RunNow { id } => {
                let jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let job = jobs
                    .iter()
                    .find(|j| j.id == id)
                    .ok_or_else(|| format!("no job {id:?}"))?;
                let run_id = self.run_now(job).await?;
                Ok(serde_json::json!({ "id": id, "run_id": run_id }))
            }
            CronControl::Stop { id } => {
                let stopped = self.stop(&id).await;
                Ok(serde_json::json!({ "id": id, "stopped": stopped }))
            }
            CronControl::Runs { job_id, limit } => {
                let mut runs = match &job_id {
                    Some(id) => store::load_runs(&self.paths, id).map_err(|e| e.to_string())?,
                    None => store::load_all_runs(&self.paths).map_err(|e| e.to_string())?,
                };
                if let Some(limit) = limit {
                    runs.truncate(limit);
                }
                Ok(serde_json::json!({ "runs": runs.iter().map(run_json).collect::<Vec<_>>() }))
            }
            CronControl::Log { run_id } => {
                let Some((job_id, run)) =
                    store::find_run(&self.paths, &run_id).map_err(|e| e.to_string())?
                else {
                    return Err(format!("no run {run_id:?}"));
                };
                let log = store::read_log(&self.paths, &job_id, &run_id).unwrap_or_default();
                Ok(serde_json::json!({
                    "job_id": job_id,
                    "run": run_json(&run),
                    "log": String::from_utf8_lossy(&log),
                }))
            }
            CronControl::PauseAll => {
                let mut settings = self.settings();
                settings.paused = true;
                store::save_settings(&self.paths.config(), &settings).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "paused": true }))
            }
            CronControl::ResumeAll => {
                let mut settings = self.settings();
                settings.paused = false;
                store::save_settings(&self.paths.config(), &settings).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "paused": false }))
            }
            CronControl::Status => {
                let jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let settings = self.settings();
                let now = self.clock.now();
                let running = self.running_job_ids().await;
                let jobs_status: Vec<serde_json::Value> = jobs
                    .iter()
                    .map(|job| {
                        let next = if job.enabled {
                            resolve_tz(&job.tz)
                                .ok()
                                .zip(Schedule::parse(&job.schedule).ok())
                                .and_then(|(tz, sched)| sched.next_after(tz, now, false))
                                .map(|dt| dt.to_rfc3339())
                        } else {
                            None
                        };
                        serde_json::json!({
                            "id": job.id,
                            "enabled": job.enabled,
                            "next_fire": next,
                            "running": running.contains(&job.id),
                        })
                    })
                    .collect();
                Ok(
                    serde_json::json!({ "paused": settings.paused, "concurrency": settings.concurrency, "jobs": jobs_status }),
                )
            }
            CronControl::TestNotify { id } => {
                let jobs = store::load_jobs(&self.paths).map_err(|e| e.to_string())?;
                let job = jobs
                    .iter()
                    .find(|j| j.id == id)
                    .ok_or_else(|| format!("no job {id:?}"))?;
                let settings = self.settings();
                let channels_cfg = super::notify::Channels::load(&self.paths, &settings);
                let selected =
                    super::notify::resolve_channels(job, &channels_cfg.default_channels());
                if selected.is_empty() {
                    return Err("no notify channels configured for this job (and none of the user's channels are enabled)".to_string());
                }
                let test_run = Run {
                    id: format!("test-{}", self.new_run_id()),
                    job_id: job.id.clone(),
                    status: RunStatus::Ok,
                    fired_at: self.clock.now_ms(),
                    started_at: Some(self.clock.now_ms()),
                    ended_at: Some(self.clock.now_ms()),
                    exit_code: Some(0),
                    summary: Some(
                        "This is a test notification from `unipi-host cron test-notify`."
                            .to_string(),
                    ),
                    reason: None,
                    session_path: None,
                    cost_usd: None,
                    missed: false,
                    notify_errors: Vec::new(),
                    wake: None,
                };
                // Bypass should_notify's events filter (a test should
                // always send, whatever notify.events says) by sending on
                // every selected channel directly.
                let (title, body) = super::notify::build_message(job, &test_run);
                let mut errors = Vec::new();
                for channel in &selected {
                    let result: anyhow::Result<()> = match channel.as_str() {
                        "ntfy" => (async {
                            let key = channels_cfg.key.ok_or_else(|| anyhow::anyhow!("no notify key"))?;
                            let topic = channels_cfg.ntfy_topic.as_deref().or(channels_cfg.ntfy.topic.as_deref());
                            let connection = crate::notify::NtfyConnection { topic: topic.map(str::to_string), ..channels_cfg.ntfy.clone() };
                            let payload = serde_json::json!({ "title": title, "body": body, "kind": "cron", "job_id": job.id, "run_id": test_run.id, "status": "ok" });
                            crate::notify::send_payload(&connection, &channels_cfg.endpoints, &key, &payload).await.map(|_| ())
                        })
                        .await,
                        "gotify" => crate::notify::send_gotify(&channels_cfg.gotify, &title, &body, 5).await,
                        "telegram" => crate::notify::send_telegram(&channels_cfg.telegram, &title, &body).await,
                        other => Err(anyhow::anyhow!("unknown notify channel {other:?}")),
                    };
                    if let Err(error) = result {
                        errors.push(format!("{channel}: {error}"));
                    }
                }
                Ok(serde_json::json!({ "id": id, "channels": selected, "errors": errors }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::schedule::Schedule;

    fn dt(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn overlap_is_not_in_pure_fire_logic() {
        // (overlap is tested at the Scheduler level below)
    }

    #[test]
    fn due_fire_on_time_is_not_missed() {
        let sched = Schedule::parse("0 * * * * *").unwrap(); // every minute
        let tz: chrono_tz::Tz = chrono_tz::UTC;
        let last = dt_to_ms(dt("2024-01-01T00:00:00Z"));
        let now = dt_to_ms(dt("2024-01-01T00:01:01Z")); // 1s after due
        let due = next_due_fire(&sched, tz, last, now, 21_600, 3).unwrap();
        assert!(!due.missed);
        assert_eq!(due.at_ms, dt_to_ms(dt("2024-01-01T00:01:00Z")));
    }

    #[test]
    fn missed_window_logic_collapses_and_drops_old_fires() {
        // A daily job: the host sleeps through several midnights, then
        // wakes. Several fires were missed; only the latest should run,
        // and only if it's still within the window.
        let sched = Schedule::parse("0 0 0 * * *").unwrap(); // daily at 00:00
        let tz = chrono_tz::UTC;
        let last = dt_to_ms(dt("2023-12-01T00:00:00Z"));
        // The host wakes 2 days + 2 hours later: two midnights were missed
        // (12-02 and 12-03), the latest one 2h ago -- within the 6h window.
        let now = dt_to_ms(dt("2023-12-03T02:00:00Z"));
        let due = next_due_fire(&sched, tz, last, now, 21_600, 3).unwrap();
        assert!(due.missed);
        assert_eq!(
            due.at_ms,
            dt_to_ms(dt("2023-12-03T00:00:00Z")),
            "collapses to the single latest missed fire, not every one"
        );
        // Beyond the window (8h since the latest missed midnight): nothing runs.
        let far_now = dt_to_ms(dt("2023-12-03T08:00:05Z"));
        assert!(next_due_fire(&sched, tz, last, far_now, 21_600, 3).is_none());
    }

    #[test]
    fn concurrency_gate_queues_fifo_and_drains_on_finish() {
        let mut gate = ConcurrencyGate::new(2);
        assert!(gate.try_start("a"));
        assert!(gate.try_start("b"));
        assert!(!gate.try_start("c"));
        assert_eq!(gate.queued().front().map(String::as_str), Some("c"));
        assert_eq!(gate.in_flight(), 2);
        assert_eq!(gate.finish(), Some("c".to_string()));
        assert_eq!(gate.in_flight(), 2);
        assert_eq!(gate.finish(), None);
        assert_eq!(gate.in_flight(), 1);
        assert_eq!(gate.finish(), None);
        assert_eq!(gate.in_flight(), 0);
    }

    #[tokio::test]
    async fn overlap_fire_is_recorded_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        let scheduler = Scheduler::new(paths.clone());
        let job = Job {
            id: "j".into(),
            name: "j".into(),
            prompt: "p".into(),
            schedule: "0 * * * * *".into(),
            tz: "UTC".into(),
            cwd: tmp.path().to_string_lossy().into_owned(),
            model: None,
            thinking: None,
            enabled: true,
            time_limit_s: 10,
            cost_cap_usd: None,
            notify: None,
            session_link: None,
            created: 0,
            updated: 0,
        };
        // Reserve the job as if already running (no execution).
        {
            let mut state = scheduler.state.lock().await;
            state.active.insert(
                "j".into(),
                Active {
                    run_id: "running-one".into(),
                    pid: None,
                },
            );
        }
        scheduler.consider_fire(&job, 1000, false, 2).await;
        let runs = store::load_runs(&paths, "j").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Skipped);
        assert_eq!(runs[0].reason.as_deref(), Some("previous run still going"));
    }
}
