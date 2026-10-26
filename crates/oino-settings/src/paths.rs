//! Filesystem layout and path helpers (pi `config.ts` + `utils/paths.ts`).
//!
//! The agent dir is `~/.oino` (pi `~/.pi/agent`), overridable with [`env::OINO_AGENT_DIR`];
//! the project dir is `<cwd>/.oino` (pi `.pi`, at cwd, never the git root).

use std::path::{Component, Path, PathBuf};

/// Project configuration directory name (pi `CONFIG_DIR_NAME` = `.pi`).
pub const CONFIG_DIR_NAME: &str = ".oino";
/// Agent directory name under the home directory (`~/.oino`).
pub const AGENT_DIR_NAME: &str = ".oino";
/// Settings file name (agent dir and project dir).
pub const SETTINGS_FILE: &str = "settings.json";
/// Keybindings file name (agent dir only).
pub const KEYBINDINGS_FILE: &str = "keybindings.json";
/// Saved project trust decisions (agent dir only).
pub const TRUST_FILE: &str = "trust.json";
/// System prompt replacement file.
pub const SYSTEM_PROMPT_FILE: &str = "SYSTEM.md";
/// System prompt addendum file.
pub const APPEND_SYSTEM_PROMPT_FILE: &str = "APPEND_SYSTEM.md";
/// Suffix of the advisory lock file next to a locked file (`settings.json.lock`).
pub const LOCK_SUFFIX: &str = ".lock";

/// Process environment variable names read by the runtime (runtime §9.5, D12).
pub mod env {
    /// Agent dir override (pi `PI_CODING_AGENT_DIR`).
    pub const OINO_AGENT_DIR: &str = "OINO_AGENT_DIR";
    /// Session dir override (pi `PI_CODING_AGENT_SESSION_DIR`).
    pub const OINO_SESSION_DIR: &str = "OINO_SESSION_DIR";
    /// Offline mode (pi `PI_OFFLINE`).
    pub const OINO_OFFLINE: &str = "OINO_OFFLINE";
    /// Prompt cache retention (`long`; pi `PI_CACHE_RETENTION`).
    pub const OINO_CACHE_RETENTION: &str = "OINO_CACHE_RETENTION";
}

/// Path resolution failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PathError {
    /// Neither `$HOME` nor the platform home directory is known.
    #[error("cannot determine the home directory")]
    NoHomeDir,
}

/// The user's home directory: `$HOME` when set and non-empty, else the platform default
/// (pi `process.env.HOME || homedir()`).
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => Some(PathBuf::from(home)),
        _ => dirs::home_dir(),
    }
}

/// The agent dir: `$OINO_AGENT_DIR` (tilde-expanded) or `~/.oino` (pi `getAgentDir`).
///
/// # Errors
///
/// [`PathError::NoHomeDir`] when the override is unset and no home directory is known.
pub fn agent_dir() -> Result<PathBuf, PathError> {
    let env_value = std::env::var(env::OINO_AGENT_DIR).ok();
    agent_dir_from(env_value.as_deref(), home_dir().as_deref())
}

/// [`agent_dir`] with explicit inputs (testable; an empty override counts as unset).
///
/// # Errors
///
/// [`PathError::NoHomeDir`] when `env_value` is unset/empty and `home` is `None`.
pub fn agent_dir_from(env_value: Option<&str>, home: Option<&Path>) -> Result<PathBuf, PathError> {
    match env_value {
        Some(value) if !value.is_empty() => Ok(normalize_path(value, home)),
        _ => home
            .map(|home| home.join(AGENT_DIR_NAME))
            .ok_or(PathError::NoHomeDir),
    }
}

/// The project config dir `<cwd>/.oino`.
#[must_use]
pub fn project_dir(cwd: &Path) -> PathBuf {
    cwd.join(CONFIG_DIR_NAME)
}

/// pi `normalizePath`: expand a leading `~` / `~/` against `home` and turn `file://` URLs into
/// paths. Everything else is returned unchanged (relative paths stay relative).
#[must_use]
pub fn normalize_path(input: &str, home: Option<&Path>) -> PathBuf {
    if let Some(home) = home {
        if input == "~" {
            return home.to_path_buf();
        }
        if let Some(rest) = input.strip_prefix("~/") {
            return home.join(rest);
        }
        #[cfg(windows)]
        if let Some(rest) = input.strip_prefix("~\\") {
            return home.join(rest);
        }
    }
    if let Some(rest) = input.strip_prefix("file://") {
        return PathBuf::from(percent_decode(rest));
    }
    PathBuf::from(input)
}

/// pi `resolvePath`: [`normalize_path`], then resolve against `base` and normalise `.`/`..`
/// lexically (Node `path.resolve`). With `trim`, surrounding whitespace is removed first.
#[must_use]
pub fn resolve_path(input: &str, base: &Path, home: Option<&Path>, trim: bool) -> PathBuf {
    let input = if trim { input.trim() } else { input };
    let normalized = normalize_path(input, home);
    if normalized.is_absolute() {
        lexical_normalize(&normalized)
    } else {
        lexical_normalize(&base.join(normalized))
    }
}

/// Normalise `.` and `..` components without touching the filesystem (Node `path.resolve`
/// semantics: `..` never climbs above the root).
#[must_use]
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !path.is_absolute() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// pi `canonicalizePath`: the real path, or `path` unchanged when it cannot be resolved.
#[must_use]
pub fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `target` equals `root` or lies below it (pi `isUnderPath`, lexical).
#[must_use]
pub fn is_under_path(target: &Path, root: &Path) -> bool {
    target.starts_with(root)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes
                    .get(index + 1)
                    .and_then(|b| char::from(*b).to_digit(16)),
                bytes
                    .get(index + 2)
                    .and_then(|b| char::from(*b).to_digit(16)),
            )
        {
            // Two hex digits always fit in one byte.
            out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_dir_prefers_non_empty_override() {
        let home = Path::new("/home/u");
        assert_eq!(
            agent_dir_from(Some("~/x"), Some(home)).unwrap(),
            PathBuf::from("/home/u/x")
        );
        assert_eq!(
            agent_dir_from(Some(""), Some(home)).unwrap(),
            PathBuf::from("/home/u/.oino")
        );
        assert_eq!(agent_dir_from(None, None), Err(PathError::NoHomeDir));
    }

    #[test]
    fn resolve_path_matches_node_resolve() {
        let home = Some(Path::new("/h"));
        let base = Path::new("/a/b");
        assert_eq!(
            resolve_path("c/../d", base, home, false),
            PathBuf::from("/a/b/d")
        );
        assert_eq!(
            resolve_path(" ~/s ", base, home, true),
            PathBuf::from("/h/s")
        );
        assert_eq!(
            resolve_path("/x/./y/..", base, home, false),
            PathBuf::from("/x")
        );
        assert_eq!(
            resolve_path("../../..", base, home, false),
            PathBuf::from("/")
        );
        assert_eq!(
            resolve_path("file:///p%20q", base, home, false),
            PathBuf::from("/p q")
        );
    }

    #[test]
    fn normalize_keeps_other_tildes() {
        assert_eq!(
            normalize_path("~user/x", Some(Path::new("/h"))),
            PathBuf::from("~user/x")
        );
    }
}
