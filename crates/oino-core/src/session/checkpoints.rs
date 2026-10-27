//! File checkpoints for per-turn undo and rewind (`_oino/checkpoints`, `_oino/rewind`; Oino
//! addition, runtime §13).
//!
//! Before a built-in `write` or `edit` call changes a file, the session records the file's
//! previous content (or its absence) under the checkpoint of the user message that started
//! the run, once per file and checkpoint. Rewinding to a user message restores every file
//! touched at or after that checkpoint to the state it had before it and, optionally, forks
//! the conversation before that message (pi `fork`). Shell commands and extension tools are
//! not tracked.
//!
//! Persisted sessions keep the checkpoints in `<agentDir>/checkpoints/<sessionId>/`
//! (`index.json` plus one blob per snapshot); in-memory sessions keep them in memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Most checkpoints kept per session (oldest dropped first).
pub const MAX_CHECKPOINTS: usize = 100;
/// Files larger than this are not snapshotted.
pub const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

/// One file of a checkpoint as frontends see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointFile {
    /// Absolute path.
    pub path: String,
    /// The file existed before the change (`false`: rewind deletes it).
    pub existed: bool,
}

/// One checkpoint (`_oino/checkpoints`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    /// User message entry that started the run.
    pub entry_id: String,
    /// Creation time (epoch ms).
    pub created_at: i64,
    /// Files changed after this checkpoint started, in first-change order.
    pub files: Vec<CheckpointFile>,
}

/// Outcome of `_oino/rewind`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindResult {
    /// Files written back.
    pub restored: Vec<String>,
    /// Files deleted (created after the checkpoint).
    pub deleted: Vec<String>,
    /// Files that could not be restored, with the error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<RewindFailure>,
    /// Text of the user message (composer prefill) when the conversation was forked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A `session_before_fork` handler cancelled the conversation fork.
    pub cancelled: bool,
}

/// A file `_oino/rewind` could not restore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindFailure {
    /// Path.
    pub path: String,
    /// Error text.
    pub error: String,
}

/// Where a snapshot's bytes live.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum Blob {
    /// The file did not exist.
    Absent,
    /// Blob file name under `blobs/`.
    File {
        /// Name.
        name: String,
    },
    /// Kept in memory (sessions without a file).
    #[serde(skip)]
    Memory(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    entry_id: String,
    created_at: i64,
    files: Vec<(String, Blob)>,
}

/// The checkpoints of one session.
#[derive(Debug, Default)]
pub(crate) struct CheckpointStore {
    dir: Option<PathBuf>,
    loaded: bool,
    list: Vec<Stored>,
    next_blob: u64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

impl CheckpointStore {
    /// Points the store at `dir` (persisted sessions) and loads its index once.
    pub(crate) fn bind(&mut self, dir: Option<PathBuf>) {
        if self.loaded && self.dir == dir {
            return;
        }
        self.dir = dir;
        self.loaded = true;
        self.list.clear();
        let Some(dir) = &self.dir else {
            return;
        };
        let index = std::fs::read(dir.join("index.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Stored>>(&bytes).ok());
        if let Some(index) = index {
            self.next_blob = index
                .iter()
                .flat_map(|c| &c.files)
                .filter_map(|(_, blob)| match blob {
                    Blob::File { name } => name.parse::<u64>().ok(),
                    _ => None,
                })
                .max()
                .map_or(0, |n| n + 1);
            self.list = index;
        }
    }

    fn persist(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        if let Ok(bytes) = serde_json::to_vec(&self.list) {
            let _ = std::fs::create_dir_all(dir);
            let tmp = dir.join("index.json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("index.json"));
            }
        }
    }

    /// Records `path`'s current content under the checkpoint of `entry_id` unless that
    /// checkpoint already has it. Returns whether a snapshot was taken.
    pub(crate) fn record(&mut self, entry_id: &str, path: &Path) -> bool {
        let key = path.display().to_string();
        let index = if let Some(index) = self.list.iter().position(|c| c.entry_id == entry_id) {
            index
        } else {
            self.list.push(Stored {
                entry_id: entry_id.to_owned(),
                created_at: now_ms(),
                files: Vec::new(),
            });
            self.list.len() - 1
        };
        if self.list[index].files.iter().any(|(p, _)| *p == key) {
            return false;
        }
        let blob = match std::fs::metadata(path) {
            Err(_) => Blob::Absent,
            Ok(meta) if !meta.is_file() || meta.len() > MAX_SNAPSHOT_BYTES => return false,
            Ok(_) => match std::fs::read(path) {
                Err(_) => return false,
                Ok(bytes) => self.store_blob(bytes),
            },
        };
        self.list[index].files.push((key, blob));
        self.evict();
        self.persist();
        true
    }

    fn store_blob(&mut self, bytes: Vec<u8>) -> Blob {
        let Some(dir) = &self.dir else {
            return Blob::Memory(bytes);
        };
        let name = self.next_blob.to_string();
        self.next_blob += 1;
        let blobs = dir.join("blobs");
        let _ = std::fs::create_dir_all(&blobs);
        match std::fs::write(blobs.join(&name), &bytes) {
            Ok(()) => Blob::File { name },
            Err(_) => Blob::Memory(bytes),
        }
    }

    fn evict(&mut self) {
        while self.list.len() > MAX_CHECKPOINTS {
            let old = self.list.remove(0);
            self.remove_blobs(&old);
        }
    }

    fn remove_blobs(&self, stored: &Stored) {
        let Some(dir) = &self.dir else {
            return;
        };
        for (_, blob) in &stored.files {
            if let Blob::File { name } = blob {
                let _ = std::fs::remove_file(dir.join("blobs").join(name));
            }
        }
    }

    /// Every checkpoint, oldest first.
    pub(crate) fn list(&self) -> Vec<Checkpoint> {
        self.list
            .iter()
            .map(|c| Checkpoint {
                entry_id: c.entry_id.clone(),
                created_at: c.created_at,
                files: c
                    .files
                    .iter()
                    .map(|(path, blob)| CheckpointFile {
                        path: path.clone(),
                        existed: *blob != Blob::Absent,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Whether a checkpoint for `entry_id` exists.
    pub(crate) fn contains(&self, entry_id: &str) -> bool {
        self.list.iter().any(|c| c.entry_id == entry_id)
    }

    /// Restores every file changed at or after the checkpoint of `entry_id` and drops those
    /// checkpoints. `None` when there is no such checkpoint.
    pub(crate) fn rewind(&mut self, entry_id: &str) -> Option<RewindResult> {
        let start = self.list.iter().position(|c| c.entry_id == entry_id)?;
        // The earliest snapshot of each path at or after `start` holds its state before.
        let mut earliest: BTreeMap<String, Blob> = BTreeMap::new();
        for stored in &self.list[start..] {
            for (path, blob) in &stored.files {
                earliest.entry(path.clone()).or_insert_with(|| blob.clone());
            }
        }
        let mut result = RewindResult::default();
        for (path, blob) in &earliest {
            let outcome = match blob {
                Blob::Absent => match std::fs::remove_file(path) {
                    Ok(()) => {
                        result.deleted.push(path.clone());
                        Ok(())
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.to_string()),
                },
                Blob::Memory(bytes) => write_back(path, bytes).map(|()| {
                    result.restored.push(path.clone());
                }),
                Blob::File { name } => self
                    .dir
                    .as_ref()
                    .ok_or_else(|| "checkpoint blob missing".to_owned())
                    .and_then(|dir| {
                        std::fs::read(dir.join("blobs").join(name)).map_err(|e| e.to_string())
                    })
                    .and_then(|bytes| write_back(path, &bytes))
                    .map(|()| result.restored.push(path.clone())),
            };
            if let Err(error) = outcome {
                result.failed.push(RewindFailure {
                    path: path.clone(),
                    error,
                });
            }
        }
        let dropped: Vec<Stored> = self.list.drain(start..).collect();
        for stored in &dropped {
            self.remove_blobs(stored);
        }
        self.persist();
        Some(result)
    }

    /// Copies the checkpoints whose entry `keep` accepts into `target` (the session a fork
    /// created keeps the file history of the entries it kept).
    pub(crate) fn copy_matching(&self, keep: impl Fn(&str) -> bool, target: &mut Self) {
        for stored in self.list.iter().filter(|c| keep(&c.entry_id)) {
            let mut copy = Stored {
                entry_id: stored.entry_id.clone(),
                created_at: stored.created_at,
                files: Vec::new(),
            };
            for (path, blob) in &stored.files {
                let blob = match blob {
                    Blob::Absent => Blob::Absent,
                    Blob::Memory(bytes) => target.store_blob(bytes.clone()),
                    Blob::File { name } => {
                        let bytes = self
                            .dir
                            .as_ref()
                            .and_then(|dir| std::fs::read(dir.join("blobs").join(name)).ok());
                        match bytes {
                            Some(bytes) => target.store_blob(bytes),
                            None => continue,
                        }
                    }
                };
                copy.files.push((path.clone(), blob));
            }
            target.list.push(copy);
        }
        target.persist();
    }
}

fn write_back(path: &str, bytes: &[u8]) -> Result<(), String> {
    let path = Path::new(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(persisted: bool) {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let a = work.join("a.txt");
        let b = work.join("b.txt");
        std::fs::write(&a, "a0").unwrap();
        let mut store = CheckpointStore::default();
        store.bind(persisted.then(|| dir.path().join("cp")));
        assert!(store.record("u1", &a));
        assert!(!store.record("u1", &a), "once per checkpoint");
        std::fs::write(&a, "a1").unwrap();
        assert!(store.record("u2", &a));
        assert!(store.record("u2", &b));
        std::fs::write(&a, "a2").unwrap();
        std::fs::write(&b, "b2").unwrap();
        assert_eq!(store.list().len(), 2);
        assert!(!store.list()[1].files[1].existed);
        if persisted {
            let mut reloaded = CheckpointStore::default();
            reloaded.bind(Some(dir.path().join("cp")));
            assert_eq!(reloaded.list(), store.list());
            store = reloaded;
        }
        let result = store.rewind("u2").unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a1");
        assert!(!b.exists());
        assert_eq!(result.restored.len(), 1);
        assert_eq!(result.deleted.len(), 1);
        assert_eq!(store.list().len(), 1);
        let result = store.rewind("u1").unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a0");
        assert_eq!(result.restored, vec![a.display().to_string()]);
        assert!(store.rewind("u1").is_none());
    }

    #[test]
    fn rewind_in_memory() {
        roundtrip(false);
    }

    #[test]
    fn rewind_persisted() {
        roundtrip(true);
    }

    #[test]
    fn copy_kept_entries() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::write(&a, "x").unwrap();
        let mut store = CheckpointStore::default();
        store.bind(None);
        store.record("u1", &a);
        store.record("u2", &a);
        let mut target = CheckpointStore::default();
        target.bind(None);
        store.copy_matching(|id| id == "u1", &mut target);
        assert_eq!(target.list().len(), 1);
        assert!(target.contains("u1"));
    }
}
