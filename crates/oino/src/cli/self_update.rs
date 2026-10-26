//! `oino update [--check]` (modes.md §1.2): self-update from the GitHub release manifest.
//!
//! Reads `release-manifest.json` of the latest release (or `OINO_REF`), picks the `oino`
//! artifact of this target, verifies its sha256 and atomically replaces the running binary
//! (renaming the old one away first, which also works for a running `.exe` on Windows).
//! `OINO_RELEASE_MANIFEST` overrides the manifest URL (mirrors, tests).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Failure of a self-update.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UpdateError {
    /// The manifest or binary could not be fetched.
    #[error("{0}")]
    Fetch(String),
    /// No artifact for this platform.
    #[error("no release binary for {0}; build from source (scripts/install.sh)")]
    NoArtifact(String),
    /// The download does not match the manifest.
    #[error("sha256 mismatch for the downloaded binary")]
    Checksum,
    /// Replacing the binary failed.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Cause.
        source: std::io::Error,
    },
}

/// `release-manifest.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseManifest {
    /// Release version (tag without `v`).
    pub version: String,
    /// Binaries.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

/// One release binary.
#[derive(Clone, Debug, Deserialize)]
pub struct Artifact {
    /// `oino` | `oino-kanboard`.
    #[serde(default)]
    pub name: String,
    /// Rust target triple.
    pub target: String,
    /// Download URL.
    pub url: String,
    /// Hex sha256.
    #[serde(default)]
    pub sha256: String,
}

/// This build's release target (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, …).
#[must_use]
pub fn current_target() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        other => format!("{arch}-{other}"),
    }
}

fn manifest_url() -> String {
    if let Ok(url) = std::env::var("OINO_RELEASE_MANIFEST")
        && !url.is_empty()
    {
        return url;
    }
    let slug = env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/");
    match std::env::var("OINO_REF").ok().filter(|r| !r.is_empty()) {
        Some(reference) => {
            format!("https://github.com/{slug}/releases/download/{reference}/release-manifest.json")
        }
        None => format!("https://github.com/{slug}/releases/latest/download/release-manifest.json"),
    }
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| UpdateError::Fetch(format!("{url}: {e}")));
    }
    let response = reqwest::get(url)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| UpdateError::Fetch(format!("{url}: {e}")))?;
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| UpdateError::Fetch(format!("{url}: {e}")))
}

/// Compares dotted versions with pre-release suffixes (`1.0.0-alpha.2` > `1.0.0-alpha.1`,
/// `1.0.0` > `1.0.0-alpha.9`).
#[must_use]
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, Option<Vec<String>>) {
        let v = v.trim_start_matches('v');
        let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
        (
            core.split('.').map(|p| p.parse().unwrap_or(0)).collect(),
            pre.map(|p| p.split('.').map(str::to_owned).collect()),
        )
    }
    let (a_core, a_pre) = parts(candidate);
    let (b_core, b_pre) = parts(current);
    if a_core != b_core {
        return a_core > b_core;
    }
    match (a_pre, b_pre) {
        (None, Some(_)) => true,
        (Some(a), Some(b)) => {
            for (x, y) in a.iter().zip(&b) {
                let ordering = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    _ => x.cmp(y),
                };
                if ordering != std::cmp::Ordering::Equal {
                    return ordering == std::cmp::Ordering::Greater;
                }
            }
            a.len() > b.len()
        }
        _ => false,
    }
}

/// Lower-case hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn replace_binary(exe: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let io = |context: &str| {
        let context = format!("{context} {}", exe.display());
        move |source| UpdateError::Io { context, source }
    };
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");
    std::fs::write(&new, bytes).map_err(io("writing next to"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
            .map_err(io("marking executable"))?;
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(io("moving aside"))?;
    if let Err(source) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(UpdateError::Io {
            context: format!("installing {}", exe.display()),
            source,
        });
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Checks for (and unless `check`, installs) a newer release; returns the message to print.
///
/// # Errors
///
/// Fetch, platform, checksum and replacement failures.
pub async fn self_update(check: bool, exe: Option<PathBuf>) -> Result<String, UpdateError> {
    let current = crate::VERSION;
    let bytes = fetch_bytes(&manifest_url()).await?;
    let manifest: ReleaseManifest = serde_json::from_slice(&bytes)
        .map_err(|e| UpdateError::Fetch(format!("invalid release manifest: {e}")))?;
    if !is_newer(&manifest.version, current) {
        return Ok(format!("Oino is up to date ({current})"));
    }
    if check {
        return Ok(format!(
            "Update available: {current} → {} (run `oino update`)",
            manifest.version
        ));
    }
    let target = current_target();
    let artifact = manifest
        .artifacts
        .iter()
        .find(|a| a.target == target && (a.name.is_empty() || a.name == "oino"))
        .ok_or_else(|| UpdateError::NoArtifact(target.clone()))?;
    let binary = fetch_bytes(&artifact.url).await?;
    if !artifact.sha256.is_empty()
        && !hex(&Sha256::digest(&binary)).eq_ignore_ascii_case(artifact.sha256.trim())
    {
        return Err(UpdateError::Checksum);
    }
    let exe = match exe {
        Some(exe) => exe,
        None => std::env::current_exe().map_err(|source| UpdateError::Io {
            context: "locating the running binary".to_owned(),
            source,
        })?,
    };
    replace_binary(&exe, &binary)?;
    Ok(format!("Updated Oino {current} → {}", manifest.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("1.0.0-alpha.2", "1.0.0-alpha.1"));
        assert!(is_newer("1.0.0", "1.0.0-alpha.9"));
        assert!(is_newer("v1.1.0", "1.0.9"));
        assert!(!is_newer("1.0.0-alpha.1", "1.0.0-alpha.1"));
        assert!(!is_newer("1.0.0-alpha.1", "1.0.0"));
        assert!(is_newer("1.0.0-alpha.10", "1.0.0-alpha.9"));
    }

    #[tokio::test]
    async fn installs_from_a_file_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let binary = b"#!/bin/sh\necho new\n";
        let digest = hex(&Sha256::digest(binary));
        std::fs::write(dir.path().join("oino-bin"), binary).unwrap();
        let exe = dir.path().join("oino");
        std::fs::write(&exe, b"old").unwrap();
        let manifest = serde_json::json!({
            "version": "99.0.0",
            "artifacts": [{"name": "oino", "target": current_target(),
                "url": format!("file://{}", dir.path().join("oino-bin").display()), "sha256": digest}]
        });
        let manifest_path = dir.path().join("release-manifest.json");
        std::fs::write(&manifest_path, manifest.to_string()).unwrap();
        let url = format!("file://{}", manifest_path.display());
        let bytes = fetch_bytes(&url).await.unwrap();
        let parsed: ReleaseManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.version, "99.0.0");
        let fetched = fetch_bytes(&parsed.artifacts[0].url).await.unwrap();
        replace_binary(&exe, &fetched).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), binary);
        assert!(!exe.with_extension("old").exists());
    }
}
