//! Which folders the phone may browse/edit besides `$HOME` (UNI-220).
//!
//! The Files screen, `blob_get{path}` (file links / attachments) and every
//! `fs_*` op accept a path when its canonical form sits under one of:
//! - `$HOME`;
//! - a live pi session's cwd (see `fs::under_session_cwd`);
//! - an **extra safe root**: the defaults [`DEFAULT_SAFE_ROOTS`] that exist
//!   on this PC, plus the owner's `unipi-host fs allow <path>` entries,
//!   minus `unipi-host fs deny <path>` carve-outs.
//!
//! Persisted in `config.json` under `"fs": {"allow": [..], "deny": [..]}`
//! (read on every request, no restart needed). Roots are canonicalized
//! (a root that is a symlink is judged by its target) and a root that is,
//! sits inside, or contains a [forbidden](forbidden_reason) system folder
//! is never used — even if hand-edited into the config.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

/// Extra roots on by default (only those that exist are used).
/// `$USER` is replaced with the host user's name.
pub const DEFAULT_SAFE_ROOTS: &[&str] = &[
    "/tmp",
    "/var/tmp",
    "/mnt",
    "/media",
    "/run/media/$USER",
    "/srv",
    "/opt",
];

/// System folders that may never be (or contain, or sit under) an extra root.
const FORBIDDEN: &[&str] = &[
    "/etc",
    "/usr",
    "/proc",
    "/sys",
    "/dev",
    "/boot",
    "/root",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/var",
    "/run",
    "/snap",
    "/lost+found",
];

/// Exceptions inside [`FORBIDDEN`] folders (exact roots and what's under them).
fn exceptions(user: &str) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/var/tmp"),
        PathBuf::from(format!("/run/media/{user}")),
    ]
}

/// The `fs` block of `config.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsAccessConfig {
    /// Extra roots the owner added (absolute, as typed after canonicalizing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    /// Carve-outs: never served through an extra root (a default root or
    /// anything under an extra root).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    fs: FsAccessConfig,
}

impl FsAccessConfig {
    /// `config.json`'s `fs` block; a missing file/block (or an unreadable
    /// one) is the default: just the built-in safe roots.
    pub fn load(path: &Path) -> anyhow::Result<FsAccessConfig> {
        Ok(crate::data_dir::read_json::<ConfigFile>(path)?
            .unwrap_or_default()
            .fs)
    }

    /// Writes the block back, keeping every other key of `config.json`.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut value: serde_json::Value =
            crate::data_dir::read_json(path)?.unwrap_or_else(|| json!({}));
        if !value.is_object() {
            value = json!({});
        }
        value["fs"] = serde_json::to_value(self)?;
        crate::data_dir::write_json(path, &value)
    }
}

/// The host user's login name (for `/run/media/$USER` and the home rule).
pub fn current_user(home: &Path) -> String {
    ["USER", "LOGNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .or_else(|| home.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Why `root` (absolute; canonical when it exists) can't be an extra root,
/// or `None` when it can. Refuses `/`, the system folders in [`FORBIDDEN`]
/// (and anything inside or containing one, except `/var/tmp` and
/// `/run/media/$USER`), `/home` and other users' homes.
pub fn forbidden_reason(root: &Path, home: &Path, user: &str) -> Option<String> {
    if !root.is_absolute() {
        return Some("it must be an absolute path".into());
    }
    let root = lexical_normalize(root);
    if root.parent().is_none() {
        return Some("that's the whole filesystem (/)".into());
    }
    let excepted = exceptions(user);
    if excepted.iter().any(|e| root.starts_with(e)) {
        return None;
    }
    for f in FORBIDDEN {
        let f = Path::new(f);
        if root.starts_with(f) {
            return Some(format!("{} is a system folder", f.display()));
        }
        if f.starts_with(&root) {
            return Some(format!("it contains the system folder {}", f.display()));
        }
    }
    // Other users' homes (and /home itself, which contains them). Our own
    // home (or something under it) is already in scope; allowing it again
    // is harmless.
    let canon_home = dunce::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if root.starts_with(&canon_home) || root.starts_with(home) {
        return None;
    }
    for homes in ["/home", "/Users"] {
        let homes = Path::new(homes);
        if root.starts_with(homes) || homes.starts_with(&root) {
            return Some("other users' homes are off limits".into());
        }
    }
    None
}

/// The resolved scope one request is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsScope {
    /// `$HOME` (as the request resolves `~`).
    pub home: PathBuf,
    /// Extra roots in effect: canonical, existing, not forbidden, not denied.
    pub extra: Vec<PathBuf>,
    /// Canonical carve-outs (`fs deny`), applied to extra roots only.
    pub deny: Vec<PathBuf>,
}

impl FsScope {
    /// Just `$HOME` (tests, and callers without a host config).
    pub fn home_only(home: impl Into<PathBuf>) -> FsScope {
        FsScope {
            home: home.into(),
            extra: Vec::new(),
            deny: Vec::new(),
        }
    }

    /// Defaults + `config` resolved on this machine (see module doc).
    pub fn resolve(home: PathBuf, config: &FsAccessConfig) -> FsScope {
        let user = current_user(&home);
        let canon = |raw: &str| -> Option<PathBuf> {
            let p = PathBuf::from(raw.replace("$USER", &user));
            if !p.is_absolute() {
                return None;
            }
            dunce::canonicalize(p).ok()
        };
        let deny: Vec<PathBuf> = config
            .deny
            .iter()
            .map(|d| canon(d).unwrap_or_else(|| lexical_normalize(Path::new(d))))
            .collect();
        let mut extra: Vec<PathBuf> = Vec::new();
        for raw in DEFAULT_SAFE_ROOTS
            .iter()
            .copied()
            .chain(config.allow.iter().map(String::as_str))
        {
            let Some(root) = canon(raw) else { continue };
            if !root.is_dir() || forbidden_reason(&root, &home, &user).is_some() {
                continue;
            }
            if deny.iter().any(|d| root.starts_with(d)) || extra.contains(&root) {
                continue;
            }
            extra.push(root);
        }
        FsScope { home, extra, deny }
    }

    /// [`resolve`](Self::resolve) with `$HOME` and `<host dir>/config.json`.
    /// A broken config falls back to the defaults (logged), never to "no
    /// access" or "everything".
    pub fn load(config_path: &Path) -> Result<FsScope, unipi_proto::AppErrorDto> {
        let home = crate::fs::home_dir()?;
        let config = FsAccessConfig::load(config_path).unwrap_or_else(|error| {
            tracing::warn!(%error, "config.json fs block unreadable; using the default safe roots");
            FsAccessConfig::default()
        });
        Ok(FsScope::resolve(home, &config))
    }

    /// Whether canonical `canon` is under `$HOME` or an extra root (and not
    /// a denied carve-out of that root).
    pub fn allows(&self, canon: &Path) -> bool {
        if canon.starts_with(&self.home) {
            return true;
        }
        self.extra.iter().any(|r| canon.starts_with(r))
            && !self.deny.iter().any(|d| canon.starts_with(d))
    }
}

/// Outcome of `unipi-host fs allow/deny` (printed by the CLI).
#[derive(Debug, PartialEq, Eq)]
pub enum Edit {
    Changed(String),
    Unchanged(String),
}

/// `unipi-host fs allow <path>`: canonicalizes `raw`, refuses forbidden
/// folders, lifts any deny on it (or under it) and adds it to `allow`
/// unless a default already covers it.
pub fn allow(config: &mut FsAccessConfig, raw: &Path, home: &Path) -> anyhow::Result<Edit> {
    let user = current_user(home);
    let root = dunce::canonicalize(raw)
        .map_err(|e| anyhow::anyhow!("can't use {}: {e}", raw.display()))?;
    anyhow::ensure!(root.is_dir(), "{} is not a folder", root.display());
    if let Some(why) = forbidden_reason(&root, home, &user) {
        anyhow::bail!("refusing {}: {why}", root.display());
    }
    let before = config.clone();
    let root_str = root.to_string_lossy().into_owned();
    config.deny.retain(|d| {
        let d = dunce::canonicalize(Path::new(d)).unwrap_or_else(|_| PathBuf::from(d));
        !d.starts_with(&root)
    });
    if let Some(ancestor) = config.deny.iter().find(|d| root.starts_with(Path::new(d))) {
        anyhow::bail!("{root_str} is inside the denied {ancestor}; run `unipi-host fs allow {ancestor}` first");
    }
    let defaults = FsScope::resolve(home.to_path_buf(), &FsAccessConfig::default());
    let covered_by = if root.starts_with(home) {
        Some(home.to_path_buf())
    } else {
        defaults.extra.iter().find(|r| root.starts_with(r)).cloned()
    };
    let covered = covered_by.is_some();
    if !covered && !config.allow.contains(&root_str) {
        config.allow.push(root_str.clone());
    }
    Ok(if *config == before {
        Edit::Unchanged(match covered_by {
            Some(by) if by != root => {
                format!("{root_str} is already allowed (under {})", by.display())
            }
            _ => format!("{root_str} is already allowed"),
        })
    } else {
        Edit::Changed(format!("Allowed {root_str}"))
    })
}

/// `unipi-host fs deny <path>`: removes an `allow` entry; when the path is
/// still reachable through another extra root (a default, or a parent the
/// owner allowed), records it as a carve-out instead.
pub fn deny(config: &mut FsAccessConfig, raw: &Path, home: &Path) -> anyhow::Result<Edit> {
    let root = dunce::canonicalize(raw).unwrap_or_else(|_| lexical_normalize(raw));
    anyhow::ensure!(
        root.is_absolute(),
        "{} must be an absolute path",
        raw.display()
    );
    let canon_home = dunce::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    anyhow::ensure!(
        !canon_home.starts_with(&root) && !root.starts_with(&canon_home),
        "{} is $HOME (or inside it), which is always in scope",
        root.display()
    );
    let before = config.clone();
    let root_str = root.to_string_lossy().into_owned();
    config.allow.retain(|a| {
        let a = dunce::canonicalize(Path::new(a)).unwrap_or_else(|_| PathBuf::from(a));
        a != root && !a.starts_with(&root)
    });
    let still = FsScope::resolve(home.to_path_buf(), config);
    let reachable = still
        .extra
        .iter()
        .any(|r| root.starts_with(r) || r.starts_with(&root));
    if reachable && !config.deny.contains(&root_str) {
        config.deny.push(root_str.clone());
    }
    Ok(if *config == before {
        Edit::Unchanged(format!("{root_str} was not allowed"))
    } else {
        Edit::Changed(format!("Denied {root_str}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn home() -> PathBuf {
        PathBuf::from("/home/me")
    }

    /// The scratch base for root-allowance tests: a *default* root outside
    /// any home. `tempfile::tempdir()` alone follows `TMPDIR`, which a
    /// wrapping agent session may point inside its own home — then the
    /// folder is neither under the (fake) home nor under a default root and
    /// these tests assert the wrong thing entirely.
    fn sys_tmp() -> &'static std::path::Path {
        #[cfg(unix)]
        {
            std::path::Path::new("/tmp")
        }
        #[cfg(not(unix))]
        {
            std::env::temp_dir()
        }
    }

    #[test]
    fn forbidden_covers_system_folders_root_and_other_homes() {
        let h = home();
        for bad in [
            "/",
            "/etc",
            "/etc/ssh",
            "/usr",
            "/usr/local",
            "/proc",
            "/sys",
            "/dev",
            "/boot",
            "/root",
            "/var",
            "/var/lib",
            "/var/log",
            "/run",
            "/run/user/1000",
            "/run/media/other",
            "/home",
            "/home/bob",
            "/home/bob/x",
            "/bin",
            "/lib64",
        ] {
            assert!(
                forbidden_reason(Path::new(bad), &h, "me").is_some(),
                "{bad} must be forbidden"
            );
        }
        // Containing a system folder is refused too ("/" handled above).
        assert!(forbidden_reason(Path::new("/var/.."), &h, "me").is_some());
        assert!(forbidden_reason(Path::new("relative"), &h, "me").is_some());
    }

    #[test]
    fn forbidden_lets_safe_roots_through() {
        let h = home();
        for ok in [
            "/tmp",
            "/tmp/x",
            "/var/tmp",
            "/var/tmp/a",
            "/mnt",
            "/mnt/usb",
            "/media",
            "/run/media/me",
            "/run/media/me/USB",
            "/srv",
            "/opt",
            "/data",
            "/home/me",
            "/home/me/proj",
        ] {
            assert_eq!(
                forbidden_reason(Path::new(ok), &h, "me"),
                None,
                "{ok} must be allowed"
            );
        }
    }

    #[test]
    fn resolve_uses_existing_defaults_only_and_skips_denied() {
        let scope = FsScope::resolve(home(), &FsAccessConfig::default());
        let tmp = Path::new("/tmp").canonicalize().unwrap();
        assert!(scope.extra.contains(&tmp), "{:?}", scope.extra);
        assert!(scope.extra.iter().all(|r| r.is_dir()));
        let denied = FsScope::resolve(
            home(),
            &FsAccessConfig {
                allow: vec![],
                deny: vec!["/tmp".into()],
            },
        );
        assert!(!denied.extra.contains(&tmp));
        assert!(!denied.allows(&tmp.join("x")));
    }

    #[test]
    fn resolve_drops_forbidden_allow_entries_even_if_hand_edited() {
        let config = FsAccessConfig {
            allow: vec!["/etc".into(), "/".into(), "/home".into(), "relative".into()],
            deny: vec![],
        };
        let scope = FsScope::resolve(home(), &config);
        for p in ["/etc", "/", "/home"] {
            assert!(!scope.extra.contains(&PathBuf::from(p)), "{p}");
        }
        assert!(!scope.allows(Path::new("/etc/passwd")));
    }

    #[cfg(unix)]
    #[test]
    fn a_root_that_is_a_symlink_to_a_system_folder_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("sneaky");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        let config = FsAccessConfig {
            allow: vec![link.to_string_lossy().into()],
            deny: vec![],
        };
        let scope = FsScope::resolve(home(), &config);
        assert!(!scope.extra.iter().any(|r| r.starts_with("/etc")));
        let mut cfg = FsAccessConfig::default();
        assert!(allow(&mut cfg, &link, &home())
            .unwrap_err()
            .to_string()
            .contains("system folder"));
    }

    #[test]
    fn allows_home_extra_roots_and_respects_carve_outs() {
        let scope = FsScope {
            home: home(),
            extra: vec![PathBuf::from("/srv")],
            deny: vec![PathBuf::from("/srv/secret")],
        };
        assert!(scope.allows(Path::new("/home/me/a")));
        assert!(scope.allows(Path::new("/srv/www/index.html")));
        assert!(!scope.allows(Path::new("/srv/secret/key")));
        assert!(!scope.allows(Path::new("/etc/hosts")));
        assert!(!scope.allows(Path::new("/srvx")));
    }

    #[test]
    fn allow_refuses_forbidden_and_missing_and_adds_custom_roots() {
        let tmp = tempfile::tempdir_in(sys_tmp()).unwrap();
        let mut cfg = FsAccessConfig::default();
        for bad in ["/etc", "/", "/usr/share", "/proc"] {
            assert!(allow(&mut cfg, Path::new(bad), &home()).is_err(), "{bad}");
        }
        assert!(allow(&mut cfg, &tmp.path().join("nope"), &home()).is_err());
        assert!(cfg.allow.is_empty());
        // A folder under /tmp is already covered by the default root.
        assert!(matches!(
            allow(&mut cfg, tmp.path(), &home()).unwrap(),
            Edit::Unchanged(_)
        ));
        assert!(cfg.allow.is_empty());
    }

    #[test]
    fn deny_then_allow_round_trips_a_default_root() {
        let mut cfg = FsAccessConfig::default();
        let tmp = Path::new("/tmp").canonicalize().unwrap();
        assert!(matches!(
            deny(&mut cfg, &tmp, &home()).unwrap(),
            Edit::Changed(_)
        ));
        assert_eq!(cfg.deny, vec![tmp.to_string_lossy().to_string()]);
        assert!(!FsScope::resolve(home(), &cfg).extra.contains(&tmp));
        assert!(matches!(
            deny(&mut cfg, &tmp, &home()).unwrap(),
            Edit::Unchanged(_)
        ));
        assert!(matches!(
            allow(&mut cfg, &tmp, &home()).unwrap(),
            Edit::Changed(_)
        ));
        assert_eq!(cfg, FsAccessConfig::default());
        assert!(FsScope::resolve(home(), &cfg).extra.contains(&tmp));
    }

    #[test]
    fn deny_carves_out_a_subfolder_and_allow_under_a_denied_parent_explains() {
        let dir = tempfile::tempdir_in(sys_tmp()).unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        let mut cfg = FsAccessConfig::default();
        deny(&mut cfg, dir.path(), &home()).unwrap();
        let scope = FsScope::resolve(home(), &cfg);
        assert!(!scope.allows(&sub.canonicalize().unwrap()));
        let err = allow(&mut cfg, &sub, &home()).unwrap_err().to_string();
        assert!(err.contains("first"), "{err}");
        allow(&mut cfg, dir.path(), &home()).unwrap();
        assert!(FsScope::resolve(home(), &cfg).allows(&sub.canonicalize().unwrap()));
    }

    #[test]
    fn deny_refuses_home() {
        let mut cfg = FsAccessConfig::default();
        assert!(deny(&mut cfg, Path::new("/home/me"), &home()).is_err());
        assert!(deny(&mut cfg, Path::new("/home/me/x"), &home()).is_err());
    }

    #[test]
    fn config_round_trips_and_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"forwarding":{"enabled":true,"ports":"any"}}"#).unwrap();
        assert_eq!(
            FsAccessConfig::load(&path).unwrap(),
            FsAccessConfig::default()
        );
        let cfg = FsAccessConfig {
            allow: vec!["/data".into()],
            deny: vec!["/opt".into()],
        };
        cfg.save(&path).unwrap();
        assert_eq!(FsAccessConfig::load(&path).unwrap(), cfg);
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["forwarding"]["ports"], "any");
        assert_eq!(raw["fs"]["allow"][0], "/data");
    }
}
