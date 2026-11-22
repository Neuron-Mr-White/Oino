//! Profile directories (`browser-control.md` §4.3): `<root>/profiles/<name>/`,
//! the Snap redirect, `oino-profile.json` metadata.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::valid_profile_name;

const META: &str = "oino-profile.json";

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `<root>/profiles/<name>`.
#[must_use]
pub(crate) fn dir(root: &Path, name: &str) -> PathBuf {
    root.join("profiles").join(name)
}

/// `--user-data-dir` for `name` (Snap browsers write only below `~/snap/<snap>/common`).
pub(crate) fn user_data_dir(root: &Path, name: &str, executable: &Path) -> PathBuf {
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
    dir(root, name)
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

/// [`cookie_domains`] with a plain directory (the store seam's delegate).
pub(crate) fn cookie_domains_of(dir: &Path) -> Vec<String> {
    cookie_domains(dir)
}

/// Every profile in a profiles dir itself (`<root>/profiles`), sorted.
pub(crate) fn list_of(profiles_dir: &Path) -> Vec<crate::profile_store::StoredProfile> {
    let mut out: Vec<crate::profile_store::StoredProfile> = std::fs::read_dir(profiles_dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_profile_name(n))
                .map(|name| crate::profile_store::StoredProfile {
                    cookie_domains: cookie_domains(&profiles_dir.join(&name)),
                    dir: profiles_dir.join(&name),
                    name,
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Every profile under `root` (sorted, `default` last when only implied).
#[allow(
    dead_code,
    reason = "kept as the root-based view the layouts tests use"
)]
pub(crate) fn list(root: &Path) -> Vec<crate::profile_store::StoredProfile> {
    list_of(&dir(root, ""))
}

/// Creates an empty profile inside a profiles dir itself.
///
/// # Errors
///
/// Invalid name, exists, or I/O.
pub(crate) fn create_in(profiles_dir: &Path, name: &str) -> Result<(), String> {
    if !valid_profile_name(name) {
        return Err(format!(
            "invalid profile name {name:?} (use a-z, 0-9, - and _)"
        ));
    }
    let d = profiles_dir.join(name);
    if d.exists() {
        return Err(format!("profile {name} exists"));
    }
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    touch_meta(&d);
    Ok(())
}

/// Creates an empty profile (the old `Paths`-based entry point).
///
/// # Errors
///
/// Invalid name, exists, or I/O.
#[allow(dead_code, reason = "Paths-based shim kept for the frontend seam")]
pub(crate) fn create(paths: &crate::hub::Paths, name: &str) -> Result<(), String> {
    create_in(&dir(&paths.root, ""), name)
}

/// Removes a profile (never `default`, never while running — the caller checks).
///
/// # Errors
///
/// `default`, invalid name, or I/O.
pub(crate) fn remove(root: &Path, name: &str) -> Result<(), String> {
    if name == "default" || !valid_profile_name(name) {
        return Err(format!("profile {name} cannot be removed"));
    }
    std::fs::remove_dir_all(dir(root, name)).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn profiles() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        create_in(&dir(root, ""), "work").unwrap();
        assert!(create_in(&dir(root, ""), "work").is_err());
        assert!(create_in(&dir(root, ""), "Bad").is_err());
        assert_eq!(cookie_domains(&dir(root, "work")), [] as [String; 0]);
        assert!(remove(root, "default").is_err());
        remove(root, "work").unwrap();
        assert_eq!(
            user_data_dir(root, "x", Path::new("/usr/bin/chromium")),
            dir(root, "x")
        );
        let list = list(root);
        assert_eq!(list, Vec::new());
        create_in(&dir(root, ""), "b").unwrap();
        create_in(&dir(root, ""), "a").unwrap();
        let names: Vec<String> = crate::profiles::list(root)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["a", "b"]);
    }
}
