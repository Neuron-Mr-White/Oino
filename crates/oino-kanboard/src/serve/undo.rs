//! UNI-57 undo tokens: opaque random tokens backed by daemon-held snapshots.
//! One move mints one token; undo consumes it. Nothing here is reachable from
//! the CLI — undo is a user surface (the web API always acts as `user`) and
//! the store lives in daemon memory, so tokens die with the daemon.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::model::Task;

/// How long an undo token stays valid after its move.
pub(crate) const DEFAULT_TTL: Duration = Duration::from_secs(30);

const TOKEN_BYTES: usize = 16;

pub(crate) struct UndoEntry {
    pub(crate) project: String,
    pub(crate) id: String,
    pub(crate) before: Task,
    pub(crate) after: Task,
    pub(crate) created: Instant,
}

#[derive(Debug)]
pub(crate) enum UndoRejection {
    /// No such token (unknown, already used, or the daemon restarted).
    Unknown,
    /// Older than the TTL.
    Expired,
    /// Offered for a different task or project (the token survives this).
    WrongTask,
}

pub(crate) struct UndoStore {
    entries: Mutex<HashMap<String, UndoEntry>>,
    ttl: Duration,
}

impl Default for UndoStore {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoStore {
    pub(crate) fn new() -> Self {
        // Tests shrink the TTL; production uses the fixed 30 seconds.
        let ttl = std::env::var("OINO_KANBOARD_UNDO_TTL_SECS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map_or(DEFAULT_TTL, Duration::from_secs);
        UndoStore {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Store one move's snapshots under a fresh opaque token.
    pub(crate) fn insert(&self, project: &str, id: &str, before: Task, after: Task) -> String {
        let token = new_token();
        if let Ok(mut entries) = self.entries.lock() {
            // Opportunistic prune: drop expired tokens while we hold the lock.
            entries.retain(|_, entry| entry.created.elapsed() < self.ttl);
            entries.insert(
                token.clone(),
                UndoEntry {
                    project: project.to_string(),
                    id: id.to_string(),
                    before,
                    after,
                    created: Instant::now(),
                },
            );
        }
        token
    }

    /// Atomically consume a token for one (project, id): expiry and unknown
    /// tokens are removed and refused; a token offered for a *different*
    /// task/project is refused but left intact — scoping must not burn the
    /// move's only undo. Everything happens under one lock hold.
    pub(crate) fn take(
        &self,
        token: &str,
        project: &str,
        id: &str,
    ) -> Result<UndoEntry, UndoRejection> {
        let mut entries = self.entries.lock().map_err(|_| UndoRejection::Unknown)?;
        {
            let entry = entries.get(token).ok_or(UndoRejection::Unknown)?;
            if entry.project != project || entry.id != id {
                return Err(UndoRejection::WrongTask);
            }
        }
        let entry = entries.remove(token).ok_or(UndoRejection::Unknown)?;
        if entry.created.elapsed() >= self.ttl {
            return Err(UndoRejection::Expired);
        }
        Ok(entry)
    }
}

static FALLBACK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 32 hex chars from OS randomness — opaque, unguessable, no new dependency.
fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    if getrandom::fill(&mut bytes).is_err() {
        // No OS randomness: fall back to time + counter entropy. Tokens stay
        // single-use and short-lived, so uniqueness is what matters here.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let count = FALLBACK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let seed = nanos ^ (u128::from(count) << 64);
        bytes = seed.to_le_bytes();
    }
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
