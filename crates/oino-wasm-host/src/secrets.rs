//! `~/.oino/secrets.json` (0600, file lock), keys namespaced `<ext-id>/<name>` (§5.9).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oino_extension_api::ops::SecretOp;
use oino_extension_api::{ExtensionId, HostError};

/// The secrets file.
#[derive(Clone, Debug)]
pub struct SecretStore {
    path: PathBuf,
}

impl SecretStore {
    /// Store at `<agent_dir>/secrets.json`.
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            path: agent_dir.join("secrets.json"),
        }
    }

    fn with_lock<R>(
        &self,
        f: impl FnOnce(&mut BTreeMap<String, String>) -> (R, bool),
    ) -> Result<R, HostError> {
        let lock_path = self.path.with_extension("json.lock");
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(&e))?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| HostError::io(&e))?;
        lock.lock().map_err(|e| HostError::io(&e))?;
        let mut map: BTreeMap<String, String> = std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (out, dirty) = f(&mut map);
        let result = if dirty {
            serde_json::to_vec_pretty(&map)
                .map_err(|e| HostError::Failed(e.to_string()))
                .and_then(|bytes| {
                    crate::kv::atomic_write(&self.path, &bytes, Some(0o600))
                        .map_err(|e| HostError::io(&e))
                })
        } else {
            Ok(())
        };
        let _ = lock.unlock();
        result.map(|()| out)
    }

    /// Runs `op` in `ext`'s namespace.
    ///
    /// # Errors
    ///
    /// I/O failures.
    pub fn apply(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError> {
        let key = |name: &str| format!("{ext}/{name}");
        match op {
            SecretOp::Get(name) => self.with_lock(|m| (m.get(&key(&name)).cloned(), false)),
            SecretOp::Set { name, value } => self.with_lock(|m| {
                m.insert(key(&name), value);
                (None, true)
            }),
            SecretOp::Delete(name) => self.with_lock(|m| {
                let old = m.remove(&key(&name));
                let dirty = old.is_some();
                (old, dirty)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaced_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path());
        let a = ExtensionId::new("a").unwrap();
        let b = ExtensionId::new("b").unwrap();
        store
            .apply(
                &a,
                SecretOp::Set {
                    name: "k".into(),
                    value: "v".into(),
                },
            )
            .unwrap();
        assert_eq!(
            store
                .apply(&a, SecretOp::Get("k".into()))
                .unwrap()
                .as_deref(),
            Some("v")
        );
        assert_eq!(store.apply(&b, SecretOp::Get("k".into())).unwrap(), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("secrets.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(
            store
                .apply(&a, SecretOp::Delete("k".into()))
                .unwrap()
                .as_deref(),
            Some("v")
        );
        assert_eq!(store.apply(&a, SecretOp::Get("k".into())).unwrap(), None);
    }
}
