//! Package manifest `extension.json` (extension-api §7) and permissions (§7.3).
//!
//! Every permission defaults to denied except `ui`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::ExtensionId;

/// Required `apiVersion` (v2, ext-abi §8 D7): a manifest written for the 1.x ABI is rejected.
pub const API_VERSION: &str = "oino.extension/2";
/// The WIT package version every component is built against (`host-info.info.api-version`).
pub const WIT_VERSION: &str = "2.0.0";
/// Manifest file name inside a package.
pub const MANIFEST_FILE: &str = "extension.json";
/// Default component file name.
pub const DEFAULT_COMPONENT: &str = "component.wasm";
/// Maximum `limits.memoryMiB`.
pub const MAX_MEMORY_MIB: u32 = 2048;

/// Reserved entry ids (ui-protocol §8) with their owner: `host` ids are created by the host
/// from its advertised services, every other id may only be declared by that extension.
pub const RESERVED_IDS: [(&str, &str); 9] = [
    ("unipi.kanboard/board", "unipi.kanboard"),
    ("unipi.cron/jobs", "unipi.cron"),
    ("host/files", "host"),
    ("host/git", "host"),
    ("unipi.browser/browser", "unipi.browser"),
    ("host/terminal", "host"),
    ("unipi.info-screen/dashboard", "unipi.info-screen"),
    ("unipi.long-horizon/visualize", "unipi.long-horizon"),
    ("unipi.file-summary/turn-files", "unipi.file-summary"),
];

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
    /// Native CLI binaries the package ships in `bin/` (ext-abi §5.2, D2):
    /// bare file names, resolved by the host's process service against
    /// `OINO_EXT_BIN_DIR`, `<package root>/bin/` and `PATH`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<String>,
    /// Limits.
    #[serde(default)]
    pub limits: Limits,
    /// UI declarations (ui-protocol §8/§9.1): reserved entry ids this extension owns and the
    /// settings namespaces it contributes. Optional.
    #[serde(default, skip_serializing_if = "UiManifest::is_empty")]
    pub ui: UiManifest,
    /// Permissions.
    #[serde(default)]
    pub permissions: Permissions,
}

/// `ui` (ui-protocol §8 rule 1, §7 static settings): `{"reserved": [id…], "settings":
/// [SettingsNamespace…]}`.
///
/// `reserved` names entries the app renders as a native component; only ids from
/// [`RESERVED_IDS`] whose owner is this extension are accepted. `settings` is the static path
/// for settings namespaces; the runtime `registry.register-settings` declaration stays valid
/// alongside it (both accepted in 2.x).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiManifest {
    /// Reserved entry ids (ui-protocol §8), e.g. `unipi.kanboard/board`.
    pub reserved: Vec<String>,
    /// `oino_protocol::ext::SettingsNamespace` JSON values (one Settings section each).
    pub settings: Vec<serde_json::Value>,
}

impl UiManifest {
    /// Whether nothing is declared (omitted from the serialized manifest).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reserved.is_empty() && self.settings.is_empty()
    }
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
    /// `hostServices`: host-service names an extension may call, or `"*"` (bundled only).
    pub host_services: Vec<String>,
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
            host_services: Vec::new(),
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

    /// `hostServices` allows calling host service `name` (`"*"` only when `bundled`).
    #[must_use]
    pub fn allows_host_service(&self, name: &str, bundled: bool) -> bool {
        self.host_services
            .iter()
            .any(|grant| if grant == "*" { bundled } else { grant == name })
    }
}

/// Manifest/package validation failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// Not JSON / wrong shape.
    #[error("invalid manifest: {0}")]
    Parse(#[from] serde_json::Error),
    /// Wrong `apiVersion` (e.g. a package built for the 1.x ABI).
    #[error(
        "apiVersion `{0}` is not supported by oino 2.x (expected `{API_VERSION}`): rebuild the \
         extension against oino:extension@{WIT_VERSION}"
    )]
    ApiVersion(String),
    /// Reserved entry id not in the ui-protocol §8 table, or owned by another extension.
    #[error("ui.reserved entry `{0}` is not a reserved id of this extension (ui-protocol §8)")]
    ReservedId(String),
    /// Malformed `ui.settings` declaration.
    #[error("ui.settings entry `{0}` is not a settings namespace declaration")]
    SettingsNamespace(String),
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

/// Settings namespace name rule (ui-protocol §7: `[a-z0-9][a-z0-9.-]*`).
fn valid_namespace(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.'))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
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
        for name in &self.binaries {
            safe_relative(&format!("bin/{name}"))?;
            if Path::new(name).components().count() != 1 || name.is_empty() {
                return Err(ManifestError::PathEscape(name.clone()));
            }
        }
        for id in &self.ui.reserved {
            if !self.owns_reserved(id) {
                return Err(ManifestError::ReservedId(id.clone()));
            }
        }
        for namespace in &self.ui.settings {
            let name = namespace
                .get("namespace")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !valid_namespace(name) || namespace.get("sections").is_none() {
                return Err(ManifestError::SettingsNamespace(name.to_owned()));
            }
        }
        Ok(())
    }

    /// Whether [`RESERVED_IDS`] lists `id` as owned by this extension (ui-protocol §8 rule 1).
    ///
    /// Host-owned ids (`host/*`) are created by the host from its advertised services, never
    /// declared by an extension.
    #[must_use]
    pub fn owns_reserved(&self, id: &str) -> bool {
        RESERVED_IDS.iter().any(|(reserved, owner)| {
            *reserved == id && *owner == self.id.as_str() && *owner != "host"
        })
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
            // A missing `bin/` entry is a load-time warning, not a package error
            // (ext-abi §5.2: the extension still loads; its tools report
            // unavailable) — one platform's dist must not brick another's load.
            if file.starts_with("bin/") && !root.join(safe_relative(file)?).exists() {
                continue;
            }
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

    /// Declared binaries missing from `<root>/bin/` (names, `.exe` accepted
    /// on Windows). Package **build** time (xtask, `oino install`’s staging)
    /// treats a non-empty result as an error; load time treats it as a warning
    /// (ext-abi §5.2).
    #[must_use]
    pub fn missing_binaries(&self, root: &Path) -> Vec<String> {
        self.binaries
            .iter()
            .filter(|name| {
                let exe = format!("{name}.exe");
                let candidates: Vec<&str> = if cfg!(windows) {
                    vec![name.as_str(), exe.as_str()]
                } else {
                    vec![name.as_str()]
                };
                !candidates
                    .iter()
                    .any(|c| root.join("bin").join(c).is_file())
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(extra: &str) -> String {
        format!(
            r#"{{"apiVersion":"oino.extension/2","id":"unipi.notify","name":"Notify","version":"3.0.0"{extra}}}"#
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
        let bad = manifest("").replace("oino.extension/2", "x");
        assert!(matches!(
            Manifest::parse(&bad),
            Err(ManifestError::ApiVersion(_))
        ));
        // A 1.x package names the rebuild in the diagnostic (ext-abi §8).
        let v1 = manifest("").replace("oino.extension/2", "oino.extension/1");
        let message = Manifest::parse(&v1).unwrap_err().to_string();
        assert!(message.contains("oino.extension/1"), "{message}");
        assert!(message.contains("oino:extension@2.0.0"), "{message}");
        assert!(message.contains("rebuild"), "{message}");
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
    fn ui_section() {
        // Omitted by default and not serialized when empty.
        let m = Manifest::parse(&manifest("")).unwrap();
        assert!(m.ui.is_empty());
        assert!(!manifest("").contains("\"ui\""));

        // Reserved ids are accepted only from the ui-protocol §8 table, owned by this package.
        let kanban = r#"{"apiVersion":"oino.extension/2","id":"unipi.kanboard","name":"K","version":"1.0.0",
            "ui":{"reserved":["unipi.kanboard/board"]}}"#;
        let m = Manifest::parse(kanban).unwrap();
        assert_eq!(m.ui.reserved, ["unipi.kanboard/board"]);
        assert!(m.owns_reserved("unipi.kanboard/board"));
        assert!(!m.owns_reserved("unipi.cron/jobs"));
        let foreign = kanban.replace("unipi.kanboard/board", "unipi.cron/jobs");
        assert!(matches!(
            Manifest::parse(&foreign),
            Err(ManifestError::ReservedId(_))
        ));
        // Host-owned ids are created by the host, never declared by an extension.
        let host = kanban.replace("unipi.kanboard/board", "host/git");
        assert!(matches!(
            Manifest::parse(&host),
            Err(ManifestError::ReservedId(_))
        ));
        // Unknown ids are rejected too.
        let unknown = kanban.replace("unipi.kanboard/board", "unipi.kanboard/other");
        assert!(matches!(
            Manifest::parse(&unknown),
            Err(ManifestError::ReservedId(_))
        ));

        // Static settings namespaces (both manifest and init-time are accepted).
        let settings = manifest(
            r#","ui":{"settings":[{"namespace":"unipi.notify","title":"Notify",
               "sections":[{"id":"s","title":"S","fields":[]}]}]}"#,
        );
        let m = Manifest::parse(&settings).unwrap();
        assert_eq!(m.ui.settings.len(), 1);
        let missing = manifest(r#","ui":{"settings":[{"namespace":"a"}]}"#);
        assert!(matches!(
            Manifest::parse(&missing),
            Err(ManifestError::SettingsNamespace(_))
        ));
        let bad_name = settings.replace("\"unipi.notify\",\"title\"", "\"Bad Name\",\"title\"");
        assert!(matches!(
            Manifest::parse(&bad_name),
            Err(ManifestError::SettingsNamespace(_))
        ));
        // Round-trips through serde with the camelCase key `ui`.
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            json.contains(r#""ui":{"reserved":[],"settings":["#),
            "{json}"
        );
    }

    #[test]
    fn permission_helpers() {
        let m = Manifest::parse(&manifest(
            r#","permissions":{"network":{"hosts":["ntfy.sh","*.example.com","*"]},
               "process":{"spawn":true,"allow":["git","rg*"]},
               "credentials":["openai"],"services":{"call":["unipi.memory/save","a/*"]},"env":["HOME"],
               "hostServices":["browser"]}"#,
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
        assert!(p.allows_host_service("browser", false));
        assert!(p.allows_host_service("browser", true)); // name grants are bundle-independent
        assert!(!p.allows_host_service("cron", false));
    }

    #[test]
    fn host_services_wildcard_is_bundled_only() {
        let m = Manifest::parse(&manifest(r#","permissions":{"hostServices":["*"]}"#)).unwrap();
        assert!(!m.permissions.allows_host_service("cron", false));
        assert!(m.permissions.allows_host_service("cron", true));
        // Default: denied.
        assert!(
            !Manifest::parse(&manifest(""))
                .unwrap()
                .permissions
                .allows_host_service("cron", true)
        );
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

    #[test]
    fn binaries_parse_validate_and_probe() {
        let m = Manifest::parse(&manifest(r#","binaries":["unipi-kanboard"]"#)).unwrap();
        assert_eq!(m.binaries, ["unipi-kanboard"]);
        // Default: none.
        assert_eq!(
            Manifest::parse(&manifest("")).unwrap().binaries,
            Vec::<String>::new()
        );
        // A declared binary must be a bare file name.
        for bad in ["../x", "sub/dir/x", "/bin/sh"] {
            assert!(
                Manifest::parse(&manifest(&format!(r#","binaries":["{bad}"]"#))).is_err(),
                "{bad}"
            );
        }
        // Load-time probe: present vs missing under bin/.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        let path = if cfg!(windows) {
            dir.path().join("bin/unipi-kanboard.exe")
        } else {
            dir.path().join("bin/unipi-kanboard")
        };
        std::fs::write(&path, b"#").unwrap();
        assert_eq!(m.missing_binaries(dir.path()), Vec::<String>::new());
        std::fs::remove_file(path).unwrap();
        assert_eq!(m.missing_binaries(dir.path()), ["unipi-kanboard"]);
        assert_eq!(
            m.missing_binaries(tempfile::tempdir().unwrap().path()),
            ["unipi-kanboard"]
        );
        // A missing bin/* checksum entry is skipped, not fatal (ext-abi §5.2).
        std::fs::write(dir.path().join("component.wasm"), b"wasm").unwrap();
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            manifest(
                r#","binaries":["unipi-kanboard"],"checksums":{"bin/unipi-kanboard":"sha256:00"}"#,
            ),
        )
        .unwrap();
        Manifest::load_package(dir.path()).unwrap();
    }
}
