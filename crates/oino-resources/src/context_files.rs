//! Project context files (pi `loadProjectContextFiles`, runtime §9.2).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use oino_settings::paths::canonicalize_or_self;
use serde::{Deserialize, Serialize};

use crate::diagnostics::ResourceDiagnostic;

/// Per-directory candidates, first existing file wins (pi `loadContextFileFromDir`).
pub const CONTEXT_FILE_CANDIDATES: [&str; 5] = [
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

/// A loaded context file (pi `{path, content}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFile {
    /// File path.
    pub path: PathBuf,
    /// Content with a leading BOM removed.
    pub content: String,
}

/// Context files plus read warnings (pi logs these to stderr).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextFiles {
    /// Agent-dir file first, then ancestors from the filesystem root down to cwd.
    pub files: Vec<ContextFile>,
    /// `Could not read …` problems.
    pub diagnostics: Vec<ResourceDiagnostic>,
}

fn load_from_dir(dir: &Path, diagnostics: &mut Vec<ResourceDiagnostic>) -> Option<ContextFile> {
    for name in CONTEXT_FILE_CANDIDATES {
        let path = dir.join(name);
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(content) => {
                let content = content
                    .strip_prefix('\u{feff}')
                    .map_or(content.clone(), str::to_owned);
                return Some(ContextFile { path, content });
            }
            Err(error) => diagnostics.push(ResourceDiagnostic::warning(
                format!("Could not read {}: {error}", path.display()),
                &path,
            )),
        }
    }
    None
}

/// Git layout around `cwd` (pi `findGitPaths`).
struct GitPaths {
    repo_dir: PathBuf,
    common_git_dir: PathBuf,
}

fn find_git_paths(cwd: &Path) -> Option<GitPaths> {
    let mut dir = cwd.to_path_buf();
    loop {
        let git = dir.join(".git");
        if let Ok(metadata) = fs::metadata(&git) {
            if metadata.is_file() {
                let content = fs::read_to_string(&git).ok()?;
                let target = content.trim().strip_prefix("gitdir: ")?;
                let git_dir = oino_settings::paths::lexical_normalize(&dir.join(target.trim()));
                if !git_dir.join("HEAD").exists() {
                    return None;
                }
                let common = match fs::read_to_string(git_dir.join("commondir")) {
                    Ok(common) => {
                        oino_settings::paths::lexical_normalize(&git_dir.join(common.trim()))
                    }
                    Err(_) => git_dir,
                };
                return Some(GitPaths {
                    repo_dir: dir,
                    common_git_dir: common,
                });
            }
            if metadata.is_dir() {
                if !git.join("HEAD").exists() {
                    return None;
                }
                return Some(GitPaths {
                    repo_dir: dir,
                    common_git_dir: git,
                });
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// pi `findShadowedContextFile`: the main repo's context file shadowed by a nested linked
/// worktree's own copy (canonical path), if any.
fn find_shadowed_context_file(cwd: &Path) -> Option<PathBuf> {
    let git = find_git_paths(cwd)?;
    let common = canonicalize_or_self(&git.common_git_dir);
    let worktree_root = canonicalize_or_self(&git.repo_dir);
    let main_root = common.parent()?.to_path_buf();
    if worktree_root == main_root || !worktree_root.starts_with(&main_root) {
        return None;
    }
    if canonicalize_or_self(&main_root.join(".git")) != common {
        return None;
    }
    let worktree_file = load_from_dir(&worktree_root, &mut Vec::new())?;
    Some(main_root.join(worktree_file.path.file_name()?))
}

/// pi `loadProjectContextFiles`: the agent-dir file, then one file per ancestor directory from
/// the filesystem root down to `cwd`; a path is loaded once and a nested git worktree's file
/// shadows the main repository's. `cwd` and `agent_dir` must be absolute.
#[must_use]
pub fn load_project_context_files(cwd: &Path, agent_dir: &Path) -> ContextFiles {
    let mut diagnostics = Vec::new();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    if let Some(global) = load_from_dir(agent_dir, &mut diagnostics) {
        seen.insert(global.path.clone());
        files.push(global);
    }
    let shadowed = find_shadowed_context_file(cwd);
    let mut ancestors = Vec::new();
    for dir in cwd.ancestors() {
        let Some(file) = load_from_dir(dir, &mut diagnostics) else {
            continue;
        };
        let is_shadowed = shadowed.as_ref().is_some_and(|shadowed| {
            file.path
                .file_name()
                .is_some_and(|name| &canonicalize_or_self(dir).join(name) == shadowed)
        });
        if !is_shadowed && seen.insert(file.path.clone()) {
            ancestors.push(file);
        }
    }
    ancestors.reverse();
    files.extend(ancestors);
    ContextFiles { files, diagnostics }
}
