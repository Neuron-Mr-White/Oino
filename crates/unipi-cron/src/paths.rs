//! Every file the scheduler owns, under `<agent dir>/cron/` (`~/.oino/cron/` by default).
//!
//! Layout:
//! - `jobs.json` — every job, written atomically under a lock.
//! - `runs/<job id>/<run id>.json` — one run record.
//! - `runs/<job id>/<run id>.log` — the run's output tail (capped at 64 KiB).
//! - `settings.json` — the scheduler's own settings block.
//! - `scheduler.lock` — the single-owner lock held while a scheduler ticks.
//! - `serve.json` — `{pid, port}` of the running daemon (its control socket).

use std::path::{Path, PathBuf};

use crate::error::{CronError, Result};

/// The agent dir override variable (same as everywhere else in Oino).
pub const AGENT_DIR_VAR: &str = "OINO_AGENT_DIR";

/// The default agent dir, `~/.oino`.
///
/// # Errors
/// [`CronError::NoDir`] when neither `$OINO_AGENT_DIR` nor the home directory resolves.
pub fn agent_dir() -> Result<PathBuf> {
    agent_dir_from(std::env::var_os(AGENT_DIR_VAR), dirs::home_dir())
}

/// Pure core of [`agent_dir`] (tests inject the inputs).
///
/// # Errors
/// [`CronError::NoDir`] when neither input resolves.
pub fn agent_dir_from(var: Option<std::ffi::OsString>, home: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = var.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    home.map(|home| home.join(".oino"))
        .ok_or_else(|| CronError::NoDir("$HOME is not set and OINO_AGENT_DIR is empty".into()))
}

/// Paths of one cron root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronPaths {
    /// `<agent dir>/cron`.
    pub root: PathBuf,
    /// The agent dir (`~/.oino`), needed to find run session files.
    pub agent_dir: PathBuf,
}

impl CronPaths {
    /// Paths for `agent_dir`.
    #[must_use]
    pub fn new(agent_dir: impl Into<PathBuf>) -> Self {
        let agent_dir = agent_dir.into();
        Self {
            root: agent_dir.join("cron"),
            agent_dir,
        }
    }

    /// The default paths (`~/.oino/cron`, `$OINO_AGENT_DIR` honoured).
    ///
    /// # Errors
    /// [`CronError::NoDir`] when no agent dir resolves.
    pub fn from_agent_dir() -> Result<Self> {
        agent_dir().map(Self::new)
    }

    /// `jobs.json`.
    #[must_use]
    pub fn jobs(&self) -> PathBuf {
        self.root.join("jobs.json")
    }
    /// The lock serialising `jobs.json` writes.
    #[must_use]
    pub fn jobs_lock(&self) -> PathBuf {
        self.root.join("jobs.lock")
    }
    /// `settings.json`.
    #[must_use]
    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    /// The single-owner scheduler lock.
    #[must_use]
    pub fn owner_lock(&self) -> PathBuf {
        self.root.join("scheduler.lock")
    }
    /// `serve.json` (the running daemon's pid and port).
    #[must_use]
    pub fn serve_info(&self) -> PathBuf {
        self.root.join("serve.json")
    }
    /// `runs/`.
    #[must_use]
    pub fn runs(&self) -> PathBuf {
        self.root.join("runs")
    }
    /// `runs/<job id>/`.
    #[must_use]
    pub fn job_runs(&self, job_id: &str) -> PathBuf {
        self.runs().join(job_id)
    }
    /// `runs/<job id>/<run id>.json`.
    #[must_use]
    pub fn run_json(&self, job_id: &str, run_id: &str) -> PathBuf {
        self.job_runs(job_id).join(format!("{run_id}.json"))
    }
    /// `runs/<job id>/<run id>.log`.
    #[must_use]
    pub fn run_log(&self, job_id: &str, run_id: &str) -> PathBuf {
        self.job_runs(job_id).join(format!("{run_id}.log"))
    }

    /// Creates the cron root if needed.
    ///
    /// # Errors
    /// [`CronError::Io`] when the directory cannot be created.
    pub fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root).map_err(|e| CronError::io(e, &self.root))
    }
}

/// Writes `bytes` to `path` atomically (temp file + rename).
///
/// # Errors
/// [`CronError::Io`] on any file error.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CronError::io(e, parent))?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| CronError::io(e, &tmp))?;
    std::fs::rename(&tmp, path).map_err(|e| CronError::io(e, path))
}

/// Writes `value` as pretty JSON atomically.
///
/// # Errors
/// [`CronError::Json`] when the value cannot be serialised, [`CronError::Io`] on write.
pub fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| CronError::json(e, path))?;
    atomic_write(path, &bytes)
}

/// Reads and parses JSON from `path`; `Ok(None)` when it does not exist.
///
/// # Errors
/// [`CronError::Io`] on read errors other than "not found", [`CronError::Json`] on bad JSON.
pub fn read_json<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| CronError::json(e, path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CronError::io(e, path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_under_the_agent_dir() {
        let paths = CronPaths::new("/tmp/agent");
        assert_eq!(paths.root, PathBuf::from("/tmp/agent/cron"));
        assert_eq!(paths.jobs(), PathBuf::from("/tmp/agent/cron/jobs.json"));
        assert_eq!(
            paths.run_log("j", "r"),
            PathBuf::from("/tmp/agent/cron/runs/j/r.log")
        );
    }

    #[test]
    fn atomic_json_round_trips_and_leaves_no_tmp() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("x.json");
        atomic_write_json(&path, &serde_json::json!({"a": 1})).unwrap();
        let back: Option<serde_json::Value> = read_json(&path).unwrap();
        assert_eq!(back, Some(serde_json::json!({"a": 1})));
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn missing_file_reads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let value: Option<serde_json::Value> = read_json(&tmp.path().join("nope.json")).unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn agent_dir_resolution_rules() {
        assert_eq!(
            agent_dir_from(Some("/tmp/agent-x".into()), None).unwrap(),
            PathBuf::from("/tmp/agent-x")
        );
        // Empty override falls through to home.
        assert_eq!(
            agent_dir_from(Some(String::new().into()), Some(PathBuf::from("/home/me"))).unwrap(),
            PathBuf::from("/home/me/.oino")
        );
        assert_eq!(
            agent_dir_from(None, Some(PathBuf::from("/home/me"))).unwrap(),
            PathBuf::from("/home/me/.oino")
        );
        assert!(agent_dir_from(None, None).is_err());
    }
}
