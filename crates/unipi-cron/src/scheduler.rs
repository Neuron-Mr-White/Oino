//! The scheduler owner: ticks, decides fires, runs jobs, and owns run lifecycle.
//!
//! Pure decision logic ([`next_due_fire`], [`ConcurrencyGate`]) is kept separate from the
//! async machinery so it's unit-testable without a tick loop or real processes.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::error::Result;
use crate::exec;
use crate::model::{CronSettings, Job, MissedPolicy, OverlapPolicy, Run, RunStatus};
use crate::paths::CronPaths;
use crate::schedule::{Schedule, resolve_tz, validate};
use crate::store;

/// A sink notified after every finished run (used to publish a bus event).
pub type RunEventSink = Arc<dyn Fn(&Job, &Run) + Send + Sync>;

// ------------------------------------------------------------- pure logic

/// One fire that is due now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueFire {
    /// The scheduled time of the fire (unix ms).
    pub at_ms: u64,
    /// Whether this is a catch-up run (noticed long after its scheduled time).
    pub missed: bool,
}

/// Decides whether a job is due to fire, collapsing several missed occurrences into the
/// single latest one and dropping it when it is older than `missed_window_s` (or the job's
/// missed policy says to skip). A fire counts as `missed` once it's more than
/// `tolerance_s` in the past by the time we notice it — ordinary tick cadence keeps on-time
/// fires under that, so only a real gap (host down, system sleep) is flagged.
#[must_use]
pub fn next_due_fire(
    schedule: &Schedule,
    tz: chrono_tz::Tz,
    last_known_ms: u64,
    now_ms: u64,
    missed_window_s: u64,
    tolerance_s: u64,
    policy: MissedPolicy,
) -> Option<DueFire> {
    let last_known = ms_to_dt(last_known_ms);
    let now = ms_to_dt(now_ms);
    // Walk backward from `now` (inclusive) to find the latest occurrence at or before it
    // — the single fire to run, however many were missed in between. Walking forward from
    // `last_known` instead would iterate every occurrence since then, unbounded when
    // `last_known` is old.
    let due = schedule.last_at_or_before(tz, now)?;
    if due <= last_known {
        return None; // already ran (or scheduled before it ever existed)
    }
    let age_s = (now - due).num_seconds().max(0).cast_unsigned();
    let missed = age_s > tolerance_s;
    if missed && (age_s > missed_window_s || policy == MissedPolicy::Skip) {
        return None;
    }
    Some(DueFire {
        at_ms: dt_to_ms(due),
        missed,
    })
}

fn ms_to_dt(ms: u64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms.cast_signed()).unwrap_or_else(Utc::now)
}
fn dt_to_ms(dt: DateTime<Utc>) -> u64 {
    dt.timestamp_millis().max(0).cast_unsigned()
}

/// A FIFO concurrency limiter: at most `capacity` running at once, the rest wait in order.
#[derive(Debug, Clone)]
pub struct ConcurrencyGate {
    capacity: usize,
    in_flight: usize,
    queue: VecDeque<String>,
}

impl ConcurrencyGate {
    /// A gate with `capacity` slots (min 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            in_flight: 0,
            queue: VecDeque::new(),
        }
    }

    /// Changes the capacity (queued items keep their order).
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
    }

    /// Tries to start `id` now. `true` = go ahead; `false` = queued (FIFO; call
    /// [`ConcurrencyGate::finish`] to pop the next one once a slot frees up).
    pub fn try_start(&mut self, id: &str) -> bool {
        if self.in_flight < self.capacity {
            self.in_flight += 1;
            true
        } else {
            self.queue.push_back(id.to_owned());
            false
        }
    }

    /// A running job ended; returns the next queued id to start, if any (the caller must
    /// actually start it; `in_flight` stays accounted for that slot).
    pub fn finish(&mut self) -> Option<String> {
        self.in_flight = self.in_flight.saturating_sub(1);
        if let Some(next) = self.queue.pop_front() {
            self.in_flight += 1;
            Some(next)
        } else {
            None
        }
    }

    /// Drops `id` from the queue without starting it (its job disappeared).
    pub fn dequeue(&mut self, id: &str) {
        self.queue.retain(|q| q != id);
    }

    /// Queued ids, in order.
    #[must_use]
    pub fn queued(&self) -> &VecDeque<String> {
        &self.queue
    }

    /// Running + queued slots.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }
}

// ------------------------------------------------------------- scheduler

/// An injectable clock (system by default; fixed in tests).
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>);

impl Clock {
    /// The system clock.
    #[must_use]
    pub fn system() -> Self {
        Self(Arc::new(Utc::now))
    }

    /// A clock reading `shared` (tests move it forward between ticks).
    #[must_use]
    pub fn fixed(shared: Arc<Mutex<DateTime<Utc>>>) -> Self {
        Self(Arc::new(move || match shared.try_lock() {
            Ok(guard) => *guard,
            Err(_) => Utc::now(),
        }))
    }

    /// Now.
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        (self.0)()
    }

    /// Now, unix ms.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        dt_to_ms(self.now())
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clock").finish_non_exhaustive()
    }
}

/// One run's in-flight reservation (queued or running), keyed by run id.
struct Active {
    job_id: String,
    pid: Option<u32>,
}

struct SchedulerState {
    /// run id → reservation.
    active: HashMap<String, Active>,
    gate: ConcurrencyGate,
}

/// Owns the cron tick loop and the run lifecycle.
#[derive(Clone)]
pub struct Scheduler {
    paths: CronPaths,
    clock: Clock,
    state: Arc<Mutex<SchedulerState>>,
    run_seq: Arc<AtomicU32>,
    /// Notified after every finished run (bus publish), if installed.
    event_sink: Option<RunEventSink>,
    /// How often [`Scheduler::tick`] is called by the owner ([`crate::serve`]); used as
    /// the on-time tolerance for [`next_due_fire`].
    pub tick_interval: Duration,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// A scheduler on the default paths with the system clock.
    ///
    /// # Errors
    /// [`crate::error::CronError::NoDir`] when no agent dir resolves.
    pub fn new() -> Result<Self> {
        Ok(Self::with_paths(CronPaths::from_agent_dir()?))
    }

    /// A scheduler on `paths` with the system clock.
    #[must_use]
    pub fn with_paths(paths: CronPaths) -> Self {
        Self::with_clock(paths, Clock::system())
    }

    /// A scheduler on `paths` with `clock`.
    #[must_use]
    pub fn with_clock(paths: CronPaths, clock: Clock) -> Self {
        Self {
            paths,
            clock,
            state: Arc::new(Mutex::new(SchedulerState {
                active: HashMap::new(),
                gate: ConcurrencyGate::new(2),
            })),
            run_seq: Arc::new(AtomicU32::new(0)),
            event_sink: None,
            tick_interval: Duration::from_secs(1),
        }
    }

    /// The scheduler's paths.
    #[must_use]
    pub fn paths(&self) -> &CronPaths {
        &self.paths
    }

    /// Installs the run-finished sink (bus publish).
    #[must_use]
    pub fn with_event_sink(mut self, sink: RunEventSink) -> Self {
        self.event_sink = Some(sink);
        self
    }

    fn new_run_id(&self) -> String {
        let n = self.run_seq.fetch_add(1, Ordering::Relaxed);
        format!("c{:x}{:04x}", self.clock.now_ms(), n)
    }

    /// The scheduler settings (defaults when unreadable).
    fn settings(&self) -> CronSettings {
        store::load_settings(&self.paths).unwrap_or_default()
    }

    /// Closes runs left unfinished by a previous owner; call once at startup.
    ///
    /// # Errors
    /// [`crate::error::CronError`] from the store.
    pub fn close_stale_runs(&self) -> Result<usize> {
        store::close_stale_runs(&self.paths, self.clock.now_ms())
    }

    /// One scheduling pass: checks every enabled job, starts due fires (respecting
    /// overlap/pause/concurrency). Called on a timer by [`crate::serve`]; harmless to call
    /// back-to-back in tests.
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
                job.missed,
            ) else {
                continue;
            };
            self.consider_fire(&job, due.at_ms, due.missed, settings.concurrency as usize)
                .await;
        }
    }

    /// Reserves `job` for one fire (or records it `skipped` on overlap), then starts it or
    /// queues it per the concurrency limit.
    pub async fn consider_fire(&self, job: &Job, fired_at: u64, missed: bool, concurrency: usize) {
        let mut state = self.state.lock().await;
        state.gate.set_capacity(concurrency);
        let already = state.active.values().filter(|a| a.job_id == job.id).count();
        if already > 0 && job.overlap == OverlapPolicy::Skip {
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
                reason: Some("previous run still going".to_owned()),
                session_path: None,
                cost_usd: None,
                missed,
                pid: None,
                notify_errors: Vec::new(),
            };
            let _ = store::save_run(&self.paths, &run);
            return;
        }
        let run_id = self.new_run_id();
        state.active.insert(
            run_id.clone(),
            Active {
                job_id: job.id.clone(),
                pid: None,
            },
        );
        let go = state.gate.try_start(&run_id);
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
                pid: None,
                notify_errors: Vec::new(),
            };
            let _ = store::save_run(&self.paths, &run);
        }
    }

    /// Records the run as running and spawns its execution.
    fn start_run<'a>(
        &'a self,
        job: Job,
        run_id: String,
        fired_at: u64,
        missed: bool,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.start_run_inner(job, run_id, fired_at, missed))
    }

    #[allow(
        clippy::unused_async,
        reason = "spawned from the boxed `start_run` future; making it sync would need another spawn"
    )]
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
            pid: None,
            notify_errors: Vec::new(),
        };
        if store::save_run(&self.paths, &run).is_err() {
            return; // nowhere to record: don't run blind
        }

        let this = self.clone();
        let job_for_run = job.clone();
        let run_id_for_run = run_id.clone();
        tokio::spawn(async move {
            let spec = exec::RunSpec {
                prompt: job_for_run.prompt.clone(),
                cwd: job_for_run.cwd.clone(),
                model: job_for_run.model.clone(),
                thinking: job_for_run.thinking.clone(),
                time_limit_s: job_for_run.time_limit_s,
                cost_cap_usd: job_for_run.cost_cap_usd,
            };
            let outcome = exec::run(&this.paths, &job_for_run.id, &run_id_for_run, &spec, {
                let recorder = this.clone();
                let run_id_for_pid = run_id_for_run.clone();
                move |pid| {
                    let recorder = recorder.clone();
                    let run_id_for_pid = run_id_for_pid.clone();
                    tokio::spawn(async move {
                        recorder.record_pid(&run_id_for_pid, pid).await;
                    });
                }
            })
            .await;
            let ended_at = this.clock.now_ms();
            let mut run = store::load_run(&this.paths, &job_for_run.id, &run_id_for_run)
                .ok()
                .flatten()
                .unwrap_or(run);
            exec::finish_run(&mut run, outcome, started_at, ended_at);
            let _ = store::save_run(&this.paths, &run);

            // A one-time `at:` job disables itself once it has run.
            if Schedule::parse(&job_for_run.schedule).is_ok_and(|s| s.is_one_time()) {
                let _ = this.set_enabled(&job_for_run.id, false);
            }

            // Best-effort notifications, after the run record exists: a failing channel is
            // recorded on the run, never turns the run into a failure.
            let settings = this.settings();
            let channels = crate::notify::Channels::load(&this.paths, &settings);
            run.notify_errors = crate::notify::dispatch(&job_for_run, &run, &channels).await;
            let _ = store::save_run(&this.paths, &run);
            if let Some(sink) = &this.event_sink {
                sink(&job_for_run, &run);
            }

            let _ = store::prune_runs(
                &this.paths,
                &job_for_run.id,
                settings.retention_runs,
                settings.retention_days,
                ended_at,
            );

            this.release(&run_id_for_run).await;
        });
    }

    /// A run finished (or was never started): frees its slot and starts the next queued
    /// run, if any.
    async fn release(&self, run_id: &str) {
        let mut state = self.state.lock().await;
        let Some(active) = state.active.remove(run_id) else {
            return;
        };
        let job_id = active.job_id;
        loop {
            let next = state.gate.finish();
            match next {
                Some(next_run_id) => {
                    let Some(next_active) = state.active.get(&next_run_id) else {
                        continue; // vanished (job removed): free its slot too
                    };
                    let next_job = next_active.job_id.clone();
                    drop(state);
                    self.start_queued(&next_run_id, &next_job).await;
                    let _ = job_id;
                    return;
                }
                None => break,
            }
        }
    }

    /// Starts a queued run whose slot opened up.
    async fn start_queued(&self, run_id: &str, job_id: &str) {
        let Ok(jobs) = store::load_jobs(&self.paths) else {
            return;
        };
        let Some(job) = jobs.into_iter().find(|j| j.id == job_id) else {
            // The job was removed while queued: drop the reservation.
            let mut state = self.state.lock().await;
            state.active.remove(run_id);
            state.gate.dequeue(run_id);
            let _ = store::load_run(&self.paths, job_id, run_id);
            return;
        };
        let Some(run) = store::load_run(&self.paths, job_id, run_id).ok().flatten() else {
            return;
        };
        self.start_run(job, run_id.to_owned(), run.fired_at, run.missed)
            .await;
    }

    /// Records the pid of a just-started run so [`Scheduler::stop`] can kill it.
    async fn record_pid(&self, run_id: &str, pid: u32) {
        let mut state = self.state.lock().await;
        if let Some(active) = state.active.get_mut(run_id) {
            active.pid = Some(pid);
        }
    }

    /// Kills every running run of a job (its process groups). Returns whether anything
    /// was killed.
    pub async fn stop(&self, job_id: &str) -> bool {
        let state = self.state.lock().await;
        let pids: Vec<u32> = state
            .active
            .values()
            .filter(|a| a.job_id == job_id)
            .filter_map(|a| a.pid)
            .collect();
        drop(state);
        if pids.is_empty() {
            return false;
        }
        for pid in pids {
            exec::kill_pid(pid);
        }
        true
    }

    /// Job ids with a live (running or queued) reservation.
    pub async fn running_job_ids(&self) -> Vec<String> {
        let state = self.state.lock().await;
        let mut ids: Vec<String> = state.active.values().map(|a| a.job_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Runs `job` right now, bypassing the schedule (still subject to overlap +
    /// concurrency). Returns the new run id, or an error message when the job is already
    /// running under the skip policy.
    ///
    /// # Errors
    /// A user-facing message when the job is already running.
    pub async fn run_now(&self, job: &Job) -> std::result::Result<String, String> {
        let concurrency = self.settings().concurrency as usize;
        let mut state = self.state.lock().await;
        state.gate.set_capacity(concurrency);
        let already = state.active.values().filter(|a| a.job_id == job.id).count();
        if already > 0 && job.overlap == OverlapPolicy::Skip {
            return Err("already running".to_owned());
        }
        let run_id = self.new_run_id();
        state.active.insert(
            run_id.clone(),
            Active {
                job_id: job.id.clone(),
                pid: None,
            },
        );
        let go = state.gate.try_start(&run_id);
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
                pid: None,
                notify_errors: Vec::new(),
            };
            let _ = store::save_run(&self.paths, &run);
        }
        Ok(run_id)
    }

    /// Sets `enabled` on a job.
    ///
    /// # Errors
    /// [`crate::error::CronError`] from the store.
    pub fn set_enabled(&self, job_id: &str, enabled: bool) -> Result<()> {
        let mut jobs = store::load_jobs(&self.paths)?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
            job.enabled = enabled;
            job.updated = self.clock.now_ms();
        }
        store::save_jobs(&self.paths, &jobs)
    }

    /// Waits until no runs are active (daemon shutdown), at most `grace`.
    pub async fn wait_idle(&self, grace: Duration) {
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let busy = {
                let state = self.state.lock().await;
                !state.active.is_empty()
            };
            if !busy || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

impl Scheduler {
    /// The scheduler's clock (for [`crate::control::handle_control`]).
    #[must_use]
    pub fn clock_now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The scheduler's clock, unix ms.
    #[must_use]
    pub fn clock_now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
}

/// Turns `name` into a short slug for a job id.
#[must_use]
pub fn slugify(name: &str) -> String {
    let mut slug: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    if slug.is_empty() {
        "job".clone_into(&mut slug);
    }
    slug.chars().take(40).collect()
}

/// A unique job id from `base` (appends `-2`, `-3`, …).
#[must_use]
pub fn unique_id(jobs: &[Job], base: &str) -> String {
    if !jobs.iter().any(|j| j.id == base) {
        return base.to_owned();
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !jobs.iter().any(|j| j.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-{:x}", std::process::id())
}

/// Validates a full job (schedule already resolved to machine form) and returns a
/// user-facing error message on failure.
/// # Errors
/// A user-facing validation message.
pub fn validate_job(job: &Job) -> std::result::Result<(), String> {
    validate(&job.schedule, &job.tz, &job.cwd)
        .map(|_| ())
        .map_err(|e| e.0)
}

/// Serialises a job for a `--json` reply.
#[must_use]
pub fn job_json(job: &Job) -> Value {
    serde_json::to_value(job).unwrap_or_else(|_| serde_json::json!({}))
}

/// Serialises a run for a `--json` reply.
#[must_use]
pub fn run_json(run: &Run) -> Value {
    serde_json::to_value(run).unwrap_or_else(|_| serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn due_fire_on_time_is_not_missed() {
        let sched = Schedule::parse("0 * * * * *").unwrap(); // every minute
        let tz = chrono_tz::UTC;
        let last = dt_to_ms(dt("2024-01-01T00:00:00Z"));
        let now = dt_to_ms(dt("2024-01-01T00:01:01Z")); // 1s after due
        let due = next_due_fire(&sched, tz, last, now, 21_600, 3, MissedPolicy::default()).unwrap();
        assert!(!due.missed);
        assert_eq!(due.at_ms, dt_to_ms(dt("2024-01-01T00:01:00Z")));
    }

    #[test]
    fn missed_window_collapses_and_drops_old_fires() {
        // A daily job: the host sleeps through several midnights, then wakes. Only the
        // latest missed fire runs, and only within the window.
        let sched = Schedule::parse("0 0 0 * * *").unwrap(); // daily at 00:00
        let tz = chrono_tz::UTC;
        let last = dt_to_ms(dt("2023-12-01T00:00:00Z"));
        // Two midnights missed; the latest one 2h ago — within the 6h window.
        let now = dt_to_ms(dt("2023-12-03T02:00:00Z"));
        let due = next_due_fire(&sched, tz, last, now, 21_600, 3, MissedPolicy::default()).unwrap();
        assert!(due.missed);
        assert_eq!(
            due.at_ms,
            dt_to_ms(dt("2023-12-03T00:00:00Z")),
            "collapses to the single latest missed fire"
        );
        // Beyond the window: nothing runs.
        let far_now = dt_to_ms(dt("2023-12-03T08:00:05Z"));
        assert!(
            next_due_fire(
                &sched,
                tz,
                last,
                far_now,
                21_600,
                3,
                MissedPolicy::default()
            )
            .is_none()
        );
        // Skip policy: even a fresh miss never runs.
        assert!(next_due_fire(&sched, tz, last, now, 21_600, 3, MissedPolicy::Skip).is_none());
    }

    #[test]
    fn concurrency_gate_queues_fifo_and_drains_on_finish() {
        let mut gate = ConcurrencyGate::new(2);
        assert!(gate.try_start("a"));
        assert!(gate.try_start("b"));
        assert!(!gate.try_start("c"));
        assert_eq!(gate.queued().front().map(String::as_str), Some("c"));
        assert_eq!(gate.in_flight(), 2);
        assert_eq!(gate.finish(), Some("c".to_owned()));
        assert_eq!(gate.in_flight(), 2);
        assert_eq!(gate.finish(), None);
        assert_eq!(gate.in_flight(), 1);
        assert_eq!(gate.finish(), None);
        assert_eq!(gate.in_flight(), 0);
    }

    #[test]
    fn concurrency_gate_drops_a_queued_id() {
        let mut gate = ConcurrencyGate::new(1);
        assert!(gate.try_start("a"));
        assert!(!gate.try_start("b"));
        gate.dequeue("b");
        assert_eq!(gate.finish(), None);
    }

    #[tokio::test]
    async fn overlap_fire_is_recorded_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CronPaths::new(tmp.path());
        let scheduler = Scheduler::with_paths(paths.clone());
        let job = Job::new(
            "j",
            "j",
            "p",
            "0 * * * * *",
            "UTC",
            tmp.path().to_string_lossy().as_ref(),
        );
        // Reserve a run of the job as if already running (no execution).
        {
            let mut state = scheduler.state.lock().await;
            state.active.insert(
                "running-one".into(),
                Active {
                    job_id: "j".into(),
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

    #[test]
    fn slugify_and_unique_ids() {
        assert_eq!(slugify("Daily Digest!"), "daily-digest");
        assert_eq!(slugify("  --"), "job");
        let jobs = vec![Job::new("a", "a", "p", "0 0 * * * *", "UTC", "/tmp")];
        assert_eq!(unique_id(&jobs, "a"), "a-2");
        assert_eq!(unique_id(&jobs, "b"), "b");
    }
}
