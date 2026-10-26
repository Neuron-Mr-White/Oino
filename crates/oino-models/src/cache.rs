//! Per-provider model list cache: `<agent>/cache/models/<provider>.json`.
//!
//! Dynamic provider registrations (`models: "dynamic"`) start from the cached list until the
//! host has called the component's `list-models` (pi keeps refreshed catalogs in
//! `models-store.json` with the same `{models, checkedAt}` entry shape).

use std::path::{Path, PathBuf};

use oino_ai::Model;
use serde::{Deserialize, Serialize};

use crate::auth::oauth::now_ms;
use crate::auth::storage::write_atomic;

/// A cached model list (pi `ModelsStoreEntry` subset).
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
    }
}
