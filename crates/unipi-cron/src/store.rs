//! Atomic storage for cron jobs and run records.
//!
//! See [`crate::paths`] for the layout. **One JSON file per job** (`jobs/<id>.json`,
//! schema `"v": 2`, camelCase, `id` = file stem — ext-abi §6.2); every writer (the cron
//! extension, the `unipi-host cron` CLI, the host) writes atomically under the jobs
//! lock. Unknown files (stem ≠ `id` field, malformed JSON, wrong `v`) are ignored with a
//! diagnostic. A pre-v2 monolithic `jobs.json` is split into `jobs/*.json` on first open
//! ([`migrate_jobs_json`], idempotent).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CronError, Result};
use crate::model::{CronSettings, Job, Run, RunStatus};
use crate::paths::{CronPaths, atomic_write_json, read_json};

/// The output tail kept per run.
pub const LOG_CAP_BYTES: usize = 64 * 1024;

/// The job-file schema version (`"v": 2`).
pub const JOB_FILE_VERSION: u8 = 2;

/// A `jobs/<id>.json` envelope: `{"v": 2, …job fields (camelCase)}`.
#[derive(Debug, Serialize, Deserialize)]
struct JobFile {
    v: u8,
    #[serde(flatten)]
    job: Job,
}

/// The v2 wire form of `job` (what a job file holds, without the `v`).
#[must_use]
pub fn job_file_json(job: &Job) -> serde_json::Value {
    serde_json::to_value(JobFile {
        v: JOB_FILE_VERSION,
        job: job.clone(),
    })
    .unwrap_or_else(|_| serde_json::json!({}))
}

/// Splits a legacy `jobs.json` (pre-v2 monolithic list) into `jobs/<id>.json` files under
/// the jobs lock and renames it to `jobs.json.imported`. A no-op when no legacy file
/// exists; job files already present win over their legacy copy. Returns how many jobs
/// were written.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`] from the read/writes.
pub fn migrate_jobs_json(paths: &CronPaths) -> Result<usize> {
    if !paths.legacy_jobs().exists() {
        return Ok(0);
    }
    paths.ensure()?;
    let _guard = JobsLock::acquire(paths)?;
    // Re-check under the lock: another process may have migrated meanwhile.
    if !paths.legacy_jobs().exists() {
        return Ok(0);
    }
    let Some(legacy): Option<Vec<Job>> = read_json(&paths.legacy_jobs())? else {
        let _ = std::fs::rename(paths.legacy_jobs(), paths.root.join("jobs.json.imported"));
        return Ok(0);
    };
    let mut written = 0;
    for job in legacy {
        let target = paths.job_file(&job.id);
        if target.exists() {
            continue; // the split file is newer
        }
        atomic_write_json(
            &target,
            &JobFile {
                v: JOB_FILE_VERSION,
                job,
            },
        )?;
        written += 1;
    }
    std::fs::rename(paths.legacy_jobs(), paths.root.join("jobs.json.imported"))
        .map_err(|e| CronError::io(e, paths.legacy_jobs()))?;
    Ok(written)
}

/// Loads every job, migrating a legacy `jobs.json` first and ignoring broken files
/// (their diagnostics are dropped; [`scan_jobs`] reports them).
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`] when the jobs dir can't be read.
pub fn load_jobs(paths: &CronPaths) -> Result<Vec<Job>> {
    Ok(scan_jobs(paths)?.0)
}

/// Loads every job plus one diagnostic per ignored file (unknown id, malformed JSON,
/// wrong schema version).
///
/// # Errors
/// [`CronError::Io`] when the jobs dir can't be read.
pub fn scan_jobs(paths: &CronPaths) -> Result<(Vec<Job>, Vec<String>)> {
    let _ = migrate_jobs_json(paths)?;
    let dir = paths.jobs_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(CronError::io(e, &dir)),
    };
    let mut jobs = Vec::new();
    let mut diagnostics = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let diagnostic = |what: &str| format!("ignored job file {}: {what}", path.display());
        match read_json::<JobFile>(&path) {
            Ok(Some(file)) if file.v != JOB_FILE_VERSION => {
                diagnostics.push(diagnostic(&format!(
                    "schema version {} ≠ {JOB_FILE_VERSION}",
                    file.v
                )));
            }
            Ok(Some(file)) if file.job.id != stem => {
                diagnostics.push(diagnostic(&format!("id {:?} ≠ file name", file.job.id)));
            }
            Ok(Some(file)) => jobs.push(file.job),
            Ok(None) => {}
            Err(e) => diagnostics.push(diagnostic(&e.to_string())),
        }
    }
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((jobs, diagnostics))
}

/// Saves one job atomically (`jobs/<id>.json`) under the jobs lock.
///
/// # Errors
/// [`CronError::Io`] when the lock or the write fails.
pub fn save_job(paths: &CronPaths, job: &Job) -> Result<()> {
    paths.ensure()?;
    let _guard = JobsLock::acquire(paths)?;
    atomic_write_json(
        &paths.job_file(&job.id),
        &JobFile {
            v: JOB_FILE_VERSION,
            job: job.clone(),
        },
    )
}

/// Saves every job (one file each) under one lock acquisition (migration, tests).
///
/// # Errors
/// [`CronError::Io`] when the lock or a write fails.
pub fn save_jobs(paths: &CronPaths, jobs: &[Job]) -> Result<()> {
    paths.ensure()?;
    let _guard = JobsLock::acquire(paths)?;
    for job in jobs {
        atomic_write_json(
            &paths.job_file(&job.id),
            &JobFile {
                v: JOB_FILE_VERSION,
                job: job.clone(),
            },
        )?;
    }
    Ok(())
}

/// Read-modify-write of one job file under the jobs lock: `apply` mutates the job; its
/// error aborts without writing. `Ok(None)` = no such file.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`] from the file; the closure's message otherwise.
pub fn update_job<R>(
    paths: &CronPaths,
    id: &str,
    apply: impl FnOnce(&mut Job) -> std::result::Result<R, String>,
) -> Result<std::result::Result<Option<(R, Job)>, String>> {
    paths.ensure()?;
    let _guard = JobsLock::acquire(paths)?;
    let Some(file) = read_json::<JobFile>(&paths.job_file(id))? else {
        return Ok(Ok(None));
    };
    let mut job = file.job;
    match apply(&mut job) {
        Ok(value) => {
            atomic_write_json(
                &paths.job_file(id),
                &JobFile {
                    v: JOB_FILE_VERSION,
                    job: job.clone(),
                },
            )?;
            Ok(Ok(Some((value, job))))
        }
        Err(message) => Ok(Err(message)),
    }
}

/// Inserts a job unless its id is taken (checked under the jobs lock).
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`]; a user-facing message when the id exists.
pub fn insert_job(paths: &CronPaths, job: &Job) -> std::result::Result<(), String> {
    let write = || -> Result<()> {
        paths.ensure()?;
        let _guard = JobsLock::acquire(paths)?;
        if paths.job_file(&job.id).exists() || load_jobs(paths)?.iter().any(|j| j.id == job.id) {
            return Err(CronError::control(format!(
                "a job {:?} already exists",
                job.id
            )));
        }
        atomic_write_json(
            &paths.job_file(&job.id),
            &JobFile {
                v: JOB_FILE_VERSION,
                job: job.clone(),
            },
        )
    };
    write().map_err(|e| e.to_string())
}

/// Removes a job file. `Ok(false)` when it didn't exist.
///
/// # Errors
/// [`CronError::Io`] on the delete.
pub fn remove_job_file(paths: &CronPaths, id: &str) -> Result<bool> {
    match std::fs::remove_file(paths.job_file(id)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(CronError::io(e, paths.job_file(id))),
    }
}

/// Serialises job-file writes (extension, CLI, host).
struct JobsLock(#[allow(dead_code)] std::fs::File);

impl JobsLock {
    fn acquire(paths: &CronPaths) -> Result<Self> {
        std::fs::create_dir_all(paths.jobs_dir())
            .map_err(|e| CronError::io(e, paths.jobs_dir()))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(paths.jobs_lock())
            .map_err(|e| CronError::io(e, paths.jobs_lock()))?;
        file.lock()
            .map_err(|e| CronError::io(e, paths.jobs_lock()))?;
        Ok(Self(file))
    }
}

/// Writes a run record atomically.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn save_run(paths: &CronPaths, run: &Run) -> Result<()> {
    atomic_write_json(&paths.run_json(&run.job_id, &run.id), run)
}

/// Loads one run.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`]; `Ok(None)` when it doesn't exist.
pub fn load_run(paths: &CronPaths, job_id: &str, run_id: &str) -> Result<Option<Run>> {
    read_json(&paths.run_json(job_id, run_id))
}

/// Every run for a job, newest first (by `fired_at`, ties by id).
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn load_runs(paths: &CronPaths, job_id: &str) -> Result<Vec<Run>> {
    let dir = paths.job_runs(job_id);
    let mut runs = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CronError::io(e, &dir)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| CronError::io(e, &dir))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && let Some(run) = read_json::<Run>(&path)?
        {
            runs.push(run);
        }
    }
    runs.sort_by(|a, b| b.fired_at.cmp(&a.fired_at).then_with(|| b.id.cmp(&a.id)));
    Ok(runs)
}

/// Finds a run by id alone, scanning every job's run directory (a run id is a short
/// unique slug; this is a user-facing lookup, not a hot path). Returns `(job_id, run)`.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn find_run(paths: &CronPaths, run_id: &str) -> Result<Option<(String, Run)>> {
    let base = paths.runs();
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CronError::io(e, &base)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| CronError::io(e, &base))?;
        if !entry
            .file_type()
            .map_err(|e| CronError::io(e, &base))?
            .is_dir()
        {
            continue;
        }
        let Some(job_id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(run) = load_run(paths, &job_id, run_id)? {
            return Ok(Some((job_id, run)));
        }
    }
    Ok(None)
}

/// Every run across every job, newest first.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn load_all_runs(paths: &CronPaths) -> Result<Vec<Run>> {
    let base = paths.runs();
    let mut runs = Vec::new();
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CronError::io(e, &base)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| CronError::io(e, &base))?;
        if entry
            .file_type()
            .map_err(|e| CronError::io(e, &base))?
            .is_dir()
            && let Some(name) = entry.file_name().to_str()
        {
            runs.extend(load_runs(paths, name)?);
        }
    }
    runs.sort_by(|a, b| b.fired_at.cmp(&a.fired_at).then_with(|| b.id.cmp(&a.id)));
    Ok(runs)
}

/// Appends to a run's log, keeping only the last [`LOG_CAP_BYTES`].
///
/// # Errors
/// [`CronError::Io`].
pub fn append_log(paths: &CronPaths, job_id: &str, run_id: &str, chunk: &[u8]) -> Result<()> {
    let path = paths.run_log(job_id, run_id);
    let mut existing = std::fs::read(&path).unwrap_or_default();
    existing.extend_from_slice(chunk);
    if existing.len() > LOG_CAP_BYTES {
        let start = existing.len() - LOG_CAP_BYTES;
        existing.drain(..start);
    }
    crate::paths::atomic_write(&path, &existing)
}

/// Reads a run's log, if any.
#[must_use]
pub fn read_log(paths: &CronPaths, job_id: &str, run_id: &str) -> Option<Vec<u8>> {
    std::fs::read(paths.run_log(job_id, run_id)).ok()
}

/// Loads the scheduler settings (missing file = defaults).
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn load_settings(paths: &CronPaths) -> Result<CronSettings> {
    Ok(read_json(&paths.settings())?.unwrap_or_default())
}

/// Saves the scheduler settings.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn save_settings(paths: &CronPaths, settings: &CronSettings) -> Result<()> {
    paths.ensure()?;
    atomic_write_json(&paths.settings(), settings)
}

/// Is there a process with `pid` alive right now?
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 only checks for existence.
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid.cast_signed()), None).is_ok()
    }
    #[cfg(not(unix))]
    {
        // Windows: no signal API; treat as alive (conservative).
        let _ = pid;
        true
    }
}

/// Marks runs left `running`/`queued` by a previous owner: finished (`failed`) when their
/// process is gone, kept as-is when it still runs. Returns how many were closed.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn close_stale_runs(paths: &CronPaths, now_ms: u64) -> Result<usize> {
    let mut closed = 0;
    for run in load_all_runs(paths)? {
        if run.status.is_finished() {
            continue;
        }
        let gone = match run.pid {
            Some(pid) => !pid_alive(pid),
            // No pid recorded (queued, or killed before the spawn callback ran).
            None => run.started_at.is_none(),
        };
        if gone {
            let mut run = run;
            run.status = RunStatus::Failed;
            run.ended_at = Some(now_ms);
            run.reason = Some("interrupted: the scheduler stopped mid-run".to_owned());
            save_run(paths, &run)?;
            closed += 1;
        }
    }
    Ok(closed)
}

/// Deletes runs (`.json` + `.log`) for a job beyond `retention_runs` (newest kept) or
/// older than `retention_days`. Runs still in progress are never pruned. Returns how
/// many were removed.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`].
pub fn prune_runs(
    paths: &CronPaths,
    job_id: &str,
    retention_runs: u32,
    retention_days: u32,
    now_ms: u64,
) -> Result<usize> {
    let mut runs = load_runs(paths, job_id)?; // newest first
    let cutoff = now_ms.saturating_sub(u64::from(retention_days) * 24 * 60 * 60 * 1000);
    let mut removed = 0usize;
    for (i, run) in runs.iter().enumerate() {
        let keep_by_count = i < retention_runs as usize;
        let keep_by_age = run.fired_at >= cutoff;
        if !run.status.is_finished() {
            continue;
        }
        if !(keep_by_count && keep_by_age) {
            let _ = std::fs::remove_file(paths.run_json(job_id, &run.id));
            let _ = std::fs::remove_file(paths.run_log(job_id, &run.id));
            removed += 1;
        }
    }
    runs.clear();
    Ok(removed)
}

/// Removes a job's whole run directory (job deletion).
pub fn remove_runs(paths: &CronPaths, job_id: &str) {
    let _ = std::fs::remove_dir_all(paths.job_runs(job_id));
}

/// The session directory name for `cwd` (mirrors `oino-session`'s encoding).
#[must_use]
pub fn encode_cwd(cwd: &Path) -> String {
    let trimmed = cwd.to_string_lossy();
    let stripped = trimmed.trim_start_matches(['/', '\\']);
    let replaced: String = stripped
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("--{replaced}--")
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

    fn job(id: &str) -> Job {
        Job {
            time_limit_s: DEFAULT_TIME_LIMIT_S,
            ..Job::new(id, "test", "do it", "0 0 * * * *", "UTC", "/tmp")
        }
    }

    fn run(job_id: &str, id: &str, fired_at: u64, status: RunStatus) -> Run {
        Run {
            id: id.into(),
            job_id: job_id.into(),
            status,
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
        }
    }

    #[test]
    fn jobs_round_trip_as_one_file_each() {
        let (_tmp, paths) = paths();
        let jobs = vec![job("a"), job("b")];
        save_jobs(&paths, &jobs).unwrap();
        assert_eq!(load_jobs(&paths).unwrap(), jobs);
        assert!(paths.job_file("a").is_file());
        // Only the job files and the lock live in jobs/.
        let mut names: Vec<String> = std::fs::read_dir(paths.jobs_dir())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![".lock".to_owned(), "a.json".to_owned(), "b.json".to_owned()]
        );
    }

    #[test]
    fn run_and_log_round_trip() {
        let (_tmp, paths) = paths();
        let r = run("job1", "run1", 100, RunStatus::Ok);
        save_run(&paths, &r).unwrap();
        assert_eq!(load_run(&paths, "job1", "run1").unwrap(), Some(r));
        append_log(&paths, "job1", "run1", b"hello ").unwrap();
        append_log(&paths, "job1", "run1", b"world").unwrap();
        assert_eq!(read_log(&paths, "job1", "run1").unwrap(), b"hello world");
    }

    #[test]
    fn log_is_capped_at_64kb_keeping_the_tail() {
        let (_tmp, paths) = paths();
        append_log(&paths, "j", "r", &vec![b'a'; LOG_CAP_BYTES]).unwrap();
        append_log(&paths, "j", "r", b"TAIL").unwrap();
        let log = read_log(&paths, "j", "r").unwrap();
        assert_eq!(log.len(), LOG_CAP_BYTES);
        assert!(log.ends_with(b"TAIL"));
    }

    #[test]
    fn find_run_scans_every_job() {
        let (_tmp, paths) = paths();
        save_run(&paths, &run("j1", "r1", 100, RunStatus::Ok)).unwrap();
        save_run(&paths, &run("j2", "r2", 200, RunStatus::Ok)).unwrap();
        let (job_id, found) = find_run(&paths, "r2").unwrap().expect("found");
        assert_eq!(job_id, "j2");
        assert_eq!(found.id, "r2");
        assert!(find_run(&paths, "missing").unwrap().is_none());
    }

    #[test]
    fn loads_runs_newest_first_across_jobs() {
        let (_tmp, paths) = paths();
        save_run(&paths, &run("j1", "r1", 100, RunStatus::Ok)).unwrap();
        save_run(&paths, &run("j1", "r2", 300, RunStatus::Ok)).unwrap();
        save_run(&paths, &run("j2", "r3", 200, RunStatus::Ok)).unwrap();
        let all = load_all_runs(&paths).unwrap();
        assert_eq!(
            all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["r2", "r3", "r1"]
        );
        let j1 = load_runs(&paths, "j1").unwrap();
        assert_eq!(
            j1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["r2", "r1"]
        );
    }

    #[test]
    fn settings_round_trip() {
        let (_tmp, paths) = paths();
        assert_eq!(load_settings(&paths).unwrap(), CronSettings::default());
        let settings = CronSettings {
            paused: true,
            concurrency: 5,
            ..CronSettings::default()
        };
        save_settings(&paths, &settings).unwrap();
        assert_eq!(load_settings(&paths).unwrap(), settings);
    }

    #[test]
    fn prune_keeps_latest_n_and_recent_days() {
        let (_tmp, paths) = paths();
        let day_ms = 24 * 60 * 60 * 1000u64;
        let now = 100 * day_ms;
        for i in 0..5 {
            // Each older by 10 days.
            save_run(
                &paths,
                &run("j", &format!("r{i}"), now - i * 10 * day_ms, RunStatus::Ok),
            )
            .unwrap();
        }
        // Keep at most 3 by count.
        let removed = prune_runs(&paths, "j", 3, 1000, now).unwrap();
        assert_eq!(removed, 2);
        let left = load_runs(&paths, "j").unwrap();
        assert_eq!(left.len(), 3);
        assert_eq!(
            left.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["r0", "r1", "r2"]
        );
    }

    #[test]
    fn prune_never_removes_a_running_run() {
        let (_tmp, paths) = paths();
        save_run(&paths, &run("j", "old-running", 0, RunStatus::Running)).unwrap();
        let removed = prune_runs(&paths, "j", 0, 0, 10_000_000).unwrap();
        assert_eq!(removed, 0);
        assert!(load_run(&paths, "j", "old-running").unwrap().is_some());
    }

    #[test]
    fn stale_running_runs_without_a_live_pid_are_closed() {
        let (_tmp, paths) = paths();
        let mut live = run("j", "live", 1, RunStatus::Running);
        live.pid = Some(std::process::id());
        save_run(&paths, &live).unwrap();
        let mut dead = run("j", "dead", 2, RunStatus::Running);
        dead.pid = Some(u32::MAX - 1); // not a real process
        dead.started_at = Some(2);
        save_run(&paths, &dead).unwrap();
        let mut queued = run("j", "queued", 3, RunStatus::Queued);
        queued.started_at = None;
        save_run(&paths, &queued).unwrap();
        let closed = close_stale_runs(&paths, 10).unwrap();
        assert_eq!(closed, 2, "dead + queued");
        assert_eq!(
            load_run(&paths, "j", "live").unwrap().unwrap().status,
            RunStatus::Running
        );
        let dead = load_run(&paths, "j", "dead").unwrap().unwrap();
        assert_eq!(dead.status, RunStatus::Failed);
        assert!(dead.reason.unwrap().contains("interrupted"));
    }

    #[test]
    fn cwd_encoding_matches_the_session_layout() {
        assert_eq!(
            encode_cwd(Path::new("/home/me/project")),
            "--home-me-project--"
        );
        assert_eq!(encode_cwd(Path::new("C:\\Users\\me")), "--C--Users-me--");
    }
}
