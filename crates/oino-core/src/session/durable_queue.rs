//! Crash-safe storage for host queue admission. This is separate from the session JSONL:
//! callers must retain an item until its durable message id is durably recorded as delivered.
//! A successful `store` is the durability barrier before acknowledging a command.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One accepted command, including images and streaming behaviour in `command`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableQueueItem {
    /// Host-assigned durable message id. Never deduplicate by text.
    pub message_id: String,
    /// Original command, preserved without expansion or input-hook replay here.
    pub command: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    items: Vec<DurableQueueItem>,
}

/// Per-session queue sidecar. The caller serializes mutations for each session.
/// There is deliberately no process-wide lock or session JSONL lock around IO.
#[derive(Clone, Debug)]
pub struct DurableQueue {
    path: PathBuf,
}

impl DurableQueue {
    /// Use `<session-file>.queue.json` next to the session, including its extension.
    #[must_use]
    pub fn new(session_file: &Path) -> Self {
        let mut path = session_file.as_os_str().to_os_string();
        path.push(".queue.json");
        Self { path: path.into() }
    }

    /// Location to include in a restore-failure diagnostic. Failed loads never delete it.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Restore FIFO order, removing only durable message ids already delivered to the JSONL.
    /// Duplicate ids within the snapshot are collapsed to their first occurrence.
    /// Missing sidecars mean an empty queue; unreadable or incompatible ones are errors.
    ///
    /// # Errors
    /// Reading or decoding the snapshot failed; the original file is retained.
    pub fn load(&self, delivered: &HashSet<String>) -> io::Result<Vec<DurableQueueItem>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_slice(&bytes).map_err(invalid_data)?;
        if value.get("version").and_then(Value::as_u64) != Some(2) {
            return Err(invalid_data("unsupported queue snapshot version"));
        }
        let snapshot: Snapshot = serde_json::from_value(value).map_err(invalid_data)?;
        let mut seen = delivered.clone();
        Ok(snapshot
            .items
            .into_iter()
            .filter(|item| seen.insert(item.message_id.clone()))
            .collect())
    }

    /// Move a failed snapshot out of the admission path, retaining it for recovery.
    /// # Errors
    /// Rename or directory sync failed; admission must remain blocked in that case.
    pub fn quarantine(&self) -> io::Result<()> {
        // Keep earlier failures too; a second damaged snapshot must not erase evidence.
        let previous = self.quarantine_path();
        if previous.exists() {
            fs::rename(
                &previous,
                previous.with_extension(format!("failed-{}", uuid::Uuid::new_v4())),
            )?;
        }
        fs::rename(&self.path, self.quarantine_path())?;
        File::open(
            self.path
                .parent()
                .ok_or_else(|| invalid_data("queue has no parent"))?,
        )?
        .sync_all()
    }

    fn quarantine_path(&self) -> PathBuf {
        self.path.with_extension("json.failed")
    }

    /// Salvage complete item objects, including those preceding a truncated JSON tail.
    #[must_use]
    pub fn recover_quarantined(&self) -> Value {
        let Ok(bytes) = fs::read(self.quarantine_path()) else {
            return Value::Array(vec![]);
        };
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            return value.get("items").cloned().unwrap_or(Value::Array(vec![]));
        }
        let mut items = Vec::new();
        let mut consumed = 0;
        for (start, byte) in bytes.iter().enumerate() {
            if start < consumed {
                continue;
            }
            if *byte != b'{' {
                continue;
            }
            let mut stream =
                serde_json::Deserializer::from_slice(&bytes[start..]).into_iter::<Value>();
            if let Some(Ok(value)) = stream.next() {
                if value.get("command").is_some() {
                    consumed = start + stream.byte_offset();
                    items.push(value);
                } else if value.get("message").and_then(Value::as_str).is_some() {
                    items.push(serde_json::json!({"command": value}));
                }
            }
        }
        Value::Array(items)
    }

    /// Atomically replace the queue, syncing both data and directory before returning.
    /// Empty queues remove the sidecar durably. Failures before rename preserve the
    /// old snapshot. A directory sync failure after rename leaves durability uncertain.
    /// An abandoned temp file after process death is never restored.
    ///
    /// # Errors
    /// Writing, renaming, removing or syncing the snapshot/directory failed.
    pub fn store(&self, items: &[DurableQueueItem]) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| invalid_data("queue has no parent"))?;
        if items.is_empty() {
            match fs::remove_file(&self.path) {
                Ok(()) => return File::open(parent)?.sync_all(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        let bytes = serde_json::to_vec(&Snapshot {
            version: 2,
            items: items.to_vec(),
        })
        .map_err(invalid_data)?;
        // Unique create-new temp names permit cleanup without touching another writer's file.
        let temp = parent.join(format!(".queue-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, text: &str) -> DurableQueueItem {
        DurableQueueItem {
            message_id: id.into(),
            command: json!({"type": "follow_up", "message": text}),
        }
    }

    #[test]
    fn restart_preserves_fifo_edits_removal_and_identical_texts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let queue = DurableQueue::new(&path);
        let mut items = vec![item("b", "same"), item("c", "same"), item("d", "remove")];
        queue.store(&items).unwrap();
        // Simulate restart with no live writer state.
        assert_eq!(
            DurableQueue::new(&path).load(&HashSet::new()).unwrap(),
            items
        );
        items[0].command["message"] = json!("edited");
        items.pop();
        items.swap(0, 1);
        queue.store(&items).unwrap();
        assert_eq!(queue.load(&HashSet::new()).unwrap(), items);
        queue.store(&[]).unwrap();
        assert!(!queue.path().exists());
        assert_eq!(
            queue.load(&HashSet::new()).unwrap(),
            [] as [DurableQueueItem; 0]
        );
    }

    #[test]
    fn crash_after_jsonl_delivery_before_queue_cleanup_does_not_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DurableQueue::new(&dir.path().join("session.jsonl"));
        queue
            .store(&[item("b", "B"), item("c", "C"), item("c", "C")])
            .unwrap();
        let delivered = HashSet::from(["b".to_owned()]);
        assert_eq!(queue.load(&delivered).unwrap(), vec![item("c", "C")]);
    }

    #[test]
    fn failed_restore_preserves_evidence_and_abandoned_temp_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DurableQueue::new(&dir.path().join("session.jsonl"));
        fs::write(dir.path().join(".queue-aborted.tmp"), b"partial").unwrap();
        assert_eq!(
            queue.load(&HashSet::new()).unwrap(),
            [] as [DurableQueueItem; 0]
        );
        fs::write(queue.path(), b"broken").unwrap();
        assert_eq!(
            queue.load(&HashSet::new()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(queue.path()).unwrap(), b"broken");
        fs::write(queue.path(), br#"{"version":99,"items":[]}"#).unwrap();
        assert!(queue.load(&HashSet::new()).is_err());
    }

    #[test]
    fn quarantine_salvages_truncated_items_and_allows_fresh_admission() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DurableQueue::new(&dir.path().join("session.jsonl"));
        let bytes = br#"{"version":99,"items":[{"commandId":"1","command":{"type":"follow_up","message":"recover me"}},{"command": "#;
        fs::write(queue.path(), bytes).unwrap();
        assert!(queue.load(&HashSet::new()).is_err());
        queue.quarantine().unwrap();
        assert_eq!(queue.recover_quarantined().as_array().unwrap().len(), 1);
        assert_eq!(
            queue.recover_quarantined()[0]["command"]["message"],
            "recover me"
        );
        queue.store(&[item("fresh", "new")]).unwrap();
        assert_eq!(
            queue.load(&HashSet::new()).unwrap(),
            vec![item("fresh", "new")]
        );
        assert_eq!(fs::read(queue.quarantine_path()).unwrap(), bytes);
    }

    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os("OINO_QUEUE_CRASH_TEST_PATH") else {
            return;
        };
        let queue = DurableQueue::new(Path::new(&path));
        queue.store(&[item("b", "B"), item("c", "C")]).unwrap();
        // Exit without running destructors, as if killed after the durability ACK barrier.
        std::process::exit(73);
    }

    #[test]
    fn process_exit_after_enqueue_restores_b_c_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::durable_queue::tests::crash_child",
                "--nocapture",
            ])
            .env("OINO_QUEUE_CRASH_TEST_PATH", &path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73));
        assert_eq!(
            DurableQueue::new(&path).load(&HashSet::new()).unwrap(),
            vec![item("b", "B"), item("c", "C")]
        );
    }

    #[test]
    #[ignore = "manual disk latency measurement; run with --ignored --nocapture"]
    fn durable_write_latency() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DurableQueue::new(&dir.path().join("session.jsonl"));
        let items = vec![item("b", "B"), item("c", "C")];
        let mut micros = Vec::new();
        for _ in 0..200 {
            let start = std::time::Instant::now();
            queue.store(&items).unwrap();
            micros.push(start.elapsed().as_micros());
        }
        micros.sort_unstable();
        let _ = Write::write_fmt(
            &mut io::stdout(),
            format_args!(
                "queue store n=200 p50_us={} p95_us={} max_us={}",
                micros[99], micros[189], micros[199]
            ),
        );
    }
}
