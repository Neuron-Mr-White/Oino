//! Directory walking helpers shared by discovery and loaders: sorted entries with followed
//! symlinks, and pi's prefixed `.gitignore`/`.ignore`/`.fdignore` matching.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Ignore files honoured during discovery (pi `IGNORE_FILE_NAMES`).
const IGNORE_FILE_NAMES: [&str; 3] = [".gitignore", ".ignore", ".fdignore"];

/// One directory entry with symlinks resolved (pi `Dirent` + `statSync` for links).
#[derive(Clone, Debug)]
pub(crate) struct DirEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
    pub(crate) is_file: bool,
}

/// Entries of `dir` sorted by name bytes (libuv `scandir` order). Broken symlinks are skipped;
/// an unreadable dir yields `None`.
pub(crate) fn read_dir_sorted(dir: &Path) -> Option<Vec<DirEntry>> {
    let mut entries: Vec<DirEntry> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let file_type = entry.file_type().ok()?;
            let (is_dir, is_file) = if file_type.is_symlink() {
                let meta = fs::metadata(&path).ok()?;
                (meta.is_dir(), meta.is_file())
            } else {
                (file_type.is_dir(), file_type.is_file())
            };
            Some(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path,
                is_dir,
                is_file,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    Some(entries)
}

/// Case-sensitive suffix test (pi `name.endsWith(".md")`; `.MD` files are not resources).
pub(crate) fn has_suffix(name: &str, suffix: &str) -> bool {
    name.ends_with(suffix)
}

/// `/`-separated path of `path` relative to `root` (pi `toPosixPath(relative(root, path))`).
pub(crate) fn posix_relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// pi's accumulated `ignore()` matcher: rules from nested ignore files are prefixed with their
/// directory relative to the walk root.
pub(crate) struct IgnoreMatcher {
    root: PathBuf,
    lines: Vec<String>,
    compiled: Option<Gitignore>,
}

impl IgnoreMatcher {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            lines: Vec::new(),
            compiled: None,
        }
    }

    /// pi `addIgnoreRules(ig, dir, root)`.
    pub(crate) fn add_rules(&mut self, dir: &Path) {
        let relative = posix_relative(&self.root, dir);
        let prefix = if relative.is_empty() {
            String::new()
        } else {
            format!("{relative}/")
        };
        for name in IGNORE_FILE_NAMES {
            let Ok(content) = fs::read_to_string(dir.join(name)) else {
                continue;
            };
            let before = self.lines.len();
            self.lines.extend(
                content
                    .split('\n')
                    .map(|line| line.strip_suffix('\r').unwrap_or(line))
                    .filter_map(|line| prefix_ignore_pattern(line, &prefix)),
            );
            if self.lines.len() != before {
                self.compiled = None;
            }
        }
    }

    /// Whether `relative` (posix, relative to the root) is ignored, including via a parent.
    pub(crate) fn ignores(&mut self, relative: &str, is_dir: bool) -> bool {
        if self.lines.is_empty() || relative.is_empty() || relative.starts_with('/') {
            return false;
        }
        if self.compiled.is_none() {
            let mut builder = GitignoreBuilder::new(&self.root);
            for line in &self.lines {
                // Invalid globs are skipped, like the `ignore` npm package does.
                let _ = builder.add_line(None, line);
            }
            self.compiled = builder.build().ok();
        }
        self.compiled.as_ref().is_some_and(|matcher| {
            matcher
                .matched_path_or_any_parents(Path::new(relative), is_dir)
                .is_ignore()
        })
    }
}

/// pi `prefixIgnorePattern`.
fn prefix_ignore_pattern(line: &str, prefix: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || (trimmed.starts_with('#') && !trimmed.starts_with("\\#")) {
        return None;
    }
    let (negated, pattern) = if let Some(rest) = line.strip_prefix('!') {
        (true, rest)
    } else if line.starts_with("\\!") {
        (false, &line[1..])
    } else {
        (false, line)
    };
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    let prefixed = format!("{prefix}{pattern}");
    Some(if negated {
        format!("!{prefixed}")
    } else {
        prefixed
    })
}
