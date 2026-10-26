//! `kv`: one JSON file (`kv.json`) per scope directory, values base64-encoded, written
//! atomically, capped at 16 MiB.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oino_extension_api::HostError;

use crate::state::lock;

/// Maximum size of one `kv.json`.
pub const KV_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Serialises access to the kv files of one extension.
#[derive(Default)]
pub(crate) struct KvStore {
    guard: Mutex<()>,
}

fn file(dir: &Path) -> PathBuf {
    dir.join("kv.json")
}

fn read_map(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read(file(dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes `bytes` to `path` atomically (temp file + rename), optionally setting `mode`.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    std::fs::rename(&tmp, path)
}

impl KvStore {
    pub(crate) fn get(&self, dir: &Path, key: &str) -> Option<Vec<u8>> {
        let _g = lock(&self.guard);
        read_map(dir).get(key).and_then(|v| STANDARD.decode(v).ok())
    }

    pub(crate) fn set(&self, dir: &Path, key: &str, value: &[u8]) -> Result<(), HostError> {
        let _g = lock(&self.guard);
        let mut map = read_map(dir);
        map.insert(key.to_owned(), STANDARD.encode(value));
        Self::write(dir, &map)
    }

    pub(crate) fn delete(&self, dir: &Path, key: &str) {
        let _g = lock(&self.guard);
        let mut map = read_map(dir);
        if map.remove(key).is_some() {
            let _ = Self::write(dir, &map);
        }
    }

    pub(crate) fn keys(&self, dir: &Path, prefix: &str) -> Vec<String> {
        let _g = lock(&self.guard);
        read_map(dir)
            .into_keys()
            .filter(|k| k.starts_with(prefix))
            .collect()
    }

    fn write(dir: &Path, map: &BTreeMap<String, String>) -> Result<(), HostError> {
        let bytes = serde_json::to_vec(map).map_err(|e| HostError::Failed(e.to_string()))?;
        if bytes.len() > KV_MAX_BYTES {
            return Err(HostError::InvalidArgument(format!(
                "kv store exceeds {KV_MAX_BYTES} bytes"
            )));
        }
        atomic_write(&file(dir), &bytes, None).map_err(|e| HostError::io(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let kv = KvStore::default();
        assert!(kv.get(dir.path(), "a").is_none());
        kv.set(dir.path(), "a:1", b"\x00\xff").unwrap();
        kv.set(dir.path(), "b", b"x").unwrap();
        assert_eq!(kv.get(dir.path(), "a:1").unwrap(), b"\x00\xff");
        assert_eq!(kv.keys(dir.path(), "a:"), vec!["a:1"]);
        kv.delete(dir.path(), "a:1");
        assert!(kv.get(dir.path(), "a:1").is_none());
        let big = vec![0u8; KV_MAX_BYTES];
        assert!(kv.set(dir.path(), "big", &big).is_err());
    }
}
