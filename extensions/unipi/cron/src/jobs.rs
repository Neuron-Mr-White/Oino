//! The job files, read and written directly (v2 ext-abi §6.4): `unipi.cron` owns its
//! CRUD against `~/.oino/cron` through its fs preopens — `/cron-root` (read: runs,
//! settings) and `/cron-jobs` (write: `jobs/<id>.json`) — so job management works
//! identically with or without a host. Live-scheduler ops go through the `cron` host
//! service instead ([`crate::lib`]).
//!
//! The job-file format is the v2 schema (`"v": 2`, camelCase, `id` = file stem — see
//! `unipi-cron::store`); every write is atomic under `jobs/.lock`, the same lock file
//! the host library takes.

use std::path::PathBuf;

use oino_extension::fs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The writable jobs preopen (`~/.oino/cron/jobs`).
pub const JOBS_DIR: &str = "/cron-jobs";
/// The read-only cron root (`~/.oino/cron`).
pub const ROOT: &str = "/cron-root";
/// The lock serialising job-file writes (shared with the host library).
const LOCK: &str = "/cron-jobs/.lock";
/// The job-file schema version.
pub const V: u8 = 2;

/// A job's notification settings (`notify`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifySpec {
    /// Channels; absent = the user's enabled ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    /// Run statuses to notify on; absent = `["failed"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
}

/// One job (`jobs/<id>.json`, camelCase — mirrors `unipi_cron::model::Job`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// Short slug; must equal the file stem.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The prompt every run sends.
    pub prompt: String,
    /// A 6-field cron (seconds first) or `at:<RFC3339>`.
    pub schedule: String,
    /// IANA zone name.
    pub tz: String,
    /// Working directory (must exist).
    pub cwd: String,
    /// Model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Thinking level override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Enabled jobs fire; disabled ones don't.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Per-run wall-clock limit, seconds.
    #[serde(default = "default_limit")]
    pub time_limit_s: u64,
    /// Cost cap (USD); `None` = no cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_usd: Option<f64>,
    /// Overlap policy: "skip" (default) or "allow".
    #[serde(default)]
    pub overlap: String,
    /// Missed-fire policy: "run-latest" (default) or "skip".
    #[serde(default)]
    pub missed: String,
    /// Notification settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<NotifySpec>,
    /// Unix ms.
    pub created: u64,
    /// Unix ms.
    pub updated: u64,
}

fn yes() -> bool {
    true
}
fn default_limit() -> u64 {
    1800
}

/// The `"v": 2` file envelope.
#[derive(Serialize, Deserialize)]
struct JobFile {
    v: u8,
    #[serde(flatten)]
    job: Job,
}

/// One run record (`runs/<job>/<run>.json`, `snake_case` — scheduler-written).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    /// Run id (also the run's session id).
    pub id: String,
    /// Owning job id.
    pub job_id: String,
    /// Current status.
    pub status: String,
    /// Unix ms when this fire was recorded.
    pub fired_at: u64,
    /// The assistant's final summary text.
    #[serde(default)]
    pub summary: Option<String>,
    /// Why a run was skipped or failed before execution.
    #[serde(default)]
    pub reason: Option<String>,
    /// The run's session file, when one was written.
    #[serde(default)]
    pub session_path: Option<String>,
}

fn job_path(id: &str) -> String {
    format!("{JOBS_DIR}/{id}.json")
}

/// Now, unix ms (the WIT clock in-guest; the system clock for native tests).
#[must_use]
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        u64::try_from(oino_extension::timers::now_ms().max(0)).unwrap_or(0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis()),
        )
        .unwrap_or(0)
    }
}

/// Loads every job plus one diagnostic per ignored file (wrong `v`, id ≠ stem,
/// malformed JSON — ignored, never fatal: management still works).
#[must_use]
pub fn scan() -> (Vec<Job>, Vec<String>) {
    let mut jobs = Vec::new();
    let mut diagnostics = Vec::new();
    let Ok(entries) = std::fs::read_dir(JOBS_DIR) else {
        return (jobs, diagnostics);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let report = |what: &str| format!("ignored {stem}.json: {what}");
        match std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<JobFile>(&b).ok())
        {
            Some(file) if file.v != V => {
                diagnostics.push(report(&format!("schema version {} \u{2260} {V}", file.v)));
            }
            Some(file) if file.job.id != stem => {
                diagnostics.push(report(&format!("id {:?} \u{2260} file name", file.job.id)));
            }
            Some(file) => jobs.push(file.job),
            None => diagnostics.push(report("not a valid job file")),
        }
    }
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    (jobs, diagnostics)
}

/// Every job (diagnostics dropped).
#[must_use]
pub fn list() -> Vec<Job> {
    scan().0
}

/// One job by id.
#[must_use]
pub fn load(id: &str) -> Option<Job> {
    let bytes = std::fs::read(job_path(id)).ok()?;
    serde_json::from_slice::<JobFile>(&bytes)
        .ok()
        .filter(|f| f.v == V)
        .map(|f| f.job)
}

/// Writes one job atomically under the jobs lock.
///
/// # Errors
/// A user-facing message on I/O or serialization failure.
pub async fn save(job: &Job) -> Result<(), String> {
    let value = JobFile {
        v: V,
        job: job.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    let _guard = fs::Lock::acquire(LOCK, true)
        .await
        .map_err(|e| e.to_string())?;
    fs::atomic_write(&job_path(&job.id), &bytes).map_err(|e| e.to_string())
}

/// Read-modify-write of one job under the lock; the closure's error aborts unchanged.
/// `Ok(None)` = no such job.
///
/// # Errors
/// A user-facing message on I/O failure.
pub async fn update<R>(
    id: &str,
    apply: impl FnOnce(&mut Job) -> Result<R, String>,
) -> Result<Option<R>, String> {
    let Some(mut job) = load(id) else {
        return Ok(None);
    };
    let value = apply(&mut job)?;
    save(&job).await?;
    Ok(Some(value))
}

/// Inserts a job unless its id is taken (checked under the lock).
///
/// # Errors
/// A user-facing message when the id exists or the write fails.
pub async fn insert(job: &Job) -> Result<(), String> {
    let _guard = fs::Lock::acquire(LOCK, true)
        .await
        .map_err(|e| e.to_string())?;
    if load(&job.id).is_some() || list().iter().any(|j| j.id == job.id) {
        return Err(format!("a job {:?} already exists", job.id));
    }
    let value = JobFile {
        v: V,
        job: job.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    fs::atomic_write(&job_path(&job.id), &bytes).map_err(|e| e.to_string())
}

/// Deletes a job file. `Ok(false)` when it didn't exist (the run history stays until
/// retention prunes it — the extension's preopen on `runs/` is read-only).
///
/// # Errors
/// A user-facing message on I/O failure.
pub fn remove(id: &str) -> Result<bool, String> {
    match std::fs::remove_file(job_path(id)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{JOBS_DIR}/{id}.json: {e}")),
    }
}

/// A job's runs, newest first.
#[must_use]
pub fn runs(job_id: &str) -> Vec<Run> {
    let dir = format!("{ROOT}/runs/{job_id}");
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(bytes) = std::fs::read(&path)
                && let Ok(run) = serde_json::from_slice::<Run>(&bytes)
            {
                found.push(run);
            }
        }
    }
    found.sort_by(|a, b| b.fired_at.cmp(&a.fired_at).then_with(|| b.id.cmp(&a.id)));
    found
}

/// Every run across jobs, newest first.
#[must_use]
pub fn all_runs() -> Vec<Run> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(format!("{ROOT}/runs")) {
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                found.extend(runs(name));
            }
        }
    }
    found.sort_by(|a, b| b.fired_at.cmp(&a.fired_at).then_with(|| b.id.cmp(&a.id)));
    found
}

/// Finds a run by id across jobs; `(job id, run, log text)`.
#[must_use]
pub fn find_run(run_id: &str) -> Option<(String, Run, String)> {
    for job in list() {
        if let Some(run) = runs(&job.id).into_iter().find(|r| r.id == run_id) {
            let log = std::fs::read_to_string(format!("{ROOT}/runs/{}/{run_id}.log", job.id))
                .unwrap_or_default();
            return Some((job.id, run, log));
        }
    }
    None
}

/// The scheduler settings (`/cron-root/settings.json`), best-effort: absent fields fall
/// back to the documented defaults (`unipi_cron::model::CronSettings`).
#[must_use]
pub fn settings() -> Value {
    std::fs::read(format!("{ROOT}/settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| {
            serde_json::json!({
                "concurrency": 2, "missed_window_s": 21600, "retention_runs": 50,
                "retention_days": 30, "paused": false, "default_time_limit_s": 1800,
                "notify_default_events": []
            })
        })
}

/// Turns a name into a short slug for a job id (mirrors the host library).
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
pub fn unique_id(base: &str) -> String {
    let jobs = list();
    if !jobs.iter().any(|j| j.id == base) {
        return base.to_owned();
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !jobs.iter().any(|j| j.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-{:x}", now_ms())
}

/// Resolves a schedule as typed (plain language or machine form) into the machine form,
/// the same code the scheduler uses (`unipi-cron-schedule`).
///
/// # Errors
/// A user-facing message when nothing matches.
pub fn resolve_schedule(input: &str, tz: &str) -> Result<String, String> {
    let zone = unipi_cron_schedule::resolve_tz(tz).map_err(|e| e.0)?;
    unipi_cron_schedule::parse_human(input, zone, unipi_cron_schedule::now()).map_err(|e| e.0)
}

/// The job's next fire (RFC3339), for list/show renders; `None` when disabled or past.
#[must_use]
pub fn next_fire(job: &Job) -> Option<String> {
    if !job.enabled {
        return None;
    }
    let tz = unipi_cron_schedule::resolve_tz(&job.tz).ok()?;
    let schedule = unipi_cron_schedule::Schedule::parse(&job.schedule).ok()?;
    schedule
        .next_after(tz, unipi_cron_schedule::now(), false)
        .map(|dt| dt.to_rfc3339())
}

/// A job as the reply JSON the renders and tools consume (camelCase + `next_fire`).
#[must_use]
pub fn job_value(job: &Job) -> Value {
    let mut value = serde_json::to_value(job).unwrap_or_else(|_| Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "next_fire".to_owned(),
            next_fire(job).map_or(Value::Null, Value::String),
        );
    }
    value
}

/// The runs directory of a job (for tests).
#[must_use]
pub fn runs_dir(job_id: &str) -> PathBuf {
    PathBuf::from(format!("{ROOT}/runs/{job_id}"))
}
