//! "No workspace" chats run in a private per-chat scratch folder owned by UniPi Oino —
//! never in the user's home (the agent could read and write all of it).
//!
//! The folder is `<scratch root>/<session id>`; the root is the OS's per-user data dir
//! (`dirs::data_local_dir`), not `~/.oino` (a cwd under the agent dir would collide with
//! `cwd/.oino` project discovery):
//!
//! | OS | Scratch root |
//! |---|---|
//! | Linux | `${XDG_DATA_HOME:-~/.local/share}/oino/scratch` |
//! | macOS | `~/Library/Application Support/oino/scratch` |
//! | Windows | `%LOCALAPPDATA%\oino\scratch` |
//!
//! `$OINO_SCRATCH_DIR` overrides the root. All scratch chats share the services set of
//! the root (`HubLimits::with_shared_services_root`), so a hundred scratch chats cost one
//! extension load, not a hundred.
//!
//! A scratch chat may move to a real workspace folder — one way only: never back to
//! scratch, never from one workspace to another ([`check_move`]).

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// `$OINO_SCRATCH_DIR` overrides the scratch root.
pub const SCRATCH_DIR_ENV: &str = "OINO_SCRATCH_DIR";

/// The scratch root from explicit inputs (pure; see [`default_scratch_root`]): the
/// override, else `<data_local>/oino/scratch`, else `<home>/.local/share/oino/scratch`,
/// else `<temp>/oino/scratch`.
#[must_use]
pub fn scratch_root_from(
    override_dir: Option<PathBuf>,
    data_local: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir;
    }
    let base = data_local
        .or_else(|| home.map(|home| home.join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("oino").join("scratch")
}

/// This process's scratch root.
#[must_use]
pub fn default_scratch_root() -> PathBuf {
    scratch_root_from(
        std::env::var_os(SCRATCH_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        dirs::data_local_dir(),
        dirs::home_dir(),
    )
}

/// The scratch folder of `session_id` under `root`.
#[must_use]
pub fn scratch_dir(root: &Path, session_id: &str) -> PathBuf {
    root.join(session_id)
}

/// Lexical normal form (`.`/`..` folded, trailing separators dropped) — no I/O, so a
/// deleted scratch folder still compares.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether `cwd` is a chat's scratch folder: a direct child of `root`.
#[must_use]
pub fn is_scratch(root: &Path, cwd: &Path) -> bool {
    let cwd = normal(cwd);
    cwd.parent() == Some(normal(root).as_path())
}

/// Whether `path` is inside the scratch area at all (the root or below).
fn in_scratch_area(root: &Path, path: &Path) -> bool {
    normal(path).starts_with(normal(root))
}

/// The one-way rule of "Move to a workspace…": only a scratch chat moves, and only to a
/// real folder outside the scratch area. Returns the user-facing refusal.
///
/// # Errors
///
/// The chat already has a workspace, or the target is not a usable workspace folder.
pub fn check_move(root: &Path, current: &Path, target: &Path) -> Result<(), String> {
    if !is_scratch(root, current) {
        return Err(
            "Only a chat without a workspace can move to one — this chat already has a workspace"
                .to_owned(),
        );
    }
    if !target.is_absolute() {
        return Err(format!("{} is not a full folder path", target.display()));
    }
    if in_scratch_area(root, target) {
        return Err("Pick a workspace folder outside UniPi's scratch folders".to_owned());
    }
    if !target.is_dir() {
        return Err(format!("{} is not a folder", target.display()));
    }
    Ok(())
}

/// What [`copy_missing`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    /// Files copied.
    pub copied: usize,
    /// Files left alone because the target already had them (never overwritten).
    pub skipped: usize,
}

/// Copies every file of `from` into `to`, recursively, **without overwriting** anything
/// that exists there; symlinks are skipped (they may point anywhere).
///
/// # Errors
///
/// Directory read or file copy failures.
pub fn copy_missing(from: &Path, to: &Path) -> io::Result<CopyReport> {
    let mut report = CopyReport::default();
    copy_tree(from, to, &mut report)?;
    Ok(report)
}

fn copy_tree(from: &Path, to: &Path, report: &mut CopyReport) -> io::Result<()> {
    let entries = match fs::read_dir(from) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            if target.exists() && !target.is_dir() {
                report.skipped += 1;
                continue;
            }
            fs::create_dir_all(&target)?;
            copy_tree(&entry.path(), &target, report)?;
        } else if kind.is_file() {
            // `create_new` refuses an existing target atomically: never overwrite.
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut out) => {
                    let mut input = fs::File::open(entry.path())?;
                    io::copy(&mut input, &mut out)?;
                    report.copied += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    report.skipped += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_per_os_shape_and_override() {
        // Linux: XDG data dir (dirs resolves `$XDG_DATA_HOME` / `~/.local/share`).
        assert_eq!(
            scratch_root_from(None, Some("/home/u/.local/share".into()), None),
            PathBuf::from("/home/u/.local/share/oino/scratch")
        );
        // macOS / Windows: whatever the data-local dir is.
        assert_eq!(
            scratch_root_from(
                None,
                Some("/Users/u/Library/Application Support".into()),
                None
            ),
            PathBuf::from("/Users/u/Library/Application Support/oino/scratch")
        );
        // No data dir: the home fallback; never the home itself.
        assert_eq!(
            scratch_root_from(None, None, Some("/home/u".into())),
            PathBuf::from("/home/u/.local/share/oino/scratch")
        );
        assert_eq!(
            scratch_root_from(Some("/tmp/s".into()), Some("/x".into()), None),
            PathBuf::from("/tmp/s")
        );
        // Not under the agent dir (`~/.oino`): no cwd/.oino discovery collision.
        assert!(!scratch_root_from(None, None, Some("/home/u".into())).starts_with("/home/u/.oino"));
    }

    #[test]
    fn scratch_folders_are_direct_children_of_the_root() {
        let root = Path::new("/d/oino/scratch");
        assert!(is_scratch(root, Path::new("/d/oino/scratch/abc")));
        assert!(is_scratch(root, Path::new("/d/oino/scratch/abc/")));
        assert!(!is_scratch(root, Path::new("/d/oino/scratch")));
        assert!(!is_scratch(root, Path::new("/d/oino/scratch/abc/sub")));
        assert!(!is_scratch(root, Path::new("/home/u")));
        assert!(!is_scratch(root, Path::new("/home/u/project")));
    }

    #[test]
    fn move_is_one_way_scratch_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("scratch");
        let chat = root.join("s1");
        let work = dir.path().join("work");
        let other = dir.path().join("other");
        for d in [&chat, &work, &other] {
            fs::create_dir_all(d).unwrap();
        }
        // scratch → workspace: allowed.
        assert_eq!(check_move(&root, &chat, &work), Ok(()));
        // workspace → scratch, workspace → workspace: refused.
        assert!(check_move(&root, &work, &chat).is_err());
        assert!(check_move(&root, &work, &other).is_err());
        // home ("No workspace" before scratch existed) is not a scratch chat either.
        assert!(check_move(&root, dir.path(), &work).is_err());
        // scratch → another scratch folder or the root: refused.
        assert!(check_move(&root, &chat, &root.join("s2")).is_err());
        assert!(check_move(&root, &chat, &root).is_err());
        // Relative or missing targets: refused.
        assert!(check_move(&root, &chat, Path::new("work")).is_err());
        assert!(check_move(&root, &chat, &dir.path().join("missing")).is_err());
    }

    #[test]
    fn copy_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");
        fs::create_dir_all(from.join("sub")).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join("a.txt"), "scratch a").unwrap();
        fs::write(from.join("b.txt"), "scratch b").unwrap();
        fs::write(from.join("sub").join("c.txt"), "scratch c").unwrap();
        fs::write(to.join("b.txt"), "workspace b").unwrap();
        let report = copy_missing(&from, &to).unwrap();
        assert_eq!(
            report,
            CopyReport {
                copied: 2,
                skipped: 1
            }
        );
        assert_eq!(fs::read_to_string(to.join("a.txt")).unwrap(), "scratch a");
        assert_eq!(fs::read_to_string(to.join("b.txt")).unwrap(), "workspace b");
        assert_eq!(
            fs::read_to_string(to.join("sub").join("c.txt")).unwrap(),
            "scratch c"
        );
        assert_eq!(
            copy_missing(&dir.path().join("gone"), &to).unwrap(),
            CopyReport::default()
        );
    }
}
