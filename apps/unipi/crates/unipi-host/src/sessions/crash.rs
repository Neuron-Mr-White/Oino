//! Crash isolation and the host's own safe mode (spec-host §4.4).
//!
//! Two things live here:
//!
//! 1. **Start history / safe mode** — the host records every start outcome in
//!    `<data dir>/start-history.json` (`{ts, outcome, uptime_s}`). A start that exits within
//!    [`FAST_FAIL_SECS`] (or dies before it ever recorded a success, the `openStart` marker)
//!    counts as a fast fail; after [`TRIGGER_FAILURES`] consecutive ones the *next* start
//!    runs with extensions off and emits a [`crate::sessions::hub::HubEvent::HostNotice`]
//!    naming the suspects found in `host.log`. A successful long-running start resets the
//!    counter. The plan is handed to core as [`ResourceFlags::safe_mode`] so the providers
//!    stay on (`-ne` semantics, spec-core §2.3.3) — this is the host-level counterpart of
//!    core's own start-failure safe mode (`oino_core::safe_mode`).
//! 2. **Panic containment helpers** — the message of a caught panic, for the per-session
//!    supervisor in [`crate::sessions::hub`].
//!
//! [`ResourceFlags::safe_mode`]: oino_core::services::ResourceFlags::safe_mode

use std::any::Any;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use oino_core::safe_mode::SafeModePlan;

/// State file name, directly in the data dir (`~/.oino/host/start-history.json`, §4.4).
pub const START_HISTORY_FILE: &str = "start-history.json";

/// `host.log`, read for suspect extension ids (§4.4; written on Windows by `logging`).
pub const LOG_FILE: &str = "host.log";

/// A start that exits within this many seconds is a failed start (§4.4).
pub const FAST_FAIL_SECS: u64 = 60;

/// Consecutive failed starts that trip safe mode (§4.4, DIRECTION §3.3.3).
pub const TRIGGER_FAILURES: u32 = 2;

/// Start records kept (the file stays small; only the trailing run matters).
pub const HISTORY_LIMIT: usize = 16;

/// How many suspects a banner names.
pub const SUSPECT_LIMIT: usize = 3;

/// How one start ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartOutcome {
    /// The host ran for at least [`FAST_FAIL_SECS`].
    Success,
    /// The process exited (or died) before that.
    FastFail,
    /// A clean, intentional exit (SIGTERM / Ctrl+C / a binary-replaced restart):
    /// not a failure — the app restarting quickly must not trip safe mode (E-10).
    Stopped,
}

/// One start record (`{ts, outcome, uptime_s}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRecord {
    /// Start time, epoch milliseconds.
    pub ts: u64,
    /// How the start ended.
    pub outcome: StartOutcome,
    /// Seconds the process stayed up.
    pub uptime_s: u64,
}

/// Persisted start history (`start-history.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartHistory {
    /// Format version (1).
    #[serde(default)]
    pub v: u32,
    /// Oldest first (max [`HISTORY_LIMIT`]).
    #[serde(default)]
    pub starts: Vec<StartRecord>,
    /// A start that has not recorded an outcome yet: the process died before
    /// [`FAST_FAIL_SECS`] (converted into a [`StartOutcome::FastFail`] by the next load).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_start: Option<u64>,
}

impl StartHistory {
    /// Consecutive [`StartOutcome::FastFail`] records at the end of the history.
    #[must_use]
    pub fn consecutive_failures(&self) -> u32 {
        self.starts
            .iter()
            .rev()
            .take_while(|record| record.outcome == StartOutcome::FastFail)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The host's safe-mode state for one data dir.
#[derive(Debug)]
pub struct HostSafeMode {
    path: PathBuf,
    history: StartHistory,
}

impl HostSafeMode {
    /// Reads `<data dir>/start-history.json`; a missing or unreadable file is a fresh
    /// history. A start the previous process never finished counts as a fast fail.
    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(START_HISTORY_FILE);
        let mut history = read_history(&path);
        history.v = 1;
        let mut safe = Self { path, history };
        if let Some(at) = safe.history.open_start.take() {
            safe.push(StartRecord {
                ts: at,
                outcome: StartOutcome::FastFail,
                uptime_s: 0,
            });
            safe.save();
        }
        safe
    }

    /// The state file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consecutive failed starts recorded so far.
    #[must_use]
    pub fn consecutive_failures(&self) -> u32 {
        self.history.consecutive_failures()
    }

    /// Whether this start must run with extensions off (§4.4).
    #[must_use]
    pub fn trips(&self) -> bool {
        self.consecutive_failures() >= TRIGGER_FAILURES
    }

    /// This start's plan, for [`oino_core::services::ResourceFlags::safe_mode`] and the
    /// startup banner. `suspects` come from `host.log` (see [`suspects_in_log`]).
    #[must_use]
    pub fn plan(&self, suspects: Vec<(String, String)>) -> SafeModePlan {
        let mut suspects = suspects;
        suspects.truncate(SUSPECT_LIMIT);
        SafeModePlan {
            disable_extensions: self.trips(),
            consecutive_failures: self.consecutive_failures(),
            suspects,
        }
    }

    /// Marks this start as begun: a process death before [`Self::record_success`] counts as
    /// a failed start.
    pub fn begin_start(&mut self) {
        self.history.open_start = Some(now_ms());
        self.save();
    }

    /// A start stayed up long enough to count (§4.4 "a successful long-running start clears
    /// the counter" and every other consecutive failure run with it).
    pub fn record_success(&mut self, uptime_s: u64) {
        self.history.open_start = None;
        self.history.starts.clear();
        self.push(StartRecord {
            ts: now_ms(),
            outcome: StartOutcome::Success,
            uptime_s,
        });
        self.save();
    }

    /// This start ended cleanly (SIGTERM/Ctrl+C, a self-update restart, the app closing):
    /// release the open marker without converting it into a fast fail (E-10 — the app
    /// killing its own host during a quick restart is not a crash) and break any
    /// consecutive-failure streak the way any non-failure end does.
    pub fn record_stopped(&mut self) {
        self.history.open_start = None;
        self.push(StartRecord {
            ts: now_ms(),
            outcome: StartOutcome::Stopped,
            uptime_s: 0,
        });
        self.save();
    }

    fn push(&mut self, record: StartRecord) {
        self.history.starts.push(record);
        let excess = self.history.starts.len().saturating_sub(HISTORY_LIMIT);
        if excess > 0 {
            self.history.starts.drain(..excess);
        }
    }

    fn save(&self) {
        let Ok(text) = serde_json::to_string_pretty(&self.history) else {
            return;
        };
        if let Err(error) = oino_settings::storage::write_atomic(&self.path, text.as_bytes()) {
            tracing::warn!(
                path = %self.path.display(),
                %error,
                "recording the host start history failed"
            );
        }
    }
}

/// [`HostSafeMode::load`] without interpreting the open marker (used by the success task,
/// which owns the marker itself).
fn read_history(path: &Path) -> StartHistory {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<StartHistory>(&text).ok())
        .unwrap_or_default()
}

/// The safe-mode banner (§4.4): what the app shows as a `HostNotice` at startup.
#[must_use]
pub fn banner(plan: &SafeModePlan) -> String {
    let mut text = String::from("Safe mode: extensions are off after ");
    text.push_str(&plan.consecutive_failures.to_string());
    text.push_str(" failed host start");
    if plan.consecutive_failures != 1 {
        text.push('s');
    }
    if !plan.suspects.is_empty() {
        text.push_str("; suspects: ");
        let named = plan
            .suspects
            .iter()
            .map(|(id, error)| format!("{id} ({error})"))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&named);
    }
    text.push_str(". Fix or remove the suspect extension, then restart the host.");
    text
}

/// Extension suspects named in a `host.log` (newest last, deduplicated, at most
/// [`SUSPECT_LIMIT`]).
///
/// Matches the loader's diagnostics (`` Failed to load extension "<id>": <error> ``) and
/// init/registration failures logged as `` … <id>: <error> ``. Ids must look like extension
/// ids — dot-separated lowercase segments (`unipi.cron`, `oino.anthropic`, …) — so ordinary
/// prose in the log never turns into a suspect.
#[must_use]
pub fn suspects_in_log(text: &str) -> Vec<(String, String)> {
    fn valid_id(id: &str) -> bool {
        !id.is_empty()
            && id.contains('.')
            && id.split('.').all(|segment| {
                let mut chars = segment.chars();
                chars
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                    && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            })
    }
    /// The id immediately before the `": "`-separated message of a tracing line.
    fn id_before_message(line: &str) -> Option<(String, String)> {
        for (index, _) in line.match_indices(": ") {
            let id = line[..index]
                .split_whitespace()
                .next_back()?
                .trim_matches('"');
            let error = line[index + 2..].trim();
            if valid_id(id) && !error.is_empty() {
                return Some((id.to_owned(), error.to_owned()));
            }
        }
        None
    }
    let mut suspects: Vec<(String, String)> = Vec::new();
    for line in text.lines().rev() {
        let Some((id, error)) = id_before_message(line) else {
            continue;
        };
        if !suspects.iter().any(|(known, _)| *known == id) {
            suspects.push((id, error));
        }
    }
    suspects.reverse();
    suspects.truncate(SUSPECT_LIMIT);
    suspects
}

/// Reads `<data dir>/host.log` and returns its suspects (empty when the file is absent —
/// on Linux/macOS the host logs to stderr, so there is nothing to read).
#[must_use]
pub fn suspects_from_log(data_dir: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(data_dir.join(LOG_FILE))
        .map(|text| suspects_in_log(&text))
        .unwrap_or_default()
}

/// The message of a panic caught by a session supervisor.
#[must_use]
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "session task panicked".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_text(outcomes: &[StartOutcome]) -> String {
        let starts: Vec<StartRecord> = outcomes
            .iter()
            .enumerate()
            .map(|(i, outcome)| StartRecord {
                ts: u64::try_from(i).unwrap(),
                outcome: *outcome,
                uptime_s: 5,
            })
            .collect();
        serde_json::to_string(&StartHistory {
            v: 1,
            starts,
            open_start: None,
        })
        .unwrap()
    }

    #[test]
    fn missing_history_is_below_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let safe = HostSafeMode::load(dir.path());
        assert_eq!(safe.consecutive_failures(), 0);
        assert!(!safe.trips());
        let plan = safe.plan(Vec::new());
        assert!(!plan.disable_extensions);
        assert!(plan.suspects.is_empty());
    }

    #[test]
    fn two_fast_fails_trip_safe_mode_and_success_resets() {
        use StartOutcome::{FastFail, Success};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(START_HISTORY_FILE),
            history_text(&[FastFail]),
        )
        .unwrap();
        let mut safe = HostSafeMode::load(dir.path());
        assert_eq!(safe.consecutive_failures(), 1);
        assert!(!safe.trips(), "one failed start stays below the threshold");

        // The previous process dies before recording a success: the open marker counts.
        safe.begin_start();
        let next = HostSafeMode::load(dir.path());
        assert_eq!(next.consecutive_failures(), 2);
        assert!(next.trips());
        let plan = next.plan(vec![("unipi.notify".into(), "init failed".into())]);
        assert!(plan.disable_extensions);
        assert!(
            banner(&plan).contains("unipi.notify (init failed)"),
            "{plan:?}"
        );

        // A long-running start clears the counter.
        let mut recovered = HostSafeMode::load(dir.path());
        recovered.record_success(FAST_FAIL_SECS);
        assert_eq!(recovered.consecutive_failures(), 0);
        assert!(!recovered.trips());
        assert_eq!(recovered.history.starts.len(), 1);
        assert_eq!(recovered.history.starts[0].outcome, StartOutcome::Success);
        assert!(read_history(&recovered.path).open_start.is_none());

        // A success in the middle of failures breaks the run (not the trailing one).
        std::fs::write(
            dir.path().join(START_HISTORY_FILE),
            history_text(&[FastFail, Success, FastFail]),
        )
        .unwrap();
        assert_eq!(HostSafeMode::load(dir.path()).consecutive_failures(), 1);
    }

    /// E-10: two quick app restarts kill the host cleanly (SIGTERM before the success
    /// marker) — a clean stop is not a failed start and must never trip safe mode.
    #[test]
    fn clean_quick_restarts_do_not_trip_safe_mode() {
        use StartOutcome::{FastFail, Stopped};
        let dir = tempfile::tempdir().unwrap();

        // Restart 1: the host begins, the app closes it before 60 s.
        let mut first = HostSafeMode::load(dir.path());
        first.begin_start();
        first.record_stopped();
        // Restart 2: same story.
        let mut second = HostSafeMode::load(dir.path());
        second.begin_start();
        second.record_stopped();

        let third = HostSafeMode::load(dir.path());
        assert_eq!(third.consecutive_failures(), 0);
        assert!(!third.trips(), "clean quick restarts are not crashes");
        assert!(!third.plan(Vec::new()).disable_extensions);
        assert!(read_history(&third.path).open_start.is_none());
        let outcomes: Vec<StartOutcome> = third
            .history
            .starts
            .iter()
            .map(|record| record.outcome)
            .collect();
        assert_eq!(outcomes, vec![Stopped, Stopped]);

        // A clean stop also breaks a failure streak that preceded it ...
        std::fs::write(
            dir.path().join(START_HISTORY_FILE),
            history_text(&[FastFail, FastFail]),
        )
        .unwrap();
        let mut after_break = HostSafeMode::load(dir.path());
        assert!(after_break.trips());
        after_break.begin_start();
        after_break.record_stopped();
        assert!(
            !HostSafeMode::load(dir.path()).trips(),
            "a clean stop ends the consecutive run"
        );

        // ... and real crashes after clean restarts still count.
        let mut crashed = HostSafeMode::load(dir.path());
        crashed.begin_start();
        let mut crashed_again = HostSafeMode::load(dir.path());
        crashed_again.begin_start();
        assert!(HostSafeMode::load(dir.path()).trips());
    }

    #[test]
    fn suspects_are_read_from_log_lines_newest_last() {
        let log = "\
2026-10-31T10:00:00Z  WARN oino_core::extensions: Failed to load extension \"unipi.notify\": init failed: boom
2026-10-31T10:00:01Z  WARN unipi_host: unipi.cron: register failed: nope
2026-10-31T10:00:02Z  WARN oino_core::extensions: Failed to load extension \"unipi.notify\": init failed: boom
not a suspect: just a plain message
";
        assert_eq!(
            suspects_in_log(log),
            vec![
                ("unipi.cron".to_owned(), "register failed: nope".to_owned()),
                ("unipi.notify".to_owned(), "init failed: boom".to_owned()),
            ]
        );
        assert!(suspects_in_log("").is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(suspects_from_log(dir.path()).is_empty(), "no host.log");
        std::fs::write(dir.path().join(LOG_FILE), log).unwrap();
        assert_eq!(suspects_from_log(dir.path()).len(), 2);
    }

    #[test]
    fn panic_messages_survive_both_payload_shapes() {
        let owned = std::panic::catch_unwind(|| panic!("tool body exploded"));
        assert!(panic_message(&*owned.unwrap_err()).contains("tool body exploded"));
        let borrowed: Box<dyn Any + Send> = Box::new("plain");
        assert_eq!(panic_message(&*borrowed), "plain");
    }
}
