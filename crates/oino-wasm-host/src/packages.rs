//! Package discovery: `<dir>/<package>/extension.json` + component (extension-api §7.1).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oino_extension_api::manifest::{Manifest, ManifestError};
use oino_extension_api::{ComponentSource, LoadedPackage};

/// A package that failed validation.
#[derive(Debug)]
pub struct PackageProblem {
    /// Package directory.
    pub path: PathBuf,
    /// Why it was skipped.
    pub error: ManifestError,
}

/// Loads the package rooted at `root`.
///
/// # Errors
///
/// Manifest or file validation failures.
pub fn load_dir(root: &Path, bundled: bool) -> Result<LoadedPackage, ManifestError> {
    let manifest = Manifest::load_package(root)?;
    let component = ComponentSource::Path(manifest.component_path(root));
    Ok(LoadedPackage {
        manifest,
        component,
        root: Some(root.to_path_buf()),
        bundled,
    })
}

/// A bundled package from its manifest text and component bytes (resources extracted to
/// `root` when present).
///
/// # Errors
///
/// Manifest validation failures.
pub fn from_bytes(
    manifest: &str,
    component: Arc<[u8]>,
    root: Option<PathBuf>,
) -> Result<LoadedPackage, ManifestError> {
    let manifest = Manifest::parse(manifest)?;
    Ok(LoadedPackage {
        manifest,
        component: ComponentSource::Bytes(component),
        root,
        bundled: true,
    })
}

/// Discovers packages in every directory of `dirs` (each child with an `extension.json`),
/// sorted by directory name for a stable load order. Later duplicates of an id are skipped.
#[must_use]
pub fn discover(dirs: &[PathBuf], bundled: bool) -> (Vec<LoadedPackage>, Vec<PackageProblem>) {
    let mut packages: Vec<LoadedPackage> = Vec::new();
    let mut problems = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut roots: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.join(oino_extension_api::manifest::MANIFEST_FILE)
                    .is_file()
            })
            .collect();
        roots.sort();
        for root in roots {
            match load_dir(&root, bundled) {
                Ok(pkg) if packages.iter().any(|p| p.manifest.id == pkg.manifest.id) => {}
                Ok(pkg) => packages.push(pkg),
                Err(error) => problems.push(PackageProblem { path: root, error }),
            }
        }
    }
    (packages, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_valid_and_reports_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("a");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("extension.json"),
            r#"{"apiVersion":"oino.extension/1","id":"x.a","name":"A","version":"1.0.0"}"#,
        )
        .unwrap();
        std::fs::write(good.join("component.wasm"), b"").unwrap();
        let bad = dir.path().join("b");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("extension.json"), "{}").unwrap();
        let (pkgs, problems) = discover(&[dir.path().to_path_buf()], false);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].manifest.id.as_str(), "x.a");
        assert_eq!(problems.len(), 1);
    }
}
