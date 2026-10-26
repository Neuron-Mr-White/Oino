//! Hint store: `/ext-global/hints.json` (unipi `~/.unipi/global/hints/hints.json`), same shape:
//! `{"counts": {"<id>": {"count", "last"}}, "learned": [...], "lastSeenVersion": "…"}`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::catalog::{Counts, Record};

/// File name under `/ext-global`.
pub const FILE: &str = "hints.json";

/// Persisted hint history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreData {
    /// Show counts.
    #[serde(default)]
    pub counts: Counts,
    /// Learned hint ids.
    #[serde(default)]
    pub learned: Vec<String>,
    /// Last seen unipi version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_version: Option<String>,
}

impl StoreData {
    /// Lenient parse (unipi `loadHintStore`: malformed parts reset to empty).
    #[must_use]
    pub fn parse(v: &Value) -> Self {
        Self {
            counts: v
                .get("counts")
                .and_then(|c| serde_json::from_value(c.clone()).ok())
                .unwrap_or_default(),
            learned: v
                .get("learned")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            last_seen_version: v
                .get("lastSeenVersion")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }
    }

    /// unipi `recordHintShow`.
    pub fn record_show(&mut self, id: &str, now_ms: i64) {
        let prev = self.counts.get(id).copied().unwrap_or_default();
        self.counts.insert(
            id.to_owned(),
            Record {
                count: prev.count + 1,
                last: now_ms,
            },
        );
    }

    /// unipi `recordHintLearned`; `true` when newly learned.
    pub fn record_learned(&mut self, id: &str) -> bool {
        if self.learned.iter().any(|l| l == id) {
            return false;
        }
        self.learned.push(id.to_owned());
        true
    }

    /// unipi `resetHintHistory` (keeps `lastSeenVersion`).
    pub fn reset(&mut self) {
        self.counts.clear();
        self.learned.clear();
    }
}

/// Store file path.
#[must_use]
pub fn path() -> PathBuf {
    unipi_kit::paths::global(FILE)
}

/// Loads the store at `path` (empty when missing or malformed).
#[must_use]
pub fn load(path: &Path) -> StoreData {
    unipi_kit::paths::read_json::<Value>(path)
        .map(|v| StoreData::parse(&v))
        .unwrap_or_default()
}

/// Saves the store (best effort, like unipi).
pub fn save(path: &Path, data: &StoreData) {
    let _ = unipi_kit::paths::write_json(path, data);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_leniently_and_round_trips() {
        let d = StoreData::parse(
            &json!({"counts": {"a": {"count": 2, "last": 5}}, "learned": ["x", 3], "lastSeenVersion": "3.0.0"}),
        );
        assert_eq!(d.counts["a"], Record { count: 2, last: 5 });
        assert_eq!(d.learned, ["x"]);
        assert_eq!(d.last_seen_version.as_deref(), Some("3.0.0"));
        assert_eq!(
            StoreData::parse(&json!({"counts": "bad"})),
            StoreData::default()
        );
        let v = serde_json::to_value(&d).unwrap_or_default();
        assert_eq!(v["counts"]["a"], json!({"count": 2, "last": 5}));
        assert_eq!(v["lastSeenVersion"], "3.0.0");
    }

    #[test]
    fn records_and_resets() {
        let mut d = StoreData::default();
        d.record_show("a", 10);
        d.record_show("a", 20);
        assert_eq!(d.counts["a"], Record { count: 2, last: 20 });
        assert!(d.record_learned("a"));
        assert!(!d.record_learned("a"));
        d.last_seen_version = Some("v".into());
        d.reset();
        assert!(d.counts.is_empty() && d.learned.is_empty());
        assert_eq!(d.last_seen_version.as_deref(), Some("v"));
    }

    #[test]
    fn file_io() {
        let dir = std::env::temp_dir().join(format!("unipi-core-hints-{}", std::process::id()));
        let p = dir.join(FILE);
        assert_eq!(load(&p), StoreData::default());
        assert!(std::fs::create_dir_all(&dir).is_ok());
        assert!(std::fs::write(&p, br#"{"learned": ["k"]}"#).is_ok());
        assert_eq!(load(&p).learned, ["k"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
