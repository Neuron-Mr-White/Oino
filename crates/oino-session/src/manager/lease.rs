//! D-LOCK single-writer lease per session file, kernel-arbitrated (spec `docs/dev/v2/specs/core.md`
//! §2.1 D-LOCK / D-TAKEOVER).
//!
//! The lease is an exclusive `flock` (POSIX) / `LockFileEx` byte-range lock (Windows) on a
//! `<session file>.lock` sibling. The kernel releases both on process death, so a crashed
//! holder never blocks a successor and there is no expiry that could expropriate a stalled
//! writer. The lock file is never unlinked on release (POSIX locks name an inode); it is
//! removed only when the session file itself is gone (best-effort — see [`SessionLease`]).
//!
//! Advisory metadata is kept next to the lock (`<session file>.lock.json`) and a takeover
//! request is signalled through `<session file>.takeover` (D-TAKEOVER). Both are hints: the
//! kernel lock remains the arbiter.
//!
//! An in-process registry guards against the same process locking twice (`flock` is
//! per-descriptor): a second acquire fails with [`SessionLockError::HeldBy`] naming the
//! caller's own process.
//!
//! NFS/home mounts are out of scope (documented in the spec).

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::fs_std::FileExt as _;
use serde::{Deserialize, Serialize};

use super::files;
use crate::error::SessionLockError;

/// Holder identity recorded in the advisory lock metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseInfo {
    /// Holder process id.
    pub pid: u32,
    /// Machine boot id the holder started under (see [`LeaseInfo::for_process`]).
    pub boot_id: String,
    /// Host name (diagnostics; same-host comparison for [`LeaseInfo::dead`]).
    pub host: String,
    /// Human label of the holding client, e.g. `oino rpc` or an embedding host.
    pub client: String,
}

/// `<session file>.lock.json` advisory metadata (D-LOCK).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LeaseMetadata {
    /// Format version.
    v: u32,
    #[serde(flatten)]
    /// Holder identity.
    info: LeaseInfo,
    /// Acquisition time (Unix ms).
    #[serde(rename = "startedAt")]
    started_at: u64,
}

/// `<session file>.takeover` request (D-TAKEOVER).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverRequest {
    /// Format version.
    pub v: u32,
    /// Who asked for the session.
    pub requester: LeaseInfo,
    /// Request time (Unix ms).
    pub at: u64,
}

/// In-process registry of held leases: `flock` is per-descriptor, so without this the same
/// process could lock a file twice (spec D-LOCK).
static SESSION_LOCKS: OnceLock<Mutex<HashMap<PathBuf, LeaseInfo>>> = OnceLock::new();
/// Process-wide boot id (first caller's state dir wins; boot ids are machine-global).
static BOOT_ID: OnceLock<String> = OnceLock::new();

fn registry() -> MutexGuard<'static, HashMap<PathBuf, LeaseInfo>> {
    SESSION_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0))
}

/// `<session file>.lock` sibling path.
#[must_use]
pub fn lock_file_path(session_file: &Path) -> PathBuf {
    sibling(session_file, ".lock")
}

/// `<session file>.lock.json` metadata path.
#[must_use]
pub fn lock_metadata_path(session_file: &Path) -> PathBuf {
    sibling(session_file, ".lock.json")
}

/// `<session file>.takeover` request path.
#[must_use]
pub fn takeover_path(session_file: &Path) -> PathBuf {
    sibling(session_file, ".takeover")
}

fn sibling(session_file: &Path, suffix: &str) -> PathBuf {
    let mut name = session_file.as_os_str().to_os_string();
    name.push(suffix);
    session_file.with_file_name(name)
}

fn hostname() -> String {
    if cfg!(target_os = "linux")
        && let Ok(host) = std::fs::read_to_string("/proc/sys/kernel/hostname")
        && !host.trim().is_empty()
    {
        return host.trim().to_owned();
    }
    if cfg!(windows)
        && let Some(host) = std::env::var_os("COMPUTERNAME")
    {
        return host.to_string_lossy().into_owned();
    }
    std::env::var_os("HOSTNAME").map_or_else(
        || "unknown-host".to_owned(),
        |host| host.to_string_lossy().into_owned(),
    )
}

/// Machine boot id: `/proc/sys/kernel/random/boot_id` (Linux), else a random id persisted
/// once under `<state dir>/oino.boot-id` (spec D-LOCK).
fn boot_id(state_dir: &Path) -> String {
    if let Ok(boot) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        && !boot.trim().is_empty()
    {
        return boot.trim().to_owned();
    }
    let path = state_dir.join("oino.boot-id");
    if let Ok(existing) = std::fs::read_to_string(&path)
        && !existing.trim().is_empty()
    {
        return existing.trim().to_owned();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(state_dir);
    let _ = std::fs::write(&path, &id);
    id
}

/// Boot id of this machine right now (cache, then `/proc`; `None` where unknowable).
fn current_boot_id() -> Option<String> {
    BOOT_ID.get().cloned().or_else(|| {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|boot| boot.trim().to_owned())
            .filter(|boot| !boot.is_empty())
    })
}

/// Whether `pid` is alive. Used for diagnostics and the takeover wait loop only — the kernel
/// lock remains the arbiter, so a wrong answer never breaks correctness (worst case: no
/// fast path / a stale wait).
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // `kill(pid, 0)`: `Ok` or `EPERM` (exists, not ours) ⇒ alive; `ESRCH` ⇒ gone.
        let sent = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap_or(-1)),
            None,
        );
        !matches!(sent, Err(nix::errno::Errno::ESRCH))
    }
    #[cfg(windows)]
    {
        // `tasklist` filter is the std-only probe; on any doubt, assume alive.
        std::process::Command::new("tasklist")
            .args(["/NH", "/FI", &format!("PID eq {pid}")])
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
            })
            .unwrap_or(true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

impl LeaseInfo {
    /// Identity of the current process as a lease holder. `state_dir` is only consulted on
    /// machines without `/proc` (see [`boot_id`]); the first call wins for the process.
    #[must_use]
    pub fn for_process(client: &str, state_dir: &Path) -> Self {
        let boot_id = BOOT_ID.get_or_init(|| boot_id(state_dir)).clone();
        Self {
            pid: std::process::id(),
            boot_id,
            host: hostname(),
            client: client.to_owned(),
        }
    }

    /// `"oino rpc (pid 123 on host)"`-style label for messages.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("{} (pid {} on {})", self.client, self.pid, self.host)
    }

    /// Same-host, same-boot holder whose process is gone. Advisory only: the kernel lock
    /// decides, this only skips the takeover dance for a certainly-dead holder (kernel
    /// release lag) and powers diagnostics.
    #[must_use]
    pub fn dead(&self) -> bool {
        self.pid != 0
            && !self.host.is_empty()
            && self.host == hostname()
            && current_boot_id().is_some_and(|boot| boot == self.boot_id)
            && !pid_alive(self.pid)
    }
}

fn read_metadata(session_file: &Path) -> LeaseInfo {
    let fallback = LeaseInfo {
        pid: 0,
        boot_id: String::new(),
        host: String::new(),
        client: "another Oino process".to_owned(),
    };
    std::fs::read_to_string(lock_metadata_path(session_file))
        .ok()
        .and_then(|text| serde_json::from_str::<LeaseMetadata>(&text).ok())
        .map_or(fallback, |metadata| metadata.info)
}

fn write_metadata(session_file: &Path, holder: &LeaseInfo) {
    let metadata = LeaseMetadata {
        v: 1,
        info: holder.clone(),
        started_at: now_ms(),
    };
    if let Ok(text) = serde_json::to_string(&metadata) {
        let _ = files::write_atomic(&lock_metadata_path(session_file), &text);
    }
}

/// A held single-writer lease on one session file. Dropping (or [`SessionLease::release`])
/// drops the kernel lock, removes the advisory metadata and any pending takeover request.
///
/// The `.lock` file itself is left in place while the session file exists (POSIX locks name
/// an inode); it is removed when the session file does not exist (a lease eagerly taken for
/// a session that was never written), best-effort on Windows where a live byte-range lock
/// blocks deletion.
#[derive(Debug)]
pub struct SessionLease {
    session_file: PathBuf,
    lock_file: PathBuf,
    file: Option<File>,
    info: LeaseInfo,
}

impl SessionLease {
    /// Acquires the exclusive lease on `session_file` for `holder`. For a session file that
    /// does not exist yet (new session, file created at the first flush) the parent directory
    /// is created so the `.lock` sibling can claim the planned path before the file exists
    /// (D-LOCK “lazily at the first flush … before the file is created”).
    ///
    /// # Errors
    ///
    /// [`SessionLockError::HeldBy`] when this process or another writer already holds it
    /// (the info names the holder when the advisory metadata is readable);
    /// [`SessionLockError::Io`] when the lock file cannot be created or locked.
    pub fn acquire(session_file: &Path, holder: &LeaseInfo) -> Result<Self, SessionLockError> {
        let session_file = files::resolve_path(session_file);
        let lock_file = lock_file_path(&session_file);
        let mut held = registry();
        if let Some(existing) = held.get(&session_file) {
            return Err(SessionLockError::HeldBy(existing.clone()));
        }
        if !session_file.exists()
            && let Some(parent) = session_file.parent()
        {
            // Pre-create the sessions layout directory for planned files so the `.lock`
            // sibling can claim the path (best-effort: the open below reports real errors).
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_file)
            .map_err(|source| SessionLockError::Io {
                path: lock_file.clone(),
                source,
            })?;
        match file.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) => {
                return Err(SessionLockError::HeldBy(read_metadata(&session_file)));
            }
            Err(source) => {
                return Err(SessionLockError::Io {
                    path: lock_file,
                    source,
                });
            }
        }
        held.insert(session_file.clone(), holder.clone());
        drop(held);
        write_metadata(&session_file, holder);
        Ok(Self {
            session_file,
            lock_file,
            file: Some(file),
            info: holder.clone(),
        })
    }

    /// The holder identity this lease was acquired for.
    #[must_use]
    pub fn info(&self) -> &LeaseInfo {
        &self.info
    }

    /// The leased session file (resolved absolute path).
    #[must_use]
    pub fn session_file(&self) -> &Path {
        &self.session_file
    }

    /// Releases the lease: deletes any pending takeover request, drops the kernel lock and
    /// removes the advisory metadata (all best-effort except the lock itself).
    pub fn release(self) {
        // `Drop` performs the release; consuming here is the explicit API.
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        registry().remove(&self.session_file);
        self.file = None; // kernel lock goes with the descriptor
        let _ = std::fs::remove_file(lock_metadata_path(&self.session_file));
        let _ = std::fs::remove_file(takeover_path(&self.session_file));
        if !self.session_file.exists() {
            let _ = std::fs::remove_file(&self.lock_file);
        }
    }
}

/// Writes the `<session file>.takeover` request asking the holder to close (D-TAKEOVER).
///
/// # Errors
///
/// [`SessionLockError::Io`] when the request cannot be written.
pub fn write_takeover_request(
    session_file: &Path,
    requester: &LeaseInfo,
) -> Result<(), SessionLockError> {
    let request = TakeoverRequest {
        v: 1,
        requester: requester.clone(),
        at: now_ms(),
    };
    let text = serde_json::to_string(&request).map_err(|source| SessionLockError::Io {
        path: takeover_path(session_file),
        source: std::io::Error::other(source),
    })?;
    files::write_atomic(&takeover_path(session_file), &text).map_err(|source| {
        SessionLockError::Io {
            path: takeover_path(session_file),
            source: std::io::Error::other(source.to_string()),
        }
    })
}

/// Reads a pending takeover request, when present and well-formed.
#[must_use]
pub fn read_takeover_request(session_file: &Path) -> Option<TakeoverRequest> {
    std::fs::read_to_string(takeover_path(session_file))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// Removes a pending takeover request (best-effort).
pub fn clear_takeover_request(session_file: &Path) {
    let _ = std::fs::remove_file(takeover_path(session_file));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(client: &str) -> LeaseInfo {
        LeaseInfo::for_process(client, std::env::temp_dir().as_path())
    }

    #[test]
    fn takeover_request_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        assert!(read_takeover_request(&file).is_none());
        write_takeover_request(&file, &info("oino test")).unwrap();
        let request = read_takeover_request(&file).unwrap();
        assert_eq!(request.requester, info("oino test"));
        assert_eq!(request.v, 1);
        clear_takeover_request(&file);
        assert!(read_takeover_request(&file).is_none());
    }

    #[test]
    fn lock_paths_are_siblings() {
        let file = Path::new("/sessions/--x--/1_a.jsonl");
        assert_eq!(lock_file_path(file).file_name().unwrap(), "1_a.jsonl.lock");
        assert_eq!(
            lock_metadata_path(file).file_name().unwrap(),
            "1_a.jsonl.lock.json"
        );
        assert_eq!(
            takeover_path(file).file_name().unwrap(),
            "1_a.jsonl.takeover"
        );
    }

    #[test]
    fn dead_flags_same_machine_pid() {
        assert!(!info("x").dead(), "this process is alive");
        let mut gone = info("x");
        gone.pid = 2_000_000; // in pid space but (practically) unmapped on this host
        assert!(gone.dead(), "an unmapped pid on the same host+boot is gone");
        let mut other_host = info("x");
        other_host.host = "elsewhere".to_owned();
        assert!(!other_host.dead(), "another host is none of our business");
    }
}
