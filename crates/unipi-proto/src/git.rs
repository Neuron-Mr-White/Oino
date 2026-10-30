//! Basic git on the PC (UNI-217). Mirrors `packages/protocol/src/git.ts`.
//!
//! One request/reply op: `hello{req:{op:"git", path, outside?, cmd:{action,…}}}`
//! → `ok{data}` (the shape depends on `cmd.action`, see below) or `err`.
//! `path` is any folder (or file) inside the repo; it goes through the same
//! scope as `fs_*` (`E_SCOPE` outside it). The host shells out to the `git`
//! CLI, never interactively (no credential prompts, no editor), with a
//! timeout. A failing git command answers `E_GIT` with git's own message
//! (and a `hint` when it's an auth / identity problem).
//!
//! | action | data |
//! |---|---|
//! | `repo` | [`GitRepoData`] (`repo:false` outside a repo — not an error) |
//! | `status` | [`GitStatusData`] |
//! | `diff` | [`GitDiffData`] (one file; `staged` = index vs HEAD, else worktree vs index; untracked = whole file) |
//! | `stage` / `unstage` | `null` (`paths`, or `all:true`) |
//! | `discard` | `null` (`confirm:true` required; untracked files are deleted) |
//! | `commit` | [`GitCommitData`] |
//! | `log` | [`GitLogData`] (`skip`/`limit`, newest first) |
//! | `show` | [`GitShowData`] |
//! | `branches` | [`GitBranchesData`] |
//! | `switch` | `null` (`create:true` makes the branch first) |
//! | `fetch` / `pull` / `push` | [`GitSyncData`] (pull is `--ff-only` unless `rebase:true`) |
//! | `stash_push` / `stash_pop` | [`GitSyncData`] |
//! | `shortstat` | [`GitShortstatData`] (UNI-257: total +/- vs HEAD incl. staged + untracked, capped; proto 1.12.0) |
//! | `turn_files` | [`GitTurnFilesData`] (v2: `status` + per-file diffs, restricted to `files`) |

use serde::{Deserialize, Serialize};

/// Highest `log` page size.
pub const GIT_LOG_MAX: u32 = 200;
/// Highest number of paths in one `stage`/`unstage`/`discard`.
pub const GIT_PATHS_MAX: usize = 1000;
/// Diff text cap (bytes) before `truncated` is set.
pub const GIT_DIFF_CAP: usize = 256 * 1024;

fn invalid_ref_char(c: char) -> bool {
    c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
}

/// A branch name or commit-ish (`main`, `origin/feat-x`, `a1b2c3d`):
/// 1..=255 chars, git's ref rules (no spaces, `..`, `~^:?*[\`, no leading
/// `-` so it can never read as an option).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct GitRefName(String);

impl GitRefName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GitRefName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if !(1..=255).contains(&len) {
            return Err(format!("ref must be 1..=255 characters, got {len}"));
        }
        if value.starts_with('-')
            || value.starts_with('/')
            || value.ends_with('/')
            || value.ends_with('.')
            || value.ends_with(".lock")
        {
            return Err("not a valid branch name".into());
        }
        if value.contains("..")
            || value.contains("//")
            || value.contains("@{")
            || value.chars().any(invalid_ref_char)
        {
            return Err("not a valid branch name".into());
        }
        Ok(GitRefName(value))
    }
}

impl<'de> Deserialize<'de> for GitRefName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        GitRefName::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A repo-relative path (as `status` reports it): 1..=4096 bytes, no NUL,
/// not absolute, no `..` segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct GitPath(String);

impl GitPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GitPath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 4096 {
            return Err(format!(
                "path must be 1..=4096 characters, got {}",
                value.len()
            ));
        }
        if value.bytes().any(|b| b == 0) {
            return Err("path must not contain NUL bytes".into());
        }
        if value.starts_with('/') || value.split('/').any(|seg| seg == "..") {
            return Err("path must be relative to the repo, without `..`".into());
        }
        Ok(GitPath(value))
    }
}

impl<'de> Deserialize<'de> for GitPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        GitPath::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A commit message: not blank, at most 20000 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct GitMessage(String);

impl GitMessage {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GitMessage {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err("commit message must not be empty".into());
        }
        if value.chars().count() > 20000 {
            return Err("commit message must be at most 20000 characters".into());
        }
        Ok(GitMessage(value))
    }
}

impl<'de> Deserialize<'de> for GitMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        GitMessage::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn paths_ok(paths: &[GitPath]) -> bool {
    paths.len() <= GIT_PATHS_MAX
}

/// What to do (`cmd.action`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GitCmd {
    Repo,
    Status,
    Diff {
        file: GitPath,
        #[serde(default)]
        staged: bool,
        /// The file is untracked (diff against nothing).
        #[serde(default)]
        untracked: bool,
    },
    Stage {
        #[serde(default)]
        paths: Vec<GitPath>,
        #[serde(default)]
        all: bool,
    },
    Unstage {
        #[serde(default)]
        paths: Vec<GitPath>,
        #[serde(default)]
        all: bool,
    },
    Discard {
        paths: Vec<GitPath>,
        #[serde(default)]
        confirm: bool,
    },
    Commit {
        message: GitMessage,
        #[serde(default)]
        amend: bool,
    },
    Log {
        #[serde(default)]
        skip: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        limit: Option<u32>,
    },
    Show {
        rev: GitRefName,
    },
    Branches,
    Switch {
        branch: GitRefName,
        #[serde(default)]
        create: bool,
    },
    Fetch,
    Pull {
        #[serde(default)]
        rebase: bool,
    },
    Push,
    StashPush {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        message: Option<GitMessage>,
    },
    StashPop,
    /// UNI-257: cheap total line counts (staged + unstaged vs HEAD, plus
    /// untracked files' lines, capped) for the chat menu's Git row.
    Shortstat,
    /// v2 (host.md §9.4): the "Last turn" source of the Git pane. The app
    /// passes the files the last turn's tool rows touched; the host answers
    /// their `status` rows plus the diff of each (≤ [`GIT_DIFF_CAP`]), so the
    /// whole last-turn set is one round trip.
    ///
    /// `staged` restricts the rows to index changes and `untracked` to
    /// untracked files (both off = any change). A file changed in both the
    /// index and the work tree yields its staged row first — that is what a
    /// commit would take.
    TurnFiles {
        files: Vec<GitPath>,
        #[serde(default)]
        staged: bool,
        #[serde(default)]
        untracked: bool,
    },
}

impl GitCmd {
    pub fn name(&self) -> &'static str {
        match self {
            GitCmd::Repo => "repo",
            GitCmd::Status => "status",
            GitCmd::Diff { .. } => "diff",
            GitCmd::Stage { .. } => "stage",
            GitCmd::Unstage { .. } => "unstage",
            GitCmd::Discard { .. } => "discard",
            GitCmd::Commit { .. } => "commit",
            GitCmd::Log { .. } => "log",
            GitCmd::Show { .. } => "show",
            GitCmd::Branches => "branches",
            GitCmd::Switch { .. } => "switch",
            GitCmd::Fetch => "fetch",
            GitCmd::Pull { .. } => "pull",
            GitCmd::Push => "push",
            GitCmd::StashPush { .. } => "stash_push",
            GitCmd::StashPop => "stash_pop",
            GitCmd::Shortstat => "shortstat",
            GitCmd::TurnFiles { .. } => "turn_files",
        }
    }

    /// Shape rules serde can't express (path counts, log page size).
    pub fn check(&self) -> Result<(), String> {
        match self {
            GitCmd::Stage { paths, all } | GitCmd::Unstage { paths, all } => {
                if !paths_ok(paths) {
                    return Err(format!("at most {GIT_PATHS_MAX} paths"));
                }
                if paths.is_empty() && !all {
                    return Err("give paths or all:true".into());
                }
            }
            GitCmd::Discard { paths, .. } => {
                if paths.is_empty() || !paths_ok(paths) {
                    return Err(format!("discard needs 1..={GIT_PATHS_MAX} paths"));
                }
            }
            GitCmd::TurnFiles { files, .. } => {
                if files.is_empty() || !paths_ok(files) {
                    return Err(format!("turn_files needs 1..={GIT_PATHS_MAX} files"));
                }
            }
            GitCmd::Log {
                limit: Some(limit), ..
            } if *limit == 0 || *limit > GIT_LOG_MAX => {
                return Err(format!("limit must be 1..={GIT_LOG_MAX}"));
            }
            _ => {}
        }
        Ok(())
    }

    /// Talks to a remote (longer timeout).
    pub fn is_network(&self) -> bool {
        matches!(self, GitCmd::Fetch | GitCmd::Pull { .. } | GitCmd::Push)
    }
}

/// `ok.data` for `repo` (and the head of `status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitRepoData {
    /// Whether `path` is inside a work tree.
    pub repo: bool,
    /// Absolute work-tree root.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub root: Option<String>,
    /// Current branch; absent when detached or outside a repo.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub branch: Option<String>,
    /// Short HEAD hash; absent before the first commit.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub head: Option<String>,
    #[serde(default)]
    pub detached: bool,
    /// `origin/main`, when the branch tracks one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    /// Remote names (`origin`, …).
    #[serde(default)]
    pub remotes: Vec<String>,
    #[serde(default)]
    pub stashes: u32,
    /// `merge` / `rebase` / `cherry-pick` / `revert` while one is in progress.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub operation: Option<String>,
}

/// Which list a changed file sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitArea {
    Staged,
    Unstaged,
    Untracked,
    Conflict,
}

/// One row of `status` (a file staged *and* changed again shows twice:
/// once `staged`, once `unstaged`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileChange {
    /// Repo-relative path.
    pub path: String,
    /// Old path of a rename/copy.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub orig: Option<String>,
    pub area: GitArea,
    /// Porcelain letter for this area: `M` `A` `D` `R` `C` `T` `?` `U`.
    pub status: String,
    /// Added / deleted lines; absent for binary files.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub add: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub del: Option<u32>,
    #[serde(default)]
    pub binary: bool,
}

/// `ok.data` for `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatusData {
    #[serde(flatten)]
    pub repo: GitRepoData,
    pub files: Vec<GitFileChange>,
    /// More than the host's cap of changed files: only the first are listed.
    #[serde(default)]
    pub more: bool,
}

/// `ok.data` for `diff`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDiffData {
    /// Unified diff (may be empty: nothing to show).
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub binary: bool,
}

/// One file of `turn_files`: its `status` row plus the diff of that change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitTurnFile {
    #[serde(flatten)]
    pub change: GitFileChange,
    pub diff: GitDiffData,
}

/// `ok.data` for `turn_files`: `status` restricted to the asked-for files,
/// each row carrying its diff. Files with no change are omitted. Outside a
/// repo it is `repo:false` with no rows (not an error).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitTurnFilesData {
    #[serde(flatten)]
    pub repo: GitRepoData,
    #[serde(default)]
    pub files: Vec<GitTurnFile>,
}

/// `ok.data` for `commit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitData {
    pub hash: String,
    pub subject: String,
}

/// One `log` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitInfo {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds (author date).
    pub time: i64,
    /// Decorations: `HEAD -> main`, `origin/main`, `tag: v1`.
    #[serde(default)]
    pub refs: Vec<String>,
}

/// `ok.data` for `log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLogData {
    pub commits: Vec<GitCommitInfo>,
    pub more: bool,
}

/// One file of `show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitShowFile {
    pub path: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub add: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub del: Option<u32>,
}

/// `ok.data` for `show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitShowData {
    pub commit: GitCommitInfo,
    /// Full message body (after the subject), may be empty.
    #[serde(default)]
    pub body: String,
    pub files: Vec<GitShowFile>,
    /// Unified diff of the whole commit (capped).
    pub diff: String,
    #[serde(default)]
    pub truncated: bool,
}

/// One `branches` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranch {
    pub name: String,
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub current: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    /// Last commit: unix seconds + subject.
    #[serde(default)]
    pub time: i64,
    #[serde(default)]
    pub subject: String,
}

/// `ok.data` for `branches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranchesData {
    pub branches: Vec<GitBranch>,
}

/// `ok.data` for `shortstat` (UNI-257): total changed lines of the work
/// tree vs HEAD (staged + unstaged), plus the line count of untracked files
/// (at most [`GIT_SHORTSTAT_UNTRACKED_MAX`] files, each ≤ 1 MiB, text only).
/// `repo:false` outside a repo (not an error); all zero = clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitShortstatData {
    pub repo: bool,
    /// Changed files (tracked + untracked).
    #[serde(default)]
    pub files: u32,
    #[serde(default)]
    pub add: u32,
    #[serde(default)]
    pub del: u32,
    /// More untracked files than the cap: `add` is a lower bound.
    #[serde(default)]
    pub capped: bool,
}

/// Most untracked files `shortstat` counts lines of.
pub const GIT_SHORTSTAT_UNTRACKED_MAX: usize = 300;

/// `ok.data` for fetch/pull/push/stash: git's own summary (trimmed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSyncData {
    pub output: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names() {
        for ok in ["main", "feat/x-1", "origin/main", "a1b2c3d", "v1.2"] {
            assert!(GitRefName::try_from(ok.to_string()).is_ok(), "{ok}");
        }
        for bad in [
            "", "-x", "--force", "a b", "a..b", "a~1", "x.lock", "a/", "/a", "a:b", "x@{1}",
        ] {
            assert!(GitRefName::try_from(bad.to_string()).is_err(), "{bad}");
        }
    }

    #[test]
    fn git_paths() {
        assert!(GitPath::try_from("src/a.rs".to_string()).is_ok());
        assert!(GitPath::try_from("-weird name.txt".to_string()).is_ok());
        for bad in ["", "/etc/passwd", "../x", "a/../../b", "a\0b"] {
            assert!(GitPath::try_from(bad.to_string()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn messages_and_checks() {
        assert!(GitMessage::try_from("  \n".to_string()).is_err());
        assert!(GitMessage::try_from("fix".to_string()).is_ok());
        assert!(GitCmd::Stage {
            paths: vec![],
            all: false
        }
        .check()
        .is_err());
        assert!(GitCmd::Stage {
            paths: vec![],
            all: true
        }
        .check()
        .is_ok());
        assert!(GitCmd::Discard {
            paths: vec![],
            confirm: true
        }
        .check()
        .is_err());
        assert!(GitCmd::Log {
            skip: 0,
            limit: Some(0)
        }
        .check()
        .is_err());
        assert!(GitCmd::Log {
            skip: 0,
            limit: Some(50)
        }
        .check()
        .is_ok());
        let cmd: GitCmd = serde_json::from_str(r#"{"action":"pull"}"#).unwrap();
        assert_eq!(cmd, GitCmd::Pull { rebase: false });
        assert!(cmd.is_network());
    }
}
