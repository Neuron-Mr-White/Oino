//! Where a host keeps its data dir — the ONE rule shared by `unipi-host`
//! and the desktop app (it used to be written out three times, so the app
//! and the host could disagree and a second host with a fresh identity
//! appeared — UNI-298).
//!
//! v2 (specs/dev/v2/specs/host.md §5.8): the dir is `~/.oino/host`
//! (`$OINO_HOST_DIR` overrides). The v1 `~/.unipi/app-host` dir (and its
//! Windows legacy fallback) belongs to the owner's old unipi and is never
//! read, written or locked by this host; the two coexist side by side.

use std::path::PathBuf;

/// `$OINO_HOST_DIR` overrides everything.
pub const OINO_HOST_DIR_ENV: &str = "OINO_HOST_DIR";

/// The data dir from explicit inputs (pure; see [`default_host_dir`]).
///
/// Unix: `$HOME/.oino/host`. Windows has no `HOME` outside ssh/Git Bash,
/// so `%USERPROFILE%` is the fallback. Unlike v1 there is no legacy-dir
/// fallback: a v2 host always uses its own dir.
pub fn host_dir_from(
    windows: bool,
    override_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    profile: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir;
    }
    let sub = |p: PathBuf| p.join(".oino").join("host");
    let base = if windows { home.or(profile) } else { home };
    base.map(sub).unwrap_or_else(|| sub(PathBuf::new()))
}

/// This process's host data dir.
pub fn default_host_dir() -> PathBuf {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    host_dir_from(
        cfg!(windows),
        env(OINO_HOST_DIR_ENV),
        env("HOME"),
        env("USERPROFILE"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sub(p: &Path) -> PathBuf {
        p.join(".oino").join("host")
    }

    #[test]
    fn unix_home_and_override() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            host_dir_from(false, None, Some(home.clone()), None),
            sub(&home)
        );
        let o = PathBuf::from("/tmp/x");
        assert_eq!(host_dir_from(false, Some(o.clone()), Some(home), None), o);
    }

    #[test]
    fn windows_prefers_home_then_profile_no_legacy() {
        let home = PathBuf::from("C:/Users/oi/home");
        let profile = PathBuf::from("C:/Users/oi");
        assert_eq!(
            host_dir_from(true, None, Some(home.clone()), Some(profile.clone())),
            sub(&home)
        );
        // No HOME (PowerShell): USERPROFILE. Never a legacy dir: v2 always
        // keeps its own identity dir.
        assert_eq!(
            host_dir_from(true, None, None, Some(profile.clone())),
            sub(&profile)
        );
    }

    #[test]
    fn no_home_at_all_is_relative() {
        assert_eq!(
            host_dir_from(false, None, None, None),
            PathBuf::new().join(".oino").join("host")
        );
    }
}
