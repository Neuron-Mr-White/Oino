//! State paths over the host preopens (unipi `src/workspace/paths.ts`, unipi-port §2.3).
//!
//! | unipi | here |
//! |---|---|
//! | `~/.unipi/global/<module>/…` | [`global`] (`/ext-global/…`) |
//! | `~/.unipi/workspace/<id>/state/<module>/…` | [`workspace`] (`/ext-workspace/<wsid>/…`) |
//! | `~/.unipi/workspace/<id>/sessions/<sid>/<module>/` | [`session`] (`/ext-session/<sid>/…`) |
//! | `~/.unipi/cache`, `logs`, `tool-results` | [`cache`], [`logs`], [`tmp`] |
//!
//! Guest paths are meaningless to the agent's `read` tool; [`host_path`] maps them to host
//! paths through `fs.preopens()` when a path must be shown to the model.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// `/ext-global/<rel>`.
#[must_use]
pub fn global(rel: &str) -> PathBuf {
    oino_extension::fs::ext_global().join(rel)
}

/// `/ext-workspace/<wsid>/<rel>` (current workspace).
#[must_use]
pub fn workspace(rel: &str) -> PathBuf {
    oino_extension::fs::ext_workspace().join(rel)
}

/// `/ext-session/<sid>/<rel>`.
#[must_use]
pub fn session(session_id: &str, rel: &str) -> PathBuf {
    oino_extension::fs::ext_session(session_id).join(rel)
}

/// `/tmp/<rel>`.
#[must_use]
pub fn tmp(rel: &str) -> PathBuf {
    Path::new("/tmp").join(rel)
}

/// `/cache/<rel>`.
#[must_use]
pub fn cache(rel: &str) -> PathBuf {
    Path::new("/cache").join(rel)
}

/// `/logs/<rel>`.
#[must_use]
pub fn logs(rel: &str) -> PathBuf {
    Path::new("/logs").join(rel)
}

/// Maps a guest path to its host path given `(guest_root, host_root)` pairs (longest root
/// wins). `None` when no preopen contains it.
#[must_use]
pub fn map_to_host(roots: &[(String, String)], guest: &Path) -> Option<PathBuf> {
    roots
        .iter()
        .filter_map(|(g, h)| {
            guest
                .strip_prefix(g)
                .ok()
                .map(|rest| (g.len(), Path::new(h).join(rest)))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, p)| p)
}

/// Host path of a guest path (through `fs.preopens()`).
#[must_use]
pub fn host_path(guest: &Path) -> Option<PathBuf> {
    let roots: Vec<(String, String)> = oino_extension::fs::preopens()
        .into_iter()
        .map(|p| (p.guest_path, p.host_path))
        .collect();
    map_to_host(&roots, guest)
}

/// unipi `sanitize`: non `[A-Za-z0-9_-]` → `_`, runs collapsed.
#[must_use]
pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            c
        } else {
            '_'
        };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    out
}

/// Creates the parent directory of `path`.
pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => std::fs::create_dir_all(p),
        _ => Ok(()),
    }
}

/// Reads a file as UTF-8 (`None` on any error; unipi `tryRead`).
#[must_use]
pub fn try_read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Deletes a file, ignoring errors (unipi `tryDelete`).
pub fn try_delete(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Reads JSON (`None` when missing or malformed; unipi `readJson`).
#[must_use]
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Writes pretty JSON atomically (host `atomic-write`), creating parents.
pub fn write_json(path: &Path, value: &impl Serialize) -> oino_extension::Result<()> {
    ensure_parent(path)
        .map_err(|e| oino_extension::Error::msg(format!("{}: {e}", path.display())))?;
    oino_extension::fs::write_json(&path.to_string_lossy(), value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_like_unipi() {
        assert_eq!(sanitize("a b/c..d"), "a_b_c_d");
        assert_eq!(sanitize("ok-name_1"), "ok-name_1");
        assert_eq!(sanitize("x  y"), "x_y");
    }

    #[test]
    fn maps_guest_to_host() {
        let roots = vec![
            (
                "/tmp".to_owned(),
                "/home/u/.oino/tmp/ext/unipi.core".to_owned(),
            ),
            (
                "/ext-global".to_owned(),
                "/home/u/.oino/state/unipi.core/global".to_owned(),
            ),
            ("/ext-global/sub".to_owned(), "/elsewhere".to_owned()),
        ];
        assert_eq!(
            map_to_host(&roots, Path::new("/tmp/tool-results/a.txt")),
            Some(PathBuf::from(
                "/home/u/.oino/tmp/ext/unipi.core/tool-results/a.txt"
            ))
        );
        assert_eq!(
            map_to_host(&roots, Path::new("/ext-global/sub/x")),
            Some(PathBuf::from("/elsewhere/x"))
        );
        assert_eq!(map_to_host(&roots, Path::new("/project/x")), None);
        assert_eq!(
            map_to_host(&roots, Path::new("/tmpfoo")),
            None,
            "component-wise prefix"
        );
    }

    #[test]
    fn json_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("unipi-kit-paths-{}", std::process::id()));
        let file = dir.join("a/b.json");
        assert!(ensure_parent(&file).is_ok());
        assert!(std::fs::write(&file, br#"{"k": 1}"#).is_ok());
        let v: Option<serde_json::Value> = read_json(&file);
        assert_eq!(v, Some(serde_json::json!({"k": 1})));
        assert!(read_json::<serde_json::Value>(&dir.join("missing.json")).is_none());
        try_delete(&file);
        assert!(try_read(&file).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
