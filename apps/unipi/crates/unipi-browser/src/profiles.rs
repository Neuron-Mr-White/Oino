//! Profile directories (browser-control.md §4.3): `~/.oino/browser/profiles/<name>/`, Snap
//! Chromium redirect, `oino-profile.json` metadata.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::valid_profile_name;
use crate::hub::Paths;

const META: &str = "oino-profile.json";

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `--user-data-dir` for `name` (Snap browsers write only below `~/snap/<snap>/common`).
pub(crate) fn user_data_dir(paths: &Paths, name: &str, executable: &Path) -> PathBuf {
    let exe = executable.to_string_lossy();
    if let Some(snap) = exe.strip_prefix("/snap/bin/")
        && let Some(home) = dirs::home_dir()
    {
        return home
            .join("snap")
            .join(snap)
            .join("common/oino-browser-profiles")
            .join(name);
    }
    paths.profile(name)
}

/// Creates or refreshes `oino-profile.json`.
pub(crate) fn touch_meta(dir: &Path) {
    let file = dir.join(META);
    let mut meta: Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({"created": now(), "cookieDomains": []}));
    meta["lastUsed"] = json!(now());
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&file, meta.to_string());
}

/// Domains with imported cookies.
pub(crate) fn cookie_domains(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(META))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v["cookieDomains"].as_array().map(|a| {
                a.iter()
                    .filter_map(|d| d.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Creates an empty profile.
///
/// # Errors
///
/// Invalid name, exists, or I/O.
pub(crate) fn create(paths: &Paths, name: &str) -> Result<(), String> {
    if !valid_profile_name(name) {
        return Err(format!(
            "invalid profile name {name:?} (use a-z, 0-9, - and _)"
        ));
    }
    let dir = paths.profile(name);
    if dir.exists() {
        return Err(format!("profile {name} exists"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    touch_meta(&dir);
    Ok(())
}

/// Removes a profile (never `default`, never while running — the caller checks).
///
/// # Errors
///
/// `default`, invalid name, or I/O.
pub(crate) fn remove(paths: &Paths, name: &str) -> Result<(), String> {
    if name == "default" || !valid_profile_name(name) {
        return Err(format!("profile {name} cannot be removed"));
    }
    std::fs::remove_dir_all(paths.profile(name)).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn profiles() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().into(),
        };
        create(&paths, "work").unwrap();
        assert!(create(&paths, "work").is_err());
        assert!(create(&paths, "Bad").is_err());
        assert_eq!(cookie_domains(&paths.profile("work")), [] as [String; 0]);
        assert!(remove(&paths, "default").is_err());
        remove(&paths, "work").unwrap();
        assert_eq!(
            user_data_dir(&paths, "x", Path::new("/usr/bin/chromium")),
            paths.profile("x")
        );
    }
}
