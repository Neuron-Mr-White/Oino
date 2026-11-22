//! `~/.oino/secrets.json` (0600, file lock), keys namespaced `<ext-id>/<name>` (§5.9).
//!
//! A thin per-extension view over [`oino_settings::SecretStore`], the same store the core
//! settings registry uses for `secret` fields (`<owner-ext-id>/<namespace>/<key>`).

use std::path::Path;

use oino_extension_api::ops::SecretOp;
use oino_extension_api::{ExtensionId, HostError};

/// The secrets file.
#[derive(Clone, Debug)]
pub struct SecretStore {
    inner: oino_settings::SecretStore,
}

impl SecretStore {
    /// Store at `<agent_dir>/secrets.json`.
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            inner: oino_settings::SecretStore::new(agent_dir),
        }
    }

    /// Runs `op` in `ext`'s namespace.
    ///
    /// # Errors
    ///
    /// I/O failures.
    pub fn apply(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError> {
        let key = |name: &str| format!("{ext}/{name}");
        let io = |e: std::io::Error| HostError::io(&e);
        match op {
            SecretOp::Get(name) => self.inner.get(&key(&name)).map_err(io),
            SecretOp::Set { name, value } => self
                .inner
                .set(&key(&name), &value)
                .map_err(io)
                .map(|()| None),
            SecretOp::Delete(name) => self.inner.delete(&key(&name)).map_err(io),
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
