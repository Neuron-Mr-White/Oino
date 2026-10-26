//! `oino-bundle`: the static table of extension components shipped inside the `oino` binary
//! and Oino Desktop (D14, extension-api §8).
//!
//! `build.rs` reads `extensions/dist/` (override with `OINO_BUNDLE_DIST`), produced by
//! `cargo xtask wasm`. Components are stored zstd-compressed and decompressed on demand;
//! resources (skills, prompts, themes, docs) are extracted once to
//! `~/.oino/bundled/<id>@<version>-<hash8>/` so tools can read them from disk.
//!
//! | Item | Use |
//! |---|---|
//! | [`BUNDLED`] | every bundled extension in load order: providers, unipi `LOAD_ORDER`, others (features `providers`, `unipi`, `fixtures`) |
//! | [`BundledExtension::component`] | decompressed component bytes |
//! | [`BundledExtension::extract`] | resources on disk (atomic directory rename) |
//! | [`DOCS`] / [`BundledDocs::extract`] | `README.md` + `docs/*.md`, the system prompt docs root |

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

/// One bundled extension.
#[derive(Debug)]
pub struct BundledExtension {
    /// Manifest id.
    pub id: &'static str,
    /// Manifest version.
    pub version: &'static str,
    /// `extension.json` text.
    pub manifest: &'static str,
    /// zstd-compressed component.
    pub component_zst: &'static [u8],
    /// `(relative path, bytes)` of resource files.
    pub resources: &'static [(&'static str, &'static [u8])],
    /// First 8 hex chars of the component sha256.
    pub hash8: &'static str,
}

/// Bundle failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BundleError {
    /// Decompression or file I/O.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Cause.
        source: std::io::Error,
    },
}

/// Every bundled extension.
pub static BUNDLED: &[BundledExtension] = include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

impl BundledExtension {
    /// Decompressed component bytes.
    ///
    /// # Errors
    ///
    /// Corrupt blob.
    pub fn component(&self) -> Result<Vec<u8>, BundleError> {
        zstd::decode_all(self.component_zst).map_err(|source| BundleError::Io {
            context: format!("decompress {}", self.id),
            source,
        })
    }

    /// `bundled/<id>@<version>-<hash8>` under `agent_dir`.
    #[must_use]
    pub fn extract_dir(&self, agent_dir: &Path) -> PathBuf {
        agent_dir
            .join("bundled")
            .join(format!("{}@{}-{}", self.id, self.version, self.hash8))
    }

    /// Extracts the manifest and resources once (write to a temp dir, then rename) and returns
    /// the package root.
    ///
    /// # Errors
    ///
    /// File I/O.
    pub fn extract(&self, agent_dir: &Path) -> Result<PathBuf, BundleError> {
        let manifest = [("extension.json", self.manifest.as_bytes())];
        extract_files(
            &self.extract_dir(agent_dir),
            "extension.json",
            manifest
                .iter()
                .copied()
                .chain(self.resources.iter().copied()),
        )
    }
}

/// The user documentation bundled into the binary (`README.md`, `docs/*.md`): the docs root
/// that the system prompt points at (runtime §9.4).
#[derive(Debug)]
pub struct BundledDocs {
    /// Version of `oino-bundle` (the Oino release).
    pub version: &'static str,
    /// First 8 hex chars of the sha256 over every file.
    pub hash8: &'static str,
    /// `(relative path, bytes)`.
    pub files: &'static [(&'static str, &'static [u8])],
}

/// The bundled documentation.
pub static DOCS: BundledDocs = include!(concat!(env!("OUT_DIR"), "/docs.rs"));

impl BundledDocs {
    /// Extracts the docs once to `agent_dir/bundled/oino.docs@<version>-<hash8>` and returns
    /// that directory (`{docs_root}` of the system prompt).
    ///
    /// # Errors
    ///
    /// File I/O.
    pub fn extract(&self, agent_dir: &Path) -> Result<PathBuf, BundleError> {
        let dir = agent_dir
            .join("bundled")
            .join(format!("oino.docs@{}-{}", self.version, self.hash8));
        extract_files(&dir, "README.md", self.files.iter().copied())
    }
}

/// Writes `files` into a temp directory next to `dir`, then renames it into place; a no-op
/// when `dir/marker` exists.
fn extract_files<'a>(
    dir: &Path,
    marker: &str,
    files: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Result<PathBuf, BundleError> {
    if dir.join(marker).is_file() {
        return Ok(dir.to_path_buf());
    }
    let io = |context: String| move |source| BundleError::Io { context, source };
    let parent = dir
        .parent()
        .map_or_else(|| dir.to_path_buf(), Path::to_path_buf);
    std::fs::create_dir_all(&parent).map_err(io(format!("create {}", parent.display())))?;
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(io(format!("create {}", tmp.display())))?;
    for (rel, bytes) in files {
        let path = tmp.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(io(format!("create {}", p.display())))?;
        }
        std::fs::write(&path, bytes).map_err(io(format!("write {rel}")))?;
    }
    match std::fs::rename(&tmp, dir) {
        Ok(()) => Ok(dir.to_path_buf()),
        // Another process won the race.
        Err(_) if dir.join(marker).is_file() => {
            let _ = std::fs::remove_dir_all(&tmp);
            Ok(dir.to_path_buf())
        }
        Err(source) => Err(BundleError::Io {
            context: format!("rename to {}", dir.display()),
            source,
        }),
    }
}

/// Looks up a bundled extension by id.
#[must_use]
pub fn get(id: &str) -> Option<&'static BundledExtension> {
    BUNDLED.iter().find(|b| b.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        for b in BUNDLED {
            assert!(b.manifest.contains(b.id));
            assert_eq!(b.hash8.len(), 8);
            assert!(get(b.id).is_some());
        }
    }

    #[test]
    fn extract_is_idempotent() {
        static RES: &[(&str, &[u8])] = &[("skills/x/SKILL.md", b"# x")];
        let zst: &'static [u8] = Box::leak(
            zstd::encode_all(&b"\0asm"[..], 1)
                .unwrap_or_default()
                .into_boxed_slice(),
        );
        let b = BundledExtension {
            id: "t.x",
            version: "1.0.0",
            manifest: "{\"id\":\"t.x\"}",
            component_zst: zst,
            resources: RES,
            hash8: "abcdef01",
        };
        let dir = tempfile::tempdir().unwrap_or_else(|e| unreachable!("{e}"));
        let root = b
            .extract(dir.path())
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert!(root.ends_with("t.x@1.0.0-abcdef01"));
        assert_eq!(
            std::fs::read(root.join("skills/x/SKILL.md")).unwrap_or_default(),
            b"# x"
        );
        assert_eq!(
            b.extract(dir.path())
                .unwrap_or_else(|e| unreachable!("{e}")),
            root
        );
        assert_eq!(b.component().unwrap_or_default(), b"\0asm");
    }

    #[test]
    fn docs_extract() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| unreachable!("{e}"));
        let root = DOCS
            .extract(dir.path())
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert!(root.join("README.md").is_file());
        assert!(DOCS.files.iter().any(|(rel, _)| rel.starts_with("docs/")));
    }
}
