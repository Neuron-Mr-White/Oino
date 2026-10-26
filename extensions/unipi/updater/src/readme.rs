//! README and changelog discovery (unipi `src/readme.ts`, `resolveChangelogPath`).
//!
//! unipi walked its npm package tree. Oino extracts bundled packages to
//! `~/.oino/bundled/<id>@<version>-<hash8>/` (extension-api §8), visible here read-only as
//! `/agent/bundled` (`fs.agentDir: read`); user-installed packages are under
//! `/agent/extensions`. The docs pseudo-package `oino.docs` carries the repo README (and
//! `CHANGELOG.md` when present).

use std::path::{Path, PathBuf};

use unipi_kit::fmt::compare_versions;

/// Docs pseudo-package id.
pub(crate) const DOCS_ID: &str = "oino.docs";

/// One README (unipi `ReadmeEntry`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Readme {
    /// Short name (`workflow`, `oino` for the docs package).
    pub name: String,
    /// Extension id (`unipi.workflow`).
    pub id: String,
    /// Installed version.
    pub version: String,
    /// Guest path of the README.
    pub path: PathBuf,
}

/// `<id>@<version>-<hash8>` → `(id, version)`.
pub(crate) fn parse_dir_name(name: &str) -> Option<(String, String)> {
    let (id, rest) = name.rsplit_once('@')?;
    if id.is_empty() || rest.is_empty() {
        return None;
    }
    let version = match rest.rsplit_once('-') {
        Some((v, hash)) if hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit()) => v,
        _ => rest,
    };
    Some((id.to_owned(), version.to_owned()))
}

/// Short display name of an extension id.
pub(crate) fn short_name(id: &str) -> String {
    if id == DOCS_ID {
        return "oino".to_owned();
    }
    id.strip_prefix("unipi.")
        .or_else(|| id.strip_prefix("oino."))
        .unwrap_or(id)
        .to_owned()
}

fn readme_in(dir: &Path) -> Option<PathBuf> {
    ["README.md", "docs/README.md", "readme.md", "Readme.md"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
}

/// Package dirs `(id, version, dir)` under `root`, newest version per id.
pub(crate) fn package_dirs(root: &Path) -> Vec<(String, String, PathBuf)> {
    let mut out: Vec<(String, String, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in rd.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (id, version) = parse_dir_name(&name).unwrap_or_else(|| (name.clone(), String::new()));
        match out.iter_mut().find(|(i, _, _)| *i == id) {
            Some(slot) if compare_versions(&version, &slot.1).is_gt() => {
                *slot = (id, version, dir);
            }
            Some(_) => {}
            None => out.push((id, version, dir)),
        }
    }
    out
}

/// unipi `discoverReadmes`: the docs README first, then every package README by name.
pub(crate) fn discover(roots: &[PathBuf]) -> Vec<Readme> {
    let mut out: Vec<Readme> = Vec::new();
    for root in roots {
        for (id, version, dir) in package_dirs(root) {
            if out.iter().any(|r| r.id == id) {
                continue;
            }
            if let Some(path) = readme_in(&dir) {
                out.push(Readme {
                    name: short_name(&id),
                    id,
                    version,
                    path,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        (b.id == DOCS_ID)
            .cmp(&(a.id == DOCS_ID))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// unipi `resolveReadmePath`: empty/`oino`/`unipi` → the docs README; else by short name or id.
pub(crate) fn resolve<'a>(readmes: &'a [Readme], name: &str) -> Option<&'a Readme> {
    let n = name.trim();
    if n.is_empty() || n == "oino" || n == "unipi" {
        return readmes.iter().find(|r| r.id == DOCS_ID);
    }
    readmes
        .iter()
        .find(|r| r.name == n || r.id == n || r.id.ends_with(&format!(".{n}")))
}

/// Bundled `CHANGELOG.md` (docs package first, then any package root).
pub(crate) fn changelog_path(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        let dirs = package_dirs(root);
        let docs_first = dirs
            .iter()
            .filter(|(id, _, _)| id == DOCS_ID)
            .chain(dirs.iter().filter(|(id, _, _)| id != DOCS_ID));
        for (_, _, dir) in docs_first {
            let p = dir.join("CHANGELOG.md");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Search roots (`/agent/bundled`, `/agent/extensions`).
pub(crate) fn roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/agent/bundled"),
        PathBuf::from("/agent/extensions"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_names() {
        assert_eq!(
            parse_dir_name("unipi.workflow@3.0.0-1a2b3c4d"),
            Some(("unipi.workflow".into(), "3.0.0".into()))
        );
        assert_eq!(
            parse_dir_name("oino.docs@1.0.0-alpha.2-deadbeef"),
            Some(("oino.docs".into(), "1.0.0-alpha.2".into()))
        );
        assert_eq!(
            parse_dir_name("x@1.0.0-alpha"),
            Some(("x".into(), "1.0.0-alpha".into()))
        );
        assert_eq!(parse_dir_name("plain"), None);
        assert_eq!(short_name("unipi.web-api"), "web-api");
        assert_eq!(short_name("oino.docs"), "oino");
        assert_eq!(short_name("acme.tool"), "acme.tool");
    }

    #[test]
    fn discovers_readmes_on_disk() {
        let root =
            std::env::temp_dir().join(format!("unipi-updater-readme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bundled = root.join("bundled");
        for (dir, readme) in [
            ("oino.docs@1.0.0-aaaaaaaa", Some("README.md")),
            ("unipi.workflow@3.0.0-bbbbbbbb", Some("docs/README.md")),
            ("unipi.workflow@2.0.0-cccccccc", Some("README.md")),
            ("unipi.mcp@3.0.0-dddddddd", None),
        ] {
            let d = bundled.join(dir);
            let _ = std::fs::create_dir_all(d.join("docs"));
            if let Some(r) = readme {
                let _ = std::fs::write(d.join(r), format!("# {dir}"));
            }
        }
        let _ = std::fs::write(
            bundled.join("oino.docs@1.0.0-aaaaaaaa/CHANGELOG.md"),
            "## [1.0.0]",
        );
        let found = discover(std::slice::from_ref(&bundled));
        let names: Vec<&str> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["oino", "workflow"],
            "docs first; no README → skipped"
        );
        assert_eq!(found[1].version, "3.0.0", "newest version wins");
        assert!(found[1].path.ends_with("docs/README.md"));
        assert_eq!(
            resolve(&found, "").map(|r| r.id.as_str()),
            Some("oino.docs")
        );
        assert_eq!(
            resolve(&found, "unipi.workflow").map(|r| r.name.as_str()),
            Some("workflow")
        );
        assert_eq!(
            resolve(&found, "workflow").map(|r| r.name.as_str()),
            Some("workflow")
        );
        assert!(resolve(&found, "nope").is_none());
        assert!(
            changelog_path(&[bundled])
                .is_some_and(|p| p.ends_with("oino.docs@1.0.0-aaaaaaaa/CHANGELOG.md"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
