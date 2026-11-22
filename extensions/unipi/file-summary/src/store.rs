//! Durable state of `unipi.file-summary` (core.md §3.1.6): content-addressed zstd blobs under
//! the session's `ext-session` directory, the in-progress turn index in kv, and the custom
//! entry payloads `unipi.file-summary/turn` / `unipi.file-summary/undo`.
//!
//! Everything the extension shows is rebuildable from those three stores: the custom entries
//! carry the finished turn summaries, the kv index carries the turn that was still running
//! when the process died, and blobs are looked up by the sha256 recorded in either.

use std::fmt::Write as _;
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Result;

/// Custom entry appended at `agent_end` with one finished turn summary.
pub const TURN_CUSTOM_TYPE: &str = "unipi.file-summary/turn";
/// Custom entry appended by every undo.
pub const UNDO_CUSTOM_TYPE: &str = "unipi.file-summary/undo";
/// Largest file that is snapshotted (`> MAX` ⇒ `skipped: "too_large"`).
pub const MAX_SNAPSHOT_BYTES: u64 = 5 * 1024 * 1024;
/// Cap of one `diff` service reply.
pub const MAX_DIFF_BYTES: usize = 1024 * 1024;
/// kv key prefix of the in-progress turn index (`turn:<turnEntryId>`).
pub const INDEX_PREFIX: &str = "turn:";
/// `skipped` reason of a file over [`MAX_SNAPSHOT_BYTES`].
pub const TOO_LARGE: &str = "too_large";
/// `skipped` reason of a file that exists but cannot be read.
pub const UNREADABLE: &str = "unreadable";

/// One touched path of a turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    /// Path as shown to the user: relative to the session cwd when inside it, else absolute.
    pub path: String,
    /// The file existed before the turn (false ⇒ the turn created it).
    pub existed: bool,
    /// sha256 of the content before the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_sha: Option<String>,
    /// sha256 of the content after the turn (absent ⇒ deleted or not comparable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sha: Option<String>,
    /// Lines added by the turn.
    #[serde(default)]
    pub add: u64,
    /// Lines removed by the turn.
    #[serde(default)]
    pub del: u64,
    /// Why the path has no snapshot (`too_large`, `unreadable`); listed, no undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl FileRecord {
    /// The path is listed but cannot be snapshotted, diffed or undone.
    #[must_use]
    pub const fn is_skipped(&self) -> bool {
        self.skipped.is_some()
    }
}

/// Payload of one `unipi.file-summary/turn` custom entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRecord {
    /// Session entry id of the user message that started the turn (the app's `turnId`).
    pub turn_entry_id: String,
    /// Touched paths in first-touch order.
    #[serde(default)]
    pub files: Vec<FileRecord>,
}

/// Payload of one `unipi.file-summary/undo` custom entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoRecord {
    /// Turn that was undone.
    pub turn_entry_id: String,
    /// Path filter of the request (`None` = every file of the turn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
    /// The undo ran with `force` (conflicts were overwritten).
    #[serde(default)]
    pub force: bool,
    /// Paths restored from their `beforeSha` blob.
    #[serde(default)]
    pub restored: Vec<String>,
    /// Paths deleted again because the turn had created them.
    #[serde(default)]
    pub deleted: Vec<String>,
    /// Paths changed since the turn ended (left alone without `force`).
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// Paths whose undo failed (I/O, unresolvable, missing blob).
    #[serde(default)]
    pub failed: Vec<String>,
}

impl UndoRecord {
    /// Paths this undo put back into their pre-turn state.
    pub fn undone_paths(&self) -> impl Iterator<Item = &str> {
        self.restored
            .iter()
            .chain(self.deleted.iter())
            .map(String::as_str)
    }
}

/// sha256 of `bytes`, lowercase hex.
#[must_use]
pub fn hash_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// sha256 of a file, streamed in chunks.
///
/// # Errors
///
/// The file cannot be read.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(buf.get(..read).unwrap_or_default());
    }
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// Compresses `bytes` into a zstd frame (pure Rust encoder, so the guest needs no C toolchain).
#[must_use]
pub fn compress(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(
        Cursor::new(bytes),
        ruzstd::encoding::CompressionLevel::Fastest,
    )
}

/// Decompresses a zstd frame; `None` for a corrupt blob.
#[must_use]
pub fn decompress(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(Cursor::new(bytes)).ok()?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}

/// `<ext-session>/<sid>/blobs`.
#[must_use]
pub fn blob_dir(session_id: &str) -> PathBuf {
    unipi_kit::paths::session(session_id, "blobs")
}

/// Path of one blob inside the session directory.
#[must_use]
pub fn blob_path(session_id: &str, sha: &str) -> PathBuf {
    blob_dir(session_id).join(format!("{sha}.zst"))
}

/// Stores a blob under its content hash; the second store of the same content is a no-op
/// (dedup). `sha` must be [`hash_hex`] of `bytes`.
///
/// # Errors
///
/// The blob directory or file cannot be written.
pub fn store_blob(session_id: &str, sha: &str, bytes: &[u8]) -> Result<()> {
    let path = blob_path(session_id, sha);
    if path.is_file() {
        return Ok(());
    }
    unipi_kit::paths::ensure_parent(&path)
        .map_err(|error| oino_extension::Error::msg(error.to_string()))?;
    oino_extension::fs::atomic_write(&path.to_string_lossy(), &compress(bytes))?;
    Ok(())
}

/// Loads and decompresses one blob.
#[must_use]
pub fn read_blob(session_id: &str, sha: &str) -> Option<Vec<u8>> {
    decompress(&std::fs::read(blob_path(session_id, sha)).ok()?)
}

/// kv key of a turn's in-progress index.
#[must_use]
pub fn index_key(turn_entry_id: &str) -> String {
    format!("{INDEX_PREFIX}{turn_entry_id}")
}

/// Writes the in-progress index of one turn (crash-safe: one key per turn).
///
/// # Errors
///
/// The kv store rejects the value.
pub fn save_index(record: &TurnRecord) -> Result<()> {
    oino_extension::kv::set_json(
        oino_extension::kv::Scope::Session,
        &index_key(&record.turn_entry_id),
        record,
    )
}

/// Reads the in-progress index of one turn.
#[must_use]
pub fn load_index(turn_entry_id: &str) -> Option<TurnRecord> {
    oino_extension::kv::get_json(
        oino_extension::kv::Scope::Session,
        &index_key(turn_entry_id),
    )
    .ok()
    .flatten()
}

/// Deletes the in-progress index of one turn.
pub fn delete_index(turn_entry_id: &str) {
    oino_extension::kv::delete(
        oino_extension::kv::Scope::Session,
        &index_key(turn_entry_id),
    );
}

/// Every in-progress turn index of the session (resume rebuild).
#[must_use]
pub fn load_indexes() -> Vec<TurnRecord> {
    oino_extension::kv::keys(oino_extension::kv::Scope::Session, INDEX_PREFIX)
        .into_iter()
        .filter_map(|key| key.strip_prefix(INDEX_PREFIX).and_then(load_index))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests assert by unwrapping")]
    use super::*;

    #[test]
    fn zstd_round_trip() {
        let text = "alpha\nbeta\ngamma\n".repeat(64);
        let packed = compress(text.as_bytes());
        assert!(
            packed.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]),
            "zstd frame magic"
        );
        assert_eq!(decompress(&packed).unwrap(), text.as_bytes());
        assert!(decompress(b"not zstd").is_none());
    }

    #[test]
    fn file_record_wire_shape() {
        let record = FileRecord {
            path: "src/lib.rs".into(),
            existed: true,
            before_sha: Some("aa".into()),
            after_sha: None,
            add: 2,
            del: 1,
            skipped: None,
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "path": "src/lib.rs", "existed": true, "beforeSha": "aa", "add": 2, "del": 1
            })
        );
        assert_eq!(serde_json::from_value::<FileRecord>(value).unwrap(), record);
    }

    #[test]
    fn undo_record_undone_paths() {
        let record = UndoRecord {
            restored: vec!["a".into()],
            deleted: vec!["b".into()],
            ..UndoRecord::default()
        };
        assert_eq!(record.undone_paths().collect::<Vec<_>>(), ["a", "b"]);
    }
}
