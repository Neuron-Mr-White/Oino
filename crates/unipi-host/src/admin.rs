//! UNI-232: helpers for the local-only admin ops (`host_info` …).
//!
//! v2 (host.md §11.1): `host_info.tools` reports what this host can use —
//! the `oino` core binary, the `unipi-kanboard` CLI, the Chromium-based
//! browsers found on the PC and whether a ready CloakBrowser build exists.
//! The v1 herdr/tmux/pi fields are gone with the multiplexer and the
//! app-bridge.

use std::path::Path;
use std::path::PathBuf;

use unipi_proto::HostTools;

use crate::data_dir::HostPaths;

/// Is `name` an executable on our PATH?
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    find_bin(name, &dirs, &|k| std::env::var(k).ok()).is_some()
}

/// The Chromium-based browser kinds installed on this PC (`chrome`,
/// `chromium`, `edge`, `brave`), in candidate order, no duplicates.
fn browser_engines(path_var: Option<&std::ffi::OsStr>) -> Vec<String> {
    let dirs: Vec<PathBuf> = path_var
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    let env = |key: &str| {
        if key == "HOME" {
            return crate::fs::home_dir_opt().map(|home| home.display().to_string());
        }
        std::env::var(key).ok()
    };
    let mut out: Vec<String> = Vec::new();
    for candidate in crate::browser::CANDIDATES {
        if out.iter().any(|kind| kind == candidate.kind) {
            continue;
        }
        if find_bin(candidate.bin, &dirs, &env).is_some() {
            out.push(candidate.kind.to_string());
        }
    }
    out
}

/// The same lookup [`crate::browser::detect_in`] does: `~/…`/`%VAR%`-expanded
/// names on the PATH, absolute paths as they are.
fn find_bin(name: &str, dirs: &[PathBuf], env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let name = crate::browser::expand(name, env)?;
    let path = Path::new(&name);
    if path.is_absolute() || name.contains('/') {
        return is_executable(path).then(|| path.to_path_buf());
    }
    dirs.iter()
        .map(|dir| dir.join(&name))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Which tools this host can use (as `host_info` reports them).
pub fn tools(paths: &HostPaths) -> HostTools {
    // A missing/corrupt browser block is the default (CloakBrowser, no
    // configured paths) — `host_info` must never fail over it.
    let config = crate::browser::BrowserConfig::load(&paths.config()).unwrap_or_default();
    let cloak = crate::browser::cloak::locate(
        config.cloak_path.as_deref(),
        config.cloak_version.as_deref(),
    )
    .ok()
    .flatten()
    .is_some();
    HostTools {
        oino: on_path("oino"),
        unipi_kanboard: on_path("unipi-kanboard"),
        browser_engines: browser_engines(std::env::var_os("PATH").as_deref()),
        cloak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_engines_finds_distinct_kinds_in_candidate_order() {
        let temp = tempfile::tempdir().unwrap();
        let bin = |name: &str| {
            let path = temp.path().join(name);
            std::fs::write(&path, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            path
        };
        // Two candidates for the same kind (chrome) must collapse to one.
        bin("google-chrome");
        bin("google-chrome-stable");
        bin("chromium");
        let path_var = std::env::join_paths([temp.path()]).unwrap();
        let engines = browser_engines(Some(&path_var));
        let chrome = engines.iter().position(|k| k == "chrome");
        let chromium = engines.iter().position(|k| k == "chromium");
        assert!(chrome.is_some(), "{engines:?}");
        assert!(chromium.is_some(), "{engines:?}");
        assert_eq!(
            engines.iter().filter(|k| *k == "chrome").count(),
            1,
            "{engines:?}"
        );
        assert!(chrome < chromium, "candidate order kept: {engines:?}");
        assert!(browser_engines(None).len() <= crate::browser::CANDIDATES.len());
    }

    #[test]
    fn tools_never_fail_on_a_bare_host_dir() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        let tools = tools(&paths);
        // Nothing to find in a fresh temp dir: both CLIs are PATH-dependent,
        // the engine list is whatever this machine has.
        assert!(tools.browser_engines.len() <= crate::browser::CANDIDATES.len());
    }
}
