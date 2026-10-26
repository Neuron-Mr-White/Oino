//! Host paths ↔ guest paths through the granted preopens.
//!
//! Users and the agent speak host paths (`/home/u/Downloads/a.png`, `./shot.png`); the guest
//! reaches them only through preopens (`/project`, `/home-downloads`, `/host-tmp`, …). Paths
//! outside every preopen are unreachable (see deviations: arbitrary host reads).

use std::path::{Component, Path, PathBuf};

/// `(guest root, host root)` pairs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roots(pub Vec<(String, String)>);

impl Roots {
    /// The live preopens.
    #[must_use]
    pub fn current() -> Self {
        Self(
            oino_extension::fs::preopens()
                .into_iter()
                .map(|p| (p.guest_path, p.host_path))
                .collect(),
        )
    }

    /// Guest path of a host path (longest host root wins).
    #[must_use]
    pub fn to_guest(&self, host: &str) -> Option<PathBuf> {
        let host = normalize(Path::new(host));
        self.0
            .iter()
            .filter_map(|(g, h)| {
                let rest = host.strip_prefix(normalize(Path::new(h))).ok()?;
                Some((h.len(), Path::new(g).join(rest)))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, p)| p)
    }

    /// Host path of a guest path (longest guest root wins).
    #[must_use]
    pub fn to_host(&self, guest: &Path) -> Option<PathBuf> {
        unipi_kit::paths::map_to_host(&self.0, guest)
    }
}

/// Lexical normalisation (`.` dropped, `..` pops).
#[must_use]
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Absolute host path of `input` (`~/…` → `home`, relative → `cwd`).
#[must_use]
pub fn absolute(input: &str, cwd: &str, home: Option<&str>) -> String {
    let expanded = match (input.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => format!("{}/{rest}", h.trim_end_matches('/')),
        _ if input == "~" => home.unwrap_or(input).to_owned(),
        _ => input.to_owned(),
    };
    let p = Path::new(&expanded);
    let joined = if p.is_absolute() || expanded.starts_with('/') {
        p.to_path_buf()
    } else {
        Path::new(cwd).join(p)
    };
    normalize(&joined).to_string_lossy().into_owned()
}

/// `$HOME` (permission `env`).
#[must_use]
pub fn home() -> Option<String> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|k| oino_extension::host::env(k).ok().flatten())
        .filter(|h| !h.is_empty())
}

/// `~`-shortened display path (`./rel` inside `cwd`).
#[must_use]
pub fn short_path(p: &str, cwd: &str, home: Option<&str>) -> String {
    if p.is_empty() {
        return String::new();
    }
    let abs = absolute(p, cwd, home);
    if let Ok(rel) = Path::new(&abs).strip_prefix(cwd)
        && !cwd.is_empty()
    {
        let rel = rel.to_string_lossy();
        return if rel.is_empty() {
            ".".into()
        } else {
            format!("./{rel}")
        };
    }
    match home {
        Some(h) if !h.is_empty() && abs.starts_with(h) => format!("~{}", &abs[h.len()..]),
        _ => abs,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        Roots(vec![
            ("/project".into(), "/work/app".into()),
            ("/home-downloads".into(), "/home/u/Downloads".into()),
            (
                "/ext-global".into(),
                "/home/u/.oino/state/unipi.utility/global".into(),
            ),
        ])
    }

    #[test]
    fn maps_both_ways() {
        let r = roots();
        assert_eq!(
            r.to_guest("/work/app/src/a.png"),
            Some(PathBuf::from("/project/src/a.png"))
        );
        assert_eq!(
            r.to_guest("/home/u/Downloads/x y.png"),
            Some(PathBuf::from("/home-downloads/x y.png"))
        );
        assert_eq!(r.to_guest("/etc/passwd"), None);
        assert_eq!(r.to_guest("/work/app/../secret"), None);
        assert_eq!(
            r.to_host(Path::new("/project/a")),
            Some(PathBuf::from("/work/app/a"))
        );
    }

    #[test]
    fn absolute_and_short_paths() {
        assert_eq!(absolute("./a/../b.png", "/w", None), "/w/b.png");
        assert_eq!(absolute("~/x.png", "/w", Some("/home/u")), "/home/u/x.png");
        assert_eq!(absolute("/abs", "/w", None), "/abs");
        assert_eq!(
            short_path("/w/src/a.rs", "/w", Some("/home/u")),
            "./src/a.rs"
        );
        assert_eq!(short_path("src/a.rs", "/w", None), "./src/a.rs");
        assert_eq!(short_path("/home/u/x", "/w", Some("/home/u")), "~/x");
        assert_eq!(short_path("/etc/x", "/w", Some("/home/u")), "/etc/x");
        assert_eq!(short_path("", "/w", None), "");
    }
}
