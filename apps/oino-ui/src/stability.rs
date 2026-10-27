//! Crash and stall diagnostics for the GPUI shells (docs/dev/v1/stability.md).
//!
//! - [`install_panic_hook`]: every panic writes `desktop-crash-<time>-<pid>.log` (thread,
//!   message, location, backtrace, recent activity) into the log directory. A panic on the UI
//!   thread or a GPUI worker aborts the process after the log is written: unwinding there kills
//!   the worker, and the main thread later panics polling the dead task (or the app freezes
//!   waiting for it). Panics on tokio threads are caught by tokio and only logged.
//! - [`Watchdog`]: the UI thread beats every [`BEAT`]; a watchdog thread logs a stall when no
//!   beat arrived for [`STALL`] (`desktop-stall-<time>-<pid>.log`: duration, the UI activity
//!   breadcrumb, the kernel state / wait channel / syscall of every thread, and with
//!   `OINO_WATCHDOG_GDB=1` a `gdb` backtrace when ptrace is permitted).
//! - [`activity`]: a breadcrumb guard naming what the UI thread is doing (shown in both logs).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// UI heartbeat period.
pub const BEAT: Duration = Duration::from_millis(250);
/// A UI thread silent this long is reported as stalled.
pub const STALL: Duration = Duration::from_secs(2);

/// What the UI thread is doing (breadcrumb) and since when (ms since process start).
static ACTIVITY: Mutex<(&str, u64)> = Mutex::new(("idle", 0));
/// Process start reference for [`ACTIVITY`] and the heartbeat.
static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn now_ms() -> u64 {
    let epoch = EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Restores the previous breadcrumb when dropped.
#[derive(Debug)]
#[must_use = "the breadcrumb is cleared when the guard drops"]
pub struct ActivityGuard {
    previous: (&'static str, u64),
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        *ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner) = self.previous;
    }
}

/// Names what the UI thread does until the guard drops (cheap: one uncontended lock).
pub fn activity(label: &'static str) -> ActivityGuard {
    let mut slot = ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner);
    let previous = *slot;
    *slot = (label, now_ms());
    ActivityGuard { previous }
}

/// The current breadcrumb and for how long it has been set.
#[must_use]
pub fn current_activity() -> (&'static str, Duration) {
    let (label, since) = *ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner);
    (label, Duration::from_millis(now_ms().saturating_sub(since)))
}

/// `YYYYMMDD-HHMMSS` (UTC) of now, for log file names.
#[must_use]
pub fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil(i64::try_from(days).unwrap_or(0));
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's `civil_from_days`).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Whether a panic on thread `name` must abort the process (UI thread and GPUI workers;
/// tokio workers `oino-core*` and other helper threads unwind as usual).
#[must_use]
pub fn fatal_thread(name: Option<&str>) -> bool {
    match name {
        Some("main") => true,
        Some(n) => n.starts_with("Worker-") || n.starts_with("gpui"),
        // Unnamed threads: GPUI's are named; others are helpers.
        None => false,
    }
}

static CRASH_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Installs the crash-log panic hook (chains the previous hook, which prints to stderr).
pub fn install_panic_hook(log_dir: &Path, app: &'static str) {
    *CRASH_DIR.lock().unwrap_or_else(PoisonError::into_inner) = Some(log_dir.to_path_buf());
    let _ = EPOCH.get_or_init(Instant::now);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().map(str::to_owned);
        let fatal = fatal_thread(name.as_deref());
        let backtrace = std::backtrace::Backtrace::force_capture();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let (activity, for_how_long) = current_activity();
        let mut text = String::new();
        let _ = writeln!(
            text,
            "{app} {} crashed (pid {})",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        );
        let _ = writeln!(
            text,
            "thread: {} ({:?})",
            name.as_deref().unwrap_or("<unnamed>"),
            thread.id()
        );
        let _ = writeln!(text, "panic: {message}");
        let _ = writeln!(text, "at: {location}");
        let _ = writeln!(
            text,
            "ui activity: {activity} (for {} ms)",
            for_how_long.as_millis()
        );
        let _ = writeln!(
            text,
            "fatal: {fatal} (UI/GPUI threads abort; tokio threads unwind)\n"
        );
        let _ = writeln!(text, "backtrace:\n{backtrace}");
        let dir = CRASH_DIR
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let file = dir.and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join(format!(
                "desktop-crash-{}-{}.log",
                stamp(),
                std::process::id()
            ));
            std::fs::write(&path, &text).ok()?;
            Some(path)
        });
        tracing::error!(
            target: "oino_crash",
            thread = name.as_deref().unwrap_or("<unnamed>"),
            %message,
            %location,
            file = %file.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            "panic"
        );
        previous(info);
        if fatal {
            std::process::abort();
        }
    }));
}

/// Main-thread stall detector.
#[derive(Clone, Debug)]
pub struct Watchdog {
    last_beat: std::sync::Arc<AtomicU64>,
    stop: std::sync::Arc<AtomicBool>,
}

/// One stall report (for tests and logs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stall {
    /// How long the UI thread has been silent.
    pub silent: Duration,
    /// Breadcrumb at detection time.
    pub activity: &'static str,
}

impl Watchdog {
    /// Starts the watchdog thread; call [`Watchdog::beat`] from the UI thread at least every
    /// [`BEAT`]. Stall reports go to `log_dir`.
    #[must_use]
    pub fn start(log_dir: PathBuf, stall: Duration) -> Self {
        let _ = EPOCH.get_or_init(Instant::now);
        let dog = Self {
            last_beat: std::sync::Arc::new(AtomicU64::new(now_ms())),
            stop: std::sync::Arc::new(AtomicBool::new(false)),
        };
        let watched = dog.clone();
        let spawned = std::thread::Builder::new()
            .name("oino-watchdog".into())
            .spawn(move || watched.run(&log_dir, stall));
        if let Err(error) = spawned {
            tracing::warn!(%error, "starting the UI watchdog");
        }
        dog
    }

    /// Records a UI heartbeat.
    pub fn beat(&self) {
        self.last_beat.store(now_ms(), Ordering::Relaxed);
    }

    /// Stops the watchdog thread (quit).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// How long since the last beat.
    #[must_use]
    pub fn silent_for(&self) -> Duration {
        Duration::from_millis(now_ms().saturating_sub(self.last_beat.load(Ordering::Relaxed)))
    }

    fn run(&self, log_dir: &Path, stall: Duration) {
        let poll = (stall / 4).max(Duration::from_millis(50));
        let mut reported: Option<Instant> = None;
        while !self.stop.load(Ordering::Relaxed) {
            std::thread::sleep(poll);
            let silent = self.silent_for();
            match (silent >= stall, reported) {
                (true, None) => {
                    reported = Some(Instant::now());
                    let (activity, _) = current_activity();
                    report_stall(log_dir, &Stall { silent, activity });
                }
                (false, Some(at)) => {
                    reported = None;
                    tracing::warn!(
                        target: "oino_watchdog",
                        recovered_after_ms = u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX)
                            + u64::try_from(stall.as_millis()).unwrap_or(0),
                        "UI thread responsive again"
                    );
                }
                _ => {}
            }
        }
    }
}

/// `state wchan syscall comm` of every thread of this process (Linux `/proc`).
#[must_use]
pub fn thread_states() -> String {
    let mut out = String::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return "(no /proc on this platform)\n".to_owned();
    };
    let mut tids: Vec<_> = tasks
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    tids.sort_by_key(|t| t.parse::<u64>().unwrap_or(0));
    for tid in tids {
        let base = format!("/proc/self/task/{tid}");
        let read = |f: &str| {
            std::fs::read_to_string(format!("{base}/{f}"))
                .map(|s| s.trim().to_owned())
                .unwrap_or_default()
        };
        let comm = read("comm");
        let stat = read("stat");
        // The state letter follows the parenthesised comm.
        let state = stat
            .rsplit_once(") ")
            .and_then(|(_, rest)| rest.split(' ').next())
            .unwrap_or("?")
            .to_owned();
        let wchan = read("wchan");
        let syscall = read("syscall");
        let syscall = syscall.split(' ').next().unwrap_or("");
        let _ = writeln!(
            out,
            "{tid:>8} {state} {comm:<20} wchan={wchan} syscall={syscall}"
        );
    }
    out
}

fn report_stall(log_dir: &Path, stall: &Stall) {
    let pid = std::process::id();
    let mut text = String::new();
    let _ = writeln!(
        text,
        "UI thread stalled for {} ms (pid {pid}); activity: {}\n",
        stall.silent.as_millis(),
        stall.activity
    );
    let _ = writeln!(text, "threads (tid state comm wchan syscall):");
    text.push_str(&thread_states());
    if std::env::var("OINO_WATCHDOG_GDB").is_ok_and(|v| v == "1") {
        text.push_str("\ngdb (main thread):\n");
        text.push_str(&gdb_backtrace(pid));
    }
    let file = std::fs::create_dir_all(log_dir).ok().and_then(|()| {
        let path = log_dir.join(format!("desktop-stall-{}-{pid}.log", stamp()));
        std::fs::write(&path, &text).ok().map(|()| path)
    });
    tracing::warn!(
        target: "oino_watchdog",
        silent_ms = u64::try_from(stall.silent.as_millis()).unwrap_or(u64::MAX),
        activity = stall.activity,
        file = %file.map(|p| p.display().to_string()).unwrap_or_default(),
        "UI thread stalled"
    );
}

/// `gdb -batch -p <pid> -ex 'thread apply 1 bt'` (needs ptrace permission: `ptrace_scope` 0
/// or `CAP_SYS_PTRACE`); bounded to 20 s.
fn gdb_backtrace(pid: u32) -> String {
    let child = std::process::Command::new("timeout")
        .args(["20", "gdb", "-batch", "-nx", "-p"])
        .arg(pid.to_string())
        .args(["-ex", "thread apply 1 bt 40"])
        .stdin(std::process::Stdio::null())
        .output();
    match child {
        Ok(out) => {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            s
        }
        Err(error) => format!("(gdb unavailable: {error})\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumbs_nest_and_restore() {
        let outer = activity("outer");
        assert_eq!(current_activity().0, "outer");
        {
            let _inner = activity("inner");
            assert_eq!(current_activity().0, "inner");
        }
        assert_eq!(current_activity().0, "outer");
        drop(outer);
    }

    #[test]
    fn fatal_threads() {
        assert!(fatal_thread(Some("main")));
        assert!(fatal_thread(Some("Worker-2")));
        assert!(!fatal_thread(Some("oino-core")));
        assert!(!fatal_thread(None));
    }

    #[test]
    fn stamps_are_civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_513), (2026, 3, 1));
        assert_eq!(stamp().len(), 15);
    }

    #[test]
    fn watchdog_reports_a_stall_and_recovers() {
        let dir = std::env::temp_dir().join(format!("oino-watchdog-{}", std::process::id()));
        let dog = Watchdog::start(dir.clone(), Duration::from_millis(200));
        std::thread::sleep(Duration::from_millis(600));
        assert!(dog.silent_for() >= Duration::from_millis(500));
        dog.beat();
        assert!(dog.silent_for() < Duration::from_millis(200));
        dog.stop();
        let logs: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("desktop-stall-")
            })
            .collect();
        assert!(!logs.is_empty(), "no stall log in {}", dir.display());
        let text = std::fs::read_to_string(logs[0].path()).unwrap();
        assert!(text.contains("UI thread stalled"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thread_states_lists_this_thread() {
        if cfg!(target_os = "linux") {
            assert!(thread_states().lines().count() >= 1);
        }
    }
}
