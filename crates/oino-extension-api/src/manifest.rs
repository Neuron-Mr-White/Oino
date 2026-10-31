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

/// Reserved entry ids (ui-protocol §8) and their owners live in a process-wide table the
/// embedding host registers at startup ([`register_reserved_owner`]): every reserved id may
/// only be declared by the extension that owns it (the special `host` owner is reserved for
/// ids the host itself creates from its advertised services and may never be declared by an
/// extension). The table starts empty — a bare SDK knows no first-party panels.
///
/// [`Manifest::owns_reserved`] derives ownership from it.
pub fn register_reserved_owner(id: &str, owner: &str) {
    reserved_owners()
        .write()
        .expect("reserved-owner registry poisoned")
        .insert(id.to_owned(), owner.to_owned());
}

/// The registered owner of a reserved entry id, when any (see [`register_reserved_owner`]).
#[must_use]
pub fn reserved_owner(id: &str) -> Option<String> {
    reserved_owners()
        .read()
        .expect("reserved-owner registry poisoned")
        .get(id)
        .cloned()
}

/// Drops every registered reserved-owner pair (tests and embedder restarts).
pub fn clear_reserved_owners() {
    reserved_owners()
        .write()
        .expect("reserved-owner registry poisoned")
        .clear();
}

fn reserved_owners() -> &'static std::sync::RwLock<std::collections::HashMap<String, String>> {
    static OWNERS: std::sync::OnceLock<
        std::sync::RwLock<std::collections::HashMap<String, String>>,
    > = std::sync::OnceLock::new();
    OWNERS.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

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
/// `reserved` names entries the app renders as a native component; an id must belong to
/// this extension's own namespace or be registered for it (see
/// [`register_reserved_owner`](crate::manifest::register_reserved_owner)). `settings` is the
/// static path for settings namespaces; the runtime `registry.register-settings`
/// declaration stays valid alongside it (both accepted in 2.x).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiManifest {
    /// Reserved entry ids (ui-protocol §8), e.g. `acme.board/board`.
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
            if !self.owns_reserved(id) && !self.in_own_namespace(id) {
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

    /// Whether the host-registered reserved table lists `id` as owned by this extension
    /// (ui-protocol §8 rule 1; see
    /// [`register_reserved_owner`](crate::manifest::register_reserved_owner)).
    ///
    /// Host-owned ids (`host/*`) are created by the host from its advertised services, never
    /// declared by an extension.
    #[must_use]
    pub fn owns_reserved(&self, id: &str) -> bool {
        reserved_owner(id).is_some_and(|owner| {
            owner == self.id.as_str() && owner != "host"
        })
    }

    /// Whether `id` lies in this extension's own namespace (`<extensionId>/<name>`): a
    /// reserved declaration for a native component an embedding host may register. Host
    /// namespaces never qualify — an extension cannot declare `host/*` ids.
    #[must_use]
    pub fn in_own_namespace(&self, id: &str) -> bool {
        self.id.as_str() != "host"
            && id.strip_prefix(self.id.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
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
            r#"{{"apiVersion":"oino.extension/2","id":"acme.notify","name":"Notify","version":"3.0.0"{extra}}}"#
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
        assert!(Manifest::parse(&manifest("").replace("acme.notify", "Bad")).is_err());
    }

    #[test]
    fn ui_section() {
        // Omitted by default and not serialized when empty.
        let m = Manifest::parse(&manifest("")).unwrap();
        assert!(m.ui.is_empty());
        assert!(!manifest("").contains("\"ui\""));

        // Reserved ids in the extension's own namespace parse (the host registers the
        // §8 owners at startup; an unregistered id simply renders as a plain tree panel);
        // foreign and `host/*` ids are rejected.
        clear_reserved_owners();
        let kanban = r#"{"apiVersion":"oino.extension/2","id":"acme.kanboard","name":"K","version":"1.0.0",
            "ui":{"reserved":["acme.kanboard/board"]}}"#;
        let m = Manifest::parse(kanban).unwrap();
        assert_eq!(m.ui.reserved, ["acme.kanboard/board"]);
        assert!(!m.owns_reserved("acme.kanboard/board"), "nothing registered");
        register_reserved_owner("acme.kanboard/board", "acme.kanboard");
        register_reserved_owner("acme.cron/jobs", "acme.cron");
        register_reserved_owner("host/git", "host");
        assert!(m.owns_reserved("acme.kanboard/board"));
        assert!(!m.owns_reserved("acme.cron/jobs"));
        assert!(!m.owns_reserved("host/git"));
        let foreign = kanban.replace("acme.kanboard/board", "acme.cron/jobs");
        assert!(matches!(
            Manifest::parse(&foreign),
            Err(ManifestError::ReservedId(_))
        ));
        // Host-owned ids are created by the host, never declared by an extension.
        let host = kanban.replace("acme.kanboard/board", "host/git");
        assert!(matches!(
            Manifest::parse(&host),
            Err(ManifestError::ReservedId(_))
        ));
        // An id outside the extension's namespace that no host registered is rejected.
        let unknown = kanban.replace("acme.kanboard/board", "acme.kanboard/other");
        assert!(Manifest::parse(&unknown).is_ok(), "own namespace is valid");
        let unregistered = kanban.replace("acme.kanboard/board", "acme.other/board");
        assert!(matches!(
            Manifest::parse(&unregistered),
            Err(ManifestError::ReservedId(_))
        ));
        clear_reserved_owners();

        // Static settings namespaces (both manifest and init-time are accepted).
        let settings = manifest(
            r#","ui":{"settings":[{"namespace":"acme.notify","title":"Notify",
               "sections":[{"id":"s","title":"S","fields":[]}]}]}"#,
        );
        let m = Manifest::parse(&settings).unwrap();
        assert_eq!(m.ui.settings.len(), 1);
        let missing = manifest(r#","ui":{"settings":[{"namespace":"a"}]}"#);
        assert!(matches!(
            Manifest::parse(&missing),
            Err(ManifestError::SettingsNamespace(_))
        ));
        let bad_name = settings.replace("\"acme.notify\",\"title\"", "\"Bad Name\",\"title\"");
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
               "credentials":["openai"],"services":{"call":["acme.memory/save","a/*"]},"env":["HOME"],
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
        assert!(p.allows_service(Some("acme.memory"), "save"));
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
        let m = Manifest::parse(&manifest(r#","binaries":["acme-kanboard"]"#)).unwrap();
        assert_eq!(m.binaries, ["acme-kanboard"]);
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
            dir.path().join("bin/acme-kanboard.exe")
        } else {
            dir.path().join("bin/acme-kanboard")
        };
        std::fs::write(&path, b"#").unwrap();
        assert_eq!(m.missing_binaries(dir.path()), Vec::<String>::new());
        std::fs::remove_file(path).unwrap();
        assert_eq!(m.missing_binaries(dir.path()), ["acme-kanboard"]);
        assert_eq!(
            m.missing_binaries(tempfile::tempdir().unwrap().path()),
            ["acme-kanboard"]
        );
        // A missing bin/* checksum entry is skipped, not fatal (ext-abi §5.2).
        std::fs::write(dir.path().join("component.wasm"), b"wasm").unwrap();
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            manifest(
                r#","binaries":["acme-kanboard"],"checksums":{"bin/acme-kanboard":"sha256:00"}"#,
            ),
        )
        .unwrap();
        Manifest::load_package(dir.path()).unwrap();
    }
}
