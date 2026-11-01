//! The PATH an app started from a menu should have had.
//!
//! An app launched from a `.desktop` file, the Dock or Start does not get the
//! shell's PATH, so the tools the agent's `bash` runs (mise / nvm Node, cargo,
//! herdr, tmux, …) would look missing. The host the app spawns therefore gets a
//! merged PATH, in order of trust:
//!
//! 1. the process PATH;
//! 2. a login-shell probe — `$SHELL -lic 'printf %s "$PATH"'` with a 5 s
//!    timeout, ignored on failure (it is only ever an improvement);
//! 3. known directories (`~/.local/bin`, mise shims and installs, nvm, pnpm,
//!    bun, cargo, …).
//!
//! The probe runs once, in the background at start (`warm`); a host spawned
//! before it lands keeps the inherited PATH. Nothing here reads or writes
//! anything outside the process: the app bundles everything it needs.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the login-shell probe may take.
pub const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// `1.2.3`, `v1.2.3`, `1.2.3-alpha.4`, `herdr 0.9.0` → `(1, 2, 3)`.
pub fn version_tuple(text: &str) -> (u64, u64, u64) {
    let digits: String = text
        .trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = digits.split('.').map(|p| p.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

// --------------------------------------------------------------------- PATH

/// Splits a `PATH` string into directories (empty entries dropped).
pub fn parse_path(value: &str) -> Vec<PathBuf> {
    value
        .split(':')
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// Deduplicates while keeping the first (most trusted) position.
pub fn dedup_dirs(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .filter(|d| seen.insert(d.clone()))
        .collect()
}

/// Where a tool may hide outside any PATH, in the order we try them.
pub fn known_dirs(home: &Path) -> Vec<PathBuf> {
    let local_share = home.join(".local").join("share");
    let mise = local_share.join("mise");
    let mut dirs = vec![home.join(".local").join("bin"), mise.join("shims")];
    // mise installs: the shims dir is the canonical entry (it points at the
    // tool's active version), and we add each tool's NEWEST install as a
    // fallback for a shims dir that is missing. Newest by version tuple, not
    // by string — "9" must not sort above "10".
    if let Ok(entries) = std::fs::read_dir(mise.join("installs")) {
        let mut tools: Vec<(String, Vec<(String, PathBuf)>)> = Vec::new();
        for tool in entries.flatten() {
            let mut versions: Vec<(String, PathBuf)> = Vec::new();
            if let Ok(list) = std::fs::read_dir(tool.path()) {
                for version in list.flatten() {
                    versions.push((
                        version.file_name().to_string_lossy().to_string(),
                        version.path().join("bin"),
                    ));
                }
            }
            tools.push((tool.file_name().to_string_lossy().to_string(), versions));
        }
        tools.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, mut versions) in tools {
            versions.sort_by(|a, b| {
                version_tuple(&b.0)
                    .cmp(&version_tuple(&a.0))
                    .then_with(|| b.0.cmp(&a.0))
            });
            if let Some((_, bin)) = versions.into_iter().next() {
                dirs.push(bin);
            }
        }
    }
    dirs.push(home.join(".nvm").join("current").join("bin"));
    dirs.push(local_share.join("pnpm"));
    dirs.push(home.join(".bun").join("bin"));
    dirs.push(home.join(".cargo").join("bin"));
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(home.join(".deno").join("bin"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

/// The login-shell probe's raw output, e.g. `/a:/b\n` → `Some("/a:/b")`.
/// The probe prints a crab banner on this PC, so only the last line that looks
/// like a PATH is used.
pub fn clean_probe_output(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty() && line.contains('/') && !line.contains(char::is_whitespace))
        .map(str::to_string)
}

/// The merged PATH: process, then the probe, then the known dirs.
pub fn merge_paths(process: &[PathBuf], probe: &[PathBuf], known: &[PathBuf]) -> Vec<PathBuf> {
    let mut all = Vec::new();
    all.extend(process.iter().cloned());
    all.extend(probe.iter().cloned());
    all.extend(known.iter().cloned());
    dedup_dirs(all)
}

/// The merged PATH as a `PATH=`-shaped string.
pub fn path_value(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(":")
}

// ------------------------------------------------------------------ merged

/// The merged PATH, computed once per process — the probe spawns a shell, and
/// the host spawn happens at most a handful of times. Empty until
/// [`merged_path_here`] has run.
static MERGED_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The merged PATH, or "" while nothing has computed it yet.
pub fn merged_path() -> String {
    MERGED_PATH.get().cloned().unwrap_or_default()
}

/// Merges the process PATH, the login-shell probe and the known dirs, and
/// caches the result for the host spawn and the unit's env file.
pub async fn merged_path_here(home: &Path) -> String {
    let value = compute_merged_path(home).await;
    let _ = MERGED_PATH.set(value.clone());
    value
}

async fn compute_merged_path(home: &Path) -> String {
    let process_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let probe = match std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    {
        Some(shell) if shell.exists() => probe_shell_path(&shell).await,
        _ => None,
    };
    path_value(&merge_paths(
        &process_path,
        &probe.unwrap_or_default(),
        &known_dirs(home),
    ))
}

/// `$SHELL -lic 'printf %s "$PATH"'`, cleaned; `None` on any failure.
pub async fn probe_shell_path(shell: &Path) -> Option<Vec<PathBuf>> {
    let out = tokio::process::Command::new(shell)
        .args(["-lic", "printf %s \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .output();
    let out = tokio::time::timeout(SHELL_PROBE_TIMEOUT, out)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    clean_probe_output(&text).map(|p| parse_path(&p))
}

/// Computes the merged PATH off the UI thread, once, at start.
pub fn warm() {
    tauri::async_runtime::spawn(async {
        let home = crate::host_supervisor::home_dir();
        let _ = merged_path_here(&home).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/tester")
    }

    #[test]
    fn merges_process_probe_and_known_dirs_without_duplicates() {
        let process = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")];
        let probe = vec![PathBuf::from("/opt/mise/shims"), PathBuf::from("/usr/bin")];
        let known = vec![PathBuf::from("/home/t/.local/bin"), PathBuf::from("/bin")];
        let merged = merge_paths(&process, &probe, &known);
        assert_eq!(
            merged,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/opt/mise/shims"),
                PathBuf::from("/home/t/.local/bin"),
            ]
        );
        assert_eq!(
            path_value(&merged),
            "/usr/bin:/bin:/opt/mise/shims:/home/t/.local/bin"
        );
    }

    #[test]
    fn the_probe_output_is_cleaned_of_banners() {
        // This PC's `bash -lic` prints a crab before the PATH.
        let raw = "\n ／l、\n（>､< ７\n/home/a:/home/b\n";
        assert_eq!(clean_probe_output(raw).as_deref(), Some("/home/a:/home/b"));
        assert_eq!(clean_probe_output("no paths here").as_deref(), None);
        assert_eq!(clean_probe_output("").as_deref(), None);
    }

    #[test]
    fn known_dirs_lead_with_local_bin_and_never_name_legacy_dirs() {
        let dirs = known_dirs(&home());
        let names: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        assert_eq!(names[0], "/home/tester/.local/bin");
        assert!(names.contains(&"/home/tester/.local/share/mise/shims".to_string()));
        assert!(
            names
                .iter()
                .all(|n| !n.contains("/.pi/") && !n.contains("pi-node") && !n.contains(".unipi")),
            "{names:?}"
        );
    }

    #[test]
    fn versions_parse_numerically() {
        assert_eq!(version_tuple("v24.16.0"), (24, 16, 0));
        assert_eq!(version_tuple("herdr 0.9.0"), (0, 9, 0));
        assert!(version_tuple("10.0.0") > version_tuple("9.9.9"));
    }
}
