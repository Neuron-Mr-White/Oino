//! Per-provider model list cache: `<agent>/cache/models/<provider>.json`.
//!
//! Dynamic provider registrations (`models: "dynamic"`) start from the cached list until the
//! host has called the component's `list-models` (refreshed catalogs live in
//! `models-store.json` with the same `{models, checkedAt}` entry shape).

use std::path::{Path, PathBuf};

use oino_ai::Model;
use serde::{Deserialize, Serialize};

use crate::auth::oauth::now_ms;
use crate::auth::storage::write_atomic;

/// A cached model list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedModels {
    /// Models of the provider.
    pub models: Vec<Model>,
    /// When the list was fetched (Unix ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
}

/// Cache read/write failures.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The file could not be read or written.
    #[error("model cache {}: {source}", path.display())]
    Io {
        /// Cache file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not a valid cache entry.
    #[error("model cache {}: {source}", path.display())]
    Parse {
        /// Cache file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: serde_json::Error,
    },
}

/// Directory of per-provider cache files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCache {
    dir: PathBuf,
}

impl ModelCache {
    /// Cache in `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<agent_dir>/cache/models`.
    #[must_use]
    pub fn for_agent_dir(agent_dir: &Path) -> Self {
        Self::new(agent_dir.join("cache").join("models"))
    }

    /// Cache file of `provider` (unsafe characters replaced by `_`).
    #[must_use]
    pub fn path(&self, provider: &str) -> PathBuf {
        let mut name: String = provider
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if name.starts_with('.') || name.is_empty() {
            name.insert(0, '_');
        }
        self.dir.join(format!("{name}.json"))
    }

    /// Reads the cached list; `None` when there is none.
    ///
    /// # Errors
    ///
    /// Unreadable or invalid cache files.
    pub fn read(&self, provider: &str) -> Result<Option<CachedModels>, CacheError> {
        let path = self.path(provider);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(CacheError::Io { path, source }),
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| CacheError::Parse { path, source })
    }

    /// Writes `models` with `checkedAt = now`.
    ///
    /// # Errors
    ///
    /// Write failures.
    pub fn write(&self, provider: &str, models: &[Model]) -> Result<(), CacheError> {
        let path = self.path(provider);
        let entry = CachedModels {
            models: models.to_vec(),
            checked_at: Some(now_ms()),
        };
        let text = serde_json::to_string_pretty(&entry).map_err(|source| CacheError::Parse {
            path: path.clone(),
            source,
        })?;
        write_atomic(&path, &text).map_err(|source| CacheError::Io { path, source })
    }

    /// How old `provider`'s cached list is (`None`: no cache or no `checkedAt`).
    #[must_use]
    pub fn age(&self, provider: &str) -> Option<std::time::Duration> {
        let checked = self.read(provider).ok().flatten()?.checked_at?;
        let age = now_ms().saturating_sub(checked);
        Some(std::time::Duration::from_millis(
            u64::try_from(age).unwrap_or(0),
        ))
    }

    /// `<dir>/@catalogue.json`: the last available-model list an app host answered (v3-perf
    /// §4.5) — what the picker shows while a chat's runtime is not up. Provider file names
    /// are sanitized to `[A-Za-z0-9._-]`, so `@` never collides with one.
    #[must_use]
    pub fn catalogue_path(&self) -> PathBuf {
        self.dir.join("@catalogue.json")
    }

    /// Reads the catalogue snapshot (raw model rows); `None` when absent or unreadable.
    #[must_use]
    pub fn read_catalogue(&self) -> Option<Vec<serde_json::Value>> {
        let text = std::fs::read_to_string(self.catalogue_path()).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        match value.get("models")? {
            serde_json::Value::Array(rows) => Some(rows.clone()),
            _ => None,
        }
    }

    /// Writes the catalogue snapshot when it differs from the stored one.
    ///
    /// # Errors
    ///
    /// Write failures.
    pub fn write_catalogue(&self, models: &[serde_json::Value]) -> Result<(), CacheError> {
        if self.read_catalogue().as_deref() == Some(models) {
            return Ok(());
        }
        let path = self.catalogue_path();
        let text = serde_json::to_string(&serde_json::json!({
            "models": models,
            "checkedAt": now_ms(),
        }))
        .map_err(|source| CacheError::Parse {
            path: path.clone(),
            source,
        })?;
        write_atomic(&path, &text).map_err(|source| CacheError::Io { path, source })
    }

    /// Deletes the cached list (missing files are fine).
    ///
    /// # Errors
    ///
    /// Delete failures other than "not found".
    pub fn remove(&self, provider: &str) -> Result<(), CacheError> {
        let path = self.path(provider);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(CacheError::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_sanitized() {
        let cache = ModelCache::new("/c");
        assert_eq!(
            cache.path("openrouter"),
            PathBuf::from("/c/openrouter.json")
        );
        assert_eq!(cache.path("../x"), PathBuf::from("/c/_.._x.json"));
        assert_ne!(cache.catalogue_path(), cache.path("_catalogue"));
    }

    #[test]
    fn catalogue_round_trips_and_ages() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(dir.path());
        assert!(cache.read_catalogue().is_none());
        let rows = vec![serde_json::json!({"provider": "p", "id": "m"})];
        cache.write_catalogue(&rows).unwrap();
        assert_eq!(cache.read_catalogue(), Some(rows));
        assert!(cache.age("p").is_none());
        cache.write("p", &[]).unwrap();
        assert!(cache.age("p").unwrap() < std::time::Duration::from_secs(5));
    }
}
