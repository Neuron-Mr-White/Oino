//! Panel settings, stored server-side at `<home>/settings.json` so every
//! browser that opens this daemon sees the same agent command and summary
//! instruction.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::{Layout, write_atomic};

/// The built-in prompt for "Summarize & archive" — kept in its own file so it
/// reads like the document it is.
pub const DEFAULT_SUMMARY_INSTRUCTION: &str = include_str!("summary-instruction.md");

/// The fixed style tail `summarize()` always appends after the (default or
/// custom) instruction — deliberately no URLs or skill references so the agent
/// never goes looking for anything.
pub const SUMMARY_STYLE: &str = "Style: plain words only. No inflated words like \"robust\", \"seamless\" or \"significant\", no em dashes, no bold, no filler or hedging. Do not invent anything: every name, number and claim must come from the tasks below; if a detail is missing, leave it out.";

/// Daemon-side settings (`<home>/settings.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PanelSettings {
    /// How to invoke pi for summaries: `[execPath, argv1?]` — argv1 is the
    /// script when pi runs under node, absent for a compiled binary. Written
    /// by the extension on session start; legacy `agentCommand` is ignored.
    #[serde(rename = "piCommand", default)]
    pub pi_command: Vec<String>,
    /// The user's available models as "provider/id" (whitelist for
    /// `summaryModel` — remote binds stay safe because only these pass).
    #[serde(rename = "models", default)]
    pub models: Vec<String>,
    /// Model for summarize runs; empty = pi's default.
    #[serde(rename = "summaryModel", default)]
    pub summary_model: String,
    /// Custom instruction for the summarize prompt; empty/whitespace = default.
    #[serde(rename = "summaryInstruction", default)]
    pub summary_instruction: String,
}

impl PanelSettings {
    /// The instruction actually used: the configured one, or the default.
    #[must_use]
    pub fn effective_instruction(&self) -> &str {
        if self.summary_instruction.trim().is_empty() {
            DEFAULT_SUMMARY_INSTRUCTION
        } else {
            &self.summary_instruction
        }
    }
}

/// `<home>/settings.json`.
#[must_use]
pub fn path(layout: &Layout) -> PathBuf {
    layout.home.join("settings.json")
}

/// Missing or unreadable settings fall back to defaults.
#[must_use]
pub fn load(layout: &Layout) -> PanelSettings {
    std::fs::read_to_string(path(layout))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes the daemon settings atomically.
pub fn save(layout: &Layout, settings: &PanelSettings) -> Result<()> {
    write_atomic(
        &path(layout),
        &format!("{}\n", serde_json::to_string_pretty(settings)?),
    )
}

// ── Oino-side kanboard settings (the `kanboard` extension namespace) ──────
//
// The board's Task defaults / session limits / Archive keys live in Oino's
// global settings file, `<agent dir>/settings.json` → `extensionSettings.kanboard`
// (agent dir = `$OINO_AGENT_DIR`, else `~/.oino`). Every other key of that
// file must survive a write.

/// Keys of the `kanboard` namespace the board panel edits.
pub const PI_SETTING_KEYS: &[&str] = &[
    "maxSessions",
    "turnAddLimit",
    "chainGate",
    "archiveAfterDays",
    "retentionDays",
];

/// Accepted `chainGate` values.
pub const CHAIN_GATES: &[&str] = &["in_review", "done"];

/// The program for a `piCommand` argv head. A bare `oino` prefers the `oino`
/// binary installed next to this one (the Oino installer ships both side by side),
/// so summaries run the same Oino even when it is not on `PATH`.
#[must_use]
pub fn resolve_program(head: &str) -> PathBuf {
    if head == "oino"
        && let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join(format!("oino{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from(head)
}

/// Settings namespace of the `unipi.kanboard` extension.
pub const NAMESPACE: &str = "kanboard";

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

/// `<agent dir>/settings.json` (Oino's global settings layer). `OINO_KANBOARD_HOME`
/// is the data dir, not the config home — it is NOT used here so the values land
/// where the extension actually reads them.
#[must_use]
pub fn pi_settings_path() -> Option<PathBuf> {
    agent_dir().map(|dir| dir.join("settings.json"))
}

fn load_settings_file_at(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Read the raw `extensionSettings.kanboard` object (absent/corrupt → empty object).
#[must_use]
pub fn load_pi_settings() -> serde_json::Map<String, serde_json::Value> {
    pi_settings_path()
        .map(|path| load_pi_settings_at(&path))
        .unwrap_or_default()
}

/// [`load_pi_settings`] for an explicit settings file.
#[must_use]
pub fn load_pi_settings_at(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    load_settings_file_at(path)
        .get("extensionSettings")
        .and_then(|v| v.get(NAMESPACE))
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Merge-write: only the keys the patch carries change; others (and every other
/// settings key) survive. tmp+rename via the crate's `write_atomic`.
pub fn patch_pi_settings(patch: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    let settings_file =
        pi_settings_path().ok_or_else(|| crate::error::Error::usage("HOME is not set"))?;
    patch_pi_settings_at(&settings_file, patch)
}

/// `map[key]` as an object, replacing a missing or non-object value.
fn object_entry<'a>(
    map: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    let slot = map
        .entry(key)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !slot.is_object() {
        *slot = serde_json::Value::Object(serde_json::Map::new());
    }
    match slot {
        serde_json::Value::Object(inner) => inner,
        _ => unreachable!("replaced by an object above"),
    }
}

/// [`patch_pi_settings`] for an explicit settings file.
pub fn patch_pi_settings_at(
    settings_file: &std::path::Path,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let mut file = load_settings_file_at(settings_file);
    let all = object_entry(&mut file, "extensionSettings");
    let ours = object_entry(all, NAMESPACE);
    for (key, value) in patch {
        ours.insert(key.clone(), value.clone());
    }
    if let Some(parent) = settings_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(
        settings_file,
        &format!("{}\n", serde_json::to_string_pretty(&file)?),
    )
}

/// Effective values = stored ⊕ defaults (what GET reports back).
#[must_use]
pub fn effective_pi_settings() -> serde_json::Map<String, serde_json::Value> {
    let stored = load_pi_settings();
    let mut out = serde_json::Map::new();
    out.insert(
        "maxSessions".to_string(),
        serde_json::Value::from(
            stored
                .get("maxSessions")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(2)
                .max(1),
        ),
    );
    out.insert(
        "turnAddLimit".to_string(),
        serde_json::Value::from(
            stored
                .get("turnAddLimit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(20),
        ),
    );
    out.insert(
        "chainGate".to_string(),
        stored
            .get("chainGate")
            .cloned()
            .filter(|v| v.as_str().is_some_and(|s| CHAIN_GATES.contains(&s)))
            .unwrap_or_else(|| "in_review".into()),
    );
    out.insert(
        "archiveAfterDays".to_string(),
        serde_json::Value::from(
            stored
                .get("archiveAfterDays")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        ),
    );
    out.insert(
        "retentionDays".to_string(),
        serde_json::Value::from(
            stored
                .get("retentionDays")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(90),
        ),
    );
    out
}

/// Validate a PUT patch into the keys that may be written (enum/number ranges).
pub fn validate_pi_patch(
    patch: &serde_json::Map<String, serde_json::Value>,
) -> std::result::Result<serde_json::Map<String, serde_json::Value>, String> {
    let mut out = serde_json::Map::new();
    for (key, value) in patch {
        match key.as_str() {
            "chainGate" => {
                let v = value.as_str().ok_or("chainGate must be a string")?;
                if !CHAIN_GATES.contains(&v) {
                    return Err(format!("unknown chainGate {v:?}"));
                }
                out.insert(key.clone(), value.clone());
            }
            "turnAddLimit" | "archiveAfterDays" | "retentionDays" => {
                if !value.is_u64() {
                    return Err(format!("{key} must be a non-negative integer"));
                }
                out.insert(key.clone(), value.clone());
            }
            "maxSessions" => {
                let v = value
                    .as_u64()
                    .ok_or("maxSessions must be a non-negative integer")?;
                if v < 1 {
                    return Err("maxSessions must be ≥ 1".to_string());
                }
                out.insert(key.clone(), value.clone());
            }
            // Legacy keys (older daemons; the removed runner's routing and
            // queue, and the removed board "blocking" task default) —
            // ignored, not rejected, so an older UI can still save.
            "agentCommand" | "defaultStrategy" | "defaultPlan" | "queueMax" | "blocking" => {}
            other => return Err(format!("unknown settings key {other:?}")),
        }
    }
    Ok(out)
}
