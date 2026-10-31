//! Git watcher (v2 app.md §3.14, WP-A15; Z-98 "auto-refresh"): fsnotify on a
//! repo's git metadata — plus the worktree on macOS/Windows — that emits
//! [`GitChangedEvent`] after a trailing debounce, which `events_open` streams
//! forward to the app as `HubEvent::GitChanged{root}` (no polling).
//!
//! Following ZCode `docs/ui/git-auto-refresh.md`:
//! - metadata paths only (gitdir, common dir, their `refs/`): 1 s debounce;
//!   this covers branch switches, commits, stage/unstage on every platform
//!   and needs no recursive walk of the worktree.
//! - the worktree recursively only where the OS watcher handles recursion
//!   cheaply (macOS FSEvents, Windows ReadDirectoryChangesW): 60 s debounce.
//!   On Linux (inotify) the worktree is **not** watched — a recursive watch
//!   has to walk every subdirectory and can stall a slow mount.
//!
//! Watchers are process-global and refcounted per canonical repo root:
//! `git_watch{root, on:true}` adds a reference, `on:false` drops it and the
//! last one out stops the watcher. A folder that is gone or not a repo
//! answers `{root: null}` and watches nothing.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use notify::Watcher as _;
use tokio::sync::{broadcast, mpsc, watch};

/// Trailing debounce of git metadata changes (index, HEAD, refs).
pub const META_DEBOUNCE: Duration = Duration::from_secs(1);
/// Trailing debounce of worktree changes (macOS/Windows watcher only).
pub const WORKTREE_DEBOUNCE: Duration = Duration::from_secs(60);
/// Buffer of the broadcast every `events_open` taps.
const EVENT_CAPACITY: usize = 64;

/// One `GitChanged` emission: the canonical work-tree root, as `git_watch`
/// answered it and the app compares against its `git status` root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitChangedEvent {
    pub root: String,
}

/// Which watcher class an fs event belongs to (its debounce).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Meta,
    Worktree,
}

struct WatchState {
    refs: usize,
    /// Held so the watcher task runs while the entry lives; dropping it
    /// (the entry removed on the last `off`) unwinds the task.
    _stop: watch::Sender<bool>,
}

/// The process-global registry of repo watchers.
pub struct GitWatchers {
    watches: Mutex<HashMap<PathBuf, WatchState>>,
    events: broadcast::Sender<GitChangedEvent>,
}

static SHARED: OnceLock<GitWatchers> = OnceLock::new();

impl GitWatchers {
    fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            watches: Mutex::new(HashMap::new()),
            events,
        }
    }

    /// The host's one registry (`serve`, the `git_watch` op, `events_open`).
    pub fn shared() -> &'static Self {
        SHARED.get_or_init(Self::new)
    }

    /// A new subscriber: every `GitChanged` after now (a lagging receiver is
    /// skipped — the next change re-notifies).
    pub fn subscribe(&self) -> broadcast::Receiver<GitChangedEvent> {
        self.events.subscribe()
    }

    /// Refcounts the watcher of the repo containing `dir`. Returns the
    /// canonical work-tree root the watcher is keyed by (events carry it), or
    /// `None` when the folder is not inside a repo / git is unavailable.
    pub async fn set(&'static self, dir: PathBuf, on: bool) -> Option<String> {
        let resolved = resolve_repo(&dir).await;
        let key = match (&resolved, on) {
            (Some(root), _) => root.0.clone(),
            // `off` for a folder that never resolved: stop by the given key.
            (None, false) => dir,
            (None, true) => return None,
        };
        if !on {
            return self.stop_if_last(&key);
        }
        {
            let mut watches = lock(&self.watches);
            if let Some(state) = watches.get_mut(&key) {
                state.refs += 1;
                return Some(key.to_string_lossy().into_owned());
            }
        }
        let (root, gitdir, commondir) = resolved?;
        let (stop, stop_rx) = watch::channel(false);
        let events = self.events.clone();
        let (spawn_root, spawn_gitdir, spawn_commondir) = (root.clone(), gitdir, commondir);
        // The caller must not race the watcher: a change written between `set`
        // returning and the task registering its paths would be missed.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            watch_repo(
                spawn_root,
                spawn_gitdir,
                spawn_commondir,
                stop_rx,
                events,
                ready_tx,
            )
            .await;
        });
        let _ = ready_rx.await;
        let mut watches = lock(&self.watches);
        // A concurrent `set` may have won the race: its watcher stays.
        watches
            .entry(key.clone())
            .and_modify(|state| state.refs += 1)
            .or_insert(WatchState {
                refs: 1,
                _stop: stop,
            });
        Some(root.to_string_lossy().into_owned())
    }

    /// Drops one reference of the watcher keyed by `key`; `Some(root)` when a
    /// watcher existed, `None` when it was not watched at all.
    fn stop_if_last(&'static self, key: &Path) -> Option<String> {
        let mut watches = lock(&self.watches);
        let state = watches.get_mut(key)?;
        state.refs = state.refs.saturating_sub(1);
        let root = key.to_string_lossy().into_owned();
        if state.refs == 0 {
            // Removing the entry drops the `stop` sender: the task unwinds.
            watches.remove(key);
        }
        Some(root)
    }
}

fn lock(
    mutex: &Mutex<HashMap<PathBuf, WatchState>>,
) -> std::sync::MutexGuard<'_, HashMap<PathBuf, WatchState>> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `(work-tree root, gitdir, common dir)` of `dir`, or `None` when git is
/// unavailable or says the folder is not inside a repo. One git run.
async fn resolve_repo(dir: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let git = crate::git::Git {
        dir: dir.to_path_buf(),
    };
    let out = git
        .run(
            &[
                "rev-parse",
                "--show-toplevel",
                "--path-format=absolute",
                "--git-dir",
                "--git-common-dir",
            ],
            Duration::from_secs(15),
        )
        .await
        .ok()?;
    if !out.ok() {
        return None;
    }
    let text = out.text();
    let mut lines = text.lines().map(str::trim);
    let root = lines.next()?.to_string();
    let gitdir = lines.next()?.to_string();
    let commondir = lines.next().unwrap_or(&gitdir).to_string();
    if root.is_empty() || gitdir.is_empty() {
        return None;
    }
    Some((
        PathBuf::from(root),
        PathBuf::from(gitdir),
        PathBuf::from(commondir),
    ))
}

/// The metadata paths worth watching: the gitdir and common dir themselves
/// (HEAD, index, config, MERGE_HEAD…) and their `refs/` trees (branch
/// updates). Existing paths only — a missing `refs/` before the first commit
/// is fine, `HEAD` in the gitdir still reports commits.
fn metadata_paths(gitdir: &Path, commondir: &Path) -> Vec<(PathBuf, bool)> {
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    let mut push = |path: PathBuf, recursive: bool| {
        if path.is_dir() && !out.iter().any(|(p, _)| p == &path) {
            out.push((path, recursive));
        }
    };
    push(gitdir.to_path_buf(), false);
    push(commondir.to_path_buf(), false);
    push(gitdir.join("refs"), true);
    push(commondir.join("refs"), true);
    out
}

/// Whether the OS watcher makes a recursive worktree watch cheap enough
/// (ZCode: macOS/Windows yes; Linux inotify would have to walk the tree).
const fn worktree_watchable() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// One repo's watcher task: fsnotify events → debounced `GitChanged`.
async fn watch_repo(
    root: PathBuf,
    gitdir: PathBuf,
    commondir: PathBuf,
    mut stop: watch::Receiver<bool>,
    events: broadcast::Sender<GitChangedEvent>,
    ready: tokio::sync::oneshot::Sender<()>,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Class>();
    let meta_tx = tx.clone();
    let Ok(mut meta_watcher) =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return; // our own `git status` reads are not changes
            }
            let _ = meta_tx.send(Class::Meta);
        })
    else {
        return;
    };
    for (path, recursive) in metadata_paths(&gitdir, &commondir) {
        let mode = if recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        if let Err(error) = meta_watcher.watch(&path, mode) {
            tracing::warn!(op = "git_watch", path = %path.display(), ?error, "cannot watch");
        }
    }
    let mut _work_watcher = None;
    if worktree_watchable() {
        // A second watcher keeps the classes apart (different debounces); the
        // worktree watch also fires for `.git` — the metadata watcher's 1 s
        // debounce answers that sooner.
        let work_tx = tx.clone();
        match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            let _ = work_tx.send(Class::Worktree);
        }) {
            Ok(mut watcher) => {
                if let Err(error) = watcher.watch(&root, notify::RecursiveMode::Recursive) {
                    tracing::warn!(op = "git_watch", path = %root.display(), ?error, "cannot watch worktree");
                } else {
                    _work_watcher = Some(watcher);
                }
            }
            Err(error) => tracing::warn!(op = "git_watch", ?error, "no fs watcher"),
        }
    }
    // Watches registered: the repo is guarded from here on (`set` awaits this).
    let _ = ready.send(());
    let root_text = root.to_string_lossy().into_owned();
    let mut meta_deadline: Option<tokio::time::Instant> = None;
    let mut work_deadline: Option<tokio::time::Instant> = None;
    loop {
        let next = [meta_deadline, work_deadline].into_iter().flatten().min();
        tokio::select! {
            // Sender dropped (`off` won) or flagged: unwind.
            _ = stop.changed() => return,
            class = rx.recv() => {
                let Some(class) = class else { return };
                let (deadline, debounce) = match class {
                    Class::Meta => (&mut meta_deadline, META_DEBOUNCE),
                    Class::Worktree => (&mut work_deadline, WORKTREE_DEBOUNCE),
                };
                *deadline = Some(tokio::time::Instant::now() + debounce);
            }
            _ = async {
                match next {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                let now = tokio::time::Instant::now();
                if meta_deadline.is_some_and(|at| at <= now) {
                    meta_deadline = None;
                    let _ = events.send(GitChangedEvent { root: root_text.clone() });
                }
                if work_deadline.is_some_and(|at| at <= now) {
                    work_deadline = None;
                    let _ = events.send(GitChangedEvent { root: root_text.clone() });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_paths_dedup_and_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let gitdir = tmp.path().join("r").join(".git");
        std::fs::create_dir_all(gitdir.join("refs")).unwrap();
        // A plain repo: the gitdir and its refs tree, non-recursive then recursive.
        assert_eq!(
            metadata_paths(&gitdir, &gitdir),
            vec![(gitdir.clone(), false), (gitdir.join("refs"), true)]
        );
        // A linked worktree: its own gitdir plus the common dir and both refs.
        let worktree = gitdir.join("worktrees").join("w");
        std::fs::create_dir_all(worktree.join("refs")).unwrap();
        assert_eq!(
            metadata_paths(&worktree, &gitdir),
            vec![
                (worktree.clone(), false),
                (gitdir.clone(), false),
                (worktree.join("refs"), true),
                (gitdir.join("refs"), true),
            ]
        );
        // A missing refs tree (before the first commit) is skipped, not watched.
        std::fs::remove_dir_all(gitdir.join("refs")).unwrap();
        assert_eq!(
            metadata_paths(&gitdir, &gitdir),
            vec![(gitdir.clone(), false)]
        );
    }
}
