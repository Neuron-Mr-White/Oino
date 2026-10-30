//! UNI-298: the ONE rule for where a host keeps its data dir, shared by
//! `unipi-host` and the desktop app (it used to be written out three times
//! and the app's copies missed the Windows legacy dir — so on a PC whose
//! host lives in `%LOCALAPPDATA%\UniPi\.unipi\app-host`, the app started a
//! *second* host with a fresh identity in `%USERPROFILE%\.unipi\app-host`,
//! and after a restart the phone/desktop "lost" the host).

use std::path::PathBuf;

/// `$UNIPI_HOST_DIR` overrides everything.
pub const UNIPI_HOST_DIR_ENV: &str = "UNIPI_HOST_DIR";

/// The data dir from explicit inputs (pure; see [`default_host_dir`]).
///
/// Unix: `$HOME/.unipi/app-host`. Windows has no `HOME` outside ssh/Git
/// Bash, so `%USERPROFILE%` is the fallback, and an identity already created
/// in the old place (`%LOCALAPPDATA%\UniPi\.unipi\app-host`) is kept so
/// paired phones stay paired.
pub fn host_dir_from(
    windows: bool,
    override_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    profile: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    has_identity: &dyn Fn(&std::path::Path) -> bool,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir;
    }
    let sub = |p: PathBuf| p.join(".unipi").join("app-host");
    if !windows {
        return home.map(sub).unwrap_or_else(|| sub(PathBuf::new()));
    }
    let preferred = home.or(profile).map(sub);
    let legacy = local_app_data.map(|l| sub(l.join("UniPi")));
    match (preferred, legacy) {
        (Some(p), Some(l)) if !has_identity(&p) && has_identity(&l) => l,
        (Some(p), _) => p,
        (None, Some(l)) => l,
        (None, None) => sub(PathBuf::new()),
    }
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
        env(UNIPI_HOST_DIR_ENV),
        env("HOME"),
        env("USERPROFILE"),
        env("LOCALAPPDATA"),
        &|dir| dir.join("secret.key").exists(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sub(p: &Path) -> PathBuf {
        p.join(".unipi").join("app-host")
    }

    #[test]
    fn windows_keeps_the_legacy_identity() {
        let profile = PathBuf::from("C:/Users/oi");
        let lad = PathBuf::from("C:/Users/oi/AppData/Local");
        let legacy = sub(&lad.join("UniPi"));
        let only_legacy = |d: &Path| d == legacy.as_path();
        // Only the legacy dir has an identity → use it (the bug: the app used the profile dir).
        assert_eq!(
            host_dir_from(
                true,
                None,
                None,
                Some(profile.clone()),
                Some(lad.clone()),
                &only_legacy
            ),
            legacy
        );
        // Both have one → the preferred profile dir.
        assert_eq!(
            host_dir_from(
                true,
                None,
                None,
                Some(profile.clone()),
                Some(lad.clone()),
                &|_| true
            ),
            sub(&profile)
        );
        // Neither → the profile dir (a fresh install).
        assert_eq!(
            host_dir_from(true, None, None, Some(profile.clone()), Some(lad), &|_| {
                false
            }),
            sub(&profile)
        );
    }

    #[test]
    fn unix_and_override() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            host_dir_from(false, None, Some(home.clone()), None, None, &|_| false),
            sub(&home)
        );
        let o = PathBuf::from("/tmp/x");
        assert_eq!(
            host_dir_from(true, Some(o.clone()), None, None, None, &|_| false),
            o
        );
    }
}
