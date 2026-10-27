//! Package manifest `extension.json` (extension-api §7) and permissions (§7.3).
//!
//! Every permission defaults to denied except `ui`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::ExtensionId;

/// Required `apiVersion`.
pub const API_VERSION: &str = "oino.extension/1";
/// Manifest file name inside a package.
pub const MANIFEST_FILE: &str = "extension.json";
/// Default component file name.
pub const DEFAULT_COMPONENT: &str = "component.wasm";
/// Maximum `limits.memoryMiB`.
pub const MAX_MEMORY_MIB: u32 = 2048;

/// Manifest `kind` (D15): `-ne` disables only `Extension`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionKind {
    /// Provider component (exports `provider`).
    Provider,
    /// Any other extension.
    #[default]
    Extension,
}

/// `extension.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Must be [`API_VERSION`].
    pub api_version: String,
    /// Extension id.
    pub id: ExtensionId,
    /// Display name.
    pub name: String,
    /// Semver version.
    pub version: String,
    /// Kind.
    #[serde(default)]
    pub kind: ExtensionKind,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// SPDX license.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// Component file (relative).
    #[serde(default = "default_component")]
    pub component: String,
    /// `file → "sha256:<hex>"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub checksums: BTreeMap<String, String>,
    /// Minimum Oino version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_oino_version: Option<String>,
    /// Provided APIs/providers.
    #[serde(default)]
    pub provides: Provides,
    /// Bundled resources.
    #[serde(default)]
    pub resources: Resources,
    /// Limits.
    #[serde(default)]
    pub limits: Limits,
    /// Permissions.
    #[serde(default)]
    pub permissions: Permissions,
}

fn default_component() -> String {
    DEFAULT_COMPONENT.to_owned()
}

/// `provides`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provides {
    /// Wire APIs implemented (`openai-completions`, …).
    #[serde(default)]
    pub apis: Vec<String>,
    /// Provider ids registered.
    #[serde(default)]
    pub providers: Vec<String>,
}

/// `resources` (paths relative to the package root).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    /// Skill directories.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Prompt template directories.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Theme directories.
    #[serde(default)]
    pub themes: Vec<String>,
    /// Docs directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

impl Resources {
    /// Every declared path.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.skills
            .iter()
            .chain(&self.prompts)
            .chain(&self.themes)
            .chain(self.docs.iter())
            .map(String::as_str)
    }
}

/// `limits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Linear memory cap in MiB (max [`MAX_MEMORY_MIB`]).
    #[serde(default = "default_memory")]
    pub memory_mib: u32,
    /// Event hook deadline.
    #[serde(default = "default_hook_timeout")]
    pub hook_timeout_ms: u64,
    /// `tool_call` hook deadline.
    #[serde(default = "default_tool_call_timeout")]
    pub tool_call_hook_timeout_ms: u64,
}

fn default_memory() -> u32 {
    512
}
fn default_hook_timeout() -> u64 {
    5000
}
fn default_tool_call_timeout() -> u64 {
    30_000
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mib: default_memory(),
            hook_timeout_ms: default_hook_timeout(),
            tool_call_hook_timeout_ms: default_tool_call_timeout(),
        }
    }
}

/// `fs.project` / `fs.paths[].access`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FsAccess {
    /// No access.
    #[default]
    None,
    /// Read-only.
    Read,
    /// Read-write.
    Write,
}

/// `fs.paths[]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsPathGrant {
    /// Host path (`~` expanded).
    pub path: String,
    /// Access.
    pub access: FsAccess,
    /// Preopen name (guest path `/<alias>`).
    pub alias: String,
}

/// `permissions` (§7.3).
#[allow(
    clippy::struct_excessive_bools,
    reason = "one field per manifest permission"
)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Permissions {
    /// Control setters/actions and session writes beyond append/send.
    pub session_control: bool,
    /// `sessions.readAll`.
    pub sessions: SessionsPermission,
    /// `tools.override`.
    pub tools: ToolsPermission,
    /// `models.complete`.
    pub models: ModelsPermission,
    /// Provider ids or `"*"`.
    pub credentials: Vec<String>,
    /// `agents.spawn`, `agents.maxDepth`.
    pub agents: AgentsPermission,
    /// `process.spawn`, `process.allow`, `process.detached`.
    pub process: ProcessPermission,
    /// `network.*`.
    pub network: NetworkPermission,
    /// `fs.*`.
    pub fs: FsPermission,
    /// Env var names.
    pub env: Vec<String>,
    /// `secrets.*`.
    pub secrets: bool,
    /// `ui.*` (default true).
    pub ui: bool,
    /// `ui.os-notify`.
    pub os_notify: bool,
    /// `ui.open-url`.
    pub open_url: bool,
    /// `ui.clipboard-*`.
    pub clipboard: ClipboardPermission,
    /// `ui.edit-externally`.
    pub edit_externally: bool,
    /// `services.call`.
    pub services: ServicesPermission,
    /// `browser.*`: control the in-app browser (browser-control.md §8.4).
    pub browser: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            session_control: false,
            sessions: SessionsPermission::default(),
            tools: ToolsPermission::default(),
            models: ModelsPermission::default(),
            credentials: Vec::new(),
            agents: AgentsPermission::default(),
            process: ProcessPermission::default(),
            network: NetworkPermission::default(),
            fs: FsPermission::default(),
            env: Vec::new(),
            secrets: false,
            ui: true,
            os_notify: false,
            open_url: false,
            clipboard: ClipboardPermission::default(),
            edit_externally: false,
            services: ServicesPermission::default(),
            browser: false,
        }
    }
}

/// `sessions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionsPermission {
    /// Read other sessions.
    pub read_all: bool,
}

/// `tools`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsPermission {
    /// Built-in tools whose render metadata may be overridden.
    #[serde(rename = "override")]
    pub override_builtins: Vec<String>,
}

/// `models`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelsPermission {
    /// Non-session completions.
    pub complete: bool,
}

/// `agents`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentsPermission {
    /// Spawn nested agents.
    pub spawn: bool,
    /// Max nesting depth (0 = host default).
    pub max_depth: u8,
}

/// `process`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessPermission {
    /// Spawn processes.
    pub spawn: bool,
    /// Allowed program names/globs (`"*"` bundled only).
    pub allow: Vec<String>,
    /// Detached daemons.
    pub detached: bool,
}

/// `network`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkPermission {
    /// Hosts / `*.domain` (`"*"` bundled only).
    pub hosts: Vec<String>,
    /// TLS fingerprint impersonation.
    pub fingerprint: bool,
    /// Loopback listener.
    pub listen_loopback: bool,
}

/// `fs`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FsPermission {
    /// `project` preopen.
    pub project: FsAccess,
    /// `agent` preopen (`none` | `read`).
    pub agent_dir: FsAccess,
    /// Extra preopens.
    pub paths: Vec<FsPathGrant>,
}

/// `clipboard`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipboardPermission {
    /// Read images.
    pub read: bool,
    /// Write text.
    pub write: bool,
}

/// `services`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServicesPermission {
    /// `<ext-id>/<name>` or `"*"`.
    pub call: Vec<String>,
}

/// Simple glob: `*` matches any run of characters, everything else literal.
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(stripped) = rest.strip_prefix(part) else {
                return false;
            };
            rest = stripped;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

impl Permissions {
    /// `network.hosts` allows `host` (`"*"` only when `bundled`).
    #[must_use]
    pub fn allows_host(&self, host: &str, bundled: bool) -> bool {
        let host = host.to_ascii_lowercase();
        self.network.hosts.iter().any(|grant| {
            let grant = grant.to_ascii_lowercase();
            if grant == "*" {
                bundled
            } else if let Some(domain) = grant.strip_prefix("*.") {
                host == domain || host.ends_with(&format!(".{domain}"))
            } else {
                grant == host
            }
        })
    }

    /// `process.allow` allows program `program` (matched by file name; `"*"` only when bundled).
    #[must_use]
    pub fn allows_program(&self, program: &str, bundled: bool) -> bool {
        if !self.process.spawn {
            return false;
        }
        let name = Path::new(program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(program);
        let stem = name.strip_suffix(".exe").unwrap_or(name);
        self.process.allow.iter().any(|grant| {
            if grant == "*" {
                bundled
            } else {
                glob_match(grant, program) || glob_match(grant, name) || glob_match(grant, stem)
            }
        })
    }

    /// `credentials` allows `provider`.
    #[must_use]
    pub fn allows_credential(&self, provider: &str) -> bool {
        self.credentials.iter().any(|c| c == "*" || c == provider)
    }

    /// `services.call` allows calling `name` on `target` (`None` = any provider of `name`).
    #[must_use]
    pub fn allows_service(&self, target: Option<&str>, name: &str) -> bool {
        self.services.call.iter().any(|grant| {
            grant == "*"
                || match grant.split_once('/') {
                    Some((ext, svc)) => {
                        (svc == "*" || svc == name) && target.is_none_or(|t| ext == "*" || ext == t)
                    }
                    None => false,
                }
        })
    }

    /// `env` allows reading `name`.
    #[must_use]
    pub fn allows_env(&self, name: &str) -> bool {
        self.env.iter().any(|e| e == name)
    }
}

/// Manifest/package validation failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// Not JSON / wrong shape.
    #[error("invalid manifest: {0}")]
    Parse(#[from] serde_json::Error),
    /// Wrong `apiVersion`.
    #[error("apiVersion must be `{API_VERSION}`, found `{0}`")]
    ApiVersion(String),
    /// Bad semver.
    #[error("version `{0}` is not semver")]
    Version(String),
    /// Provider without APIs.
    #[error("kind `provider` requires non-empty provides.apis")]
    ProviderWithoutApis,
    /// Memory limit out of range.
    #[error("limits.memoryMiB must be 1..={MAX_MEMORY_MIB}, found {0}")]
    Memory(u32),
    /// Missing file.
    #[error("missing package file `{}`", .0.display())]
    Missing(PathBuf),
    /// Path escapes the package root.
    #[error("path `{0}` escapes the package root")]
    PathEscape(String),
    /// Checksum mismatch.
    #[error("checksum mismatch for `{file}`: expected {expected}, found {actual}")]
    Checksum {
        /// File.
        file: String,
        /// Declared.
        expected: String,
        /// Computed.
        actual: String,
    },
    /// I/O.
    #[error("{}: {source}", .path.display())]
    Io {
        /// Path.
        path: PathBuf,
        /// Error.
        source: std::io::Error,
    },
}

/// `sha256:<hex>` of `bytes`.
#[must_use]
pub fn sha256_tag(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn safe_relative(path: &str) -> Result<&Path, ManifestError> {
    let p = Path::new(path);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ManifestError::PathEscape(path.to_owned()));
    }
    Ok(p)
}

impl Manifest {
    /// Parses and validates manifest JSON (no file checks).
    ///
    /// # Errors
    ///
    /// See [`ManifestError`].
    pub fn parse(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Field validation (§7.2).
    ///
    /// # Errors
    ///
    /// See [`ManifestError`].
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.api_version != API_VERSION {
            return Err(ManifestError::ApiVersion(self.api_version.clone()));
        }
        semver::Version::parse(&self.version)
            .map_err(|_| ManifestError::Version(self.version.clone()))?;
        if self.kind == ExtensionKind::Provider && self.provides.apis.is_empty() {
            return Err(ManifestError::ProviderWithoutApis);
        }
        if self.limits.memory_mib == 0 || self.limits.memory_mib > MAX_MEMORY_MIB {
            return Err(ManifestError::Memory(self.limits.memory_mib));
        }
        safe_relative(&self.component)?;
        for path in self.resources.paths() {
            safe_relative(path)?;
        }
        Ok(())
    }

    /// Reads `<root>/extension.json`, validates it, checks that the component and declared
    /// resources exist and that declared checksums match.
    ///
    /// # Errors
    ///
    /// See [`ManifestError`].
    pub fn load_package(root: &Path) -> Result<Self, ManifestError> {
        let path = root.join(MANIFEST_FILE);
        let text =
            std::fs::read_to_string(&path).map_err(|source| ManifestError::Io { path, source })?;
        let manifest = Self::parse(&text)?;
        manifest.verify_files(root)?;
        Ok(manifest)
    }

    /// File checks of [`Manifest::load_package`].
    ///
    /// # Errors
    ///
    /// See [`ManifestError`].
    pub fn verify_files(&self, root: &Path) -> Result<(), ManifestError> {
        for rel in std::iter::once(self.component.as_str()).chain(self.resources.paths()) {
            let path = root.join(safe_relative(rel)?);
            if !path.exists() {
                return Err(ManifestError::Missing(path));
            }
        }
        for (file, expected) in &self.checksums {
            let path = root.join(safe_relative(file)?);
            let bytes =
                std::fs::read(&path).map_err(|source| ManifestError::Io { path, source })?;
            let actual = sha256_tag(&bytes);
            if &actual != expected {
                return Err(ManifestError::Checksum {
                    file: file.clone(),
                    expected: expected.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }

    /// Component path inside `root`.
    #[must_use]
    pub fn component_path(&self, root: &Path) -> PathBuf {
        root.join(&self.component)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(extra: &str) -> String {
        format!(
            r#"{{"apiVersion":"oino.extension/1","id":"unipi.notify","name":"Notify","version":"3.0.0"{extra}}}"#
        )
    }

    #[test]
    fn parses_defaults() {
        let m = Manifest::parse(&manifest("")).unwrap();
        assert_eq!(m.kind, ExtensionKind::Extension);
        assert_eq!(m.component, "component.wasm");
        assert!(m.permissions.ui);
        assert!(!m.permissions.process.spawn);
        assert_eq!(m.limits.memory_mib, 512);
    }

    #[test]
    fn rejects_invalid() {
        assert!(matches!(
            Manifest::parse(&manifest(r#","kind":"provider""#)),
            Err(ManifestError::ProviderWithoutApis)
        ));
        let bad = manifest("").replace("oino.extension/1", "x");
        assert!(matches!(
            Manifest::parse(&bad),
            Err(ManifestError::ApiVersion(_))
        ));
        let bad = manifest("").replace("3.0.0", "3");
        assert!(matches!(
            Manifest::parse(&bad),
            Err(ManifestError::Version(_))
        ));
        assert!(matches!(
            Manifest::parse(&manifest(r#","component":"../x.wasm""#)),
            Err(ManifestError::PathEscape(_))
        ));
        assert!(Manifest::parse(&manifest("").replace("unipi.notify", "Bad")).is_err());
    }

    #[test]
    fn permission_helpers() {
        let m = Manifest::parse(&manifest(
            r#","permissions":{"network":{"hosts":["ntfy.sh","*.example.com","*"]},
               "process":{"spawn":true,"allow":["git","rg*"]},
               "credentials":["openai"],"services":{"call":["unipi.memory/save","a/*"]},"env":["HOME"]}"#,
        ))
        .unwrap();
        let p = &m.permissions;
        assert!(p.allows_host("ntfy.sh", false));
        assert!(p.allows_host("api.example.com", false));
        assert!(p.allows_host("example.com", false));
        assert!(!p.allows_host("evil.com", false));
        assert!(p.allows_host("evil.com", true));
        assert!(p.allows_program("/usr/bin/git", false));
        assert!(p.allows_program("rg", false));
        assert!(!p.allows_program("bash", false));
        assert!(p.allows_credential("openai") && !p.allows_credential("anthropic"));
        assert!(p.allows_service(Some("unipi.memory"), "save"));
        assert!(p.allows_service(None, "save"));
        assert!(p.allows_service(Some("a"), "anything"));
        assert!(!p.allows_service(Some("b"), "save"));
        assert!(p.allows_env("HOME") && !p.allows_env("PATH"));
    }

    #[test]
    fn package_files_and_checksums() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("component.wasm"), b"wasm").unwrap();
        let tag = sha256_tag(b"wasm");
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            manifest(&format!(r#","checksums":{{"component.wasm":"{tag}"}}"#)),
        )
        .unwrap();
        Manifest::load_package(dir.path()).unwrap();
        std::fs::write(dir.path().join("component.wasm"), b"other").unwrap();
        assert!(matches!(
            Manifest::load_package(dir.path()),
            Err(ManifestError::Checksum { .. })
        ));
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            manifest(r#","resources":{"skills":["skills"]}"#),
        )
        .unwrap();
        assert!(matches!(
            Manifest::load_package(dir.path()),
            Err(ManifestError::Missing(_))
        ));
        assert!(glob_match("a*c", "abbc") && !glob_match("a*c", "abd") && glob_match("*", ""));
    }
}
