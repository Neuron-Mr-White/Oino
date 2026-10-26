//! Daemon bookkeeping: `daemon.json`, the single-instance lock, pid liveness,
//! `status` and `stop`.

use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::store::{Layout, write_atomic};

/// Default idle timeout of the daemon, in minutes.
pub const DEFAULT_IDLE_MIN: u64 = 10;
/// How long `stop` waits for the daemon to exit before reporting failure.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// `daemon.json`: where the running daemon listens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// Daemon process id.
    pub pid: u32,
    /// Bound port.
    pub port: u16,
    /// Binary version.
    pub version: String,
    /// Start time.
    #[serde(rename = "startedAt", serialize_with = "crate::model::serialize_iso")]
    pub started_at: DateTime<Utc>,
    /// Bind address (defaults to 127.0.0.1 for entries written before this field).
    #[serde(default = "default_host")]
    pub host: String,
    /// Access token; present only for non-loopback binds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// `<home>/daemon.json`.
#[must_use]
pub fn info_path(layout: &Layout) -> PathBuf {
    layout.home.join("daemon.json")
}

/// `<home>/daemon.lock`.
#[must_use]
pub fn lock_path(layout: &Layout) -> PathBuf {
    layout.home.join("daemon.lock")
}

/// Reads `daemon.json` (`None` when missing or unreadable).
#[must_use]
pub fn read_info(layout: &Layout) -> Option<DaemonInfo> {
    let text = fs::read_to_string(info_path(layout)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes `daemon.json` atomically.
pub fn write_info(layout: &Layout, info: &DaemonInfo) -> Result<()> {
    fs::create_dir_all(&layout.home)?;
    write_atomic(
        &info_path(layout),
        &format!("{}\n", serde_json::to_string_pretty(info)?),
    )
}

/// Removes `daemon.json` (best effort).
pub fn remove_info(layout: &Layout) {
    let _ = fs::remove_file(info_path(layout));
}

/// Held for the daemon's lifetime; releases the flock on drop.
pub struct DaemonLock {
    file: File,
}

/// Exclusive daemon lock, or `None` when another daemon already holds it.
pub fn take_lock(layout: &Layout) -> Result<Option<DaemonLock>> {
    fs::create_dir_all(&layout.home)?;
    let path = lock_path(layout);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|err| Error::Io(format!("cannot open {}: {err}", path.display())))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(DaemonLock { file })),
        Err(_) => Ok(None),
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Is this pid alive? `kill(pid, 0)` semantics, except that a **zombie** (a
/// process that exited but whose parent has not reaped it yet) counts as dead —
/// otherwise `stop` and the stale-run check would wait forever on a corpse.
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            // Format: `<pid> (<comm>) <state> …` — comm may contain spaces/parens,
            // so read the state after the LAST ')'.
            if let Some(close) = stat.rfind(')')
                && stat[close + 1..].trim_start().starts_with('Z')
            {
                return false;
            }
        }
    }
    #[cfg(unix)]
    {
        // EPERM means the process exists but belongs to someone else.
        let alive = match i32::try_from(pid) {
            Ok(raw) => matches!(
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(raw), None),
                Ok(()) | Err(nix::errno::Errno::EPERM)
            ),
            Err(_) => false,
        };
        // kill(pid, 0) succeeds on zombies too. /proc does not exist off Linux
        // (macOS), so fall back to `ps` state there: a `Z*` state (or no such
        // process at all) means dead.
        #[cfg(target_os = "linux")]
        {
            alive
        }
        #[cfg(all(unix, not(target_os = "linux")))]
        {
            if !alive {
                return false;
            }
            match std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
            {
                Ok(out) => {
                    let stat = String::from_utf8_lossy(&out.stdout);
                    let stat = stat.trim();
                    !(stat.is_empty() || stat.starts_with('Z'))
                }
                // `ps` missing or refused: trust kill(0).
                Err(_) => true,
            }
        }
    }
    #[cfg(not(unix))]
    {
        // `tasklist` prints the process as a CSV row when it exists and an
        // "INFO: No tasks…" line otherwise; if it can't run, assume alive.
        match std::process::Command::new("tasklist")
            .arg("/FI")
            .arg(format!("PID eq {pid}"))
            .args(["/NH", "/FO", "CSV"])
            .output()
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\"")),
            Err(_) => true,
        }
    }
}

#[cfg(unix)]
fn send_terminate(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(|raw| {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(raw),
            nix::sys::signal::Signal::SIGTERM,
        )
        .is_ok()
    })
}

#[cfg(windows)]
fn send_terminate(pid: u32) -> bool {
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// `status`: the recorded daemon plus whether that pid is actually alive.
pub fn status(layout: &Layout) -> Result<Value> {
    match read_info(layout) {
        Some(info) => Ok(json!({
            "daemon": info,
            "alive": pid_alive(info.pid),
            "home": layout.home.to_string_lossy(),
            "binary": current_binary(),
            "hostname": crate::commands::hostname(),
        })),
        None => Ok(json!({
            "daemon": Value::Null,
            "alive": false,
            "home": layout.home.to_string_lossy(),
            "binary": current_binary(),
            "hostname": crate::commands::hostname(),
        })),
    }
}

/// This binary's absolute path (`null` when the OS cannot say) — Oino hands it to
/// the agent so its bash calls work even when the binary is not on `PATH`.
fn current_binary() -> Value {
    std::env::current_exe().ok().map_or(Value::Null, |path| {
        Value::String(path.to_string_lossy().into_owned())
    })
}

/// `stop`: SIGTERM the recorded pid and wait up to [`STOP_TIMEOUT`].
pub fn stop(layout: &Layout, timeout: Duration) -> Result<Value> {
    let Some(info) = read_info(layout) else {
        return Ok(json!({ "stopped": false, "reason": "no daemon.json — nothing to stop" }));
    };
    if !pid_alive(info.pid) {
        remove_info(layout);
        return Ok(json!({
            "stopped": false,
            "pid": info.pid,
            "reason": "recorded pid is not alive; removed the stale daemon.json",
        }));
    }
    let signalled = send_terminate(info.pid);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(info.pid) {
            remove_info(layout);
            return Ok(json!({ "stopped": true, "pid": info.pid }));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(json!({
        "stopped": false,
        "pid": info.pid,
        "signalled": signalled,
        "reason": format!("pid {} still alive after {}s", info.pid, timeout.as_secs()),
    }))
}
