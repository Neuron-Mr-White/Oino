//! Path resolution of tool arguments (pi `tools/path-utils.ts` + `utils/paths.ts`).

use unicode_normalization::UnicodeNormalization;

use crate::env::{ExecutionEnv, path_exists};
use crate::node_path;

/// Narrow no-break space macOS puts before `AM`/`PM` in screenshot names.
const NARROW_NO_BREAK_SPACE: char = '\u{202F}';

/// An argument that cannot be turned into a path (Node `fileURLToPath` failures).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// `file://host/...` with a host other than `localhost`.
    #[error("File URL host must be \"localhost\" or empty on {0}")]
    FileUrlHost(&'static str),
    /// `%2F` (or `%5C` on Windows) inside a file URL path.
    #[error("File URL path must not include encoded / characters")]
    EncodedSeparator,
}

fn is_unicode_space(ch: char) -> bool {
    matches!(
        ch,
        '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn home_dir() -> String {
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok())
        .unwrap_or_else(|| "/".to_owned())
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        std::env::consts::OS
    }
}

/// Converts Git Bash / MSYS / Cygwin / WSL drive paths (`/c/x`, `/mnt/c/x`) to `C:\x`.
#[cfg(windows)]
fn normalize_windows_shell_path(path: &str) -> String {
    if !path.starts_with('/') || path.starts_with("//") || path.contains('\\') {
        return path.to_owned();
    }
    let rest = path
        .strip_prefix("/mnt/")
        .or_else(|| path.strip_prefix("/cygdrive/"))
        .unwrap_or(&path[1..]);
    let mut chars = rest.chars();
    let Some(drive) = chars.next().filter(char::is_ascii_alphabetic) else {
        return path.to_owned();
    };
    let tail = chars.as_str();
    if !(tail.is_empty() || tail.starts_with('/')) {
        return path.to_owned();
    }
    let suffix = tail.strip_prefix('/').unwrap_or("").replace('/', "\\");
    format!("{}:\\{suffix}", drive.to_ascii_uppercase())
}

fn percent_decode(text: &str) -> Result<String, PathError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let value = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = value {
                if value == b'/' || (cfg!(windows) && value == b'\\') {
                    return Err(PathError::EncodedSeparator);
                }
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Node `fileURLToPath` for `file://` URLs.
fn file_url_to_path(url: &str) -> Result<String, PathError> {
    let rest = &url["file://".len()..];
    let (host, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let path = path.split(['?', '#']).next().unwrap_or(path);
    if !host.is_empty() && host != "localhost" && !cfg!(windows) {
        return Err(PathError::FileUrlHost(platform_name()));
    }
    percent_decode(path)
}

/// pi `normalizePath(path, {normalizeUnicodeSpaces, stripAtPrefix})` (`expandPath`): unicode
/// spaces → space, leading `@` stripped, `~` expanded, `file://` URLs converted.
///
/// # Errors
/// Invalid `file://` URLs.
pub fn expand_path(input: &str) -> Result<String, PathError> {
    normalize(input, true)
}

fn normalize(input: &str, tool_argument: bool) -> Result<String, PathError> {
    let mut normalized = if tool_argument {
        input
            .chars()
            .map(|ch| if is_unicode_space(ch) { ' ' } else { ch })
            .collect::<String>()
    } else {
        input.to_owned()
    };
    if tool_argument && let Some(stripped) = normalized.strip_prefix('@') {
        normalized = stripped.to_owned();
    }
    #[cfg(windows)]
    {
        normalized = normalize_windows_shell_path(&normalized);
    }
    if normalized == "~" {
        return Ok(home_dir());
    }
    if let Some(rest) = normalized.strip_prefix("~/").or_else(|| {
        cfg!(windows)
            .then(|| normalized.strip_prefix("~\\"))
            .flatten()
    }) {
        return Ok(node_path::join(&home_dir(), rest));
    }
    if normalized.starts_with("file://") {
        return file_url_to_path(&normalized);
    }
    Ok(normalized)
}

/// pi `resolveToCwd`: [`expand_path`], then resolved against `cwd` (Node `path.resolve`).
///
/// # Errors
/// Invalid `file://` URLs.
pub fn resolve_to_cwd(path: &str, cwd: &str) -> Result<String, PathError> {
    let normalized = expand_path(path)?;
    let base = normalize(cwd, false)?;
    let base = node_path::resolve("/", &base);
    Ok(node_path::resolve(&base, &normalized))
}

fn macos_screenshot_variant(path: &str) -> String {
    // pi: replace(/ (AM|PM)\./gi, "\u202F$1.")
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(index) = rest.find(' ') {
        out.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let marker = after.get(..3).filter(|candidate| {
            let upper = candidate.to_ascii_uppercase();
            upper == "AM." || upper == "PM."
        });
        if let Some(marker) = marker {
            out.push(NARROW_NO_BREAK_SPACE);
            out.push_str(marker);
            rest = &after[3..];
        } else {
            out.push(' ');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn nfd_variant(path: &str) -> String {
    path.nfd().collect()
}

fn curly_quote_variant(path: &str) -> String {
    path.replace('\'', "\u{2019}")
}

/// pi `resolveReadPathAsync`: [`resolve_to_cwd`], then the first existing variant among the
/// macOS screenshot `AM`/`PM` narrow-NBSP form, NFD, curly apostrophe and NFD + curly
/// apostrophe; the resolved path when none exists.
///
/// # Errors
/// Invalid `file://` URLs.
pub async fn resolve_read_path(
    env: &dyn ExecutionEnv,
    path: &str,
    cwd: &str,
) -> Result<String, PathError> {
    let resolved = resolve_to_cwd(path, cwd)?;
    if path_exists(env, &resolved).await {
        return Ok(resolved);
    }
    let nfd = nfd_variant(&resolved);
    let candidates = [
        macos_screenshot_variant(&resolved),
        nfd.clone(),
        curly_quote_variant(&resolved),
        curly_quote_variant(&nfd),
    ];
    for candidate in candidates {
        if candidate != resolved && path_exists(env, &candidate).await {
            return Ok(candidate);
        }
    }
    Ok(resolved)
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn expands_and_strips() {
        assert_eq!(expand_path("@a\u{a0}b").unwrap(), "a b");
        assert_eq!(resolve_to_cwd("x/../y", "/w").unwrap(), "/w/y");
        assert_eq!(resolve_to_cwd("/abs/", "/w").unwrap(), "/abs");
        assert_eq!(resolve_to_cwd("file:///a%20b/c", "/w").unwrap(), "/a b/c");
        assert!(resolve_to_cwd("file://host/a", "/w").is_err());
    }

    #[test]
    fn screenshot_variant() {
        assert_eq!(
            macos_screenshot_variant("/s/Shot 1.2.3 pm.png"),
            "/s/Shot 1.2.3\u{202f}pm.png"
        );
        assert_eq!(macos_screenshot_variant("/a b"), "/a b");
    }
}
