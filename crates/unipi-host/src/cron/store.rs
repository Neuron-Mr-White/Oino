//! Atomic storage for cron jobs and run records (UNI-314).
//!
//! Layout under `<host dir>/cron/`:
//! - `jobs.json`: `Vec<Job>`, written atomically (tmp + rename, like
//!   everything in [`crate::data_dir`]).
//! - `runs/<job id>/<run id>.json`: one [`Run`] record.
//! - `runs/<job id>/<run id>.log`: the output tail, capped at 64 KB.

use std::path::{Path, PathBuf};

use super::model::{CronSettings, Job, Run};
use crate::data_dir::{read_json, write_atomic, write_json, HostPaths};

/// The output tail kept per run.
pub const LOG_CAP_BYTES: usize = 64 * 1024;

pub fn cron_dir(paths: &HostPaths) -> PathBuf {
    paths.root.join("cron")
}

pub fn jobs_path(paths: &HostPaths) -> PathBuf {
    cron_dir(paths).join("jobs.json")
}

pub fn runs_dir(paths: &HostPaths, job_id: &str) -> PathBuf {
    cron_dir(paths).join("runs").join(job_id)
}

pub fn run_json_path(paths: &HostPaths, job_id: &str, run_id: &str) -> PathBuf {
    runs_dir(paths, job_id).join(format!("{run_id}.json"))
}

pub fn run_log_path(paths: &HostPaths, job_id: &str, run_id: &str) -> PathBuf {
    runs_dir(paths, job_id).join(format!("{run_id}.log"))
}

/// Loads every job, oldest-created first if the file predates ordering
/// guarantees (we don't resort — insertion order is preserved on save).
pub fn load_jobs(paths: &HostPaths) -> anyhow::Result<Vec<Job>> {
    Ok(read_json(&jobs_path(paths))?.unwrap_or_default())
}

pub fn save_jobs(paths: &HostPaths, jobs: &[Job]) -> anyhow::Result<()> {
    write_json(&jobs_path(paths), &jobs.to_vec())
}

/// Writes a run record atomically.
pub fn save_run(paths: &HostPaths, run: &Run) -> anyhow::Result<()> {
    write_json(&run_json_path(paths, &run.job_id, &run.id), run)
}

pub fn load_run(paths: &HostPaths, job_id: &str, run_id: &str) -> anyhow::Result<Option<Run>> {
    read_json(&run_json_path(paths, job_id, run_id))
}

/// Every run for a job, newest first (by `fired_at`, ties by id).
pub fn load_runs(paths: &HostPaths, job_id: &str) -> anyhow::Result<Vec<Run>> {
    let dir = runs_dir(paths, job_id);
    let mut runs = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(run) = read_json::<Run>(&path)? {
                runs.push(run);
            }
        }
    }
    runs.sort_by(|a, b| b.fired_at.cmp(&a.fired_at).then_with(|| b.id.cmp(&a.id)));
    Ok(runs)
}

/// Finds a run by id alone, scanning every job's run directory (a run id
/// is a short hex-ish slug made by the scheduler, effectively unique;
/// this is only used by `cron log <run id>`, a user-facing lookup, not a
/// hot path). Returns `(job_id, run)`.
pub fn find_run(paths: &HostPaths, run_id: &str) -> anyhow::Result<Option<(String, Run)>> {
    let base = cron_dir(paths).join("runs");
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(job_id) = entry.file_name().to_str().map(str::to_string) else { continue };
        if let Some(run) = load_run(paths, &job_id, run_id)? {
            return Ok(Some((job_id, run)));
        }
    }
    Ok(None)
}

/// Every run across every job, newest first.
pub fn load_all_runs(paths: &HostPaths) -> anyhow::Result<Vec<Run>> {
    let base = cron_dir(paths).join("runs");
    let mut runs = Vec::new();
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                runs.extend(load_runs(paths, name)?);
            }
        }
    }
    runs.sort_by(|a, b| b.fired_at.cmp(&a.fired_at).then_with(|| b.id.cmp(&a.id)));
    Ok(runs)
}

/// Appends to a run's log, keeping only the last [`LOG_CAP_BYTES`].
pub fn append_log(paths: &HostPaths, job_id: &str, run_id: &str, chunk: &[u8]) -> anyhow::Result<()> {
    let path = run_log_path(paths, job_id, run_id);
    let mut existing = std::fs::read(&path).unwrap_or_default();
    existing.extend_from_slice(chunk);
    if existing.len() > LOG_CAP_BYTES {
        let start = existing.len() - LOG_CAP_BYTES;
        existing.drain(..start);
    }
    write_atomic(&path, &existing)?;
    Ok(())
}

pub fn read_log(paths: &HostPaths, job_id: &str, run_id: &str) -> Option<Vec<u8>> {
    std::fs::read(run_log_path(paths, job_id, run_id)).ok()
}

/// Settings live in `config.json`'s `cron` block; this reads just that
/// block so callers that only touch cron don't need `HostConfig`.
pub fn load_settings(config_path: &Path) -> anyhow::Result<CronSettings> {
    let raw: Option<serde_json::Value> = read_json(config_path)?;
    let settings = raw.and_then(|v| v.get("cron").cloned()).map(serde_json::from_value).transpose()?.unwrap_or_default();
    Ok(settings)
}

/// Writes the `cron` settings block, preserving every other top-level key.
pub fn save_settings(config_path: &Path, settings: &CronSettings) -> anyhow::Result<()> {
    let mut raw: serde_json::Value = read_json(config_path)?.unwrap_or_else(|| serde_json::json!({}));
    if !raw.is_object() {
        raw = serde_json::json!({});
    }
    raw["cron"] = serde_json::to_value(settings)?;
    write_json(config_path, &raw)
}

/// Deletes every run (`.json` + `.log`) for a job beyond `retention_runs`
/// (newest kept) or older than `retention_days`. Returns how many were
/// removed.
pub fn prune_runs(paths: &HostPaths, job_id: &str, retention_runs: u32, retention_days: u32, now_ms: u64) -> anyhow::Result<usize> {
    let mut runs = load_runs(paths, job_id)?; // newest first
    let cutoff = now_ms.saturating_sub(u64::from(retention_days) * 24 * 60 * 60 * 1000);
    let mut removed = 0usize;
    for (i, run) in runs.iter().enumerate() {
        let keep_by_count = i < retention_runs as usize;
        let keep_by_age = run.fired_at >= cutoff;
        // Never prune a run that is still in progress.
        if matches!(run.status, super::model::RunStatus::Running | super::model::RunStatus::Queued) {
            continue;
        }
        if !(keep_by_count && keep_by_age) {
            let _ = std::fs::remove_file(run_json_path(paths, job_id, &run.id));
            let _ = std::fs::remove_file(run_log_path(paths, job_id, &run.id));
            removed += 1;
        }
    }
    runs.clear();
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::model::RunStatus;

    fn job(id: &str) -> Job {
        Job {
            id: id.into(),
            name: "test".into(),
            prompt: "do it".into(),
            schedule: "0 0 * * * *".into(),
            tz: "UTC".into(),
            cwd: "/tmp".into(),
            model: None,
            thinking: None,
            enabled: true,
            time_limit_s: 1800,
            cost_cap_usd: None,
            notify: None,
            session_link: None,
            created: 1,
            updated: 1,
        }
    }

    fn run(job_id: &str, id: &str, fired_at: u64, status: RunStatus) -> Run {
        Run { id: id.into(), job_id: job_id.into(), status, fired_at, started_at: None, ended_at: None, exit_code: None, summary: None, reason: None, session_path: None, cost_usd: None, missed: false, notify_errors: Vec::new(), wake: None }
    }

    #[test]
    fn jobs_round_trip_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        let jobs = vec![job("a"), job("b")];
        save_jobs(&paths, &jobs).unwrap();
        let back = load_jobs(&paths).unwrap();
        assert_eq!(back, jobs);
        // No stray tmp files left behind.
        let leftover: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-")).collect();
        assert!(leftover.is_empty());
    }

    #[test]
    fn run_and_log_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        let r = run("job1", "run1", 100, RunStatus::Ok);
        save_run(&paths, &r).unwrap();
        assert_eq!(load_run(&paths, "job1", "run1").unwrap(), Some(r));
        append_log(&paths, "job1", "run1", b"hello ").unwrap();
        append_log(&paths, "job1", "run1", b"world").unwrap();
        assert_eq!(read_log(&paths, "job1", "run1").unwrap(), b"hello world");
    }

    #[test]
    fn log_is_capped_at_64kb_keeping_the_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        append_log(&paths, "j", "r", &vec![b'a'; LOG_CAP_BYTES]).unwrap();
        append_log(&paths, "j", "r", b"TAIL").unwrap();
        let log = read_log(&paths, "j", "r").unwrap();
        assert_eq!(log.len(), LOG_CAP_BYTES);
        assert!(log.ends_with(b"TAIL"));
    }

    #[test]
    fn find_run_scans_every_job() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        save_run(&paths, &run("j1", "r1", 100, RunStatus::Ok)).unwrap();
        save_run(&paths, &run("j2", "r2", 200, RunStatus::Ok)).unwrap();
        let (job_id, found) = find_run(&paths, "r2").unwrap().expect("found");
        assert_eq!(job_id, "j2");
        assert_eq!(found.id, "r2");
        assert!(find_run(&paths, "missing").unwrap().is_none());
    }

    #[test]
    fn loads_runs_newest_first_across_jobs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        save_run(&paths, &run("j1", "r1", 100, RunStatus::Ok)).unwrap();
        save_run(&paths, &run("j1", "r2", 300, RunStatus::Ok)).unwrap();
        save_run(&paths, &run("j2", "r3", 200, RunStatus::Ok)).unwrap();
        let all = load_all_runs(&paths).unwrap();
        assert_eq!(all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["r2", "r3", "r1"]);
        let j1 = load_runs(&paths, "j1").unwrap();
        assert_eq!(j1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["r2", "r1"]);
    }

    #[test]
    fn settings_round_trip_next_to_other_config_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(&path, r#"{"forwarding":{"enabled":true}}"#).unwrap();
        let mut settings = load_settings(&path).unwrap();
        assert_eq!(settings, CronSettings::default());
        settings.paused = true;
        settings.concurrency = 5;
        save_settings(&path, &settings).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["forwarding"]["enabled"], true);
        assert_eq!(raw["cron"]["paused"], true);
        let back = load_settings(&path).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn prune_keeps_latest_n_and_recent_days() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        let day_ms = 24 * 60 * 60 * 1000u64;
        let now = 100 * day_ms;
        for i in 0..5 {
            // Each older by 10 days.
            save_run(&paths, &run("j", &format!("r{i}"), now - i * 10 * day_ms, RunStatus::Ok)).unwrap();
        }
        // Keep at most 3 by count.
        let removed = prune_runs(&paths, "j", 3, 1000, now).unwrap();
        assert_eq!(removed, 2);
        let left = load_runs(&paths, "j").unwrap();
        assert_eq!(left.len(), 3);
        assert_eq!(left.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["r0", "r1", "r2"]);
    }

    #[test]
    fn prune_never_removes_a_running_run() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        save_run(&paths, &run("j", "old-running", 0, RunStatus::Running)).unwrap();
        let removed = prune_runs(&paths, "j", 0, 0, 10_000_000).unwrap();
        assert_eq!(removed, 0);
        assert!(load_run(&paths, "j", "old-running").unwrap().is_some());
    }
}
