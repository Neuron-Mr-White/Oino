//! Registering every opened workspace in zoxide (owner request 2026-11-10): the app's
//! "Quick navigate" ([`crate::superagent_session`]'s zoxide listing and the desktop's
//! `local_zoxide`) only ranks folders zoxide itself has seen, so `session_new{cwd}` must
//! teach it each workspace.
//!
//! Best-effort by contract: a missing binary, a missing `PATH` or any spawn failure is a
//! debug log, never an error — session creation never waits for or fails on this.

use std::path::{Path, PathBuf};

/// Whether `cwd` is a workspace worth teaching zoxide (pure; unit-tested): a real,
/// absolute folder that exists, and not a trivial or agent-internal location.
#[must_use]
pub fn should_register(cwd: Option<&Path>, home: Option<&Path>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    if !cwd.is_absolute() || !cwd.is_dir() {
        return false;
    }
    // The empty path, `/`, and (when known) the user's home itself.
    if cwd.as_os_str().is_empty() || cwd.parent().is_none() {
        return false;
    }
    if home.is_some_and(|home| cwd == home) {
        return false;
    }
    // Temp areas (the host lists them as safe roots; nothing to navigate to).
    for tmp in [Path::new("/tmp"), Path::new("/var/tmp")] {
        if cwd.starts_with(tmp) {
            return false;
        }
    }
    // Oino scratch folders (`<root>/oino/scratch/<session id>`, scratch.rs): private
    // per-chat folders, not workspaces. The exact-root check keeps a *user* folder that
    // happens to be called "scratch" registrable.
    if let Some(root) = cwd.parent() {
        if root.file_name().is_some_and(|name| name == "scratch") {
            let Some(grand) = root.parent() else {
                return false;
            };
            if grand.file_name().is_some_and(|name| name == "oino") {
                return false;
            }
        }
    }
    true
}

/// Where `zoxide` lives: the process `PATH` first, then the dirs the desktop app's
/// PATH-merge knows (`src-tauri/src/shell_path.rs`): the app host's `PATH` may lack
/// `~/.local/bin` (where the installer puts zoxide), the mise shims and cargo.
#[must_use]
pub fn zoxide_exe() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = crate::fs::home_dir_opt() {
        let local_share = home.join(".local").join("share");
        let mise = local_share.join("mise");
        dirs.push(home.join(".local").join("bin"));
        dirs.push(mise.join("shims"));
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
                versions.sort_by(|a, b| b.0.cmp(&a.0));
                if let Some((_, bin)) = versions.into_iter().next() {
                    dirs.push(bin);
                }
            }
        }
        dirs.push(home.join(".cargo").join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/usr/bin"));
    dirs.dedup();
    dirs.iter()
        .map(|dir| dir.join("zoxide"))
        .find(|candidate| std::fs::metadata(candidate).is_ok_and(|meta| meta.is_file()))
}

/// `zoxide add <cwd>`, spawned (never awaited by the caller). All failures — resolve,
/// spawn, non-zero exit — are debug logs.
pub fn register(cwd: PathBuf) {
    let Some(exe) = zoxide_exe() else {
        tracing::debug!(cwd = %cwd.display(), "zoxide not found; workspace not registered");
        return;
    };
    tokio::spawn(async move {
        let spawned = tokio::process::Command::new(exe)
            .arg("add")
            .arg("--")
            .arg(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        match spawned {
            Ok(status) if status.success() => {}
            Ok(status) => {
                tracing::debug!(
                    cwd = %cwd.display(),
                    "zoxide add exited with {status}"
                );
            }
            Err(error) => {
                tracing::debug!(cwd = %cwd.display(), "zoxide add failed: {error}");
            }
        }
    });
}

/// Most folders one `fs_zoxide` answer carries (the picker lists a screenful).
pub const QUERY_LIMIT_MAX: usize = 50;
/// Most words of a query handed to zoxide.
const QUERY_WORDS_MAX: usize = 8;
/// Most zoxide lines scanned for folders the peer may list.
const QUERY_SCAN_MAX: usize = 4000;

/// `zoxide query -ls` output → `(score, path)` best first (`  12.5 /home/u/proj`). Pure.
#[must_use]
pub fn parse_scored(out: &str) -> Vec<(f64, String)> {
    let mut hits: Vec<(f64, String)> = out
        .lines()
        .filter_map(|line| {
            let (score, path) = line.trim().split_once(char::is_whitespace)?;
            let score = score.trim().parse::<f64>().ok()?;
            let path = path.trim();
            (!path.is_empty()).then(|| (score, path.to_owned()))
        })
        .collect();
    // Stable: ties keep zoxide's own order.
    hits.sort_by(|a, b| b.0.total_cmp(&a.0));
    hits
}

/// The folders of a zoxide answer the peer may see: each one must be a real workspace-worthy
/// folder ([`should_register`]: exists, not `/`, home, temp or an Oino scratch) and inside
/// `scope` after canonicalising (a symlink out of the allowed roots is dropped). Hits are
/// de-duplicated by canonical path and cut at `limit`. Pure over the filesystem it is given.
#[must_use]
pub fn visible_hits(
    scored: &[(f64, String)],
    scope: &crate::fs_scope::FsScope,
    limit: usize,
) -> Vec<unipi_proto::FsZoxideHit> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (score, raw) in scored.iter().take(QUERY_SCAN_MAX) {
        if out.len() >= limit {
            break;
        }
        let path = Path::new(raw);
        if !should_register(Some(path), Some(&scope.home)) {
            continue;
        }
        let Ok(canon) = dunce::canonicalize(path) else {
            continue;
        };
        if !scope.allows(&canon) || !seen.insert(canon.clone()) {
            continue;
        }
        out.push(unipi_proto::FsZoxideHit {
            path: canon.to_string_lossy().into_owned(),
            score: *score,
        });
    }
    out
}

/// `fs_zoxide`: what `zoxide query -ls <words>` ranks on this PC, scope-filtered.
/// `available` is false when zoxide is not installed (or cannot run), never an error.
pub async fn query(
    scope: &crate::fs_scope::FsScope,
    words: &str,
    limit: Option<u32>,
) -> unipi_proto::FsZoxideData {
    let unavailable = || unipi_proto::FsZoxideData {
        available: false,
        hits: Vec::new(),
    };
    let Some(exe) = zoxide_exe() else {
        return unavailable();
    };
    let limit = limit.map_or(QUERY_LIMIT_MAX, |n| (n as usize).clamp(1, QUERY_LIMIT_MAX));
    let words: Vec<&str> = words.split_whitespace().take(QUERY_WORDS_MAX).collect();
    let run = tokio::process::Command::new(exe)
        .args(["query", "--list", "--score", "--"])
        .args(&words)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let Ok(Ok(output)) = tokio::time::timeout(std::time::Duration::from_secs(3), run).await else {
        return unavailable();
    };
    // Exit 1 is "no match": zoxide is there, it just has nothing for these words.
    let scored = parse_scored(&String::from_utf8_lossy(&output.stdout));
    let scope = scope.clone();
    let hits = tokio::task::spawn_blocking(move || visible_hits(&scored, &scope, limit))
        .await
        .unwrap_or_default();
    unipi_proto::FsZoxideData {
        available: true,
        hits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real dir next to the test binary, i.e. inside the build's target dir (never the source
    /// tree; tempfile itself lands under `/tmp`, which the tmp-skip rightly refuses).
    fn real_dir(name: &str) -> PathBuf {
        let root = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("zoxide-should-register")
            .join(name);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn skips_none_root_home_tmp_and_scratch() {
        // Missing cwd — the "No workspace" path never reaches this, but the pure
        // function must still refuse.
        assert!(!should_register(None, Some(Path::new("/home/u"))));
        // A non-existent or non-directory "cwd" is refused outright.
        assert!(!should_register(
            Some(Path::new("/no/such/dir")),
            Some(Path::new("/home/u"))
        ));
        assert!(!should_register(
            Some(Path::new("")),
            Some(Path::new("/home/u"))
        ));
        // The filesystem root, and the user's home itself.
        assert!(!should_register(
            Some(Path::new("/")),
            Some(Path::new("/home/u"))
        ));
        let home = real_dir("home");
        assert!(!should_register(Some(&home), Some(&home)));
        // Temp areas (a tempfile dir sits under /tmp on this host).
        let tmp = tempfile::tempdir().unwrap();
        assert!(tmp.path().starts_with("/tmp") || tmp.path().starts_with("/var/tmp"));
        assert!(!should_register(Some(tmp.path()), Some(&home)));
        // Oino scratch folders: `<root>/oino/scratch/<session id>` — the OS data dir
        // layout and the agent-dir layout (tests) both; a scratch-named dir elsewhere
        // is a real workspace and stays registrable.
        let agent_dir = real_dir("agent");
        let scratch_root = agent_dir.join("oino").join("scratch");
        let scratch_chat = scratch_root.join("0190abcd");
        std::fs::create_dir_all(&scratch_chat).unwrap();
        assert!(!should_register(Some(&scratch_chat), Some(&home)));
        let near_miss = agent_dir.join("other").join("scratch").join("0190abcd");
        std::fs::create_dir_all(&near_miss).unwrap();
        assert!(should_register(Some(&near_miss), Some(&home)));
    }

    #[test]
    fn registers_real_workspaces() {
        let root = real_dir("workspaces");
        let ws = root.join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        assert!(should_register(Some(&ws), Some(&root)));
        // A workspace literally called "scratch" outside the oino layout is real.
        let scratch = root.join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        assert!(should_register(Some(&scratch), Some(&root)));
        // The home's parent ("if $HOME were /" — parent() is Some) still refuses.
        assert!(!should_register(
            Some(Path::new("/home/u")),
            Some(Path::new("/home/u"))
        ));
    }

    // ── fs_zoxide: parse + peer scope filtering ──

    use crate::fs_scope::FsScope;

    /// A fresh `home` and `elsewhere` pair of real dirs (outside /tmp, see `real_dir`).
    fn pair(name: &str) -> (PathBuf, PathBuf) {
        let base = real_dir(&format!("visible-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let elsewhere = base.join("elsewhere");
        for dir in [&home, &elsewhere] {
            std::fs::create_dir_all(dir).unwrap();
        }
        (
            dunce::canonicalize(home).unwrap(),
            dunce::canonicalize(elsewhere).unwrap(),
        )
    }

    fn scored(paths: &[&Path]) -> Vec<(f64, String)> {
        paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    100.0 - f64::from(u32::try_from(i).unwrap()),
                    p.to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn parse_scored_orders_best_first_and_skips_junk() {
        let hits = parse_scored(
            "   4.0 /home/u/a\n  12.5 /home/u/my proj\nnot a line\n   4.0 /home/u/b\n",
        );
        let paths: Vec<_> = hits.iter().map(|h| h.1.as_str()).collect();
        assert_eq!(paths, ["/home/u/my proj", "/home/u/a", "/home/u/b"]);
        assert!((hits[0].0 - 12.5).abs() < 1e-9);
    }

    #[test]
    fn visible_hits_keeps_zoxide_order_and_drops_unlistable_folders() {
        let (home, elsewhere) = pair("scope");
        let a = home.join("a");
        let b = home.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let missing = home.join("gone");
        let scope = FsScope::home_only(&home);
        let hits = visible_hits(&scored(&[&b, &elsewhere, &missing, &home, &a]), &scope, 10);
        let paths: Vec<_> = hits.iter().map(|h| h.path.as_str()).collect();
        // zoxide's order; outside-scope, missing and the home itself never leak.
        assert_eq!(paths, [b.to_str().unwrap(), a.to_str().unwrap()]);
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn visible_hits_honours_extra_roots_denies_and_limit() {
        let (home, elsewhere) = pair("extra");
        let project = elsewhere.join("project");
        let secret = elsewhere.join("secret");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&secret).unwrap();
        let mut scope = FsScope::home_only(&home);
        scope.extra.push(elsewhere.clone());
        scope.deny.push(secret.clone());
        let hits = visible_hits(&scored(&[&project, &secret]), &scope, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, project.to_str().unwrap());
        // `limit` cuts the answer.
        let a = home.join("a");
        let b = home.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(
            visible_hits(&scored(&[&a, &b, &project]), &scope, 2).len(),
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn visible_hits_drops_symlinks_that_escape_and_dedups_aliases() {
        let (home, elsewhere) = pair("links");
        let real = home.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let escape = home.join("escape");
        let alias = home.join("alias");
        std::os::unix::fs::symlink(&elsewhere, &escape).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let scope = FsScope::home_only(&home);
        let hits = visible_hits(&scored(&[&escape, &alias, &real]), &scope, 10);
        let paths: Vec<_> = hits.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(paths, [real.to_str().unwrap()]);
    }
}
