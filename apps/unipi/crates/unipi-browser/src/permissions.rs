//! Per-origin site-permission grants (browser-control.md §10.3):
//! `~/.oino/browser/site-permissions.json` = `{origin: {perm: allow|block}}`, atomic writes.
//! Only the user changes them; the agent has no tool for it.

use std::collections::HashMap;
use std::path::Path;

/// Grants by origin.
pub(crate) type Grants = HashMap<String, HashMap<String, String>>;

/// Reads the grants file (missing / invalid ⇒ empty).
pub(crate) fn load(path: &Path) -> Grants {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Sets (`Some`) or resets (`None` = ask/default) one permission of `origin`.
///
/// # Errors
///
/// I/O.
pub(crate) fn set(
    path: &Path,
    origin: &str,
    perm: &str,
    setting: Option<&str>,
) -> std::io::Result<Grants> {
    let mut g = load(path);
    let entry = g.entry(origin.to_owned()).or_default();
    match setting {
        Some(s) if s == "allow" || s == "block" => {
            entry.insert(perm.to_owned(), s.to_owned());
        }
        _ => {
            entry.remove(perm);
        }
    }
    if entry.is_empty() {
        g.remove(origin);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&g).unwrap_or_default())?;
    std::fs::rename(&tmp, path)?;
    tracing::info!(
        "[browser-permission] decision={} origin={origin} perm={perm} source=user",
        setting.unwrap_or("ask")
    );
    Ok(g)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("site-permissions.json");
        assert!(load(&p).is_empty());
        set(&p, "https://meet.example", "camera", Some("allow")).unwrap();
        set(&p, "https://meet.example", "microphone", Some("block")).unwrap();
        let g = load(&p);
        assert_eq!(g["https://meet.example"]["camera"], "allow");
        set(&p, "https://meet.example", "camera", None).unwrap();
        set(&p, "https://meet.example", "microphone", None).unwrap();
        assert!(load(&p).is_empty());
    }
}
