#![doc = r#"Shared identity, manifest, package, permission, provenance, diagnostic,
compatibility, and conflict contracts for the Oino extension kernel.

This crate is deliberately data-oriented. It must stay independent of TUI,
provider, harness, runtime, filesystem, and package-manager implementation
crates so it can be reused by Oino core, future WASM hosts, validators, SDKs,
and tests.
"#]
#![forbid(unsafe_code)]

use semver::{Version, VersionReq};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{collections::BTreeSet, fmt, path::PathBuf};
use thiserror::Error;

pub const MANIFEST_FILE: &str = "oino.extension.json";
pub const PACKAGE_MANIFEST_FILE: &str = "oino.package.json";
pub const CURRENT_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion(1);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExtensionCoreError {
    #[error("{kind} is required")]
    MissingIdentifier { kind: &'static str },
    #[error("{kind} `{value}` is invalid; use lowercase letters, digits, '.', '_' or '-', separated by non-empty segments")]
    InvalidIdentifier { kind: &'static str, value: String },
    #[error("protocol version {0} is not supported by this Oino build")]
    UnsupportedProtocol(u16),
    #[error("runtime entry is required for {0} extensions")]
    MissingRuntimeEntry(RuntimeKind),
    #[error("Oino compatibility requirement `{requirement}` is invalid: {message}")]
    InvalidCompatibility {
        requirement: String,
        message: String,
    },
    #[error("manifest contribution `{contribution_id}` requires permission `{permission}`")]
    MissingContributionPermission {
        contribution_id: ContributionId,
        permission: String,
    },
    #[error("package manifest must reference at least one extension, resource, or asset")]
    EmptyPackage,
}

macro_rules! define_id_type {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ExtensionCoreError> {
                let value = value.into();
                validate_identifier($kind, &value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

define_id_type!(ExtensionId, "extension id");
define_id_type!(PackageId, "package id");
define_id_type!(ContributionId, "contribution id");

fn validate_identifier(kind: &'static str, value: &str) -> Result<(), ExtensionCoreError> {
    if value.is_empty() {
        return Err(ExtensionCoreError::MissingIdentifier { kind });
    }
    if value.starts_with('.')
        || value.starts_with('_')
        || value.starts_with('-')
        || value.ends_with('.')
        || value.ends_with('_')
        || value.ends_with('-')
        || value.contains("..")
        || value.contains("__")
        || value.contains("--")
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(ExtensionCoreError::InvalidIdentifier {
            kind,
            value: value.into(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProtocolVersion(pub u16);

impl ProtocolVersion {
    #[must_use]
    pub fn is_supported(self) -> bool {
        self.0 > 0 && self.0 <= CURRENT_PROTOCOL_VERSION.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OinoCompatibility {
    requirement: String,
    parsed: VersionReq,
}

impl OinoCompatibility {
    pub fn parse(requirement: impl Into<String>) -> Result<Self, ExtensionCoreError> {
        let requirement = requirement.into();
        let parsed = VersionReq::parse(&requirement).map_err(|err| {
            ExtensionCoreError::InvalidCompatibility {
                requirement: requirement.clone(),
                message: err.to_string(),
            }
        })?;
        Ok(Self {
            requirement,
            parsed,
        })
    }

    #[must_use]
    pub fn matches(&self, current: &Version) -> bool {
        self.parsed.matches(current)
    }

    #[must_use]
    pub fn requirement(&self) -> &str {
        &self.requirement
    }
}

impl Default for OinoCompatibility {
    fn default() -> Self {
        Self {
            requirement: "*".into(),
            parsed: VersionReq::STAR,
        }
    }
}

impl fmt::Display for OinoCompatibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.requirement)
    }
}

impl Serialize for OinoCompatibility {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.requirement)
    }
}

impl<'de> Deserialize<'de> for OinoCompatibility {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let requirement = String::deserialize(deserializer)?;
        Self::parse(requirement).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceScope {
    BuiltIn,
    Global,
    Project,
    Session,
    Development,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    BuiltIn,
    LocalExtension,
    LocalPackage,
    InstalledPackage,
    RegistryPackage,
    WasmModule,
    NativeSidecar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Discovered,
    Validated,
    Enabled,
    Disabled,
    Active,
    Unhealthy,
    Blocked,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub scope: SourceScope,
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: ExtensionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<PackageId>,
    #[serde(default)]
    pub display_name: String,
    pub version: Version,
    #[serde(default)]
    pub oino: OinoCompatibility,
    #[serde(default = "current_protocol_version")]
    pub protocol: ProtocolVersion,
    pub runtime: RuntimeDescriptor,
    #[serde(default)]
    pub permissions: ExtensionPermissions,
    #[serde(default)]
    pub contributes: ExtensionContributions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(default)]
    pub metadata: Value,
}

impl ExtensionManifest {
    pub fn validate(&self) -> Result<(), ExtensionCoreError> {
        if !self.protocol.is_supported() {
            return Err(ExtensionCoreError::UnsupportedProtocol(self.protocol.0));
        }
        self.runtime.validate()?;
        self.contributes.validate_permissions(&self.permissions)?;
        Ok(())
    }

    #[must_use]
    pub fn compatible_with(&self, current_version: &Version) -> bool {
        self.oino.matches(current_version)
    }

    #[must_use]
    pub fn display_label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            self.id.as_str()
        } else {
            &self.display_name
        }
    }
}

fn current_protocol_version() -> ProtocolVersion {
    CURRENT_PROTOCOL_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    #[serde(default)]
    pub kind: RuntimeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
}

impl RuntimeDescriptor {
    pub fn validate(&self) -> Result<(), ExtensionCoreError> {
        let missing_entry = self
            .entry
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true);
        if self.kind.requires_entry() && missing_entry {
            return Err(ExtensionCoreError::MissingRuntimeEntry(self.kind));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    #[default]
    Wasm,
    BuiltIn,
    NativeSidecar,
}

impl RuntimeKind {
    #[must_use]
    pub fn requires_entry(self) -> bool {
        !matches!(self, Self::BuiltIn)
    }
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Wasm => "wasm",
            Self::BuiltIn => "built-in",
            Self::NativeSidecar => "native sidecar",
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionPermissions {
    #[serde(default)]
    pub tools: BTreeSet<String>,
    #[serde(default)]
    pub commands: BTreeSet<String>,
    #[serde(default)]
    pub host_capabilities: BTreeSet<String>,
    #[serde(default)]
    pub ui: BTreeSet<String>,
    #[serde(default)]
    pub filesystem: Vec<FilesystemPermission>,
    #[serde(default)]
    pub shell_process: ShellProcessPermission,
    #[serde(default)]
    pub network: NetworkPermission,
    #[serde(default)]
    pub secrets: BTreeSet<String>,
    #[serde(default)]
    pub session_persistence: BTreeSet<PersistenceScope>,
    #[serde(default)]
    pub provider_mutation: BTreeSet<ProviderMutationKind>,
    #[serde(default)]
    pub package_management: BTreeSet<PackageOperation>,
}

impl ExtensionPermissions {
    #[must_use]
    pub fn allows_tool(&self, id: &ContributionId) -> bool {
        allows_named(&self.tools, id.as_str())
    }

    #[must_use]
    pub fn allows_command(&self, id: &ContributionId) -> bool {
        allows_named(&self.commands, id.as_str())
    }

    #[must_use]
    pub fn allows_host_capability(&self, capability: &str) -> bool {
        allows_named(&self.host_capabilities, capability)
    }

    #[must_use]
    pub fn allows_ui_surface(&self, surface: &str) -> bool {
        allows_named(&self.ui, surface)
    }
}

fn allows_named(values: &BTreeSet<String>, name: &str) -> bool {
    values.contains("*") || values.contains(name)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemPermission {
    pub access: FilesystemAccess,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellProcessPermission {
    #[serde(default)]
    pub allowed: bool,
    #[serde(default)]
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPermission {
    #[serde(default)]
    pub raw_network: bool,
    #[serde(default)]
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceScope {
    Session,
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMutationKind {
    Metadata,
    RequestHeaders,
    RequestBody,
    ResponseMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOperation {
    Install,
    Update,
    Remove,
    Publish,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtensionContributions {
    #[serde(default)]
    pub tools: Vec<ToolContribution>,
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub keymaps: Vec<KeymapContribution>,
    #[serde(default)]
    pub hooks: Vec<HookContribution>,
    #[serde(default)]
    pub ui_surfaces: Vec<UiSurfaceContribution>,
    #[serde(default)]
    pub settings_pages: Vec<SettingsPageContribution>,
    #[serde(default)]
    pub themes: Vec<ThemeContribution>,
    #[serde(default)]
    pub providers: Vec<ProviderContribution>,
    #[serde(default)]
    pub resources: Vec<ResourceContribution>,
    #[serde(default)]
    pub autosuggest_providers: Vec<AutosuggestContribution>,
    #[serde(default)]
    pub renderers: Vec<RendererContribution>,
}

impl ExtensionContributions {
    pub fn validate_permissions(
        &self,
        permissions: &ExtensionPermissions,
    ) -> Result<(), ExtensionCoreError> {
        for contribution in &self.tools {
            if !permissions.allows_tool(&contribution.id) {
                return Err(missing_permission(&contribution.id, "tools"));
            }
        }
        for contribution in &self.commands {
            if !permissions.allows_command(&contribution.id) {
                return Err(missing_permission(&contribution.id, "commands"));
            }
        }
        for contribution in &self.ui_surfaces {
            if !permissions.allows_ui_surface(contribution.surface.as_permission_name()) {
                return Err(missing_permission(
                    &contribution.id,
                    contribution.surface.as_permission_name(),
                ));
            }
        }
        Ok(())
    }
}

fn missing_permission(id: &ContributionId, permission: &str) -> ExtensionCoreError {
    ExtensionCoreError::MissingContributionPermission {
        contribution_id: id.clone(),
        permission: permission.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolContribution {
    pub id: ContributionId,
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub execution_mode: ToolExecutionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    #[default]
    Parallel,
    Sequential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandContribution {
    pub id: ContributionId,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeymapContribution {
    pub id: ContributionId,
    pub action: String,
    #[serde(default)]
    pub default_bindings: Vec<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookContribution {
    pub id: ContributionId,
    pub event: HookEventKind,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub mode: HookMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    Startup,
    ResourceDiscovery,
    Session,
    Input,
    Command,
    BeforeAgentTurn,
    AfterAgentTurn,
    Context,
    ProviderRequest,
    ProviderResponse,
    MessageStream,
    ToolCall,
    ToolResult,
    ToolUpdate,
    ModelSelection,
    Compaction,
    Reload,
    PackageLifecycle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookMode {
    #[default]
    Observe,
    Mutable,
    Cancellable,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceContribution {
    pub id: ContributionId,
    pub surface: UiSurfaceKind,
    pub title: String,
    #[serde(default)]
    pub state_schema: Option<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSurfaceKind {
    Sidebar,
    FloatingPanel,
    Footer,
    MainPanel,
    SettingsPage,
    Autosuggest,
    TranscriptRenderer,
    MessageRenderer,
    ToolRenderer,
    Notification,
    Health,
}

impl UiSurfaceKind {
    #[must_use]
    pub fn as_permission_name(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::FloatingPanel => "floating_panel",
            Self::Footer => "footer",
            Self::MainPanel => "main_panel",
            Self::SettingsPage => "settings_page",
            Self::Autosuggest => "autosuggest",
            Self::TranscriptRenderer => "transcript_renderer",
            Self::MessageRenderer => "message_renderer",
            Self::ToolRenderer => "tool_renderer",
            Self::Notification => "notification",
            Self::Health => "health",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsPageContribution {
    pub id: ContributionId,
    pub title: String,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeContribution {
    pub id: ContributionId,
    pub path: String,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContribution {
    pub id: ContributionId,
    pub provider_id: String,
    #[serde(default)]
    pub model_ids: Vec<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceContribution {
    pub id: ContributionId,
    pub kind: ResourceKind,
    pub path: String,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Prompt,
    Skill,
    SystemPrompt,
    ProjectInstructions,
    Theme,
    Asset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutosuggestContribution {
    pub id: ContributionId,
    pub trigger: String,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererContribution {
    pub id: ContributionId,
    pub target: RendererTarget,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererTarget {
    TranscriptMessage,
    ToolCall,
    ToolResult,
    MarkdownBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub id: PackageId,
    #[serde(default)]
    pub display_name: String,
    pub version: Version,
    #[serde(default)]
    pub oino: OinoCompatibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub source: Option<SourceDescriptor>,
    #[serde(default)]
    pub extensions: Vec<PackageExtensionRef>,
    #[serde(default)]
    pub resources: Vec<PackageResourceRef>,
    #[serde(default)]
    pub assets: Vec<PackageAssetRef>,
    #[serde(default)]
    pub dependencies: Vec<PackageDependency>,
    #[serde(default)]
    pub permissions: ExtensionPermissions,
    #[serde(default)]
    pub trust: TrustMetadata,
}

impl PackageManifest {
    pub fn validate(&self) -> Result<(), ExtensionCoreError> {
        if self.extensions.is_empty() && self.resources.is_empty() && self.assets.is_empty() {
            return Err(ExtensionCoreError::EmptyPackage);
        }
        Ok(())
    }

    #[must_use]
    pub fn compatible_with(&self, current_version: &Version) -> bool {
        self.oino.matches(current_version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageExtensionRef {
    pub manifest: String,
    #[serde(default = "default_enabled")]
    pub enabled_by_default: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageResourceRef {
    pub kind: ResourceKind,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageAssetRef {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDependency {
    pub id: PackageId,
    pub version: OinoCompatibility,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustMetadata {
    #[serde(default)]
    pub reviewed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default)]
    pub advisories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source: SourceDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<PackageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionDiagnostic {
    pub severity: DiagnosticSeverity,
    pub phase: DiagnosticPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<PackageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contribution_id: Option<ContributionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    #[serde(default)]
    pub health: HealthState,
}

impl ExtensionDiagnostic {
    #[must_use]
    pub fn format_message(&self) -> String {
        let mut parts = vec![format!("{:?}", self.severity).to_lowercase()];
        parts.push(format!("phase={:?}", self.phase).to_lowercase());
        if let Some(package_id) = &self.package_id {
            parts.push(format!("package={package_id}"));
        }
        if let Some(extension_id) = &self.extension_id {
            parts.push(format!("extension={extension_id}"));
        }
        if let Some(contribution_id) = &self.contribution_id {
            parts.push(format!("contribution={contribution_id}"));
        }
        if let Some(source_path) = &self.source_path {
            parts.push(format!("path={}", source_path.display()));
        }
        let mut message = format!("{}: {}", parts.join(" "), self.message);
        if let Some(remediation) = &self.remediation {
            message.push_str("; remediation: ");
            message.push_str(remediation);
        }
        message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPhase {
    Discovery,
    ManifestParse,
    Compatibility,
    Permission,
    RegistryComposition,
    RuntimeLoad,
    RuntimeExecute,
    UiUpdate,
    PackageInstall,
    PackageUpdate,
    PackageRemove,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    #[default]
    Healthy,
    Degraded,
    Unhealthy,
    Disabled,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    Namespaced,
    FirstWins,
    LastWins,
    UserOverride,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictPolicy {
    #[serde(default = "default_conflict_strategy")]
    pub strategy: ConflictStrategy,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_user_override")]
    pub allow_user_override: bool,
}

impl Default for ConflictPolicy {
    fn default() -> Self {
        Self {
            strategy: default_conflict_strategy(),
            priority: 0,
            allow_user_override: default_user_override(),
        }
    }
}

fn default_conflict_strategy() -> ConflictStrategy {
    ConflictStrategy::Namespaced
}

fn default_user_override() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn manifest_identity_permissions_and_serde_round_trip() -> Result<(), Box<dyn Error>> {
        let json = r#"
        {
          "id": "acme.web",
          "display_name": "Acme Web",
          "version": "1.2.3",
          "oino": "^0.1",
          "runtime": { "kind": "wasm", "entry": "plugin.wasm", "abi": "json-v1" },
          "permissions": {
            "tools": ["web_search"],
            "host_capabilities": ["host.web.search"],
            "ui": ["sidebar"],
            "filesystem": [{ "access": "read", "paths": ["docs/**"] }],
            "shell_process": { "allowed": false },
            "network": { "raw_network": false, "hosts": [] },
            "secrets": ["openrouter"],
            "session_persistence": ["project"],
            "provider_mutation": ["metadata"],
            "package_management": ["install"]
          },
          "contributes": {
            "tools": [{
              "id": "web_search",
              "description": "Search the web",
              "input_schema": { "type": "object" },
              "execution_mode": "parallel"
            }],
            "ui_surfaces": [{
              "id": "web.sidebar",
              "surface": "sidebar",
              "title": "Web"
            }]
          }
        }
        "#;
        let manifest: ExtensionManifest = serde_json::from_str(json)?;
        manifest.validate()?;
        let current = Version::parse("0.1.5")?;
        assert!(manifest.compatible_with(&current));
        assert_eq!(manifest.display_label(), "Acme Web");
        let encoded = serde_json::to_string(&manifest)?;
        let decoded: ExtensionManifest = serde_json::from_str(&encoded)?;
        assert_eq!(decoded.id.as_str(), "acme.web");
        assert!(decoded
            .permissions
            .allows_host_capability("host.web.search"));
        Ok(())
    }

    #[test]
    fn invalid_extension_id_is_rejected() {
        let json = r#"
        {
          "id": "Bad Id",
          "version": "1.0.0",
          "runtime": { "kind": "wasm", "entry": "plugin.wasm" }
        }
        "#;
        let result = serde_json::from_str::<ExtensionManifest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn source_identity_round_trips() -> Result<(), Box<dyn Error>> {
        let source = SourceDescriptor {
            scope: SourceScope::Project,
            kind: SourceKind::LocalPackage,
            path: Some(PathBuf::from(".oino/extensions/acme")),
            registry: None,
        };
        let encoded = serde_json::to_string(&source)?;
        let decoded: SourceDescriptor = serde_json::from_str(&encoded)?;
        assert_eq!(decoded.scope, SourceScope::Project);
        assert_eq!(decoded.kind, SourceKind::LocalPackage);
        Ok(())
    }

    #[test]
    fn compatibility_rejection_is_explicit() -> Result<(), Box<dyn Error>> {
        let manifest: ExtensionManifest = serde_json::from_str(
            r#"{
              "id": "acme.future",
              "version": "1.0.0",
              "oino": ">=2.0.0",
              "runtime": { "kind": "wasm", "entry": "future.wasm" }
            }"#,
        )?;
        manifest.validate()?;
        let current = Version::parse("0.1.0")?;
        assert!(!manifest.compatible_with(&current));

        let invalid = serde_json::from_str::<ExtensionManifest>(
            r#"{
              "id": "acme.badcompat",
              "version": "1.0.0",
              "oino": "not a version req",
              "runtime": { "kind": "wasm", "entry": "future.wasm" }
            }"#,
        );
        assert!(invalid.is_err());
        Ok(())
    }

    #[test]
    fn package_manifest_covers_installed_and_registry_sources() -> Result<(), Box<dyn Error>> {
        let package: PackageManifest = serde_json::from_str(
            r#"{
              "id": "acme.extension-pack",
              "display_name": "Acme Extension Pack",
              "version": "0.3.0",
              "oino": "^0.1",
              "publisher": "acme",
              "source": { "scope": "global", "kind": "registry_package", "registry": "fixture" },
              "extensions": [{ "manifest": "extensions/web/oino.extension.json" }],
              "resources": [{ "kind": "skill", "path": "skills/review/SKILL.md" }],
              "assets": [{ "path": "assets/icon.png", "checksum": "sha256:test" }],
              "trust": { "reviewed": true, "checksum": "sha256:package" }
            }"#,
        )?;
        package.validate()?;
        assert!(package.compatible_with(&Version::parse("0.1.2")?));
        assert_eq!(package.extensions.len(), 1);
        assert!(package.trust.reviewed);
        Ok(())
    }

    #[test]
    fn empty_package_is_invalid() -> Result<(), Box<dyn Error>> {
        let package: PackageManifest = serde_json::from_str(
            r#"{
              "id": "acme.empty",
              "version": "1.0.0"
            }"#,
        )?;
        let result = package.validate();
        assert!(matches!(result, Err(ExtensionCoreError::EmptyPackage)));
        Ok(())
    }

    #[test]
    fn missing_contribution_permission_is_reported() -> Result<(), Box<dyn Error>> {
        let manifest: ExtensionManifest = serde_json::from_str(
            r#"{
              "id": "acme.tool",
              "version": "1.0.0",
              "runtime": { "kind": "wasm", "entry": "tool.wasm" },
              "contributes": {
                "tools": [{ "id": "danger_tool", "description": "danger" }]
              }
            }"#,
        )?;
        let result = manifest.validate();
        assert!(matches!(
            result,
            Err(ExtensionCoreError::MissingContributionPermission { .. })
        ));
        Ok(())
    }

    #[test]
    fn diagnostic_formatting_is_actionable() -> Result<(), Box<dyn Error>> {
        let diagnostic = ExtensionDiagnostic {
            severity: DiagnosticSeverity::Error,
            phase: DiagnosticPhase::Permission,
            package_id: Some(PackageId::new("acme.pack")?),
            extension_id: Some(ExtensionId::new("acme.web")?),
            contribution_id: Some(ContributionId::new("web_search")?),
            source_path: Some(PathBuf::from(".oino/extensions/acme/oino.extension.json")),
            message: "missing host capability permission".into(),
            remediation: Some("add host.web.search or disable the tool".into()),
            health: HealthState::Blocked,
        };
        let message = diagnostic.format_message();
        assert!(message.contains("extension=acme.web"));
        assert!(message.contains("contribution=web_search"));
        assert!(message.contains("remediation"));
        Ok(())
    }

    #[test]
    fn conflict_policy_defaults_to_namespaced_user_override() {
        let policy = ConflictPolicy::default();
        assert_eq!(policy.strategy, ConflictStrategy::Namespaced);
        assert_eq!(policy.priority, 0);
        assert!(policy.allow_user_override);
    }
}
