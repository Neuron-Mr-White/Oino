//! `memory` settings (unipi `settings.ts`) and the pending-write journal (unipi `pending.ts`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unipi_kit::settings::{Field, Namespace, Section, Values, choice};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.memory";
/// Namespace.
pub const NAMESPACE: &str = "memory";

/// Save pass mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    /// Background side agent.
    Side,
    /// Next-turn reminder.
    Reminder,
    /// Nothing.
    Off,
}

/// Resolved switches.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "mirrors unipi MemoryConfig")]
pub struct Config {
    /// First-turn reminder.
    pub recall_at_start: bool,
    /// `memory_store` / `memory_delete` active.
    pub write: bool,
    /// Wake-up text in the reminder.
    pub wake_up: bool,
    /// Save pass.
    pub save_mode: SaveMode,
    /// Start the daemon when absent.
    pub auto_start_daemon: bool,
    /// Daily `PyPI` check + upgrade.
    pub auto_update: bool,
    /// Debug log.
    pub debug: bool,
}

/// Defaults (`recall` is not defaulted so legacy booleans can decide).
#[must_use]
pub fn defaults() -> Value {
    json!({"write": true, "saveMode": "side", "autoStartDaemon": false, "mempalaceAutoUpdate": true, "debug": false})
}

/// unipi `readMemoryConfig` over the merged namespace values.
#[must_use]
pub fn read(v: &Values) -> Config {
    let raw = &v.0;
    let recall = match raw.get("recall").and_then(Value::as_str) {
        Some(r) => r.to_owned(),
        None if raw.get("recallAtStart") == Some(&Value::Bool(false)) => "off".into(),
        None if raw.get("wakeUp") == Some(&Value::Bool(false)) => "reminder".into(),
        None => "wake-up".into(),
    };
    let (recall_at_start, wake_up) = match recall.as_str() {
        "off" => (false, false),
        "reminder" => (true, false),
        _ => (true, true),
    };
    Config {
        recall_at_start,
        write: v.bool("write", true),
        wake_up,
        save_mode: match v.str("saveMode", "side") {
            "reminder" => SaveMode::Reminder,
            "off" => SaveMode::Off,
            _ => SaveMode::Side,
        },
        auto_start_daemon: v.bool("autoStartDaemon", false),
        auto_update: v.bool("mempalaceAutoUpdate", true),
        debug: v.bool("debug", false),
    }
}

/// The hub schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Memory", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("mempalace", "MemPalace")
                .field(
                    Field::enumeration(
                        "recall",
                        "Recall at start",
                        vec![
                            choice("off", "off", "no first-turn reminder at all"),
                            choice("reminder", "reminder", "first-turn memory reminder only"),
                            choice("wake-up", "reminder + wake-up", "reminder plus the mempalace wake-up summary"),
                        ],
                    )
                    .description("What the first turn puts in front of the agent. Search/list tools stay available either way.")
                    .default_value("wake-up"),
                )
                .field(Field::boolean("write", "Write memory").description("Gives the agent the memory_store / memory_delete tools plus the end-of-task save nudge."))
                .field(
                    Field::enumeration(
                        "saveMode",
                        "Save mode",
                        vec![
                            choice("side", "side agent", "a background side session saves after the task"),
                            choice("reminder", "reminder", "an end-of-task note asks the agent to save"),
                            choice("off", "off", "no save pass at all"),
                        ],
                    )
                    .description("How the end-of-task save happens: a background side session, a next-turn reminder, or nothing."),
                )
                .field(Field::boolean("autoStartDaemon", "MemPalace daemon").description("Off: use a running daemon or write directly. On: Oino starts one; MemPalace servers in other tools turn read-only."))
                .field(Field::boolean("mempalaceAutoUpdate", "MemPalace auto-update").description("Keep the MemPalace install current with a daily PyPI check and uv upgrade."))
                .field(Field::boolean("debug", "Debug log").description("Log memory activity to logs/memory.log.").advanced()),
        )
}

/// `~/.config/mempalace/agent-hooks.json` `enabled: false` disables both reminders.
#[must_use]
pub fn agent_hooks_enabled(config_dir: Option<&Path>) -> bool {
    config_dir
        .and_then(|d| std::fs::read(d.join("agent-hooks.json")).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_none_or(|v| v.get("enabled") != Some(&Value::Bool(false)))
}

/// One journaled write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOp {
    /// `store | delete`.
    pub kind: String,
    /// Host path of the md file.
    pub file: String,
    /// Project.
    pub project: String,
    /// Id.
    pub id: String,
    /// ISO time.
    pub enqueued_at: String,
    /// Lock holder text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
}

/// The journal at `<root>/.pending.json`.
#[derive(Clone, Debug)]
pub struct Journal<'a>(pub &'a Path);

impl Journal<'_> {
    fn path(&self) -> std::path::PathBuf {
        self.0.join(".pending.json")
    }

    /// Ops.
    #[must_use]
    pub fn read(&self) -> Vec<PendingOp> {
        std::fs::read(self.path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("ops").cloned())
            .and_then(|o| serde_json::from_value(o).ok())
            .unwrap_or_default()
    }

    /// Replaces the ops.
    pub fn write(&self, ops: &[PendingOp]) {
        let _ = std::fs::create_dir_all(self.0);
        let _ = std::fs::write(
            self.path(),
            serde_json::to_vec_pretty(&json!({"version": 1, "ops": ops})).unwrap_or_default(),
        );
    }

    /// Adds (replacing the same kind+file).
    pub fn enqueue(&self, op: PendingOp) {
        let mut ops: Vec<PendingOp> = self
            .read()
            .into_iter()
            .filter(|o| !(o.kind == op.kind && o.file == op.file))
            .collect();
        ops.push(op);
        self.write(&ops);
    }

    /// Removes.
    pub fn drop_op(&self, op: &PendingOp) {
        let ops: Vec<PendingOp> = self
            .read()
            .into_iter()
            .filter(|o| !(o.kind == op.kind && o.file == op.file))
            .collect();
        self.write(&ops);
    }

    /// Count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.read().len()
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn config() {
        let c = read(&Values(defaults()));
        assert!(c.recall_at_start && c.wake_up && c.write && c.save_mode == SaveMode::Side);
        let c = read(&Values(json!({"recallAtStart": false})));
        assert!(!c.recall_at_start && !c.wake_up);
        let c = read(&Values(json!({"wakeUp": false})));
        assert!(c.recall_at_start && !c.wake_up);
        let c = read(&Values(json!({"recall": "reminder", "saveMode": "off"})));
        assert!(c.recall_at_start && !c.wake_up && c.save_mode == SaveMode::Off);
        assert!(agent_hooks_enabled(None));
        let d = crate::files::tmp("hooks");
        std::fs::write(d.join("agent-hooks.json"), "{\"enabled\": false}").unwrap_or_default();
        assert!(!agent_hooks_enabled(Some(&d)));
        assert_eq!(namespace().to_json()["namespace"], "memory");
    }

    #[test]
    fn journal() {
        let d = crate::files::tmp("journal");
        let j = Journal(&d);
        let op = |k: &str, f: &str| PendingOp {
            kind: k.into(),
            file: f.into(),
            project: "p".into(),
            id: "i".into(),
            enqueued_at: "t".into(),
            held_by: None,
        };
        j.enqueue(op("store", "/a"));
        j.enqueue(op("store", "/a"));
        j.enqueue(op("delete", "/a"));
        assert_eq!(j.count(), 2);
        j.drop_op(&op("store", "/a"));
        assert_eq!(j.read(), [op("delete", "/a")]);
    }
}
