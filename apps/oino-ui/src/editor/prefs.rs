//! Files-page preferences, persisted in `editor.json` next to `desktop.json`
//! (own store file, same ownership precedent as `hosts.json` — `host.rs` and
//! `AppSettings` stay untouched).

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Editor settings (round5 #24).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EditorPrefs {
    /// Attach `rust-analyzer` to Rust files (diagnostics, hover, completions,
    /// format-on-save). Off = plain editor, no server is ever spawned.
    pub lsp: bool,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self { lsp: true }
    }
}

impl EditorPrefs {
    /// Reads `agent_dir/editor.json`; a missing or malformed file is the default.
    #[must_use]
    pub fn load(agent_dir: &Path) -> Self {
        std::fs::read_to_string(agent_dir.join("editor.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the store (best effort; the editor works without persistence).
    pub fn save(&self, agent_dir: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(agent_dir.join("editor.json"), json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_defaults_to_lsp_on() {
        let dir = std::env::temp_dir().join(format!("oino-prefs-{}", std::process::id()));
        let prefs = EditorPrefs::load(&dir);
        assert!(prefs.lsp);
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("oino-prefs-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let prefs = EditorPrefs { lsp: false };
        prefs.save(&dir);
        assert!(!EditorPrefs::load(&dir).lsp);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
