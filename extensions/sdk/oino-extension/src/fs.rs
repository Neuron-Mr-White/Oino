//! Files: named preopens (`/ext-global`, `/ext-workspace/<wsid>`, `/ext-session/<sid>`, `/tmp`,
//! `/cache`, `/logs`, `/project`, `/agent/<sub>`), atomic writes, locks and watchers. Plain
//! `std::fs` works on preopen paths.

use std::path::PathBuf;

use crate::Result;
use crate::bindings::oino::extension::fs as wf;
pub use crate::bindings::oino::extension::fs::{FsEvent, Preopen};

/// Every preopen.
#[must_use]
pub fn preopens() -> Vec<Preopen> {
    wf::preopens()
}

/// `/ext-global`.
#[must_use]
pub fn ext_global() -> PathBuf {
    PathBuf::from("/ext-global")
}

/// `/ext-workspace/<wsid>` for the current workspace (created on demand).
#[must_use]
pub fn ext_workspace() -> PathBuf {
    let dir = PathBuf::from("/ext-workspace").join(crate::host::workspace().id);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// `/ext-session/<sid>` (created on demand).
#[must_use]
pub fn ext_session(session_id: &str) -> PathBuf {
    let safe: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dir = PathBuf::from("/ext-session").join(safe);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Atomic write (temp + rename) to a writable preopen path.
pub fn atomic_write(path: &str, data: &[u8]) -> Result<()> {
    Ok(wf::atomic_write(path, data, None)?)
}

/// Reads JSON from a preopen path (`None` when missing).
pub fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::Error::msg(format!("{path}: {e}"))),
    }
}

/// Writes JSON atomically.
pub fn write_json(path: &str, value: &impl serde::Serialize) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

/// An advisory file lock; released on drop.
pub struct Lock(wf::Lock);

impl Lock {
    /// Acquires a lock on `path`.
    pub async fn acquire(path: &str, exclusive: bool) -> Result<Self> {
        Ok(Self(wf::Lock::acquire(path.to_owned(), exclusive).await?))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// A file watcher.
pub struct Watcher(wf::Watcher);

impl Watcher {
    /// Watches `path`.
    pub fn watch(path: &str, recursive: bool) -> Result<Self> {
        Ok(Self(wf::Watcher::watch(path, recursive)?))
    }
    /// Next batch of changes (`None` when closed).
    pub async fn next(&self) -> Option<Vec<FsEvent>> {
        self.0.next().await
    }
}
