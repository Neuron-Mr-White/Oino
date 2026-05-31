#![forbid(unsafe_code)]

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DEFAULT_OWNER_REPO: &str = "Neuron-Mr-White/Oino";
const MANIFEST_FILE: &str = "release-manifest.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OinoUpdatePlan {
    pub mode: OinoUpdateMode,
    pub tag: Option<String>,
    pub force_source: bool,
}

impl Default for OinoUpdatePlan {
    fn default() -> Self {
        Self {
            mode: OinoUpdateMode::Core,
            tag: None,
            force_source: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OinoUpdateMode {
    Check,
    Core,
    Extensions,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub tag: String,
    #[serde(default)]
    pub version: Option<Version>,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<ReleaseArtifact>,
    #[serde(default)]
    pub source: Option<SourceFallback>,
    #[serde(default)]
    pub extensions: Option<ExtensionBundle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseArtifact {
    pub target: String,
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFallback {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionBundle {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub built_in_root: Option<String>,
    #[serde(default)]
    pub additional_root: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("update command error: {0}")]
    InvalidCommand(String),
    #[error("failed to request `{url}`: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("release manifest at `{url}` returned HTTP {status}")]
    HttpStatus {
        url: String,
        status: reqwest::StatusCode,
    },
    #[error("failed to parse release manifest from `{url}`: {source}")]
    ManifestParse { url: String, source: reqwest::Error },
    #[error("no binary artifact for target `{target}` in release `{tag}`")]
    NoArtifact { target: String, tag: String },
    #[error("artifact kind `{kind}` is not supported for direct hot update; expected `binary`")]
    UnsupportedArtifactKind { kind: String },
    #[error("checksum mismatch for `{url}`: expected {expected}, got {actual}")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("current executable path is unavailable")]
    CurrentExeUnavailable,
    #[error("core binary hot update is not supported on Windows while Oino is running; use the install script or update after exit")]
    WindowsHotUpdateUnsupported,
    #[error("failed to install update: {0}")]
    Io(#[from] io::Error),
}

pub fn parse_oino_update_args(args: &[&str]) -> Result<OinoUpdatePlan, UpdateError> {
    let mut plan = OinoUpdatePlan::default();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "check" | "--check" => plan.mode = OinoUpdateMode::Check,
            "core" => plan.mode = OinoUpdateMode::Core,
            "extensions" | "extension" | "--extensions" => plan.mode = OinoUpdateMode::Extensions,
            "all" | "--all" => plan.mode = OinoUpdateMode::All,
            "source" | "--source" => plan.force_source = true,
            "--tag" | "tag" => {
                index += 1;
                let Some(tag) = args.get(index) else {
                    return Err(UpdateError::InvalidCommand(
                        "missing tag after --tag".into(),
                    ));
                };
                plan.tag = Some((*tag).to_string());
            }
            value if value.starts_with("--tag=") => {
                let tag = value.trim_start_matches("--tag=").trim();
                if tag.is_empty() {
                    return Err(UpdateError::InvalidCommand(
                        "missing tag after --tag=".into(),
                    ));
                }
                plan.tag = Some(tag.to_string());
            }
            value if value.starts_with('-') => {
                return Err(UpdateError::InvalidCommand(format!(
                    "unknown update flag `{value}`"
                )));
            }
            value if looks_like_tag(value) => plan.tag = Some(value.to_string()),
            value => {
                return Err(UpdateError::InvalidCommand(format!(
                    "unknown update argument `{value}`"
                )));
            }
        }
        index += 1;
    }
    Ok(plan)
}

fn looks_like_tag(value: &str) -> bool {
    value.starts_with('v') && value.chars().skip(1).any(|ch| ch.is_ascii_digit())
}

fn manifest_not_found(err: &UpdateError) -> bool {
    matches!(
        err,
        UpdateError::HttpStatus { status, .. } if *status == reqwest::StatusCode::NOT_FOUND
    )
}

fn missing_manifest_message(plan: &OinoUpdatePlan) -> String {
    let target = plan
        .tag
        .as_deref()
        .map(|tag| format!("release tag `{tag}`"))
        .unwrap_or_else(|| "latest GitHub release".into());
    format!(
        "No Oino release manifest is published for {target} yet. Use `oino update --source` for source/cargo guidance, run `OINO_FROM_SOURCE=1 sh scripts/install.sh`, or publish a `v*` release tag with `release-manifest.json`."
    )
}

pub fn release_manifest_url(tag: Option<&str>) -> String {
    if let Ok(url) = env::var("OINO_UPDATE_MANIFEST_URL") {
        if !url.trim().is_empty() {
            return url;
        }
    }
    let repo = env::var("OINO_UPDATE_REPO").unwrap_or_else(|_| DEFAULT_OWNER_REPO.to_string());
    match tag {
        Some(tag) => format!("https://github.com/{repo}/releases/download/{tag}/{MANIFEST_FILE}"),
        None => format!("https://github.com/{repo}/releases/latest/download/{MANIFEST_FILE}"),
    }
}

pub async fn fetch_release_manifest(
    tag: Option<&str>,
) -> Result<(String, ReleaseManifest), UpdateError> {
    let url = release_manifest_url(tag);
    let response = reqwest::get(&url)
        .await
        .map_err(|source| UpdateError::Request {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::HttpStatus { url, status });
    }
    let manifest =
        response
            .json::<ReleaseManifest>()
            .await
            .map_err(|source| UpdateError::ManifestParse {
                url: url.clone(),
                source,
            })?;
    Ok((url, manifest))
}

pub fn current_target() -> String {
    let arch = match env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "arm" => "armv7",
        other => other,
    };
    let os = match env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        "windows" => "pc-windows-msvc",
        "freebsd" => "unknown-freebsd",
        other => other,
    };
    format!("{arch}-{os}")
}

pub fn current_executable_path() -> Option<PathBuf> {
    env::current_exe().ok()
}

pub fn format_check_result(manifest_url: &str, manifest: &ReleaseManifest) -> String {
    let target = current_target();
    let matching = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.target == target);
    let version = manifest
        .version
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| manifest.tag.clone());
    let mut lines = vec![format!(
        "Latest Oino release: {version} ({}) from {manifest_url}",
        manifest.tag
    )];
    if let Some(artifact) = matching {
        lines.push(format!("Binary available for {target}: {}", artifact.url));
    } else if manifest.source.is_some() {
        lines.push(format!(
            "No binary artifact for {target}; source/cargo fallback is available."
        ));
    } else {
        lines.push(format!(
            "No binary artifact for {target}, and this manifest does not advertise a source fallback."
        ));
    }
    if manifest.extensions.is_some() {
        lines.push("Extension bundle available for this release.".into());
    }
    lines.join("\n")
}

pub async fn check_for_update(plan: &OinoUpdatePlan) -> Result<String, UpdateError> {
    match fetch_release_manifest(plan.tag.as_deref()).await {
        Ok((url, manifest)) => Ok(format_check_result(&url, &manifest)),
        Err(err) if manifest_not_found(&err) => Ok(missing_manifest_message(plan)),
        Err(err) => Err(err),
    }
}

pub async fn install_core_update(plan: &OinoUpdatePlan) -> Result<String, UpdateError> {
    if cfg!(windows) {
        return Err(UpdateError::WindowsHotUpdateUnsupported);
    }
    let (manifest_url, manifest) = match fetch_release_manifest(plan.tag.as_deref()).await {
        Ok(result) => result,
        Err(err) if manifest_not_found(&err) => return Ok(missing_manifest_message(plan)),
        Err(err) => return Err(err),
    };
    let target = current_target();
    if plan.force_source {
        return Ok(format_source_fallback_message(
            &manifest_url,
            &manifest,
            &target,
        ));
    }
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.target == target)
        .ok_or_else(|| UpdateError::NoArtifact {
            target: target.clone(),
            tag: manifest.tag.clone(),
        })?;
    let kind = artifact.kind.as_deref().unwrap_or("binary");
    if kind != "binary" {
        return Err(UpdateError::UnsupportedArtifactKind {
            kind: kind.to_string(),
        });
    }
    let bytes = download_bytes(&artifact.url).await?;
    if let Some(expected) = &artifact.sha256 {
        verify_sha256(&artifact.url, &bytes, expected)?;
    }
    let executable = current_executable_path().ok_or(UpdateError::CurrentExeUnavailable)?;
    install_binary_atomically(&executable, &bytes)?;
    Ok(format!(
        "Installed Oino {} for {target} from {}. Restart Oino to use the updated binary.",
        manifest.tag, artifact.url
    ))
}

fn format_source_fallback_message(
    manifest_url: &str,
    manifest: &ReleaseManifest,
    target: &str,
) -> String {
    let source = manifest
        .source
        .as_ref()
        .map(|source| source.url.as_str())
        .unwrap_or("the release source archive");
    format!(
        "Source fallback selected for {target} from {manifest_url}. Build with: OINO_REF={} sh scripts/install.sh # source: {source}",
        manifest.tag
    )
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    let response = reqwest::get(url)
        .await
        .map_err(|source| UpdateError::Request {
            url: url.to_string(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::HttpStatus {
            url: url.to_string(),
            status,
        });
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|source| UpdateError::Request {
            url: url.to_string(),
            source,
        })
}

fn verify_sha256(url: &str, bytes: &[u8], expected: &str) -> Result<(), UpdateError> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch {
            url: url.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
}

fn install_binary_atomically(destination: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let dir = destination.parent().unwrap_or_else(|| Path::new("."));
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp = dir.join(format!(".oino-update-{}-{stamp}", std::process::id()));
    fs::write(&temp, bytes)?;
    copy_executable_permissions(destination, &temp)?;
    fs::rename(&temp, destination)?;
    Ok(())
}

#[cfg(unix)]
fn copy_executable_permissions(source: &Path, target: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(source)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0o755);
    fs::set_permissions(target, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn copy_executable_permissions(_source: &Path, _target: &Path) -> Result<(), UpdateError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_update_modes_and_tags() {
        assert_eq!(
            parse_oino_update_args(&["check"]).unwrap().mode,
            OinoUpdateMode::Check
        );
        assert_eq!(
            parse_oino_update_args(&["extensions"]).unwrap().mode,
            OinoUpdateMode::Extensions
        );
        let plan = parse_oino_update_args(&["all", "--tag", "v1.2.3", "--source"]).unwrap();
        assert_eq!(plan.mode, OinoUpdateMode::All);
        assert_eq!(plan.tag.as_deref(), Some("v1.2.3"));
        assert!(plan.force_source);
    }

    #[test]
    fn missing_manifest_message_is_actionable() {
        let message = missing_manifest_message(&OinoUpdatePlan::default());
        assert!(message.contains("No Oino release manifest"));
        assert!(message.contains("OINO_FROM_SOURCE=1 sh scripts/install.sh"));
        assert!(message.contains("v*"));
    }

    #[test]
    fn parses_manifest() {
        let manifest: ReleaseManifest = serde_json::from_str(
            r#"{
              "tag": "v1.2.3",
              "version": "1.2.3",
              "artifacts": [{"target":"x86_64-unknown-linux-gnu","url":"https://example/oino","sha256":"abc"}],
              "source": {"url":"https://example/source.tar.gz"},
              "extensions": {"url":"https://example/extensions.tar.gz","built_in_root":"extensions/built-in","additional_root":"extensions/additional"}
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.tag, "v1.2.3");
        assert_eq!(manifest.artifacts[0].target, "x86_64-unknown-linux-gnu");
    }
}
