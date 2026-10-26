//! Stash register with JSON file persistence (unipi `src/registers.ts`).
//!
//! unipi kept `.unipi/config/input-shortcuts.json` in the project; Oino keeps the same shape
//! `{"stash": "…"}` in workspace state (`/ext-workspace/<wsid>/input-shortcuts.json`, D10: no
//! implicit writes into projects). Old numbered registers are ignored on load and dropped on
//! save; writes are atomic (temp + rename) and best-effort.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name inside the workspace state directory.
pub const FILE: &str = "input-shortcuts.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    stash: String,
}

/// The stash (lazy-loaded).
#[derive(Debug, Default)]
pub struct Stash {
    path: Option<PathBuf>,
    value: Option<String>,
}

/// Parses a stash file; corrupt or partial files yield `""`.
#[must_use]
pub fn parse(bytes: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.get("stash")?.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

impl Stash {
    /// A stash persisted at `path`.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            value: None,
        }
    }

    /// Points the stash at another file (new workspace) and forgets the loaded value.
    pub fn set_path(&mut self, path: PathBuf) {
        if self.path.as_ref() != Some(&path) {
            *self = Self::at(path);
        }
    }

    fn load(&mut self) -> &mut String {
        let path = self.path.clone();
        self.value.get_or_insert_with(|| {
            path.and_then(|p| std::fs::read(p).ok())
                .map(|b| parse(&b))
                .unwrap_or_default()
        })
    }

    /// Stash contents.
    pub fn get(&mut self) -> String {
        self.load().clone()
    }

    /// Replaces and persists the stash.
    pub fn set(&mut self, text: &str) {
        text.clone_into(self.load());
        if let Some(path) = &self.path {
            save(path, text);
        }
    }
}

fn save(path: &Path, text: &str) {
    let Ok(json) = serde_json::to_vec_pretty(&Data {
        stash: text.to_owned(),
    }) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("unipi-is-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn creates_file_on_first_write_and_roundtrips() {
        let d = dir("roundtrip");
        let path = d.join(FILE);
        let mut s = Stash::at(path.clone());
        assert_eq!(s.get(), "");
        s.set("hello world");
        assert!(path.exists());
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v, serde_json::json!({"stash": "hello world"}));
        assert_eq!(Stash::at(path).get(), "hello world");
        assert!(!d.join("input-shortcuts.json.tmp").exists());
    }

    #[test]
    fn old_registers_corrupt_and_partial_files() {
        assert_eq!(parse(br#"{"stash":"s","0":"a","1":"b"}"#), "s");
        assert_eq!(parse(b"{not json"), "");
        assert_eq!(parse(b"{}"), "");
        assert_eq!(parse(br#"{"stash": 3}"#), "");
        let d = dir("legacy");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE), br#"{"stash":"kept","5":"x"}"#).unwrap();
        let mut s = Stash::at(d.join(FILE));
        s.set("new");
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.join(FILE)).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"stash": "new"}),
            "numbered registers dropped on save"
        );
    }
}
