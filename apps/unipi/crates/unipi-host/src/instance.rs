//! UNI-228 (plan H1, docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md §3.2): one
//! `serve` per data dir. Two hosts on the same `secret.key` would fight over
//! the relay home and make phones flap, so `serve` takes an exclusive lock on
//! `<dir>/host.lock` for its whole life (the OS drops it when the process
//! dies — no stale-lock cleanup needed). The file also records who runs it:
//! `{pid, version, mode, started_at}`, which `unipi-host status` and the
//! desktop app read ("Run by: this app / a terminal").
//!
//! v2 (spec §5.2): modes are `App` (the desktop app's sidecar) and `Manual`
//! (a terminal). There is no service install any more — systemd/LaunchAgent/
//! logon-task modes are gone with the `linger` subcommand.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data_dir::HostPaths;

/// Exit code for "another serve already owns this data dir" (EX_TEMPFAIL).
pub const EXIT_ALREADY_RUNNING: i32 = 75;

/// Who started this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Spawned by the UniPi desktop app (bundled sidecar).
    App,
    /// Started by hand in a terminal.
    #[default]
    Manual,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::App => "app",
            Mode::Manual => "manual",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "app" => Some(Mode::App),
            "manual" => Some(Mode::Manual),
            _ => None,
        }
    }
    /// The mode when `--mode` isn't given. v2 has no service install, so a
    /// host not started by the app is always a manual one.
    pub fn detect() -> Mode {
        Mode::Manual
    }
}

/// What `host.lock` holds while a host runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub pid: u32,
    pub version: String,
    pub mode: Mode,
    /// Unix seconds.
    pub started_at: u64,
}

impl HostPaths {
    pub fn host_lock(&self) -> std::path::PathBuf {
        self.root.join("host.lock")
    }
}

/// Holding this keeps the lock; drop it (or exit) to release.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    pub owner: Owner,
}

/// Why `acquire` didn't get the lock.
#[derive(Debug)]
pub enum AcquireError {
    /// Another live host holds it (its owner info, when readable).
    Held(Option<Owner>),
    Io(std::io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquireError::Held(Some(o)) => write!(
                f,
                "unipi-host is already running (pid {}, mode {}, version {})",
                o.pid,
                o.mode.as_str(),
                o.version
            ),
            AcquireError::Held(None) => write!(f, "unipi-host is already running"),
            AcquireError::Io(e) => write!(f, "can't lock the host data dir: {e}"),
        }
    }
}
impl std::error::Error for AcquireError {}

fn open(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

fn read_owner(file: &mut File) -> Option<Owner> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut text).ok()?;
    serde_json::from_str(text.trim()).ok()
}

/// Take the data dir's exclusive lock and record `mode` as its owner.
pub fn acquire(paths: &HostPaths, mode: Mode) -> Result<InstanceLock, AcquireError> {
    let mut file = open(&paths.host_lock()).map_err(AcquireError::Io)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            let mut owner = read_owner(&mut file);
            for _ in 0..20 {
                if owner.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
                owner = read_owner(&mut file);
            }
            return Err(AcquireError::Held(owner));
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(AcquireError::Io(e)),
    }
    let owner = Owner {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode,
        started_at: crate::pairing::now_unix(),
    };
    let body = serde_json::to_string(&owner).expect("owner serializes");
    file.set_len(0).map_err(AcquireError::Io)?;
    file.seek(SeekFrom::Start(0)).map_err(AcquireError::Io)?;
    file.write_all(body.as_bytes())
        .and_then(|_| file.flush())
        .map_err(AcquireError::Io)?;
    Ok(InstanceLock { _file: file, owner })
}

/// `unipi-host status`: is a host running on this dir, and who runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Status {
    /// No host: the lock is free (a leftover file from a dead host is ignored).
    NotRunning {
        /// A crashed/killed host left its owner info behind.
        #[serde(skip_serializing_if = "Option::is_none")]
        last: Option<Owner>,
    },
    Running {
        owner: Option<Owner>,
    },
}

/// Probe the lock without keeping it.
pub fn status(paths: &HostPaths) -> std::io::Result<Status> {
    let path = paths.host_lock();
    if !path.exists() {
        return Ok(Status::NotRunning { last: None });
    }
    let mut file = open(&path)?;
    match file.try_lock() {
        Ok(()) => {
            let last = read_owner(&mut file);
            let _ = file.unlock();
            Ok(Status::NotRunning { last })
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            // The owner writes its info right after locking: give it a moment.
            let mut owner = read_owner(&mut file);
            for _ in 0..20 {
                if owner.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
                owner = read_owner(&mut file);
            }
            Ok(Status::Running { owner })
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// `--exit-with-parent <pid>`: end this process within ~1 s of `pid` dying
/// (the desktop app's sidecar must not outlive the app). Linux also sets
/// PR_SET_PDEATHSIG when `pid` is our real parent; the poll covers the rest
/// (macOS, reparenting races).
pub fn exit_with_parent(pid: u32, on_exit: impl FnOnce() + Send + 'static) {
    #[cfg(target_os = "linux")]
    unsafe {
        if libc::getppid() as u32 == pid {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        }
    }
    std::thread::spawn(move || loop {
        if !alive(pid) {
            on_exit();
            std::process::exit(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    });
}

/// Is `pid` a live process?
pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0): ESRCH = gone; EPERM = alive but not ours.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_refused_with_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(dir.path());
        let first = acquire(&paths, Mode::App).unwrap();
        match acquire(&paths, Mode::Manual) {
            Err(AcquireError::Held(Some(o))) => {
                assert_eq!(o.pid, std::process::id());
                assert_eq!(o.mode, Mode::App);
            }
            other => panic!("expected Held, got {other:?}"),
        }
        assert!(matches!(
            status(&paths).unwrap(),
            Status::Running {
                owner: Some(Owner {
                    mode: Mode::App,
                    ..
                })
            }
        ));
        drop(first);
        // Released: the leftover file reports the last owner, not "running".
        // (Parallel tests that fork can hold a copy of the fd until their exec — wait it out.)
        let mut st = status(&paths).unwrap();
        for _ in 0..40 {
            if matches!(st, Status::NotRunning { .. }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            st = status(&paths).unwrap();
        }
        assert!(matches!(st, Status::NotRunning { last: Some(_) }), "{st:?}");
        let again = acquire(&paths, Mode::Manual).unwrap();
        assert_eq!(again.owner.mode, Mode::Manual);
    }

    #[test]
    fn status_without_a_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            status(&HostPaths::from_dir(dir.path())).unwrap(),
            Status::NotRunning { last: None }
        );
    }

    #[test]
    fn modes_round_trip() {
        for m in [Mode::App, Mode::Manual] {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert_eq!(Mode::parse("x"), None);
        assert_eq!(Mode::parse("service"), None, "no service install in v2");
        assert_eq!(Mode::detect(), Mode::Manual);
    }
}
