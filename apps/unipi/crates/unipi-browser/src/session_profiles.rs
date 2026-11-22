//! Per-session profile choices (`browser_profile use`), persisted under the
//! browser root so a host restart or a reloaded session keeps the chat on its
//! chosen profile instead of silently falling back to the default.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const FILE: &str = "session-profiles.json";

type Map = HashMap<String, String>;

fn path(root: &Path) -> PathBuf {
    root.join(FILE)
}

fn read(root: &Path) -> Map {
    std::fs::read_to_string(path(root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Atomic write (temp file + rename): a crash leaves the old choice behind,
/// never a torn file.
fn write(root: &Path, map: &Map) {
    if std::fs::create_dir_all(root).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(map) else {
        return;
    };
    let tmp = root.join(format!(".{FILE}.tmp-{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path(root));
    }
}

/// The profile persisted for `session` (`None`: no choice yet).
#[must_use]
pub(crate) fn get(root: &Path, session: &str) -> Option<String> {
    read(root).get(session).cloned()
}

/// Remembers `profile` for `session` (overwrites a previous choice).
pub(crate) fn set(root: &Path, session: &str, profile: &str) {
    let mut map = read(root);
    map.insert(session.to_owned(), profile.to_owned());
    write(root, &map);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_survives_a_fresh_read() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(get(dir.path(), "s1"), None);
        set(dir.path(), "s1", "shopping");
        set(dir.path(), "s2", "work-2");
        assert_eq!(get(dir.path(), "s1").as_deref(), Some("shopping"));
        // A fresh read of the same file (what a restarted service does).
        let again = read(dir.path());
        assert_eq!(again.get("s2").map(String::as_str), Some("work-2"));
        // Overwriting keeps one entry per session.
        set(dir.path(), "s1", "ace-jobs");
        let map = read(dir.path());
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("s1").map(String::as_str), Some("ace-jobs"));
    }
}
