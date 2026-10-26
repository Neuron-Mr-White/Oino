//! Node `path` semantics on strings (`resolve`, `join`, `dirname`, `basename`, `relative`).
//!
//! The tools report paths in text the model reads, so they keep pi's lexical path handling
//! instead of `std::path` (which neither collapses `..` nor strips trailing separators).
//! POSIX rules apply on every platform; on Windows `\` is also accepted as a separator and a
//! drive prefix (`C:`) counts as absolute.

/// The platform separator Node prints.
#[cfg(windows)]
pub(crate) const SEP: char = '\\';
/// The platform separator Node prints.
#[cfg(not(windows))]
pub(crate) const SEP: char = '/';

fn is_sep(ch: char) -> bool {
    ch == '/' || (cfg!(windows) && ch == '\\')
}

/// Node `path.isAbsolute`.
#[must_use]
pub(crate) fn is_absolute(path: &str) -> bool {
    if path.starts_with(is_sep) {
        return true;
    }
    if cfg!(windows) {
        let bytes = path.as_bytes();
        return bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && is_sep(char::from(bytes[2]));
    }
    false
}

/// Splits a leading root (`/` or `C:\`) from the rest.
fn split_root(path: &str) -> (&str, &str) {
    if cfg!(windows) {
        let bytes = path.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            let rest = &path[2..];
            if rest.starts_with(is_sep) {
                return (&path[..3], &path[3..]);
            }
            return (&path[..2], rest);
        }
    }
    if path.starts_with(is_sep) {
        (&path[..1], &path[1..])
    } else {
        ("", path)
    }
}

/// Lexically normalises the segments of an absolute path (`.` dropped, `..` collapsed).
fn normalize_absolute(root: &str, rest: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in rest.split(is_sep) {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let root = if cfg!(windows) {
        root.replace('/', "\\")
    } else {
        root.to_owned()
    };
    let sep = SEP.to_string();
    format!("{root}{}", parts.join(&sep))
}

/// Node `path.resolve(base, path)` where `base` is absolute (the tool cwd).
#[must_use]
pub(crate) fn resolve(base: &str, path: &str) -> String {
    if is_absolute(path) {
        let (root, rest) = split_root(path);
        return normalize_absolute(root, rest);
    }
    let (root, rest) = split_root(base);
    let combined = format!("{rest}/{path}");
    normalize_absolute(if root.is_empty() { "/" } else { root }, &combined)
}

/// Node `path.join(a, b)` for an absolute `a`.
#[must_use]
pub(crate) fn join(base: &str, child: &str) -> String {
    let (root, rest) = split_root(base);
    normalize_absolute(
        if root.is_empty() { "/" } else { root },
        &format!("{rest}/{child}"),
    )
}

/// Node `path.dirname` for an absolute, normalised path.
#[must_use]
pub(crate) fn dirname(path: &str) -> String {
    let (root, rest) = split_root(path);
    match rest.rfind(is_sep) {
        Some(index) => format!("{root}{}", &rest[..index]),
        None => root.to_owned(),
    }
}

/// Node `path.basename`.
#[must_use]
pub(crate) fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(is_sep);
    trimmed.rsplit(is_sep).next().unwrap_or(trimmed).to_owned()
}

/// Node `path.relative(from, to)` for absolute paths.
#[must_use]
pub(crate) fn relative(from: &str, to: &str) -> String {
    let from = resolve("/", from);
    let to = resolve("/", to);
    if from == to {
        return String::new();
    }
    let from_parts: Vec<&str> = split_root(&from)
        .1
        .split(is_sep)
        .filter(|s| !s.is_empty())
        .collect();
    let to_parts: Vec<&str> = split_root(&to)
        .1
        .split(is_sep)
        .filter(|s| !s.is_empty())
        .collect();
    let common = from_parts
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = vec![".."; from_parts.len() - common];
    out.extend(&to_parts[common..]);
    let sep = SEP.to_string();
    out.join(&sep)
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn resolve_normalises() {
        assert_eq!(resolve("/a/b", "c/../d/./e/"), "/a/b/d/e");
        assert_eq!(resolve("/a/b", "/x//y/.."), "/x");
        assert_eq!(resolve("/a", "../../.."), "/");
        assert_eq!(resolve("/a/", "."), "/a");
    }

    #[test]
    fn dir_and_base_names() {
        assert_eq!(dirname("/a/b/c"), "/a/b");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(basename("/a/b/c.txt"), "c.txt");
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative("/a/b", "/a/b/c/d"), "c/d");
        assert_eq!(relative("/a/b", "/a/x"), "../x");
        assert_eq!(relative("/a/b", "/a/b"), "");
    }
}
