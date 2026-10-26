//! Host-path helpers (node `path.resolve` / containment on POSIX-style host paths) and
//! host ↔ guest mapping over the preopens.

/// node `path.resolve(base, p)` for `/`-separated paths (`.`/`..` collapsed, no trailing `/`).
#[must_use]
pub fn resolve(base: &str, p: &str) -> String {
    let joined = if p.starts_with('/') || base.is_empty() {
        p.to_owned()
    } else {
        format!("{base}/{p}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `target` equals `root` or lies under it (both resolved).
#[must_use]
pub fn is_inside(target: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let root = resolve("/", root);
    let target = resolve("/", target);
    target == root || target.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// Last path segment (node `basename`).
#[must_use]
pub fn basename(p: &str) -> &str {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p)
}

/// Guest path of a host path inside a preopen (`/project/...`), or `None`.
#[must_use]
pub fn guest_of(roots: &[(String, String)], host: &str) -> Option<String> {
    roots
        .iter()
        .filter(|(_, h)| is_inside(host, h))
        .max_by_key(|(_, h)| h.len())
        .map(|(g, h)| {
            let rest = resolve("/", host)[resolve("/", h).len()..].to_owned();
            format!("{}{}", g.trim_end_matches('/'), rest)
        })
}

/// Preopen `(guest, host)` pairs.
#[must_use]
pub fn roots() -> Vec<(String, String)> {
    oino_extension::fs::preopens()
        .into_iter()
        .map(|p| (p.guest_path, p.host_path))
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn resolving() {
        assert_eq!(resolve("/w/p", "src/a.ts"), "/w/p/src/a.ts");
        assert_eq!(resolve("/w/p", "/etc/hosts"), "/etc/hosts");
        assert_eq!(resolve("/w/p", "../q/./x"), "/w/q/x");
        assert!(is_inside("/w/p/src", "/w/p"));
        assert!(is_inside("/w/p", "/w/p/"));
        assert!(!is_inside("/w/pq", "/w/p"));
        assert!(!is_inside("/w", ""));
        assert_eq!(basename("/home/me/project"), "project");
        let roots = vec![("/project".to_owned(), "/w/p".to_owned())];
        assert_eq!(
            guest_of(&roots, "/w/p/docs/x.md").as_deref(),
            Some("/project/docs/x.md")
        );
        assert_eq!(guest_of(&roots, "/other"), None);
    }
}
