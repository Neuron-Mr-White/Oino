//! Result cache (unipi `src/cache.ts`): one JSON file per entry under `/cache/web/`,
//! `sha256("<provider>:<key>")` names, 1 h TTL, expired entries pruned at start and end.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// unipi default TTL.
pub(crate) const DEFAULT_TTL_MS: i64 = 3_600_000;

/// unipi `CacheEntry`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    /// Hash key.
    pub key: String,
    /// Payload.
    pub data: Value,
    /// Epoch ms.
    pub timestamp: i64,
    /// TTL.
    pub ttl_ms: i64,
    /// Producer.
    pub provider: String,
    /// Cache key text (unipi `url` field).
    pub url: String,
}

/// unipi `CacheStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Entries.
    pub total_entries: usize,
    /// Bytes.
    pub total_size_bytes: u64,
    /// Expired.
    pub expired_entries: usize,
}

/// The cache over a directory.
#[derive(Clone, Debug)]
pub(crate) struct Cache {
    dir: PathBuf,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(unipi_kit::paths::cache("web"))
    }
}

fn hash(provider: &str, url: &str) -> String {
    let digest = Sha256::digest(format!("{provider}:{url}").as_bytes());
    digest.iter().fold(String::with_capacity(64), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}

impl Cache {
    /// A cache rooted at `dir`.
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    fn files(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// unipi `get` (expired entries are deleted).
    pub(crate) fn get(&self, url: &str, provider: &str, now: i64) -> Option<Value> {
        let key = hash(provider, url);
        let path = self.file(&key);
        let entry: Entry = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
        if now - entry.timestamp > entry.ttl_ms {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        Some(entry.data)
    }

    /// unipi `set`.
    pub(crate) fn set(&self, url: &str, provider: &str, data: &Value, now: i64) {
        let key = hash(provider, url);
        let entry = Entry {
            key: key.clone(),
            data: data.clone(),
            timestamp: now,
            ttl_ms: DEFAULT_TTL_MS,
            provider: provider.to_owned(),
            url: url.to_owned(),
        };
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(text) = serde_json::to_vec_pretty(&entry) {
            let _ = std::fs::write(self.file(&key), text);
        }
    }

    /// unipi `clear` → entries removed.
    pub(crate) fn clear(&self) -> usize {
        self.files()
            .iter()
            .filter(|p| std::fs::remove_file(p).is_ok())
            .count()
    }

    /// unipi `clearExpired` (unreadable entries are deleted too).
    pub(crate) fn clear_expired(&self, now: i64) -> usize {
        let mut n = 0;
        for p in self.files() {
            let expired = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Entry>(&b).ok())
                .is_none_or(|e| now - e.timestamp > e.ttl_ms);
            if expired && std::fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// unipi `getStats`.
    pub(crate) fn stats(&self, now: i64) -> Stats {
        let mut s = Stats::default();
        for p in self.files() {
            s.total_entries += 1;
            s.total_size_bytes += std::fs::metadata(&p).map_or(0, |m| m.len());
            if read_entry(&p).is_some_and(|e| now - e.timestamp > e.ttl_ms) {
                s.expired_entries += 1;
            }
        }
        s
    }
}

fn read_entry(p: &Path) -> Option<Entry> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

/// unipi `generateSmartFetchKey` (`url:browser:format:maxChars`, raw options).
pub(crate) fn smart_fetch_key(
    url: &str,
    browser: Option<&str>,
    format: Option<&str>,
    max_chars: Option<u64>,
) -> String {
    format!(
        "{url}:{}:{}:{}",
        browser.unwrap_or(""),
        format.unwrap_or(""),
        max_chars.map(|m| m.to_string()).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn get_set_expire_clear() {
        let dir = std::env::temp_dir().join(format!("unipi-web-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let c = Cache::new(dir.clone());
        assert_eq!(c.get("u", "smart-fetch", 0), None);
        c.set("u", "smart-fetch", &json!({"content": "x"}), 1000);
        assert_eq!(
            c.get("u", "smart-fetch", 2000),
            Some(json!({"content": "x"}))
        );
        assert_eq!(
            c.get("u", "other", 2000),
            None,
            "provider is part of the key"
        );
        c.set("old", "p", &json!(1), 0);
        let _ = std::fs::write(dir.join("junk.json"), b"not json");
        let s = c.stats(DEFAULT_TTL_MS + 10);
        assert_eq!(s.total_entries, 3);
        assert_eq!(s.expired_entries, 1);
        assert!(s.total_size_bytes > 0);
        assert_eq!(
            c.clear_expired(DEFAULT_TTL_MS + 10),
            2,
            "expired + unreadable"
        );
        assert_eq!(c.get("old", "p", DEFAULT_TTL_MS + 10), None);
        assert_eq!(c.clear(), 1);
        assert_eq!(c.stats(0).total_entries, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys() {
        assert_eq!(
            smart_fetch_key("u", Some("chrome_145"), None, Some(100)),
            "u:chrome_145::100"
        );
        assert_eq!(smart_fetch_key("u", None, None, None), "u:::");
        assert_eq!(hash("p", "u").len(), 64);
    }
}
