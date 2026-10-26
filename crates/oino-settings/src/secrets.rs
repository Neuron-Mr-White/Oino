//! `<agent>/secrets.json` (extension-api §5.9): a flat `{key: value}` string map, mode 0600,
//! guarded by the same `<file>.lock` advisory lock as settings files and replaced atomically.
//!
//! Keys are namespaced by their users: extensions get `<ext-id>/<name>`; secret fields of an
//! extension settings namespace live at `<owner-ext-id>/<namespace>/<key>` (runtime §10.3), so
//! a guest that stored `secrets.set("<namespace>/<key>")` itself and the host settings registry
//! address the same entry.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::storage::{FileLock, read_optional, write_atomic};

/// File name of the secrets store inside the agent dir.
pub const SECRETS_FILE: &str = "secrets.json";

/// The secrets file. Cheap to clone; every operation re-reads the file under the lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretStore {
    path: PathBuf,
}

impl SecretStore {
    /// Store at `<agent_dir>/secrets.json`.
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            path: agent_dir.join(SECRETS_FILE),
        }
    }

    /// The file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Value of `key`.
    ///
    /// # Errors
    ///
    /// Lock or read failures (a missing or unparsable file reads as empty).
    pub fn get(&self, key: &str) -> io::Result<Option<String>> {
        self.update(|map| (map.get(key).cloned(), false))
    }

    /// Whether `key` holds a value.
    ///
    /// # Errors
    ///
    /// See [`Self::get`].
    pub fn contains(&self, key: &str) -> io::Result<bool> {
        self.update(|map| (map.contains_key(key), false))
    }

    /// Stores `value` at `key`.
    ///
    /// # Errors
    ///
    /// Lock, read or write failures.
    pub fn set(&self, key: &str, value: &str) -> io::Result<()> {
        self.update(|map| {
            let dirty = map.get(key).map(String::as_str) != Some(value);
            map.insert(key.to_owned(), value.to_owned());
            ((), dirty)
        })
    }

    /// Removes `key`, returning the old value.
    ///
    /// # Errors
    ///
    /// Lock, read or write failures.
    pub fn delete(&self, key: &str) -> io::Result<Option<String>> {
        self.update(|map| {
            let old = map.remove(key);
            let dirty = old.is_some();
            (old, dirty)
        })
    }

    /// Locked read-modify-write; `f` returns `(result, dirty)`.
    fn update<R>(
        &self,
        f: impl FnOnce(&mut BTreeMap<String, String>) -> (R, bool),
    ) -> io::Result<R> {
        let _lock = FileLock::acquire(&self.path)?;
        let mut map: BTreeMap<String, String> = read_optional(&self.path)?
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default();
        let (out, dirty) = f(&mut map);
        if dirty {
            let bytes = serde_json::to_vec_pretty(&map).map_err(io::Error::other)?;
            write_atomic(&self.path, &bytes)?;
            restrict(&self.path)?;
        }
        Ok(out)
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the unix variant"
)]
fn restrict(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_delete_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path());
        assert_eq!(store.get("a/k").unwrap(), None);
        store.set("a/k", "v").unwrap();
        assert_eq!(store.get("a/k").unwrap().as_deref(), Some("v"));
        assert!(store.contains("a/k").unwrap());
        assert!(!store.contains("b/k").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(store.delete("a/k").unwrap().as_deref(), Some("v"));
        assert_eq!(store.delete("a/k").unwrap(), None);
        assert_eq!(store.get("a/k").unwrap(), None);
    }
}
