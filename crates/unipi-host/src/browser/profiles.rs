//! Browser profiles (UNI-292): one persistent `default` profile plus named
//! ones, each a directory `<host dir>/browser-profiles/<name>` that is the
//! browser's `--user-data-dir` (cookies, logins, the CloakBrowser
//! fingerprint seed `.cloakbrowser-seed`) and holds our own metadata file
//! [`META_FILE`].
//!
//! A snap browser (the Chrome fallback only) can't write there; it gets
//! `~/snap/<snap>/common/unipi-browser-profile` for `default` (the path
//! UNI-287 used, so existing logins stay) and
//! `~/snap/<snap>/common/unipi-browser-profiles/<name>` for the others.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The profile every app op uses unless told otherwise.
pub const DEFAULT: &str = "default";
/// Longest profile name.
pub const MAX_NAME: usize = 40;
/// Our metadata inside a profile dir.
pub const META_FILE: &str = "unipi-profile.json";
/// `<host dir>/browser-profiles`.
pub const DIR: &str = "browser-profiles";
/// The single profile dir before UNI-292 (migrated into `default`).
pub const LEGACY_DIR: &str = "browser-profile";

/// `[a-z0-9_-]{1,40}`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// [`valid_name`] with the error people see.
pub fn check_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        valid_name(name),
        "bad profile name {name:?}: use 1–{MAX_NAME} characters of a–z, 0–9, - and _"
    );
    Ok(())
}

/// `<host dir>/browser-profiles`.
pub fn root(host_dir: &Path) -> PathBuf {
    host_dir.join(DIR)
}

/// `<host dir>/browser-profiles/<name>` (name already validated).
pub fn dir(host_dir: &Path, name: &str) -> PathBuf {
    root(host_dir).join(name)
}

/// A short stable key for a host dir (snap profiles live outside it).
pub fn host_key(host_dir: &Path) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(host_dir.to_string_lossy().as_bytes());
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// The snap browser's writable profile for `name` of `host_dir`. Only the
/// standard host dir (`~/.unipi/app-host`) and its `default` keep the
/// UNI-287 path, so existing logins stay; every other host dir / profile
/// gets its own `unipi-browser-profiles/<host key>/<name>` (a test host must
/// never share — or kill — the real host's browser).
pub fn snap_dir(home: &Path, snap: &str, host_dir: &Path, name: &str) -> PathBuf {
    let common = home.join("snap").join(snap).join("common");
    if name == DEFAULT && host_dir == home.join(".unipi").join("app-host") {
        common.join(super::SNAP_PROFILE_NAME)
    } else {
        common
            .join("unipi-browser-profiles")
            .join(host_key(host_dir))
            .join(name)
    }
}

/// The `--user-data-dir` for the Chrome-fallback binary `bin` (snap-aware).
pub fn chrome_dir(bin: &Path, host_dir: &Path, name: &str, home: Option<&Path>) -> PathBuf {
    match (super::snap_name_of(bin), home) {
        (Some(snap), Some(home)) => snap_dir(home, &snap, host_dir, name),
        _ => dir(host_dir, name),
    }
}

/// What we remember about a profile (`unipi-profile.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// Unix ms.
    #[serde(default)]
    pub created: u64,
    /// Unix ms of the last start / reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
    /// The agent session that used it last (`browser start --session`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session: Option<String>,
    /// Sites whose cookies were imported into it (UNI-294: `unipi-host
    /// browser cookies`).
    #[serde(default)]
    pub cookie_domains: Vec<CookieDomain>,
}

/// One domain cookies were imported for (`unipi-profile.json`'s
/// `cookie_domains`) — never the cookie values themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieDomain {
    pub domain: String,
    /// Unix ms of the import.
    pub imported_at: u64,
}

/// Records (or bumps) `domain` as imported at `now_ms`; moves it to the end.
pub fn record_cookie_domain(
    host_dir: &Path,
    name: &str,
    domain: &str,
    now_ms: u64,
) -> anyhow::Result<()> {
    let Some(mut meta) = load(host_dir, name)? else {
        return Ok(());
    };
    meta.cookie_domains.retain(|d| d.domain != domain);
    meta.cookie_domains.push(CookieDomain {
        domain: domain.to_string(),
        imported_at: now_ms,
    });
    save(host_dir, name, &meta)
}

pub fn meta_path(host_dir: &Path, name: &str) -> PathBuf {
    dir(host_dir, name).join(META_FILE)
}

/// The profile's metadata; `None` when the profile doesn't exist.
pub fn load(host_dir: &Path, name: &str) -> anyhow::Result<Option<Meta>> {
    let d = dir(host_dir, name);
    if !d.is_dir() {
        return Ok(None);
    }
    Ok(Some(
        crate::data_dir::read_json::<Meta>(&d.join(META_FILE))
            .ok()
            .flatten()
            .unwrap_or_else(|| Meta {
                created: created_of(&d),
                ..Meta::default()
            }),
    ))
}

pub fn save(host_dir: &Path, name: &str, meta: &Meta) -> anyhow::Result<()> {
    crate::data_dir::write_json(&meta_path(host_dir, name), meta)
}

fn created_of(d: &Path) -> u64 {
    d.metadata()
        .and_then(|m| m.created().or_else(|_| m.modified()))
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Creates a profile (the dir + metadata). Errors when it exists.
pub fn create(host_dir: &Path, name: &str, now_ms: u64) -> anyhow::Result<Meta> {
    check_name(name)?;
    let d = dir(host_dir, name);
    anyhow::ensure!(!d.exists(), "profile {name:?} already exists");
    std::fs::create_dir_all(&d)?;
    let meta = Meta {
        created: now_ms,
        ..Meta::default()
    };
    save(host_dir, name, &meta)?;
    Ok(meta)
}

/// The profile dir, creating `default` (only) when missing.
pub fn ensure(host_dir: &Path, name: &str, now_ms: u64) -> anyhow::Result<PathBuf> {
    check_name(name)?;
    let d = dir(host_dir, name);
    if d.is_dir() {
        return Ok(d);
    }
    anyhow::ensure!(
        name == DEFAULT,
        "no browser profile {name:?} (create it: unipi-host browser profile create {name})"
    );
    create(host_dir, name, now_ms)?;
    Ok(d)
}

/// Stamps `last_used` (and `last_session` when given).
pub fn touch(
    host_dir: &Path,
    name: &str,
    now_ms: u64,
    session: Option<&str>,
) -> anyhow::Result<()> {
    let Some(mut meta) = load(host_dir, name)? else {
        return Ok(());
    };
    meta.last_used = Some(now_ms);
    if let Some(session) = session.map(str::trim).filter(|s| !s.is_empty()) {
        meta.last_session = Some(session.chars().take(200).collect());
    }
    save(host_dir, name, &meta)
}

/// Every profile name (sorted, `default` first), only valid names.
pub fn list(host_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root(host_dir))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| valid_name(n))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by(|a, b| (a != DEFAULT, a).cmp(&(b != DEFAULT, b)));
    names
}

/// Deletes a profile (never `default`). The caller checks it isn't running.
pub fn remove(host_dir: &Path, name: &str, home: Option<&Path>) -> anyhow::Result<()> {
    check_name(name)?;
    anyhow::ensure!(name != DEFAULT, "the default profile can't be removed");
    let d = dir(host_dir, name);
    anyhow::ensure!(d.is_dir(), "no browser profile {name:?}");
    std::fs::remove_dir_all(&d)?;
    // Its snap twins (the Chrome fallback on a snap browser), if any.
    if let Some(home) = home {
        if let Ok(snaps) = std::fs::read_dir(home.join("snap")) {
            for snap in snaps.flatten() {
                let twin = snap
                    .path()
                    .join("common")
                    .join("unipi-browser-profiles")
                    .join(host_key(host_dir))
                    .join(name);
                if twin.is_dir() {
                    let _ = std::fs::remove_dir_all(twin);
                }
            }
        }
    }
    Ok(())
}

/// Moves the pre-UNI-292 `<host dir>/browser-profile` to `browser-profiles/default`
/// (once; never over an existing `default`). Returns whether it moved.
pub fn migrate(host_dir: &Path, now_ms: u64) -> anyhow::Result<bool> {
    let legacy = host_dir.join(LEGACY_DIR);
    let target = dir(host_dir, DEFAULT);
    if !legacy.is_dir() {
        return Ok(false);
    }
    if target.exists() {
        tracing::warn!(op = "browser", legacy = %legacy.display(), "both the old browser-profile and browser-profiles/default exist; leaving the old one alone");
        return Ok(false);
    }
    std::fs::create_dir_all(root(host_dir))?;
    std::fs::rename(&legacy, &target)?;
    if !target.join(META_FILE).exists() {
        save(
            host_dir,
            DEFAULT,
            &Meta {
                created: created_of(&target).min(now_ms).max(1),
                ..Meta::default()
            },
        )?;
    }
    tracing::info!(
        op = "browser",
        "moved the old browser-profile to browser-profiles/default"
    );
    Ok(true)
}

/// One running browser, for [`pick_eviction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub name: String,
    /// A viewer or a CDP client (agent) uses it.
    pub busy: bool,
    /// Last activity, any monotonic unit (larger = more recent).
    pub last_active: u64,
}

/// Starting `starting` with `running` browsers already up and a cap of
/// `max` (0 acts as 1): `Ok(vec![])` = room, `Ok(names)` = stop these
/// least-recently-used idle ones first (more than one when the cap was
/// lowered), `Err` = not enough idle ones — the others are in use.
pub fn pick_eviction(running: &[Slot], starting: &str, max: usize) -> Result<Vec<String>, String> {
    let others: Vec<&Slot> = running.iter().filter(|s| s.name != starting).collect();
    let need = (others.len() + 1).saturating_sub(max.max(1));
    if need == 0 {
        return Ok(Vec::new());
    }
    let mut idle: Vec<&&Slot> = others.iter().filter(|s| !s.busy).collect();
    idle.sort_by_key(|s| (s.last_active, s.name.clone()));
    if idle.len() < need {
        return Err(format!(
            "{} browser profiles are already running ({}) and {} in use; stop one first (unipi-host browser stop --profile <name>) or raise browser.max_running",
            others.len(),
            others.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "),
            if idle.is_empty() { "all are".to_string() } else { format!("{} of them are", others.len() - idle.len()) },
        ));
    }
    Ok(idle
        .into_iter()
        .take(need)
        .map(|s| s.name.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        for ok in ["default", "a", "work-2", "x_y", &"a".repeat(40)] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            "Work",
            "a b",
            "a/b",
            "..",
            ".x",
            "é",
            &"a".repeat(41),
            "a.b",
        ] {
            assert!(!valid_name(bad), "{bad}");
            assert!(check_name(bad).is_err());
        }
    }

    #[test]
    fn layout() {
        let host = Path::new("/h");
        assert_eq!(
            dir(host, "default"),
            Path::new("/h/browser-profiles/default")
        );
        assert_eq!(dir(host, "test1"), Path::new("/h/browser-profiles/test1"));
        let home = Path::new("/home/me");
        let real = Path::new("/home/me/.unipi/app-host");
        assert_eq!(
            chrome_dir(
                Path::new("/usr/bin/google-chrome"),
                host,
                "test1",
                Some(home)
            ),
            Path::new("/h/browser-profiles/test1")
        );
        // The standard host dir's default keeps the UNI-287 snap path …
        assert_eq!(
            chrome_dir(Path::new("/snap/bin/chromium"), real, "default", Some(home)),
            Path::new("/home/me/snap/chromium/common/unipi-browser-profile")
        );
        // … everything else is scoped by host dir, so hosts never share one.
        let key = host_key(host);
        assert_eq!(key.len(), 8);
        assert_ne!(key, host_key(real));
        assert_eq!(
            chrome_dir(Path::new("/snap/bin/chromium"), host, "default", Some(home)),
            Path::new("/home/me/snap/chromium/common/unipi-browser-profiles")
                .join(&key)
                .join("default")
        );
        assert_eq!(
            chrome_dir(Path::new("/snap/bin/chromium"), real, "test1", Some(home)),
            Path::new("/home/me/snap/chromium/common/unipi-browser-profiles")
                .join(host_key(real))
                .join("test1")
        );
        assert_eq!(
            chrome_dir(Path::new("/snap/bin/chromium"), host, "test1", None),
            Path::new("/h/browser-profiles/test1")
        );
    }

    #[test]
    fn create_list_touch_remove() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        assert!(list(h).is_empty());
        assert!(ensure(h, "nope", 1)
            .unwrap_err()
            .to_string()
            .contains("profile create nope"));
        assert_eq!(ensure(h, DEFAULT, 5).unwrap(), dir(h, DEFAULT));
        create(h, "zeta", 10).unwrap();
        create(h, "alpha", 11).unwrap();
        assert!(create(h, "alpha", 12).is_err(), "exists");
        assert!(create(h, "Bad", 12).is_err());
        std::fs::create_dir_all(root(h).join("Not Valid")).unwrap();
        assert_eq!(list(h), ["default", "alpha", "zeta"]);
        touch(h, "alpha", 99, Some("sess-1")).unwrap();
        let meta = load(h, "alpha").unwrap().unwrap();
        assert_eq!(
            (meta.created, meta.last_used, meta.last_session.as_deref()),
            (11, Some(99), Some("sess-1"))
        );
        assert!(meta.cookie_domains.is_empty());
        // The cookie field round-trips (the cookie import writes it).
        record_cookie_domain(h, "alpha", "x.com", 100).unwrap();
        let domains = load(h, "alpha").unwrap().unwrap().cookie_domains;
        assert_eq!(
            (domains[0].domain.as_str(), domains[0].imported_at),
            ("x.com", 100)
        );
        // Importing again for the same domain bumps it instead of duplicating.
        record_cookie_domain(h, "alpha", "x.com", 200).unwrap();
        let domains = load(h, "alpha").unwrap().unwrap().cookie_domains;
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].imported_at, 200);
        assert!(remove(h, DEFAULT, None)
            .unwrap_err()
            .to_string()
            .contains("default"));
        remove(h, "alpha", None).unwrap();
        assert!(load(h, "alpha").unwrap().is_none());
        assert!(remove(h, "alpha", None).is_err());
    }

    #[test]
    fn migrates_the_old_single_profile_once() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        assert!(!migrate(h, 1).unwrap(), "nothing to move");
        std::fs::create_dir_all(h.join(LEGACY_DIR).join("Default")).unwrap();
        std::fs::write(h.join(LEGACY_DIR).join("Default").join("Cookies"), "c").unwrap();
        assert!(migrate(h, 1_000).unwrap());
        assert!(!h.join(LEGACY_DIR).exists());
        assert_eq!(
            std::fs::read_to_string(dir(h, DEFAULT).join("Default/Cookies")).unwrap(),
            "c"
        );
        assert!(load(h, DEFAULT).unwrap().unwrap().created > 0);
        // An old dir showing up again never replaces the new default.
        std::fs::create_dir_all(h.join(LEGACY_DIR)).unwrap();
        assert!(!migrate(h, 2_000).unwrap());
        assert!(h.join(LEGACY_DIR).exists() && dir(h, DEFAULT).join("Default/Cookies").exists());
    }

    #[test]
    fn cap_stops_the_least_recently_used_idle_one() {
        let s = |name: &str, busy, last_active| Slot {
            name: name.into(),
            busy,
            last_active,
        };
        let none: Vec<String> = Vec::new();
        // Room left.
        assert_eq!(pick_eviction(&[s("a", true, 1)], "b", 3), Ok(none.clone()));
        // Already running: never counts against itself.
        assert_eq!(
            pick_eviction(
                &[s("a", false, 1), s("b", false, 2), s("c", false, 3)],
                "b",
                3
            ),
            Ok(none.clone())
        );
        // Full: the oldest idle one goes, busy ones are kept.
        let full = [s("a", true, 1), s("b", false, 5), s("c", false, 3)];
        assert_eq!(pick_eviction(&full, "d", 3), Ok(vec!["c".to_string()]));
        // Cap 1.
        assert_eq!(
            pick_eviction(&[s("default", false, 9)], "x", 1),
            Ok(vec!["default".to_string()])
        );
        assert_eq!(
            pick_eviction(&[s("default", false, 9)], "x", 0),
            Ok(vec!["default".to_string()]),
            "0 acts as 1"
        );
        // The cap was lowered under what runs: as many as needed, oldest first.
        assert_eq!(
            pick_eviction(
                &[s("a", false, 7), s("b", false, 2), s("c", false, 5)],
                "d",
                1
            ),
            Ok(vec!["b".to_string(), "c".to_string(), "a".to_string()])
        );
        // Not enough idle ones: refused, nothing stopped.
        let e = pick_eviction(&[s("a", true, 1), s("b", false, 2)], "c", 1).unwrap_err();
        assert!(e.contains("1 of them are in use"), "{e}");
        // All busy: refused with a clear error.
        let e = pick_eviction(&[s("a", true, 1), s("b", true, 2)], "c", 2).unwrap_err();
        assert!(
            e.contains("all are in use") && e.contains("(a, b)") && e.contains("max_running"),
            "{e}"
        );
    }
}
