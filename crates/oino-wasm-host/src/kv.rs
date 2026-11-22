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

/// Locks a stable sidecar inode, never the JSON inode replaced by `atomic_write`.
/// The file lock is released on drop, including when a worker is killed.
fn lock_scope(dir: &Path) -> std::io::Result<Option<std::fs::File>> {
    if std::env::var("OINO_WORKERS").as_deref() != Ok("1") {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("kv.lock"))?;
    file.lock()?;
    Ok(Some(file))
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
        let _file = lock_scope(dir).map_err(|e| HostError::io(&e))?;
        let mut map = read_map(dir);
        map.insert(key.to_owned(), STANDARD.encode(value));
        Self::write(dir, &map)
    }

    pub(crate) fn delete(&self, dir: &Path, key: &str) {
        let _g = lock(&self.guard);
        let Ok(_file) = lock_scope(dir) else {
            return;
        };
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

    // Spawn this same test binary so the contention is between OS processes,
    // not merely independent mutexes in one address space.
    #[test]
    fn kv_process_writer() {
        let Some(dir) = std::env::var_os("OINO_KV_STRESS_DIR") else {
            return;
        };
        let prefix = std::env::var("OINO_KV_STRESS_PREFIX").unwrap();
        let kv = KvStore::default();
        for n in 0..100 {
            let key = format!("{prefix}:{n}");
            kv.set(Path::new(&dir), &key, key.as_bytes()).unwrap();
            kv.set(Path::new(&dir), &format!("{key}:temporary"), b"x")
                .unwrap();
            kv.delete(Path::new(&dir), &format!("{key}:temporary"));
        }
    }

    #[test]
    fn kv_two_processes_lose_no_updates() {
        let dir = tempfile::tempdir().unwrap();
        let mut children: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|prefix| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "kv::tests::kv_process_writer"])
                    .env("OINO_WORKERS", "1")
                    .env("OINO_KV_STRESS_DIR", dir.path())
                    .env("OINO_KV_STRESS_PREFIX", prefix)
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let kv = KvStore::default();
        assert_eq!(kv.keys(dir.path(), "").len(), 200);
        for prefix in ["a", "b"] {
            for n in 0..100 {
                let key = format!("{prefix}:{n}");
                assert_eq!(kv.get(dir.path(), &key).unwrap(), key.as_bytes());
            }
        }
    }

    #[test]
    fn kv_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let kv = KvStore::default();
        assert!(kv.get(dir.path(), "a").is_none());
        kv.set(dir.path(), "a:1", b"\x00\xff").unwrap();
        kv.set(dir.path(), "b", b"x").unwrap();
        assert_eq!(
            dir.path().join("kv.lock").exists(),
            std::env::var("OINO_WORKERS").as_deref() == Ok("1")
        );
        assert_eq!(kv.get(dir.path(), "a:1").unwrap(), b"\x00\xff");
        assert_eq!(kv.keys(dir.path(), "a:"), vec!["a:1"]);
        kv.delete(dir.path(), "a:1");
        assert!(kv.get(dir.path(), "a:1").is_none());
        let big = vec![0u8; KV_MAX_BYTES];
        assert!(kv.set(dir.path(), "big", &big).is_err());
    }
}
