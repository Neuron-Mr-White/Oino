//! Session meta (app spec §3.14, WP-A4): the per-user flags the app sets on a
//! session — pin (Z-14), unread (Z-82/Z-86), title lock (Z-15) — plus the
//! per-connection focus registry that decides Z-86's unread rule.
//!
//! - [`MetaStore`] persists `session-meta.json` in the host data dir
//!   (`~/.oino/host/`, §3.14). Writes are atomic (`.tmp-*` then rename); reads
//!   tolerate a missing or corrupt file (flags start clear, the file is
//!   rewritten on the next change).
//! - [`FocusRegistry`] tracks which session each live `events_open` connection
//!   focuses (`Control::Focus`, sent on that stream). A connection's focus ends
//!   with the stream; "no connection has it focused" is what sets `unread` when
//!   a run ends or a session needs the user.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::card::SessionCard;
use unipi_proto::session::SessionMetaPatch;

/// The flags of one session as stored (absent = all clear).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMetaFlags {
    /// Pinned to the sidebar's "Pinned" group (Z-14).
    #[serde(default)]
    pub pinned: bool,
    /// The session ended a run / needed the user unseen (Z-82/Z-86).
    #[serde(default)]
    pub unread: bool,
    /// A manual name beats auto-rename (Z-15).
    #[serde(default)]
    pub title_locked: bool,
    /// When the composer draft was last saved (epoch ms; 0 = none; owner 2026-11-06).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub draft_at: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// The largest draft document the host keeps (attachments are blob refs, so this is text).
pub const DRAFT_MAX_BYTES: usize = 512 * 1024;

impl SessionMetaFlags {
    /// Applies a patch's present fields.
    #[must_use]
    pub fn patched(mut self, patch: SessionMetaPatch) -> Self {
        if let Some(pinned) = patch.pinned {
            self.pinned = pinned;
        }
        if let Some(unread) = patch.unread {
            self.unread = unread;
        }
        if let Some(title_locked) = patch.title_locked {
            self.title_locked = title_locked;
        }
        self
    }

    /// True when no flag is set (the record is dropped from the file).
    #[must_use]
    pub fn is_clear(self) -> bool {
        !(self.pinned || self.unread || self.title_locked || self.draft_at != 0)
    }
}

/// `session-meta.json`: `{ "version": 1, "sessions": { "<id>": flags } }`.
#[derive(Debug, Serialize, Deserialize)]
struct MetaFile {
    version: u32,
    sessions: HashMap<String, SessionMetaFlags>,
}

impl Default for MetaFile {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: HashMap::new(),
        }
    }
}

/// The persisted meta of every session (WP-A4 §3.14).
pub struct MetaStore {
    path: PathBuf,
    flags: Mutex<HashMap<String, SessionMetaFlags>>,
}

impl MetaStore {
    /// Loads `session-meta.json` under `dir` (a missing or corrupt file starts
    /// clear — the flags are rebuildable from the app and never crash the host).
    #[must_use]
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("session-meta.json");
        let flags = std::fs::read(&path).ok().and_then(|bytes| {
            serde_json::from_slice::<MetaFile>(&bytes)
                .ok()
                .map(|file| file.sessions)
        });
        Self {
            path,
            flags: Mutex::new(flags.unwrap_or_default()),
        }
    }

    /// The flags of `session_id` (clear when never set).
    #[must_use]
    pub fn flags(&self, session_id: &str) -> SessionMetaFlags {
        lock(&self.flags)
            .get(session_id)
            .copied()
            .unwrap_or_default()
    }

    /// Applies a patch and persists; `true` when the file changed.
    pub fn patch(&self, session_id: &str, patch: SessionMetaPatch) -> bool {
        let mut flags = lock(&self.flags);
        let next = flags
            .get(session_id)
            .copied()
            .unwrap_or_default()
            .patched(patch);
        let changed = flags.get(session_id).copied() != Some(next);
        if next.is_clear() {
            flags.remove(session_id);
        } else {
            flags.insert(session_id.to_owned(), next);
        }
        changed && self.persist(&flags)
    }

    /// Merges the flags onto a card (cards without meta stay untouched).
    #[must_use]
    pub fn onto(&self, mut card: SessionCard) -> SessionCard {
        let flags = self.flags(&card.session_id);
        card.pinned = flags.pinned;
        card.unread = flags.unread;
        card.title_locked = flags.title_locked;
        card.draft_at = flags.draft_at;
        card
    }

    /// `drafts/<id>.json` next to `session-meta.json` (`None` for an id that is not a plain
    /// file name — ids come from the host's own records, this is belt and braces).
    fn draft_path(&self, session_id: &str) -> Option<PathBuf> {
        let safe = !session_id.is_empty()
            && session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        safe.then(|| {
            self.path
                .parent()
                .map(|dir| dir.join("drafts").join(format!("{session_id}.json")))
        })
        .flatten()
    }

    /// The session's stored draft (owner 2026-11-06), when there is one.
    #[must_use]
    pub fn draft(&self, session_id: &str) -> Option<serde_json::Value> {
        let path = self.draft_path(session_id)?;
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Stores (`Some`) or drops (`None`) the session's draft and stamps `draft_at` (`now_ms`, 0
    /// when cleared) into the meta. Errors only on a failed write of the draft itself.
    ///
    /// # Errors
    ///
    /// The draft file cannot be written.
    pub fn set_draft(
        &self,
        session_id: &str,
        draft: Option<&serde_json::Value>,
        now_ms: u64,
    ) -> std::io::Result<()> {
        let path = self.draft_path(session_id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad session id")
        })?;
        match draft {
            Some(value) => {
                let bytes = serde_json::to_vec(value)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
                std::fs::write(&tmp, &bytes)?;
                std::fs::rename(&tmp, &path)?;
            }
            None => match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            },
        }
        let at = if draft.is_some() { now_ms.max(1) } else { 0 };
        let mut flags = lock(&self.flags);
        let mut next = flags.get(session_id).copied().unwrap_or_default();
        next.draft_at = at;
        if next.is_clear() {
            flags.remove(session_id);
        } else {
            flags.insert(session_id.to_owned(), next);
        }
        self.persist(&flags);
        Ok(())
    }

    /// Writes the file atomically; `false` (with a log line) when it cannot.
    fn persist(&self, flags: &HashMap<String, SessionMetaFlags>) -> bool {
        let file = MetaFile {
            version: 1,
            sessions: flags.clone(),
        };
        let Some(dir) = self.path.parent() else {
            return false;
        };
        let bytes = match serde_json::to_vec_pretty(&file) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, "serializing session-meta.json");
                return false;
            }
        };
        if let Err(error) = std::fs::create_dir_all(dir) {
            tracing::warn!(%error, path = %dir.display(), "creating the session meta dir");
            return false;
        }
        let tmp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        let write = std::fs::File::create(&tmp)
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(&bytes).and_then(|()| file.sync_all())
            })
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        match write {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, path = %self.path.display(), "persisting session-meta.json");
                let _ = std::fs::remove_file(&tmp);
                false
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Which session each live `events_open` connection focuses (`Control::Focus`,
/// app spec §3.14). A connection without a focus is `None`; entries leave with
/// their stream ([`FocusRegistry::release`]).
#[derive(Default)]
pub struct FocusRegistry {
    next: AtomicU64,
    focused: Mutex<HashMap<u64, Option<String>>>,
}

impl FocusRegistry {
    /// Registers a new connection; it starts focused on nothing.
    #[must_use]
    pub fn acquire(&self) -> u64 {
        let token = self.next.fetch_add(1, Ordering::AcqRel) + 1;
        lock(&self.focused).insert(token, None);
        token
    }

    /// Sets what `token`'s connection focuses (`None` = no session); returns the
    /// previous focus of that connection.
    pub fn set(&self, token: u64, session: Option<&str>) -> Option<String> {
        lock(&self.focused)
            .insert(token, session.map(str::to_owned))
            .flatten()
    }

    /// Drops `token`'s connection (the stream ended); returns its last focus.
    pub fn release(&self, token: u64) -> Option<String> {
        lock(&self.focused).remove(&token).flatten()
    }

    /// Is any live connection focused on `session_id`? (Z-86's "unseen" test.)
    #[must_use]
    pub fn is_focused(&self, session_id: &str) -> bool {
        lock(&self.focused)
            .values()
            .any(|focus| focus.as_deref() == Some(session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(pinned: bool, unread: bool, title_locked: bool) -> SessionMetaFlags {
        SessionMetaFlags {
            pinned,
            unread,
            title_locked,
            draft_at: 0,
        }
    }

    #[test]
    fn flags_patch_and_clear() {
        let clear = SessionMetaFlags::default();
        assert!(clear.is_clear());
        let pinned = clear.patched(SessionMetaPatch {
            pinned: Some(true),
            ..SessionMetaPatch::default()
        });
        assert_eq!(pinned, flags(true, false, false));
        let unread_too = pinned.patched(SessionMetaPatch {
            unread: Some(true),
            ..SessionMetaPatch::default()
        });
        assert_eq!(unread_too, flags(true, true, false));
        assert!(unread_too
            .patched(SessionMetaPatch {
                pinned: Some(false),
                unread: Some(false),
                title_locked: Some(false)
            })
            .is_clear());
    }

    #[test]
    fn store_persists_and_reload_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = MetaStore::open(dir.path());
        assert_eq!(store.flags("s1"), SessionMetaFlags::default());
        assert!(store.patch(
            "s1",
            SessionMetaPatch {
                pinned: Some(true),
                ..SessionMetaPatch::default()
            }
        ));
        assert!(
            !store.patch(
                "s1",
                SessionMetaPatch {
                    pinned: Some(true),
                    ..SessionMetaPatch::default()
                }
            ),
            "no-op"
        );
        assert!(store.patch(
            "s2",
            SessionMetaPatch {
                unread: Some(true),
                ..SessionMetaPatch::default()
            }
        ));
        // A fresh store (a restart) reads the same flags.
        let reloaded = MetaStore::open(dir.path());
        assert_eq!(reloaded.flags("s1"), flags(true, false, false));
        assert_eq!(reloaded.flags("s2"), flags(false, true, false));
        assert_eq!(reloaded.flags("s3"), SessionMetaFlags::default());
        // Clearing every flag drops the record.
        assert!(reloaded.patch(
            "s1",
            SessionMetaPatch {
                pinned: Some(false),
                ..SessionMetaPatch::default()
            }
        ));
        assert_eq!(
            MetaStore::open(dir.path()).flags("s1"),
            SessionMetaFlags::default()
        );
    }

    #[test]
    fn a_missing_or_corrupt_file_starts_clear() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            MetaStore::open(dir.path()).flags("s1"),
            SessionMetaFlags::default()
        );
        std::fs::write(dir.path().join("session-meta.json"), "{not json").unwrap();
        assert_eq!(
            MetaStore::open(dir.path()).flags("s1"),
            SessionMetaFlags::default()
        );
    }

    #[test]
    fn focus_tracks_connections() {
        let registry = FocusRegistry::default();
        let a = registry.acquire();
        let b = registry.acquire();
        assert!(!registry.is_focused("s1"));
        assert_eq!(registry.set(a, Some("s1")), None);
        assert!(registry.is_focused("s1"));
        assert_eq!(registry.set(a, Some("s2")), Some("s1".to_owned()));
        assert!(!registry.is_focused("s1"));
        registry.set(b, Some("s1"));
        assert!(registry.is_focused("s1"));
        assert_eq!(registry.release(b), Some("s1".to_owned()));
        assert!(!registry.is_focused("s1"));
        assert_eq!(registry.set(a, None), Some("s2".to_owned()));
        assert!(!registry.is_focused("s2"));
    }

    #[test]
    fn drafts_round_trip_and_stamp_the_card() {
        let dir = tempfile::tempdir().unwrap();
        let store = MetaStore::open(dir.path());
        let doc = serde_json::json!({"v": 1, "text": "half a thought"});
        store.set_draft("s-1", Some(&doc), 42).unwrap();
        assert_eq!(store.draft("s-1"), Some(doc));
        assert_eq!(store.flags("s-1").draft_at, 42);
        // Survives a reload of the meta file.
        assert_eq!(MetaStore::open(dir.path()).flags("s-1").draft_at, 42);
        store.set_draft("s-1", None, 50).unwrap();
        assert_eq!(store.draft("s-1"), None);
        assert!(store.flags("s-1").is_clear());
        // A path-like id never touches the disk.
        assert!(store
            .set_draft("../x", Some(&serde_json::json!({})), 1)
            .is_err());
        assert_eq!(store.draft("../x"), None);
    }
}
