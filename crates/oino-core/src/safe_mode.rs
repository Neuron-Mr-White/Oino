//! Safe mode (X-SAFE, spec-core §2.3.3): after two consecutive failed starts, the next start
//! runs with extensions off and providers on (`-ne` semantics — the agent stays usable), and
//! a banner diagnostic names the recorded suspects.
//!
//! State lives in `<agent>/state/oino.safe-mode.json`:
//! `{"v":1,"consecutiveFailures":n,"lastFailures":[{"at":ms,"where":"services|session",
//! "extensionId?":"…","error":"…"}]}` (max [`ENTRY_LIMIT`] entries).
//!
//! Protocol (driven by `crates/oino/src/startup.rs`; the embedding host joins later):
//!
//! 1. [`SafeMode::load`] — a start that began but never finished (`openStart` still set: the
//!    process died before success or failure was recorded) counts as a failed start.
//! 2. [`SafeMode::begin_start`] before services are created, [`SafeMode::plan`] for the
//!    start's [`crate::services::ResourceFlags::safe_mode`].
//! 3. [`SafeMode::note_suspect`] for every extension named by a load/init failure during the
//!    failed start, then [`SafeMode::record_failure`] (services/session create errors and
//!    fatal startup diagnostics count; session-selection and model-config errors do not —
//!    those clear the marker via [`SafeMode::end_start`]).
//! 4. [`SafeMode::record_success`] once the runtime bound its first session — resets.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// State file name under `<agent>/state/`.
pub const STATE_FILE: &str = "oino.safe-mode.json";

/// Consecutive failed starts that trip safe mode.
pub const TRIGGER_FAILURES: u32 = 2;

/// Recorded failure entries kept (spec: max 8).
pub const ENTRY_LIMIT: usize = 8;

/// Where a start failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureStage {
    /// `Services::create` (or a fatal startup diagnostic it produced).
    Services,
    /// `AgentSessionRuntime::create` / the session never bound.
    Session,
}

/// One recorded failure (`lastFailures` entry; `extensionId` names a suspect).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureEntry {
    /// Epoch milliseconds.
    pub at: u64,
    /// Serialized as `where`.
    #[serde(rename = "where")]
    pub stage: FailureStage,
    /// The extension the failure names, when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
    /// The error text.
    pub error: String,
}

impl FailureEntry {
    /// A failure entry stamped now.
    #[must_use]
    pub fn new(stage: FailureStage, error: impl Into<String>) -> Self {
        Self {
            at: now_ms(),
            stage,
            extension_id: None,
            error: error.into(),
        }
    }
}

/// Persisted safe-mode state (`oino.safe-mode.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeModeState {
    /// Format version (1).
    #[serde(default)]
    pub v: u32,
    /// Failed starts in a row.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Newest last (max [`ENTRY_LIMIT`]).
    #[serde(default)]
    pub last_failures: Vec<FailureEntry>,
    /// A start that began but never finished: the process died before success or failure was
    /// recorded. Converted into a failure by the next [`SafeMode::load`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_start: Option<u64>,
}

/// What a start does about extensions (X-SAFE).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SafeModePlan {
    /// `consecutiveFailures >= 2`: run with extensions off, providers on.
    pub disable_extensions: bool,
    /// The recorded failure count (banner text).
    pub consecutive_failures: u32,
    /// Extension ids named by the recorded failures, with their errors (load order preserved).
    pub suspects: Vec<(String, String)>,
}

/// The loaded (or new) safe-mode state of one agent dir.
#[derive(Debug)]
pub struct SafeMode {
    path: PathBuf,
    state: SafeModeState,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The banner diagnostic for a tripped plan (X-SAFE): the TUI shows it as a startup warning.
#[must_use]
pub fn banner(plan: &SafeModePlan) -> String {
    let mut text = format!(
        "Safe mode: extensions are off after {} failed start{}",
        plan.consecutive_failures,
        if plan.consecutive_failures == 1 {
            ""
        } else {
            "s"
        }
    );
    if !plan.suspects.is_empty() {
        let suspects = plan
            .suspects
            .iter()
            .map(|(id, error)| format!("{id} ({error})"))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str("; suspects: ");
        text.push_str(&suspects);
    }
    text
}

impl SafeMode {
    /// Reads `<agent>/state/oino.safe-mode.json`; a missing or unreadable file is a fresh
    /// state. A start the previous process never finished (`openStart`) counts as failed.
    #[must_use]
    pub fn load(agent_dir: &Path) -> Self {
        let path = agent_dir.join("state").join(STATE_FILE);
        let mut state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<SafeModeState>(&text).ok())
            .unwrap_or_default();
        state.v = 1;
        let mut safe = Self { path, state };
        if let Some(at) = safe.state.open_start.take() {
            safe.state.last_failures.push(FailureEntry {
                at,
                stage: FailureStage::Session,
                extension_id: None,
                error: "the process exited before startup completed".to_owned(),
            });
            safe.state.consecutive_failures = safe.state.consecutive_failures.saturating_add(1);
            safe.trim();
            safe.save();
        }
        safe
    }

    /// This start's plan.
    #[must_use]
    pub fn plan(&self) -> SafeModePlan {
        SafeModePlan {
            disable_extensions: self.state.consecutive_failures >= TRIGGER_FAILURES,
            consecutive_failures: self.state.consecutive_failures,
            suspects: {
                let mut suspects = Vec::new();
                for entry in &self.state.last_failures {
                    if let Some(id) = &entry.extension_id
                        && !suspects
                            .iter()
                            .any(|(known, _): &(String, String)| known == id)
                    {
                        suspects.push((id.clone(), entry.error.clone()));
                    }
                }
                suspects
            },
        }
    }

    /// Marks the start as begun (a process death before the next write counts as a failure).
    pub fn begin_start(&mut self) {
        self.state.open_start = Some(now_ms());
        self.save();
    }

    /// Clears the begun marker without counting a failure (user-input startup errors:
    /// session selection, `--name` validation, model configuration).
    pub fn end_start(&mut self) {
        if self.state.open_start.take().is_some() {
            self.save();
        }
    }

    /// Records an extension the failed start named (does not count as a start failure; only
    /// called on failed-start paths, before [`Self::record_failure`]).
    pub fn note_suspect(&mut self, id: &str, error: impl Into<String>) {
        self.state.last_failures.push(FailureEntry {
            at: now_ms(),
            stage: FailureStage::Services,
            extension_id: Some(id.to_owned()),
            error: error.into(),
        });
        self.trim();
        self.save();
    }

    /// Records a failed start and clears the begun marker.
    pub fn record_failure(&mut self, entry: FailureEntry) {
        self.state.open_start = None;
        self.state.consecutive_failures = self.state.consecutive_failures.saturating_add(1);
        self.state.last_failures.push(entry);
        self.trim();
        self.save();
    }

    /// A start reached a bound session: reset (X-SAFE "success resets").
    pub fn record_success(&mut self) {
        self.state.open_start = None;
        self.state.consecutive_failures = 0;
        self.state.last_failures.clear();
        self.save();
    }

    fn trim(&mut self) {
        let excess = self.state.last_failures.len().saturating_sub(ENTRY_LIMIT);
        if excess > 0 {
            self.state.last_failures.drain(..excess);
        }
    }

    fn save(&self) {
        if let Ok(text) = serde_json::to_string(&self.state)
            && let Err(error) = oino_settings::storage::write_atomic(&self.path, text.as_bytes())
            && let Some(parent) = self.path.parent()
        {
            tracing::warn!(
                "writing {} failed: {error}",
                parent.join(STATE_FILE).display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(dir: &Path) -> SafeMode {
        SafeMode::load(&dir.join("agent"))
    }

    #[test]
    fn missing_state_is_fresh_and_below_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let plan = load(dir.path()).plan();
        assert!(!plan.disable_extensions);
        assert_eq!(plan.consecutive_failures, 0);
        assert_eq!(plan.suspects.len(), 0);
    }

    #[test]
    fn two_recorded_failures_trip_the_plan_and_success_resets() {
        let dir = tempfile::tempdir().unwrap();
        let mut safe = load(dir.path());
        safe.note_suspect("acme.notify", "init failed: boom");
        safe.record_failure(FailureEntry::new(FailureStage::Services, "Error: boom"));
        safe.record_failure(FailureEntry::new(FailureStage::Session, "Error: crash"));
        drop(safe);

        let mut safe = load(dir.path());
        let plan = safe.plan();
        assert!(plan.disable_extensions);
        assert_eq!(plan.consecutive_failures, 2);
        assert_eq!(
            plan.suspects,
            vec![("acme.notify".to_owned(), "init failed: boom".to_owned())]
        );
        let banner = banner(&plan);
        assert!(banner.starts_with("Safe mode: extensions are off after 2 failed starts"));
        assert!(
            banner.contains("suspects: acme.notify (init failed: boom)"),
            "{banner}"
        );

        safe.record_success();
        drop(safe);
        let plan = load(dir.path()).plan();
        assert!(!plan.disable_extensions);
        assert_eq!(plan.suspects.len(), 0);
    }

    #[test]
    fn unfinished_start_counts_as_a_failed_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut safe = load(dir.path());
        safe.begin_start();
        safe.record_failure(FailureEntry::new(FailureStage::Services, "Error: one"));
        drop(safe);

        // A second start begins, then the process dies before success or failure is recorded.
        let mut second = load(dir.path());
        assert_eq!(second.plan().consecutive_failures, 1);
        second.begin_start();
        drop(second);

        // The open marker converts into the second failure: safe mode trips.
        let plan = load(dir.path()).plan();
        assert!(plan.disable_extensions);
        assert_eq!(plan.consecutive_failures, 2);
    }

    #[test]
    fn user_input_errors_clear_the_marker_without_counting() {
        let dir = tempfile::tempdir().unwrap();
        let mut safe = load(dir.path());
        safe.begin_start();
        safe.end_start();
        drop(safe);
        assert_eq!(load(dir.path()).plan().consecutive_failures, 0);
    }

    #[test]
    fn failure_entries_are_capped_at_eight() {
        let dir = tempfile::tempdir().unwrap();
        let mut safe = load(dir.path());
        for i in 0..6 {
            safe.note_suspect(&format!("acme.ext{i}"), "boom");
            safe.record_failure(FailureEntry::new(
                FailureStage::Services,
                format!("Error: {i}"),
            ));
        }
        assert_eq!(safe.state.last_failures.len(), ENTRY_LIMIT);
        let reloaded = load(dir.path());
        assert_eq!(reloaded.state.last_failures.len(), ENTRY_LIMIT);
        // The newest entries survive: the oldest suspects were dropped.
        assert_eq!(
            reloaded.state.last_failures.last().unwrap().error,
            "Error: 5"
        );
    }

    #[test]
    fn state_file_shape_matches_the_spec() {
        let dir = tempfile::tempdir().unwrap();
        let mut safe = load(dir.path());
        safe.note_suspect("acme.notify", "init failed: boom");
        safe.record_failure(FailureEntry::new(FailureStage::Session, "Error: crash"));
        drop(safe);

        let text =
            std::fs::read_to_string(dir.path().join("agent/state").join(STATE_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["consecutiveFailures"], 1);
        let entries = value["lastFailures"].as_array().unwrap();
        assert!(entries[0]["extensionId"].is_string());
        assert_eq!(entries[0]["where"], "services");
        assert!(entries[0].get("at").is_some_and(serde_json::Value::is_u64));
        assert_eq!(entries[1]["where"], "session");
        assert!(entries[1].get("extensionId").is_none());
        assert!(!text.contains("openStart"));
    }
}
