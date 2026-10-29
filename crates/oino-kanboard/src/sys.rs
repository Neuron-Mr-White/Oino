//! Host-facing process and path helpers: agent-dir resolution, pid liveness
//! (moved out of `daemon` when the board went in-process, round 5).

use std::fs;
use std::path::PathBuf;

/// Oino's agent dir: `$OINO_AGENT_DIR` (tilde-expanded), else `~/.oino`.
#[must_use]
pub fn agent_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Ok(dir) = std::env::var("OINO_AGENT_DIR")
        && !dir.trim().is_empty()
    {
        let dir = dir.trim();
        if let Some(rest) = dir.strip_prefix("~/") {
            return home.map(|h| h.join(rest));
        }
        return Some(PathBuf::from(dir));
    }
    home.map(|h| h.join(".oino"))
}

/// Is this pid alive? `kill(pid, 0)` semantics, except that a **zombie** (a
/// process that exited but whose parent has not reaped it yet) counts as dead —
/// otherwise the stale-claim reaper would wait forever on a corpse.
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
