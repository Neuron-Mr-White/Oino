#![doc = r#"Shared data contracts for the Oino extension kernel.

`oino-extension-core` defines the serializable vocabulary that every extension-facing
crate agrees on: identifiers, manifests, package metadata, permissions, contributions,
registries, policy, provenance, diagnostics, health, persistence, UI-surface contracts,
and community registry metadata.

## Boundary

This crate is deliberately data-oriented. It validates shapes and composes registry
state, but it does not discover files, install packages, execute extension code, broker
runtime capabilities, render TUI surfaces, persist session files, call providers, or run
tools. Those responsibilities belong to the manager, runtime, app/TUI, session,
provider, and tool crates. Keeping this crate independent lets Oino core, future WASM
hosts, SDK validators, devkit commands, and tests reuse one contract instead of drifting
schemas.

## Public API map

- [`ExtensionId`], [`PackageId`], and [`ContributionId`] are validated lowercase id
  newtypes used across manifests, registry keys, diagnostics, and policy settings.
- [`MANIFEST_FILE`], [`PACKAGE_MANIFEST_FILE`], [`CURRENT_PROTOCOL_VERSION`],
  [`ProtocolVersion`], [`OinoCompatibility`], [`SourceDescriptor`], [`SourceScope`],
  [`SourceKind`], and [`LifecycleState`] describe compatibility and provenance without
  touching the filesystem.
- [`ExtensionManifest`], [`PackageManifest`], [`RuntimeDescriptor`], [`RuntimeKind`],
  and [`ExtensionPermissions`] are the JSON manifest contracts reviewed before runtime
  code is visible or executable.
- [`ExtensionContributions`] groups declarative contribution families such as
  [`ToolContribution`], [`CommandContribution`], [`KeymapContribution`],
  [`HookContribution`], [`UiSurfaceContribution`], [`SettingsPageContribution`],
  [`ThemeContribution`], [`ProviderContribution`], [`ResourceContribution`],
  [`PersistenceContribution`], [`AutosuggestContribution`], [`RendererContribution`],
  [`DiagnosticContribution`], and [`HealthContribution`].
- [`UiSurfaceKind`], [`UiLayoutPolicy`], [`UiFocusPolicy`], [`UiKeyDispatchPolicy`],
  [`UiSurfaceStateUpdate`], and [`UiSurfaceValidationError`] define host-rendered UI
  contracts while keeping Ratatui state out of the manifest layer.
- [`PersistenceRecord`] and [`ExtensionSessionEntry`] keep extension-owned state
  inspectable as typed JSON data without loading extension runtime code.
- [`CommunityRegistryIndex`], [`CommunityPackageMetadata`], [`TrustMetadata`],
  [`SecurityAdvisory`], [`Provenance`], [`ExtensionDiagnostic`], and [`HealthState`]
  describe registry, trust, audit, and health surfaces shown by higher layers.
- [`ContributionMetadata`], [`RegistryEntryKey`], [`RegistryPolicy`],
  [`ContributionRegistry`], [`TypedContributionRegistry`], [`RegistrySnapshot`],
  [`RegistryDiff`], [`RegistryFamily`], and the `*Registry` type aliases are the generic
  policy/composition layer used by built-ins and external packages.
- [`ExtensionCoreError`] and [`RegistryValidationError`] keep manifest and registry
  failures typed so manager, SDK, and app code can show actionable diagnostics.

## Contributor rules

Keep this crate dependency-light, deterministic, and serialization-first. Prefer typed
Serde-compatible structs and enums over open-ended JSON unless the field is intentionally
extension-defined. Do not add filesystem discovery, package lifecycle side effects,
runtime execution, capability policy, provider protocol details, or TUI rendering here.
When changing manifest fields, enum tags, id validation, compatibility checks, registry
precedence, or permission semantics, update the extension kernel docs, SDK templates,
fixture manifests, validation tests, and any user-facing install/review guidance in the
same change.
"#]
#![forbid(unsafe_code)]

use semver::{Version, VersionReq};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
};
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    #[default]
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

    #[must_use]
    pub fn allows_persistence_scope(&self, scope: PersistenceScope) -> bool {
        self.session_persistence.contains(&scope)
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
    pub auth_providers: Vec<AuthContribution>,
    #[serde(default)]
    pub resources: Vec<ResourceContribution>,
    #[serde(default)]
    pub persistence: Vec<PersistenceContribution>,
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
        for contribution in &self.persistence {
            if !permissions.allows_persistence_scope(contribution.scope) {
                return Err(missing_permission(&contribution.id, "session_persistence"));
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
    pub context: String,
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
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    ThinkingSelection,
    Compaction,
    Tree,
    Reload,
    Install,
    Update,
    Remove,
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
    pub layout: UiLayoutPolicy,
    #[serde(default)]
    pub visibility: UiVisibilityPolicy,
    #[serde(default)]
    pub focus: UiFocusPolicy,
    #[serde(default)]
    pub key_dispatch: UiKeyDispatchPolicy,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSurfaceKind {
    Sidebar,
    FloatingPanel,
    Footer,
    FooterTop,
    FooterBottom,
    InlineStatus,
    MainPanel,
    Header,
    WidgetAboveComposer,
    WidgetBelowComposer,
    SettingsPage,
    Autosuggest,
    Overlay,
    TranscriptRenderer,
    MessageRenderer,
    Theme,
    ToolCallRenderer,
    ToolResultRenderer,
    ToolRenderer,
    Notification,
    Status,
    WorkingIndicator,
    Editor,
    Health,
}

impl UiSurfaceKind {
    #[must_use]
    pub fn as_permission_name(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::FloatingPanel => "floating_panel",
            Self::Footer => "footer",
            Self::FooterTop => "footer_top",
            Self::FooterBottom => "footer_bottom",
            Self::InlineStatus => "inline_status",
            Self::MainPanel => "main_panel",
            Self::Header => "header",
            Self::WidgetAboveComposer => "widget_above_composer",
            Self::WidgetBelowComposer => "widget_below_composer",
            Self::SettingsPage => "settings_page",
            Self::Autosuggest => "autosuggest",
            Self::Overlay => "overlay",
            Self::TranscriptRenderer => "transcript_renderer",
            Self::MessageRenderer => "message_renderer",
            Self::Theme => "theme",
            Self::ToolCallRenderer => "tool_call_renderer",
            Self::ToolResultRenderer => "tool_result_renderer",
            Self::ToolRenderer => "tool_renderer",
            Self::Notification => "notification",
            Self::Status => "status",
            Self::WorkingIndicator => "working_indicator",
            Self::Editor => "editor",
            Self::Health => "health",
        }
    }
}

impl UiSurfaceKind {
    #[must_use]
    pub fn default_slot(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar:right",
            Self::FloatingPanel => "floating:center",
            Self::Footer => "footer:status",
            Self::FooterTop => "footer:top",
            Self::FooterBottom => "footer:bottom",
            Self::InlineStatus => "status:inline",
            Self::MainPanel => "main:primary",
            Self::Header => "header:top",
            Self::WidgetAboveComposer => "composer:above",
            Self::WidgetBelowComposer => "composer:below",
            Self::SettingsPage => "settings:extension",
            Self::Autosuggest => "autosuggest:provider",
            Self::Overlay => "overlay:extension",
            Self::TranscriptRenderer => "renderer:transcript",
            Self::MessageRenderer => "renderer:message",
            Self::Theme => "theme:tokens",
            Self::ToolCallRenderer => "renderer:tool-call",
            Self::ToolResultRenderer => "renderer:tool-result",
            Self::ToolRenderer => "renderer:tool",
            Self::Notification => "notification:status",
            Self::Status => "status:footer",
            Self::WorkingIndicator => "working:indicator",
            Self::Editor => "editor:main",
            Self::Health => "health:summary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiLayoutPolicy {
    #[serde(default = "default_ui_slot")]
    pub slot: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_ui_min_width")]
    pub min_width: u16,
    #[serde(default = "default_ui_min_height")]
    pub min_height: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u16>,
    #[serde(default)]
    pub tiny_terminal: UiTinyTerminalFallback,
}

impl Default for UiLayoutPolicy {
    fn default() -> Self {
        Self {
            slot: default_ui_slot(),
            priority: 0,
            min_width: default_ui_min_width(),
            min_height: default_ui_min_height(),
            max_width: None,
            tiny_terminal: UiTinyTerminalFallback::default(),
        }
    }
}

fn default_ui_slot() -> String {
    "primary".into()
}

fn default_ui_min_width() -> u16 {
    20
}

fn default_ui_min_height() -> u16 {
    3
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiTinyTerminalFallback {
    Hide,
    #[default]
    CompactBadge,
    StatusLine,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiFocusPolicy {
    #[default]
    None,
    Focusable,
    ModalTrap,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiVisibilityPolicy {
    Hidden,
    #[default]
    Visible,
    UserToggleable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiKeyDispatchPolicy {
    #[serde(default)]
    pub scopes: BTreeSet<String>,
    #[serde(default = "default_key_pass_through")]
    pub pass_through: bool,
}

fn default_key_pass_through() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSurfaceStateUpdate {
    pub surface_id: ContributionId,
    pub owner_extension_id: ExtensionId,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub actions: Vec<UiSurfaceAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceAction {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceConflict {
    pub surface: UiSurfaceKind,
    pub slot: String,
    pub owners: Vec<ContributionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSurfaceLayoutDecision {
    Render,
    CompactBadge,
    StatusLine,
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiSurfaceValidationError {
    UnknownSurface {
        surface_id: ContributionId,
    },
    MissingOwner {
        surface_id: ContributionId,
    },
    InvalidOwner {
        surface_id: ContributionId,
        expected: ExtensionId,
        actual: ExtensionId,
    },
    BadStateShape {
        surface_id: ContributionId,
        expected: String,
    },
    BlankActionId {
        surface_id: ContributionId,
    },
    BlankActionLabel {
        surface_id: ContributionId,
    },
    UndeclaredKeyScope {
        surface_id: ContributionId,
        scope: String,
    },
}

impl fmt::Display for UiSurfaceValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSurface { surface_id } => {
                write!(formatter, "unknown UI surface `{surface_id}`")
            }
            Self::MissingOwner { surface_id } => write!(
                formatter,
                "UI surface `{surface_id}` does not declare an owning extension"
            ),
            Self::InvalidOwner {
                surface_id,
                expected,
                actual,
            } => write!(
                formatter,
                "UI surface `{surface_id}` is owned by `{expected}`, not `{actual}`"
            ),
            Self::BadStateShape {
                surface_id,
                expected,
            } => write!(
                formatter,
                "UI surface `{surface_id}` state must match `{expected}`"
            ),
            Self::BlankActionId { surface_id } => {
                write!(formatter, "UI surface `{surface_id}` action id is blank")
            }
            Self::BlankActionLabel { surface_id } => {
                write!(formatter, "UI surface `{surface_id}` action label is blank")
            }
            Self::UndeclaredKeyScope { surface_id, scope } => write!(
                formatter,
                "UI surface `{surface_id}` action uses undeclared key scope `{scope}`"
            ),
        }
    }
}

impl std::error::Error for UiSurfaceValidationError {}

pub fn validate_ui_surface_update(
    contribution: &ActiveContribution<UiSurfaceContribution>,
    update: &UiSurfaceStateUpdate,
) -> Result<(), UiSurfaceValidationError> {
    if contribution.effective_id != update.surface_id {
        return Err(UiSurfaceValidationError::UnknownSurface {
            surface_id: update.surface_id.clone(),
        });
    }
    let Some(expected_owner) = contribution.entry.metadata.extension_id.clone() else {
        return Err(UiSurfaceValidationError::MissingOwner {
            surface_id: update.surface_id.clone(),
        });
    };
    if expected_owner != update.owner_extension_id {
        return Err(UiSurfaceValidationError::InvalidOwner {
            surface_id: update.surface_id.clone(),
            expected: expected_owner,
            actual: update.owner_extension_id.clone(),
        });
    }
    if let Some(expected) = contribution.entry.contribution.state_schema.as_deref() {
        let valid = match expected {
            "any" => true,
            "object" => update.state.is_object(),
            "array" => update.state.is_array(),
            "string" => update.state.is_string(),
            "number" => update.state.is_number(),
            "boolean" => update.state.is_boolean(),
            _ => update.state.is_object(),
        };
        if !valid {
            return Err(UiSurfaceValidationError::BadStateShape {
                surface_id: update.surface_id.clone(),
                expected: expected.into(),
            });
        }
    }
    for action in &update.actions {
        if action.id.trim().is_empty() {
            return Err(UiSurfaceValidationError::BlankActionId {
                surface_id: update.surface_id.clone(),
            });
        }
        if action.label.trim().is_empty() {
            return Err(UiSurfaceValidationError::BlankActionLabel {
                surface_id: update.surface_id.clone(),
            });
        }
        if let Some(scope) = action.key_scope.as_deref() {
            if !contribution
                .entry
                .contribution
                .key_dispatch
                .scopes
                .contains(scope)
            {
                return Err(UiSurfaceValidationError::UndeclaredKeyScope {
                    surface_id: update.surface_id.clone(),
                    scope: scope.into(),
                });
            }
        }
    }
    Ok(())
}

#[must_use]
pub fn ui_surface_layout_decision(
    contribution: &UiSurfaceContribution,
    terminal_width: u16,
    terminal_height: u16,
) -> UiSurfaceLayoutDecision {
    if contribution.visibility == UiVisibilityPolicy::Hidden {
        return UiSurfaceLayoutDecision::Hide;
    }
    if terminal_width < contribution.layout.min_width
        || terminal_height < contribution.layout.min_height
    {
        return match contribution.layout.tiny_terminal {
            UiTinyTerminalFallback::Hide => UiSurfaceLayoutDecision::Hide,
            UiTinyTerminalFallback::CompactBadge => UiSurfaceLayoutDecision::CompactBadge,
            UiTinyTerminalFallback::StatusLine => UiSurfaceLayoutDecision::StatusLine,
        };
    }
    UiSurfaceLayoutDecision::Render
}

#[must_use]
pub fn detect_ui_surface_conflicts(
    contributions: &[ActiveContribution<UiSurfaceContribution>],
) -> Vec<UiSurfaceConflict> {
    let mut by_slot: BTreeMap<(UiSurfaceKind, String), Vec<ContributionId>> = BTreeMap::new();
    for contribution in contributions {
        let slot = if contribution.entry.contribution.layout.slot == "primary" {
            contribution
                .entry
                .contribution
                .surface
                .default_slot()
                .to_string()
        } else {
            contribution.entry.contribution.layout.slot.clone()
        };
        by_slot
            .entry((contribution.entry.contribution.surface, slot))
            .or_default()
            .push(contribution.effective_id.clone());
    }
    by_slot
        .into_iter()
        .filter_map(|((surface, slot), owners)| {
            (owners.len() > 1).then_some(UiSurfaceConflict {
                surface,
                slot,
                owners,
            })
        })
        .collect()
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
    pub tokens: BTreeMap<String, String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContribution {
    pub id: ContributionId,
    pub provider_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub model_ids: Vec<String>,
    #[serde(default)]
    pub privacy: ProviderPrivacyPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<ProviderRuntimeContribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRuntimeContribution {
    #[serde(default)]
    pub protocol: ProviderRuntimeProtocol,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_url: Option<String>,
    #[serde(default)]
    pub api_key: ProviderRuntimeSecret,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub model_id: ProviderRuntimeModelIdPolicy,
    /// Optional host-side override conventions for runtime endpoint/config discovery.
    ///
    /// These fields document and customize how the host may read per-provider
    /// config from `~/.oino/extensions/<provider-id>/config.json` and environment
    /// variables before falling back to manifest URLs.
    #[serde(default)]
    pub config: ProviderRuntimeConfigContribution,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRuntimeConfigContribution {
    /// JSON string key used to override `base_url` in the provider config file.
    /// Dotted keys are supported by hosts that implement nested config lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url_key: Option<String>,
    /// JSON string key used to override the runtime health/models URL.
    /// Dotted keys are supported by hosts that implement nested config lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_url_key: Option<String>,
    /// Additional environment variables checked before generic
    /// `<PROVIDER_ID>_BASE_URL` resolution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_url_env: Vec<String>,
    /// Additional environment variables checked before generic
    /// `<PROVIDER_ID>_HEALTH_URL` resolution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health_url_env: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRuntimeProtocol {
    #[default]
    OpenAiChatCompletions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderRuntimeSecret {
    #[default]
    None,
    EnvVar {
        name: String,
    },
    ExtensionConfig {
        key: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRuntimeModelIdPolicy {
    /// Send the Oino model id without the `provider:` prefix.
    #[default]
    StripProviderPrefix,
    /// Send the full `provider:model` identifier downstream.
    PreserveFullIdentifier,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPrivacyPolicy {
    #[serde(default)]
    pub can_receive_prompts: bool,
    #[serde(default)]
    pub can_receive_tools: bool,
    #[serde(default)]
    pub can_mutate_requests: bool,
}

/// Extension contribution for custom auth providers.
///
/// Extensions can register auth providers that support API key, OAuth, or device code flows.
/// The host will call the extension's handler for credential storage, retrieval, and flow initiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContribution {
    pub id: ContributionId,
    /// Provider identifier (e.g., "my-custom-provider")
    pub provider_id: String,
    /// Human-readable display name
    #[serde(default)]
    pub display_name: String,
    /// Auth flow type
    #[serde(default)]
    pub auth_flow: AuthFlowType,
    /// Environment variable for API key (if applicable)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
    /// Setup URL for the provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
    /// Handler function name for custom auth flows
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

/// Auth flow type for extension auth contributions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthFlowType {
    /// Simple API key storage
    #[default]
    ApiKey,
    /// OAuth 2.0 Authorization Code flow with PKCE
    OAuth,
    /// Device Code flow (for GitHub Copilot, etc.)
    DeviceCode,
    /// Custom flow handled by extension handler
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistenceContribution {
    pub id: ContributionId,
    pub scope: PersistenceScope,
    pub key: String,
    #[serde(default = "default_persistence_schema_version")]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default = "default_persistence_max_bytes")]
    pub max_bytes: usize,
    #[serde(default)]
    pub migration: PersistenceMigrationPolicy,
    #[serde(default)]
    pub cleanup: PersistenceCleanupPolicy,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

fn default_persistence_schema_version() -> u32 {
    1
}

fn default_persistence_max_bytes() -> usize {
    64 * 1024
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "policy", content = "details")]
pub enum PersistenceMigrationPolicy {
    #[default]
    None,
    HostCopyForward,
    ExtensionHook {
        handler: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceCleanupPolicy {
    #[default]
    DeleteOnUninstall,
    RetainWithTombstone,
    Retain,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistenceRecord {
    pub owner_extension_id: ExtensionId,
    pub scope: PersistenceScope,
    pub key: String,
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(default)]
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub tombstoned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionSessionEntry {
    pub owner_extension_id: ExtensionId,
    pub key: String,
    pub schema_version: u32,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
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
    pub label: String,
    #[serde(default)]
    pub items: Vec<AutosuggestItem>,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutosuggestItem {
    pub label: String,
    pub replacement: String,
    #[serde(default)]
    pub detail: String,
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
    pub examples: Vec<PackageAssetRef>,
    #[serde(default)]
    pub docs: Vec<PackageAssetRef>,
    #[serde(default)]
    pub dependencies: Vec<PackageDependency>,
    #[serde(default)]
    pub permissions: ExtensionPermissions,
    #[serde(default)]
    pub trust: TrustMetadata,
}

impl PackageManifest {
    pub fn validate(&self) -> Result<(), ExtensionCoreError> {
        if self.extensions.is_empty()
            && self.resources.is_empty()
            && self.assets.is_empty()
            && self.examples.is_empty()
            && self.docs.is_empty()
        {
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
pub struct CommunityRegistryIndex {
    #[serde(default = "default_registry_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub packages: Vec<CommunityPackageMetadata>,
    #[serde(default)]
    pub advisories: Vec<SecurityAdvisory>,
}

fn default_registry_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityPackageMetadata {
    pub id: PackageId,
    pub version: Version,
    pub publisher: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_path: Option<PathBuf>,
    #[serde(default)]
    pub assets: Vec<RegistryAssetMetadata>,
    #[serde(default)]
    pub oino: OinoCompatibility,
    #[serde(default)]
    pub dependencies: Vec<PackageDependency>,
    #[serde(default)]
    pub permissions: ExtensionPermissions,
    #[serde(default)]
    pub trust: TrustMetadata,
    #[serde(default)]
    pub update_policy: RegistryUpdatePolicy,
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation_message: Option<String>,
    #[serde(default)]
    pub advisories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAssetMetadata {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryUpdatePolicy {
    #[default]
    Manual,
    Compatible,
    Latest,
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangelogEntry {
    pub version: Version,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAdvisory {
    pub id: String,
    pub package_id: PackageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affected: Option<OinoCompatibility>,
    pub severity: AdvisorySeverity,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub patched_versions: Vec<OinoCompatibility>,
    #[serde(default)]
    pub withdrawn: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorySeverity {
    Low,
    Moderate,
    High,
    Critical,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    #[default]
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

impl SourceScope {
    #[must_use]
    pub fn precedence(self) -> u8 {
        match self {
            Self::BuiltIn => 0,
            Self::Global => 10,
            Self::Project => 20,
            Self::Session => 30,
            Self::Development => 40,
        }
    }

    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::Global => "global",
            Self::Project => "project",
            Self::Session => "session",
            Self::Development => "development",
        }
    }
}

impl SourceKind {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::LocalExtension => "local_extension",
            Self::LocalPackage => "local_package",
            Self::InstalledPackage => "installed_package",
            Self::RegistryPackage => "registry_package",
            Self::WasmModule => "wasm_module",
            Self::NativeSidecar => "native_sidecar",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegistryEntryKey(String);

impl RegistryEntryKey {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RegistryEntryKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionMetadata {
    pub id: ContributionId,
    pub source: SourceDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<PackageId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(default)]
    pub lifecycle: LifecycleState,
    #[serde(default)]
    pub compatibility: RegistryCompatibility,
    #[serde(default)]
    pub permission: PermissionDecision,
    #[serde(default)]
    pub health: HealthState,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

impl ContributionMetadata {
    #[must_use]
    pub fn new(id: ContributionId, source: SourceDescriptor) -> Self {
        Self {
            id,
            source,
            package_id: None,
            extension_id: None,
            provenance: None,
            lifecycle: LifecycleState::Validated,
            compatibility: RegistryCompatibility::Compatible,
            permission: PermissionDecision::Granted,
            health: HealthState::Healthy,
            conflict: ConflictPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_extension_id(mut self, extension_id: ExtensionId) -> Self {
        self.extension_id = Some(extension_id);
        self
    }

    #[must_use]
    pub fn with_package_id(mut self, package_id: PackageId) -> Self {
        self.package_id = Some(package_id);
        self
    }

    #[must_use]
    pub fn with_conflict(mut self, conflict: ConflictPolicy) -> Self {
        self.conflict = conflict;
        self
    }

    #[must_use]
    pub fn with_compatibility(mut self, compatibility: RegistryCompatibility) -> Self {
        self.compatibility = compatibility;
        self
    }

    #[must_use]
    pub fn with_permission(mut self, permission: PermissionDecision) -> Self {
        self.permission = permission;
        self
    }

    #[must_use]
    pub fn with_health(mut self, health: HealthState) -> Self {
        self.health = health;
        self
    }

    #[must_use]
    pub fn with_lifecycle(mut self, lifecycle: LifecycleState) -> Self {
        self.lifecycle = lifecycle;
        self
    }

    #[must_use]
    pub fn priority(&self) -> i32 {
        self.conflict.priority
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum RegistryCompatibility {
    #[default]
    Compatible,
    Incompatible(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum PermissionDecision {
    #[default]
    Granted,
    PendingReview(String),
    Denied(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry<T> {
    pub key: RegistryEntryKey,
    pub metadata: ContributionMetadata,
    pub contribution: T,
}

impl<T> RegistryEntry<T> {
    #[must_use]
    pub fn new(key: RegistryEntryKey, metadata: ContributionMetadata, contribution: T) -> Self {
        Self {
            key,
            metadata,
            contribution,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownContributionPolicy {
    Enabled,
    PendingReview,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceScopePolicy {
    #[serde(default = "default_unknown_contribution_policy")]
    pub unknown_contributions: UnknownContributionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precedence: Option<u8>,
}

impl Default for SourceScopePolicy {
    fn default() -> Self {
        Self {
            unknown_contributions: default_unknown_contribution_policy(),
            precedence: None,
        }
    }
}

fn default_unknown_contribution_policy() -> UnknownContributionPolicy {
    UnknownContributionPolicy::Enabled
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyToggle {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryPolicy {
    #[serde(default)]
    pub enabled_extensions: BTreeSet<ExtensionId>,
    #[serde(default)]
    pub disabled_extensions: BTreeSet<ExtensionId>,
    #[serde(default)]
    pub enabled_packages: BTreeSet<PackageId>,
    #[serde(default)]
    pub disabled_packages: BTreeSet<PackageId>,
    #[serde(default)]
    pub enabled_contributions: BTreeSet<ContributionId>,
    #[serde(default)]
    pub disabled_contributions: BTreeSet<ContributionId>,
    #[serde(default)]
    pub enabled_entries: BTreeSet<RegistryEntryKey>,
    #[serde(default)]
    pub disabled_entries: BTreeSet<RegistryEntryKey>,
    #[serde(default)]
    pub overrides: BTreeMap<ContributionId, RegistryEntryKey>,
    #[serde(default)]
    pub priority_overrides: BTreeMap<RegistryEntryKey, i32>,
    #[serde(default)]
    pub source_scopes: BTreeMap<SourceScope, SourceScopePolicy>,
}

impl RegistryPolicy {
    #[must_use]
    pub fn safe_defaults() -> Self {
        let mut policy = Self::default();
        policy.source_scopes.insert(
            SourceScope::BuiltIn,
            SourceScopePolicy {
                unknown_contributions: UnknownContributionPolicy::Enabled,
                precedence: Some(SourceScope::BuiltIn.precedence()),
            },
        );
        for scope in [
            SourceScope::Global,
            SourceScope::Project,
            SourceScope::Session,
            SourceScope::Development,
        ] {
            policy.source_scopes.insert(
                scope,
                SourceScopePolicy {
                    unknown_contributions: UnknownContributionPolicy::PendingReview,
                    precedence: Some(scope.precedence()),
                },
            );
        }
        policy
    }

    #[must_use]
    pub fn is_disabled<T>(&self, entry: &RegistryEntry<T>) -> Option<InactiveReason> {
        if self.disabled_entries.contains(&entry.key) {
            return Some(InactiveReason::DisabledByPolicy(
                "entry disabled by policy".into(),
            ));
        }
        if self.disabled_contributions.contains(&entry.metadata.id) {
            return Some(InactiveReason::DisabledByPolicy(format!(
                "contribution `{}` disabled by policy",
                entry.metadata.id
            )));
        }
        if entry
            .metadata
            .extension_id
            .as_ref()
            .is_some_and(|extension_id| self.disabled_extensions.contains(extension_id))
        {
            return Some(InactiveReason::DisabledByPolicy(
                "extension disabled by policy".into(),
            ));
        }
        if entry
            .metadata
            .package_id
            .as_ref()
            .is_some_and(|package_id| self.disabled_packages.contains(package_id))
        {
            return Some(InactiveReason::DisabledByPolicy(
                "package disabled by policy".into(),
            ));
        }
        None
    }

    #[must_use]
    pub fn source_default_reason<T>(&self, entry: &RegistryEntry<T>) -> Option<InactiveReason> {
        if self.is_explicitly_enabled(entry) {
            return None;
        }
        let scope_policy = self.source_scopes.get(&entry.metadata.source.scope)?;
        match scope_policy.unknown_contributions {
            UnknownContributionPolicy::Enabled => None,
            UnknownContributionPolicy::PendingReview => {
                Some(InactiveReason::PermissionPending(format!(
                    "{} source contributions require review before activation",
                    entry.metadata.source.scope.slug()
                )))
            }
            UnknownContributionPolicy::Disabled => Some(InactiveReason::DisabledByPolicy(format!(
                "{} source contributions are disabled by default",
                entry.metadata.source.scope.slug()
            ))),
        }
    }

    #[must_use]
    pub fn effective_priority<T>(&self, entry: &RegistryEntry<T>) -> i32 {
        self.priority_overrides
            .get(&entry.key)
            .copied()
            .unwrap_or_else(|| entry.metadata.priority())
    }

    #[must_use]
    pub fn source_precedence(&self, scope: SourceScope) -> u8 {
        self.source_scopes
            .get(&scope)
            .and_then(|policy| policy.precedence)
            .unwrap_or_else(|| scope.precedence())
    }

    fn is_explicitly_enabled<T>(&self, entry: &RegistryEntry<T>) -> bool {
        self.enabled_entries.contains(&entry.key)
            || self.enabled_contributions.contains(&entry.metadata.id)
            || entry
                .metadata
                .extension_id
                .as_ref()
                .is_some_and(|extension_id| self.enabled_extensions.contains(extension_id))
            || entry
                .metadata
                .package_id
                .as_ref()
                .is_some_and(|package_id| self.enabled_packages.contains(package_id))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceScopePolicySettings {
    pub unknown_contributions: Option<UnknownContributionPolicy>,
    pub precedence: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionPolicySettings {
    pub extensions: BTreeMap<ExtensionId, PolicyToggle>,
    pub packages: BTreeMap<PackageId, PolicyToggle>,
    pub contributions: BTreeMap<ContributionId, PolicyToggle>,
    pub entries: BTreeMap<RegistryEntryKey, PolicyToggle>,
    pub overrides: BTreeMap<ContributionId, RegistryEntryKey>,
    pub priority_overrides: BTreeMap<RegistryEntryKey, i32>,
    pub source_scopes: BTreeMap<SourceScope, SourceScopePolicySettings>,
}

impl ExtensionPolicySettings {
    #[must_use]
    pub fn merge(global: &Self, project: &Self) -> Self {
        let mut merged = global.clone();
        merge_toggle_map(&mut merged.extensions, &project.extensions);
        merge_toggle_map(&mut merged.packages, &project.packages);
        merge_toggle_map(&mut merged.contributions, &project.contributions);
        merge_toggle_map(&mut merged.entries, &project.entries);
        merged.overrides.extend(project.overrides.clone());
        merged
            .priority_overrides
            .extend(project.priority_overrides.clone());
        for (scope, settings) in &project.source_scopes {
            let target = merged.source_scopes.entry(*scope).or_default();
            if settings.unknown_contributions.is_some() {
                target.unknown_contributions = settings.unknown_contributions;
            }
            if settings.precedence.is_some() {
                target.precedence = settings.precedence;
            }
        }
        merged
    }

    #[must_use]
    pub fn to_registry_policy(&self) -> RegistryPolicy {
        let mut policy = RegistryPolicy::safe_defaults();
        apply_toggle_map(
            &self.extensions,
            &mut policy.enabled_extensions,
            &mut policy.disabled_extensions,
        );
        apply_toggle_map(
            &self.packages,
            &mut policy.enabled_packages,
            &mut policy.disabled_packages,
        );
        apply_toggle_map(
            &self.contributions,
            &mut policy.enabled_contributions,
            &mut policy.disabled_contributions,
        );
        apply_toggle_map(
            &self.entries,
            &mut policy.enabled_entries,
            &mut policy.disabled_entries,
        );
        policy.overrides = self.overrides.clone();
        policy.priority_overrides = self.priority_overrides.clone();
        for (scope, settings) in &self.source_scopes {
            let target = policy.source_scopes.entry(*scope).or_default();
            if let Some(unknown_contributions) = settings.unknown_contributions {
                target.unknown_contributions = unknown_contributions;
            }
            if let Some(precedence) = settings.precedence {
                target.precedence = Some(precedence);
            }
        }
        policy
    }

    #[must_use]
    pub fn merged_registry_policy(global: &Self, project: &Self) -> RegistryPolicy {
        Self::merge(global, project).to_registry_policy()
    }

    pub fn from_optional_json(text: Option<&str>) -> serde_json::Result<Self> {
        text.map_or_else(|| Ok(Self::default()), serde_json::from_str)
    }
}

fn merge_toggle_map<K: Ord + Clone>(
    target: &mut BTreeMap<K, PolicyToggle>,
    overlay: &BTreeMap<K, PolicyToggle>,
) {
    target.extend(overlay.clone());
}

fn apply_toggle_map<K: Ord + Clone>(
    toggles: &BTreeMap<K, PolicyToggle>,
    enabled: &mut BTreeSet<K>,
    disabled: &mut BTreeSet<K>,
) {
    for (key, toggle) in toggles {
        match toggle {
            PolicyToggle::Enabled => {
                enabled.insert(key.clone());
                disabled.remove(key);
            }
            PolicyToggle::Disabled => {
                disabled.insert(key.clone());
                enabled.remove(key);
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionRegistry<T> {
    entries: BTreeMap<RegistryEntryKey, RegistryEntry<T>>,
}

impl<T> ContributionRegistry<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, entry: RegistryEntry<T>) -> Option<RegistryEntry<T>> {
        self.entries.insert(entry.key.clone(), entry)
    }

    pub fn unregister(&mut self, key: &RegistryEntryKey) -> Option<RegistryEntry<T>> {
        self.entries.remove(key)
    }

    pub fn entries(&self) -> impl Iterator<Item = &RegistryEntry<T>> {
        self.entries.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: Clone> ContributionRegistry<T> {
    #[must_use]
    pub fn compose(&self, policy: &RegistryPolicy) -> RegistrySnapshot<T> {
        let mut inactive = Vec::new();
        let mut diagnostics = Vec::new();
        let mut candidates_by_id: BTreeMap<ContributionId, Vec<RegistryEntry<T>>> = BTreeMap::new();

        for entry in self.entries.values() {
            if let Some(reason) = inactive_reason(entry, policy) {
                diagnostics.push(diagnostic_for_inactive(entry, &reason));
                inactive.push(InactiveContribution {
                    entry: entry.clone(),
                    reason,
                });
                continue;
            }
            candidates_by_id
                .entry(entry.metadata.id.clone())
                .or_default()
                .push(entry.clone());
        }

        let mut active = Vec::new();
        for (id, mut entries) in candidates_by_id {
            sort_entries(&mut entries, policy);
            if entries.len() == 1 {
                if let Some(entry) = entries.into_iter().next() {
                    active.push(ActiveContribution {
                        effective_id: id,
                        entry,
                    });
                }
                continue;
            }

            if let Some(override_key) = policy.overrides.get(&id) {
                if let Some(selected) = entries
                    .iter()
                    .find(|entry| &entry.key == override_key)
                    .cloned()
                {
                    for entry in entries {
                        if entry.key == selected.key {
                            active.push(ActiveContribution {
                                effective_id: id.clone(),
                                entry,
                            });
                        } else {
                            inactive.push(InactiveContribution {
                                entry,
                                reason: InactiveReason::OverriddenByUser(override_key.clone()),
                            });
                        }
                    }
                    continue;
                }
                diagnostics.push(conflict_diagnostic(
                    DiagnosticSeverity::Warning,
                    &entries[0],
                    format!(
                        "override `{override_key}` for contribution `{id}` did not match any candidate"
                    ),
                ));
            }

            if entries
                .iter()
                .any(|entry| entry.metadata.conflict.strategy == ConflictStrategy::Error)
            {
                diagnostics.push(conflict_diagnostic(
                    DiagnosticSeverity::Error,
                    &entries[0],
                    format!("duplicate contribution id `{id}` is configured as an error"),
                ));
                for entry in entries {
                    inactive.push(InactiveContribution {
                        entry,
                        reason: InactiveReason::ConflictError,
                    });
                }
                continue;
            }

            let strategy = group_strategy(&entries);
            match strategy {
                ConflictStrategy::Namespaced => {
                    diagnostics.push(conflict_diagnostic(
                        DiagnosticSeverity::Warning,
                        &entries[0],
                        format!("duplicate contribution id `{id}` was resolved with namespacing"),
                    ));
                    for (index, entry) in entries.into_iter().enumerate() {
                        let effective_id = if index == 0 {
                            id.clone()
                        } else {
                            namespaced_id(&entry)
                        };
                        active.push(ActiveContribution {
                            effective_id,
                            entry,
                        });
                    }
                }
                ConflictStrategy::FirstWins => {
                    push_single_winner(id, entries, 0, &mut active, &mut inactive);
                }
                ConflictStrategy::LastWins | ConflictStrategy::UserOverride => {
                    let winner_index = entries.len().saturating_sub(1);
                    push_single_winner(id, entries, winner_index, &mut active, &mut inactive);
                }
                ConflictStrategy::Error => {}
            }
        }

        sort_active(&mut active, policy);
        inactive.sort_by(|left, right| entry_cmp(&left.entry, &right.entry, policy));
        diagnostics.sort_by_key(ExtensionDiagnostic::format_message);
        RegistrySnapshot {
            active,
            inactive,
            diagnostics,
        }
    }
}

fn inactive_reason<T>(entry: &RegistryEntry<T>, policy: &RegistryPolicy) -> Option<InactiveReason> {
    if matches!(entry.metadata.lifecycle, LifecycleState::Removed) {
        return Some(InactiveReason::Removed);
    }
    if !matches!(
        entry.metadata.health,
        HealthState::Healthy | HealthState::Degraded
    ) {
        return Some(InactiveReason::Unhealthy(entry.metadata.health));
    }
    match &entry.metadata.compatibility {
        RegistryCompatibility::Compatible => {}
        RegistryCompatibility::Incompatible(reason) => {
            return Some(InactiveReason::Incompatible(reason.clone()));
        }
    }
    match &entry.metadata.permission {
        PermissionDecision::Granted => {}
        PermissionDecision::PendingReview(reason) => {
            return Some(InactiveReason::PermissionPending(reason.clone()));
        }
        PermissionDecision::Denied(reason) => {
            return Some(InactiveReason::PermissionDenied(reason.clone()));
        }
    }
    policy
        .is_disabled(entry)
        .or_else(|| policy.source_default_reason(entry))
}

fn group_strategy<T>(entries: &[RegistryEntry<T>]) -> ConflictStrategy {
    entries
        .iter()
        .map(|entry| entry.metadata.conflict.strategy)
        .find(|strategy| *strategy != ConflictStrategy::Namespaced)
        .unwrap_or(ConflictStrategy::Namespaced)
}

fn sort_entries<T>(entries: &mut [RegistryEntry<T>], policy: &RegistryPolicy) {
    entries.sort_by(|left, right| entry_cmp(left, right, policy));
}

fn sort_active<T>(active: &mut [ActiveContribution<T>], policy: &RegistryPolicy) {
    active.sort_by(|left, right| {
        entry_cmp(&left.entry, &right.entry, policy)
            .then(left.effective_id.cmp(&right.effective_id))
    });
}

fn entry_cmp<T>(
    left: &RegistryEntry<T>,
    right: &RegistryEntry<T>,
    policy: &RegistryPolicy,
) -> std::cmp::Ordering {
    policy
        .source_precedence(left.metadata.source.scope)
        .cmp(&policy.source_precedence(right.metadata.source.scope))
        .then_with(|| {
            policy
                .effective_priority(right)
                .cmp(&policy.effective_priority(left))
        })
        .then_with(|| left.metadata.id.cmp(&right.metadata.id))
        .then_with(|| left.key.cmp(&right.key))
}

fn push_single_winner<T: Clone>(
    id: ContributionId,
    entries: Vec<RegistryEntry<T>>,
    winner_index: usize,
    active: &mut Vec<ActiveContribution<T>>,
    inactive: &mut Vec<InactiveContribution<T>>,
) {
    for (index, entry) in entries.into_iter().enumerate() {
        if index == winner_index {
            active.push(ActiveContribution {
                effective_id: id.clone(),
                entry,
            });
        } else {
            inactive.push(InactiveContribution {
                entry,
                reason: InactiveReason::ConflictShadowed,
            });
        }
    }
}

fn namespaced_id<T>(entry: &RegistryEntry<T>) -> ContributionId {
    let namespace = entry
        .metadata
        .extension_id
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| entry.metadata.package_id.as_ref().map(ToString::to_string))
        .unwrap_or_else(|| {
            format!(
                "{}.{}",
                entry.metadata.source.scope.slug(),
                entry.metadata.source.kind.slug()
            )
        });
    let candidate = format!("{}.{}", namespace, entry.metadata.id);
    ContributionId::new(candidate).unwrap_or_else(|_| entry.metadata.id.clone())
}

fn diagnostic_for_inactive<T>(
    entry: &RegistryEntry<T>,
    reason: &InactiveReason,
) -> ExtensionDiagnostic {
    let severity = match reason {
        InactiveReason::PermissionPending(_)
        | InactiveReason::DisabledByPolicy(_)
        | InactiveReason::Removed
        | InactiveReason::OverriddenByUser(_)
        | InactiveReason::ConflictShadowed => DiagnosticSeverity::Info,
        InactiveReason::Incompatible(_) | InactiveReason::Unhealthy(_) => {
            DiagnosticSeverity::Warning
        }
        InactiveReason::PermissionDenied(_) | InactiveReason::ConflictError => {
            DiagnosticSeverity::Error
        }
    };
    ExtensionDiagnostic {
        severity,
        phase: DiagnosticPhase::RegistryComposition,
        package_id: entry.metadata.package_id.clone(),
        extension_id: entry.metadata.extension_id.clone(),
        contribution_id: Some(entry.metadata.id.clone()),
        source_path: entry.metadata.source.path.clone(),
        message: reason.message(),
        remediation: reason.remediation(),
        health: reason.health(),
    }
}

fn conflict_diagnostic<T>(
    severity: DiagnosticSeverity,
    entry: &RegistryEntry<T>,
    message: String,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity,
        phase: DiagnosticPhase::RegistryComposition,
        package_id: entry.metadata.package_id.clone(),
        extension_id: entry.metadata.extension_id.clone(),
        contribution_id: Some(entry.metadata.id.clone()),
        source_path: entry.metadata.source.path.clone(),
        message,
        remediation: Some(
            "adjust contribution ids, conflict policy, priority, or user override".into(),
        ),
        health: if severity == DiagnosticSeverity::Error {
            HealthState::Blocked
        } else {
            HealthState::Degraded
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveContribution<T> {
    pub effective_id: ContributionId,
    pub entry: RegistryEntry<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InactiveContribution<T> {
    pub entry: RegistryEntry<T>,
    pub reason: InactiveReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason", content = "details")]
pub enum InactiveReason {
    DisabledByPolicy(String),
    Incompatible(String),
    PermissionPending(String),
    PermissionDenied(String),
    Unhealthy(HealthState),
    Removed,
    OverriddenByUser(RegistryEntryKey),
    ConflictShadowed,
    ConflictError,
}

impl InactiveReason {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::DisabledByPolicy(reason)
            | Self::Incompatible(reason)
            | Self::PermissionPending(reason)
            | Self::PermissionDenied(reason) => reason.clone(),
            Self::Unhealthy(health) => format!("contribution health is {health:?}"),
            Self::Removed => "contribution was removed".into(),
            Self::OverriddenByUser(key) => {
                format!("contribution overridden by user selection `{key}`")
            }
            Self::ConflictShadowed => "contribution shadowed by conflict resolution".into(),
            Self::ConflictError => "contribution blocked by duplicate-id conflict".into(),
        }
    }

    #[must_use]
    pub fn remediation(&self) -> Option<String> {
        match self {
            Self::DisabledByPolicy(_) => {
                Some("enable the extension or contribution in settings".into())
            }
            Self::Incompatible(_) => {
                Some("install a compatible package version or update Oino".into())
            }
            Self::PermissionPending(_) => {
                Some("review and approve the requested permission".into())
            }
            Self::PermissionDenied(_) => {
                Some("grant permission or disable the contribution".into())
            }
            Self::Unhealthy(_) => {
                Some("inspect extension diagnostics and reload when fixed".into())
            }
            Self::Removed => None,
            Self::OverriddenByUser(_) | Self::ConflictShadowed | Self::ConflictError => Some(
                "adjust contribution ids, priority, conflict policy, or override settings".into(),
            ),
        }
    }

    #[must_use]
    pub fn health(&self) -> HealthState {
        match self {
            Self::DisabledByPolicy(_) | Self::Removed | Self::OverriddenByUser(_) => {
                HealthState::Disabled
            }
            Self::Incompatible(_) | Self::PermissionPending(_) | Self::ConflictShadowed => {
                HealthState::Degraded
            }
            Self::PermissionDenied(_) | Self::Unhealthy(_) | Self::ConflictError => {
                HealthState::Blocked
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySnapshot<T> {
    pub active: Vec<ActiveContribution<T>>,
    pub inactive: Vec<InactiveContribution<T>>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
}

impl<T> RegistrySnapshot<T> {
    #[must_use]
    pub fn active_ids(&self) -> Vec<&ContributionId> {
        self.active
            .iter()
            .map(|contribution| &contribution.effective_id)
            .collect()
    }
}

impl<T: Clone + PartialEq> RegistrySnapshot<T> {
    #[must_use]
    pub fn diff(&self, next: &Self) -> RegistryDiff<T> {
        let previous_by_id = self
            .active
            .iter()
            .map(|entry| (&entry.effective_id, entry))
            .collect::<BTreeMap<_, _>>();
        let next_by_id = next
            .active
            .iter()
            .map(|entry| (&entry.effective_id, entry))
            .collect::<BTreeMap<_, _>>();

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();

        for (id, next_entry) in &next_by_id {
            match previous_by_id.get(id) {
                None => added.push((*next_entry).clone()),
                Some(previous_entry)
                    if previous_entry.entry.key != next_entry.entry.key
                        || previous_entry.entry.contribution != next_entry.entry.contribution =>
                {
                    changed.push(ChangedContribution {
                        effective_id: (*id).clone(),
                        previous: (*previous_entry).clone(),
                        next: (*next_entry).clone(),
                    });
                }
                Some(_) => {}
            }
        }

        for (id, previous_entry) in &previous_by_id {
            if !next_by_id.contains_key(id) {
                removed.push((*previous_entry).clone());
            }
        }

        RegistryDiff {
            added,
            removed,
            changed,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryDiff<T> {
    pub added: Vec<ActiveContribution<T>>,
    pub removed: Vec<ActiveContribution<T>>,
    pub changed: Vec<ChangedContribution<T>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedContribution<T> {
    pub effective_id: ContributionId,
    pub previous: ActiveContribution<T>,
    pub next: ActiveContribution<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryFamily {
    Tool,
    Command,
    Keymap,
    Hook,
    UiSurface,
    SettingsPage,
    Theme,
    ProviderModel,
    AuthProvider,
    Resource,
    Persistence,
    Autosuggest,
    TranscriptRenderer,
    MessageRenderer,
    ToolRenderer,
    Diagnostic,
    Health,
}

impl RegistryFamily {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Command => "command",
            Self::Keymap => "keymap",
            Self::Hook => "hook",
            Self::UiSurface => "ui_surface",
            Self::SettingsPage => "settings_page",
            Self::Theme => "theme",
            Self::ProviderModel => "provider_model",
            Self::AuthProvider => "auth_provider",
            Self::Resource => "resource",
            Self::Persistence => "persistence",
            Self::Autosuggest => "autosuggest",
            Self::TranscriptRenderer => "transcript_renderer",
            Self::MessageRenderer => "message_renderer",
            Self::ToolRenderer => "tool_renderer",
            Self::Diagnostic => "diagnostic",
            Self::Health => "health",
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("invalid {family} contribution `{contribution_id}`: {message}")]
pub struct RegistryValidationError {
    pub family: &'static str,
    pub contribution_id: ContributionId,
    pub message: String,
}

impl RegistryValidationError {
    #[must_use]
    pub fn new(
        family: RegistryFamily,
        contribution_id: ContributionId,
        message: impl Into<String>,
    ) -> Self {
        Self {
            family: family.label(),
            contribution_id,
            message: message.into(),
        }
    }
}

pub trait RegistryContribution: Clone {
    fn contribution_id(&self) -> &ContributionId;
    fn conflict_policy(&self) -> ConflictPolicy;
    fn validate_for_registry(&self, family: RegistryFamily) -> Result<(), RegistryValidationError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticContribution {
    pub id: ContributionId,
    pub title: String,
    #[serde(default)]
    pub default_severity: DiagnosticSeverity,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthContribution {
    pub id: ContributionId,
    pub title: String,
    #[serde(default)]
    pub default_state: HealthState,
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

macro_rules! impl_registry_contribution {
    ($type:ty, $validator:expr) => {
        impl RegistryContribution for $type {
            fn contribution_id(&self) -> &ContributionId {
                &self.id
            }

            fn conflict_policy(&self) -> ConflictPolicy {
                self.conflict
            }

            fn validate_for_registry(
                &self,
                family: RegistryFamily,
            ) -> Result<(), RegistryValidationError> {
                $validator(self, family)
            }
        }
    };
}

fn validate_required(
    family: RegistryFamily,
    id: &ContributionId,
    field: &'static str,
    value: &str,
) -> Result<(), RegistryValidationError> {
    if value.trim().is_empty() {
        Err(RegistryValidationError::new(
            family,
            id.clone(),
            format!("{field} is required"),
        ))
    } else {
        Ok(())
    }
}

fn validate_renderer_family(
    family: RegistryFamily,
    id: &ContributionId,
    target: RendererTarget,
) -> Result<(), RegistryValidationError> {
    let matches_family = matches!(
        (family, target),
        (
            RegistryFamily::TranscriptRenderer,
            RendererTarget::TranscriptMessage
        ) | (
            RegistryFamily::MessageRenderer,
            RendererTarget::TranscriptMessage
        ) | (
            RegistryFamily::MessageRenderer,
            RendererTarget::MarkdownBlock
        ) | (RegistryFamily::ToolRenderer, RendererTarget::ToolCall)
            | (RegistryFamily::ToolRenderer, RendererTarget::ToolResult)
    );
    if matches_family {
        Ok(())
    } else {
        Err(RegistryValidationError::new(
            family,
            id.clone(),
            format!("renderer target `{target:?}` does not match registry family"),
        ))
    }
}

impl_registry_contribution!(ToolContribution, |item: &ToolContribution, family| {
    validate_required(family, &item.id, "description", &item.description)
});

impl_registry_contribution!(CommandContribution, |item: &CommandContribution, family| {
    validate_required(family, &item.id, "description", &item.description)
});

impl_registry_contribution!(KeymapContribution, |item: &KeymapContribution, family| {
    validate_required(family, &item.id, "action", &item.action)
});

impl_registry_contribution!(HookContribution, |item: &HookContribution, family| {
    if matches!(
        item.mode,
        HookMode::Mutable | HookMode::Cancellable | HookMode::Blocking
    ) && item
        .handler
        .as_deref()
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
    {
        Err(RegistryValidationError::new(
            family,
            item.id.clone(),
            "mutable, cancellable, and blocking hooks require a handler",
        ))
    } else {
        Ok(())
    }
});

impl_registry_contribution!(
    UiSurfaceContribution,
    |item: &UiSurfaceContribution, family| {
        validate_required(family, &item.id, "title", &item.title)
    }
);

impl_registry_contribution!(
    SettingsPageContribution,
    |item: &SettingsPageContribution, family| {
        validate_required(family, &item.id, "title", &item.title)
    }
);

impl_registry_contribution!(ThemeContribution, |item: &ThemeContribution, family| {
    validate_required(family, &item.id, "path", &item.path)
});

impl_registry_contribution!(
    ProviderContribution,
    |item: &ProviderContribution, family| {
        validate_required(family, &item.id, "provider_id", &item.provider_id).and_then(|()| {
            if let Some(runtime) = &item.runtime {
                validate_required(family, &item.id, "runtime.base_url", &runtime.base_url)?;
                if matches!(runtime.api_key, ProviderRuntimeSecret::EnvVar { ref name } if name.trim().is_empty())
                    || matches!(runtime.api_key, ProviderRuntimeSecret::ExtensionConfig { ref key } if key.trim().is_empty())
                {
                    return Err(RegistryValidationError::new(
                        family,
                        item.id.clone(),
                        "runtime api key reference must not be empty",
                    ));
                }
                if runtime.config.base_url_key.as_deref().is_some_and(|key| key.trim().is_empty())
                    || runtime.config.health_url_key.as_deref().is_some_and(|key| key.trim().is_empty())
                    || runtime.config.base_url_env.iter().any(|name| name.trim().is_empty())
                    || runtime.config.health_url_env.iter().any(|name| name.trim().is_empty())
                {
                    return Err(RegistryValidationError::new(
                        family,
                        item.id.clone(),
                        "runtime config override keys and env vars must not be empty",
                    ));
                }
            }
            Ok(())
        })
    }
);

impl_registry_contribution!(AuthContribution, |item: &AuthContribution, family| {
    validate_required(family, &item.id, "provider_id", &item.provider_id)
});

impl_registry_contribution!(
    ResourceContribution,
    |item: &ResourceContribution, family| {
        validate_required(family, &item.id, "path", &item.path)
    }
);

impl_registry_contribution!(
    PersistenceContribution,
    |item: &PersistenceContribution, family| {
        validate_required(family, &item.id, "key", &item.key).and_then(|()| {
            if item.schema_version == 0 {
                Err(RegistryValidationError::new(
                    family,
                    item.id.clone(),
                    "schema_version must be greater than zero",
                ))
            } else if item.max_bytes == 0 {
                Err(RegistryValidationError::new(
                    family,
                    item.id.clone(),
                    "max_bytes must be greater than zero",
                ))
            } else {
                Ok(())
            }
        })
    }
);

impl_registry_contribution!(
    AutosuggestContribution,
    |item: &AutosuggestContribution, family| {
        validate_required(family, &item.id, "trigger", &item.trigger)
    }
);

impl_registry_contribution!(
    RendererContribution,
    |item: &RendererContribution, family| {
        validate_renderer_family(family, &item.id, item.target)
    }
);

impl_registry_contribution!(
    DiagnosticContribution,
    |item: &DiagnosticContribution, family| {
        validate_required(family, &item.id, "title", &item.title)
    }
);

impl_registry_contribution!(HealthContribution, |item: &HealthContribution, family| {
    validate_required(family, &item.id, "title", &item.title)
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedContributionRegistry<T> {
    family: RegistryFamily,
    registry: ContributionRegistry<T>,
}

impl<T: RegistryContribution> TypedContributionRegistry<T> {
    #[must_use]
    pub fn new(family: RegistryFamily) -> Self {
        Self {
            family,
            registry: ContributionRegistry::new(),
        }
    }

    pub fn register_entry(
        &mut self,
        key: RegistryEntryKey,
        mut metadata: ContributionMetadata,
        contribution: T,
    ) -> Result<Option<RegistryEntry<T>>, RegistryValidationError> {
        contribution.validate_for_registry(self.family)?;
        metadata.id = contribution.contribution_id().clone();
        metadata.conflict = contribution.conflict_policy();
        Ok(self
            .registry
            .register(RegistryEntry::new(key, metadata, contribution)))
    }

    pub fn unregister(&mut self, key: &RegistryEntryKey) -> Option<RegistryEntry<T>> {
        self.registry.unregister(key)
    }

    #[must_use]
    pub fn compose(&self, policy: &RegistryPolicy) -> RegistrySnapshot<T>
    where
        T: Clone,
    {
        self.registry.compose(policy)
    }

    #[must_use]
    pub fn family(&self) -> RegistryFamily {
        self.family
    }

    #[must_use]
    pub fn inner(&self) -> &ContributionRegistry<T> {
        &self.registry
    }
}

pub type ToolRegistry = TypedContributionRegistry<ToolContribution>;
pub type CommandRegistry = TypedContributionRegistry<CommandContribution>;
pub type KeymapRegistry = TypedContributionRegistry<KeymapContribution>;
pub type HookRegistry = TypedContributionRegistry<HookContribution>;
pub type UiSurfaceRegistry = TypedContributionRegistry<UiSurfaceContribution>;
pub type SettingsPageRegistry = TypedContributionRegistry<SettingsPageContribution>;
pub type ThemeRegistry = TypedContributionRegistry<ThemeContribution>;
pub type ProviderModelRegistry = TypedContributionRegistry<ProviderContribution>;
pub type AuthProviderRegistry = TypedContributionRegistry<AuthContribution>;
pub type ResourceRegistry = TypedContributionRegistry<ResourceContribution>;
pub type PersistenceRegistry = TypedContributionRegistry<PersistenceContribution>;
pub type AutosuggestRegistry = TypedContributionRegistry<AutosuggestContribution>;
pub type TranscriptRendererRegistry = TypedContributionRegistry<RendererContribution>;
pub type MessageRendererRegistry = TypedContributionRegistry<RendererContribution>;
pub type ToolRendererRegistry = TypedContributionRegistry<RendererContribution>;
pub type DiagnosticRegistry = TypedContributionRegistry<DiagnosticContribution>;
pub type HealthRegistry = TypedContributionRegistry<HealthContribution>;

impl TypedContributionRegistry<ToolContribution> {
    #[must_use]
    pub fn tools() -> Self {
        Self::new(RegistryFamily::Tool)
    }
}

impl TypedContributionRegistry<CommandContribution> {
    #[must_use]
    pub fn commands() -> Self {
        Self::new(RegistryFamily::Command)
    }
}

impl TypedContributionRegistry<KeymapContribution> {
    #[must_use]
    pub fn keymaps() -> Self {
        Self::new(RegistryFamily::Keymap)
    }
}

impl TypedContributionRegistry<HookContribution> {
    #[must_use]
    pub fn hooks() -> Self {
        Self::new(RegistryFamily::Hook)
    }
}

impl TypedContributionRegistry<UiSurfaceContribution> {
    #[must_use]
    pub fn ui_surfaces() -> Self {
        Self::new(RegistryFamily::UiSurface)
    }
}

impl TypedContributionRegistry<SettingsPageContribution> {
    #[must_use]
    pub fn settings_pages() -> Self {
        Self::new(RegistryFamily::SettingsPage)
    }
}

impl TypedContributionRegistry<ThemeContribution> {
    #[must_use]
    pub fn themes() -> Self {
        Self::new(RegistryFamily::Theme)
    }
}

impl TypedContributionRegistry<ProviderContribution> {
    #[must_use]
    pub fn providers_models() -> Self {
        Self::new(RegistryFamily::ProviderModel)
    }
}

impl TypedContributionRegistry<AuthContribution> {
    #[must_use]
    pub fn auth_providers() -> Self {
        Self::new(RegistryFamily::AuthProvider)
    }
}

impl TypedContributionRegistry<ResourceContribution> {
    #[must_use]
    pub fn resources() -> Self {
        Self::new(RegistryFamily::Resource)
    }
}

impl TypedContributionRegistry<PersistenceContribution> {
    #[must_use]
    pub fn persistence() -> Self {
        Self::new(RegistryFamily::Persistence)
    }
}

impl TypedContributionRegistry<AutosuggestContribution> {
    #[must_use]
    pub fn autosuggest_providers() -> Self {
        Self::new(RegistryFamily::Autosuggest)
    }
}

impl TypedContributionRegistry<RendererContribution> {
    #[must_use]
    pub fn transcript_renderers() -> Self {
        Self::new(RegistryFamily::TranscriptRenderer)
    }

    #[must_use]
    pub fn message_renderers() -> Self {
        Self::new(RegistryFamily::MessageRenderer)
    }

    #[must_use]
    pub fn tool_renderers() -> Self {
        Self::new(RegistryFamily::ToolRenderer)
    }
}

impl TypedContributionRegistry<DiagnosticContribution> {
    #[must_use]
    pub fn diagnostics() -> Self {
        Self::new(RegistryFamily::Diagnostic)
    }
}

impl TypedContributionRegistry<HealthContribution> {
    #[must_use]
    pub fn health() -> Self {
        Self::new(RegistryFamily::Health)
    }
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
              "examples": [{ "path": "examples/sidebar" }],
              "docs": [{ "path": "docs/README.md" }],
              "trust": { "reviewed": true, "checksum": "sha256:package" }
            }"#,
        )?;
        package.validate()?;
        assert!(package.compatible_with(&Version::parse("0.1.2")?));
        assert_eq!(package.extensions.len(), 1);
        assert_eq!(package.examples[0].path, "examples/sidebar");
        assert_eq!(package.docs[0].path, "docs/README.md");
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

    fn registry_source(scope: SourceScope, kind: SourceKind, name: &str) -> SourceDescriptor {
        SourceDescriptor {
            scope,
            kind,
            path: Some(PathBuf::from(format!(".oino/test/{name}"))),
            registry: None,
        }
    }

    fn registry_entry(
        key: &str,
        id: &str,
        source: SourceDescriptor,
        contribution: &str,
    ) -> Result<RegistryEntry<String>, Box<dyn Error>> {
        let metadata = ContributionMetadata::new(ContributionId::new(id)?, source);
        Ok(RegistryEntry::new(
            RegistryEntryKey::new(key),
            metadata,
            contribution.into(),
        ))
    }

    #[test]
    fn registry_registers_sources_orders_and_unregisters() -> Result<(), Box<dyn Error>> {
        let mut registry: ContributionRegistry<String> = ContributionRegistry::new();
        let sources = [
            (
                "builtin",
                "builtin_tool",
                SourceScope::BuiltIn,
                SourceKind::BuiltIn,
            ),
            (
                "global",
                "global_tool",
                SourceScope::Global,
                SourceKind::InstalledPackage,
            ),
            (
                "project",
                "project_tool",
                SourceScope::Project,
                SourceKind::LocalPackage,
            ),
            (
                "session",
                "session_tool",
                SourceScope::Session,
                SourceKind::WasmModule,
            ),
            (
                "dev",
                "dev_tool",
                SourceScope::Development,
                SourceKind::LocalExtension,
            ),
        ];
        for (key, id, scope, kind) in sources {
            registry.register(registry_entry(
                key,
                id,
                registry_source(scope, kind, key),
                id,
            )?);
        }
        assert_eq!(registry.len(), 5);
        let removed = registry.unregister(&RegistryEntryKey::new("session"));
        assert!(removed.is_some());
        let snapshot = registry.compose(&RegistryPolicy::default());
        let ids = snapshot
            .active
            .iter()
            .map(|entry| entry.effective_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec!["builtin_tool", "global_tool", "project_tool", "dev_tool"]
        );
        assert!(snapshot.diagnostics.is_empty());
        Ok(())
    }

    #[test]
    fn registry_namespaces_duplicate_ids_by_default() -> Result<(), Box<dyn Error>> {
        let mut registry: ContributionRegistry<String> = ContributionRegistry::new();
        registry.register(registry_entry(
            "builtin-read",
            "read",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "builtin-read"),
            "builtin read",
        )?);
        let metadata = ContributionMetadata::new(
            ContributionId::new("read")?,
            registry_source(SourceScope::Project, SourceKind::WasmModule, "acme-read"),
        )
        .with_extension_id(ExtensionId::new("acme.tools")?);
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("acme-read"),
            metadata,
            "extension read".into(),
        ));

        let snapshot = registry.compose(&RegistryPolicy::default());
        let ids = snapshot
            .active
            .iter()
            .map(|entry| entry.effective_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["read", "acme.tools.read"]);
        assert_eq!(snapshot.diagnostics.len(), 1);
        assert!(snapshot.diagnostics[0].message.contains("namespacing"));
        Ok(())
    }

    #[test]
    fn registry_applies_user_overrides_and_disable_policy() -> Result<(), Box<dyn Error>> {
        let conflict = ConflictPolicy {
            strategy: ConflictStrategy::UserOverride,
            priority: 0,
            allow_user_override: true,
        };
        let mut registry: ContributionRegistry<String> = ContributionRegistry::new();
        let global_metadata = ContributionMetadata::new(
            ContributionId::new("runner")?,
            registry_source(
                SourceScope::Global,
                SourceKind::InstalledPackage,
                "global-runner",
            ),
        )
        .with_extension_id(ExtensionId::new("global.runner")?)
        .with_conflict(conflict);
        let project_metadata = ContributionMetadata::new(
            ContributionId::new("runner")?,
            registry_source(
                SourceScope::Project,
                SourceKind::LocalPackage,
                "project-runner",
            ),
        )
        .with_extension_id(ExtensionId::new("project.runner")?)
        .with_conflict(conflict);
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("global-runner"),
            global_metadata,
            "global".into(),
        ));
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("project-runner"),
            project_metadata,
            "project".into(),
        ));

        let mut policy = RegistryPolicy::default();
        policy.overrides.insert(
            ContributionId::new("runner")?,
            RegistryEntryKey::new("global-runner"),
        );
        let snapshot = registry.compose(&policy);
        assert_eq!(snapshot.active.len(), 1);
        assert_eq!(snapshot.active[0].entry.contribution, "global");
        assert_eq!(snapshot.inactive.len(), 1);
        assert!(matches!(
            snapshot.inactive[0].reason,
            InactiveReason::OverriddenByUser(_)
        ));

        policy
            .disabled_extensions
            .insert(ExtensionId::new("global.runner")?);
        let disabled_snapshot = registry.compose(&policy);
        assert_eq!(disabled_snapshot.active.len(), 1);
        assert_eq!(disabled_snapshot.active[0].entry.contribution, "project");
        assert!(disabled_snapshot
            .inactive
            .iter()
            .any(|entry| matches!(entry.reason, InactiveReason::DisabledByPolicy(_))));
        Ok(())
    }

    #[test]
    fn registry_reports_incompatible_and_denied_entries() -> Result<(), Box<dyn Error>> {
        let mut registry: ContributionRegistry<String> = ContributionRegistry::new();
        let incompatible = ContributionMetadata::new(
            ContributionId::new("future_tool")?,
            registry_source(SourceScope::Project, SourceKind::LocalPackage, "future"),
        )
        .with_compatibility(RegistryCompatibility::Incompatible(
            "requires Oino >=2".into(),
        ));
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("future"),
            incompatible,
            "future".into(),
        ));
        let denied = ContributionMetadata::new(
            ContributionId::new("shell_tool")?,
            registry_source(SourceScope::Project, SourceKind::WasmModule, "shell"),
        )
        .with_permission(PermissionDecision::Denied(
            "shell/process permission denied".into(),
        ));
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("shell"),
            denied,
            "shell".into(),
        ));
        let pending = ContributionMetadata::new(
            ContributionId::new("review_tool")?,
            registry_source(SourceScope::Project, SourceKind::WasmModule, "review"),
        )
        .with_permission(PermissionDecision::PendingReview(
            "permission review required".into(),
        ));
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("review"),
            pending,
            "review".into(),
        ));

        let snapshot = registry.compose(&RegistryPolicy::default());
        assert!(snapshot.active.is_empty());
        assert_eq!(snapshot.inactive.len(), 3);
        assert!(snapshot
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning));
        assert!(snapshot
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error));
        assert!(snapshot
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.health == HealthState::Degraded));
        Ok(())
    }

    #[test]
    fn registry_snapshot_diff_tracks_added_removed_and_changed() -> Result<(), Box<dyn Error>> {
        let mut before = ContributionRegistry::new();
        before.register(registry_entry(
            "alpha",
            "alpha",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "alpha"),
            "v1",
        )?);
        before.register(registry_entry(
            "beta",
            "beta",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "beta"),
            "same",
        )?);

        let mut after = ContributionRegistry::new();
        after.register(registry_entry(
            "alpha",
            "alpha",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "alpha"),
            "v2",
        )?);
        after.register(registry_entry(
            "gamma",
            "gamma",
            registry_source(SourceScope::Project, SourceKind::LocalPackage, "gamma"),
            "new",
        )?);

        let before_snapshot = before.compose(&RegistryPolicy::default());
        let after_snapshot = after.compose(&RegistryPolicy::default());
        let diff = before_snapshot.diff(&after_snapshot);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].effective_id.as_str(), "gamma");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].effective_id.as_str(), "beta");
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].effective_id.as_str(), "alpha");
        assert_eq!(diff.changed[0].previous.entry.contribution, "v1");
        assert_eq!(diff.changed[0].next.entry.contribution, "v2");
        Ok(())
    }

    #[test]
    fn extension_policy_settings_merge_project_precedence() -> Result<(), Box<dyn Error>> {
        let extension_id = ExtensionId::new("acme.runner")?;
        let contribution_id = ContributionId::new("runner")?;
        let entry_key = RegistryEntryKey::new("project-runner");

        let mut global = ExtensionPolicySettings::default();
        global
            .extensions
            .insert(extension_id.clone(), PolicyToggle::Disabled);
        global
            .contributions
            .insert(contribution_id.clone(), PolicyToggle::Disabled);
        global.overrides.insert(
            contribution_id.clone(),
            RegistryEntryKey::new("global-runner"),
        );
        global
            .priority_overrides
            .insert(RegistryEntryKey::new("global-runner"), 10);
        global.source_scopes.insert(
            SourceScope::Project,
            SourceScopePolicySettings {
                unknown_contributions: Some(UnknownContributionPolicy::PendingReview),
                precedence: Some(30),
            },
        );

        let mut project = ExtensionPolicySettings::default();
        project
            .extensions
            .insert(extension_id.clone(), PolicyToggle::Enabled);
        project
            .contributions
            .insert(contribution_id.clone(), PolicyToggle::Enabled);
        project
            .overrides
            .insert(contribution_id.clone(), entry_key.clone());
        project.priority_overrides.insert(entry_key.clone(), 50);
        project.source_scopes.insert(
            SourceScope::Project,
            SourceScopePolicySettings {
                unknown_contributions: Some(UnknownContributionPolicy::Enabled),
                precedence: Some(5),
            },
        );

        let merged = ExtensionPolicySettings::merge(&global, &project);
        assert_eq!(
            merged.extensions.get(&extension_id),
            Some(&PolicyToggle::Enabled)
        );
        assert_eq!(
            merged.contributions.get(&contribution_id),
            Some(&PolicyToggle::Enabled)
        );
        assert_eq!(merged.overrides.get(&contribution_id), Some(&entry_key));
        assert_eq!(merged.priority_overrides.get(&entry_key), Some(&50));
        assert_eq!(
            merged
                .source_scopes
                .get(&SourceScope::Project)
                .and_then(|settings| settings.unknown_contributions),
            Some(UnknownContributionPolicy::Enabled)
        );
        assert_eq!(
            merged
                .source_scopes
                .get(&SourceScope::Project)
                .and_then(|settings| settings.precedence),
            Some(5)
        );

        let policy = merged.to_registry_policy();
        assert!(policy.enabled_extensions.contains(&extension_id));
        assert!(!policy.disabled_extensions.contains(&extension_id));
        assert_eq!(policy.overrides.get(&contribution_id), Some(&entry_key));
        assert_eq!(
            policy.effective_priority(&RegistryEntry::new(
                entry_key,
                ContributionMetadata::new(
                    contribution_id,
                    registry_source(
                        SourceScope::Project,
                        SourceKind::LocalPackage,
                        "project-runner"
                    ),
                ),
                String::new(),
            )),
            50
        );
        assert_eq!(policy.source_precedence(SourceScope::Project), 5);
        Ok(())
    }

    #[test]
    fn safe_default_policy_keeps_builtins_and_reviews_unknown_external(
    ) -> Result<(), Box<dyn Error>> {
        let mut registry = ContributionRegistry::new();
        registry.register(registry_entry(
            "builtin-read",
            "read",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "builtin-read"),
            "builtin",
        )?);
        let external_metadata = ContributionMetadata::new(
            ContributionId::new("process_manager")?,
            registry_source(
                SourceScope::Project,
                SourceKind::LocalPackage,
                "process-manager",
            ),
        )
        .with_extension_id(ExtensionId::new("acme.process")?);
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("process-manager"),
            external_metadata,
            "external".into(),
        ));

        let default_policy = ExtensionPolicySettings::default().to_registry_policy();
        let snapshot = registry.compose(&default_policy);
        assert_eq!(snapshot.active.len(), 1);
        assert_eq!(snapshot.active[0].effective_id.as_str(), "read");
        assert_eq!(snapshot.inactive.len(), 1);
        assert!(matches!(
            snapshot.inactive[0].reason,
            InactiveReason::PermissionPending(_)
        ));

        let mut disabled_by_scope = ExtensionPolicySettings::default();
        disabled_by_scope.source_scopes.insert(
            SourceScope::Project,
            SourceScopePolicySettings {
                unknown_contributions: Some(UnknownContributionPolicy::Disabled),
                precedence: None,
            },
        );
        let disabled_snapshot = registry.compose(&disabled_by_scope.to_registry_policy());
        assert!(matches!(
            disabled_snapshot.inactive[0].reason,
            InactiveReason::DisabledByPolicy(_)
        ));

        let mut explicitly_enabled = disabled_by_scope;
        explicitly_enabled
            .extensions
            .insert(ExtensionId::new("acme.process")?, PolicyToggle::Enabled);
        let enabled_snapshot = registry.compose(&explicitly_enabled.to_registry_policy());
        let ids = enabled_snapshot
            .active
            .iter()
            .map(|entry| entry.effective_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["read", "process_manager"]);
        Ok(())
    }

    #[test]
    fn extension_policy_settings_missing_and_invalid_json() -> Result<(), Box<dyn Error>> {
        let missing = ExtensionPolicySettings::from_optional_json(None)?;
        assert_eq!(missing, ExtensionPolicySettings::default());

        let parsed = ExtensionPolicySettings::from_optional_json(Some(
            r#"{
              "extensions": { "acme.process": "enabled" },
              "contributions": { "process_manager": "disabled" },
              "source_scopes": {
                "project": { "unknown_contributions": "pending_review", "precedence": 15 }
              }
            }"#,
        ))?;
        assert_eq!(
            parsed.extensions.get(&ExtensionId::new("acme.process")?),
            Some(&PolicyToggle::Enabled)
        );
        assert_eq!(
            parsed
                .contributions
                .get(&ContributionId::new("process_manager")?),
            Some(&PolicyToggle::Disabled)
        );

        let invalid = ExtensionPolicySettings::from_optional_json(Some(
            r#"{ "overrides": { "Bad Id": "entry" } }"#,
        ));
        assert!(invalid.is_err());
        Ok(())
    }

    #[test]
    fn extension_policy_overrides_priorities_and_invalid_override_diagnostics_survive_reload(
    ) -> Result<(), Box<dyn Error>> {
        let conflict = ConflictPolicy {
            strategy: ConflictStrategy::UserOverride,
            priority: 0,
            allow_user_override: true,
        };
        let mut settings = ExtensionPolicySettings::default();
        settings.source_scopes.insert(
            SourceScope::Global,
            SourceScopePolicySettings {
                unknown_contributions: Some(UnknownContributionPolicy::Enabled),
                precedence: None,
            },
        );
        settings.source_scopes.insert(
            SourceScope::Project,
            SourceScopePolicySettings {
                unknown_contributions: Some(UnknownContributionPolicy::Enabled),
                precedence: None,
            },
        );
        settings.overrides.insert(
            ContributionId::new("runner")?,
            RegistryEntryKey::new("global-runner"),
        );
        settings
            .priority_overrides
            .insert(RegistryEntryKey::new("beta"), 100);

        let encoded = serde_json::to_string(&settings)?;
        let decoded: ExtensionPolicySettings = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, settings);
        let policy = decoded.to_registry_policy();

        let mut registry = ContributionRegistry::new();
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("global-runner"),
            ContributionMetadata::new(
                ContributionId::new("runner")?,
                registry_source(
                    SourceScope::Global,
                    SourceKind::InstalledPackage,
                    "global-runner",
                ),
            )
            .with_conflict(conflict),
            "global".into(),
        ));
        registry.register(RegistryEntry::new(
            RegistryEntryKey::new("project-runner"),
            ContributionMetadata::new(
                ContributionId::new("runner")?,
                registry_source(
                    SourceScope::Project,
                    SourceKind::LocalPackage,
                    "project-runner",
                ),
            )
            .with_conflict(conflict),
            "project".into(),
        ));
        registry.register(registry_entry(
            "alpha",
            "alpha",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "alpha"),
            "alpha",
        )?);
        registry.register(registry_entry(
            "beta",
            "beta",
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, "beta"),
            "beta",
        )?);

        let snapshot = registry.compose(&policy);
        let active = snapshot
            .active
            .iter()
            .map(|entry| {
                (
                    entry.effective_id.as_str(),
                    entry.entry.contribution.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            active,
            vec![("beta", "beta"), ("alpha", "alpha"), ("runner", "global")]
        );

        let mut invalid_override_policy = policy;
        invalid_override_policy.overrides.insert(
            ContributionId::new("runner")?,
            RegistryEntryKey::new("missing-runner"),
        );
        let invalid_snapshot = registry.compose(&invalid_override_policy);
        assert!(invalid_snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.message.contains("did not match")
        }));
        Ok(())
    }

    fn typed_metadata(id: &ContributionId) -> ContributionMetadata {
        ContributionMetadata::new(
            id.clone(),
            registry_source(SourceScope::BuiltIn, SourceKind::BuiltIn, id.as_str()),
        )
    }

    fn assert_typed_valid<T>(
        mut registry: TypedContributionRegistry<T>,
        contribution: T,
    ) -> Result<(), Box<dyn Error>>
    where
        T: RegistryContribution + Clone,
    {
        let id = contribution.contribution_id().clone();
        let previous = registry.register_entry(
            RegistryEntryKey::new(format!("valid-{id}")),
            typed_metadata(&id),
            contribution,
        )?;
        assert!(previous.is_none());
        let snapshot = registry.compose(&RegistryPolicy::default());
        assert_eq!(snapshot.active.len(), 1);
        assert_eq!(snapshot.active[0].effective_id, id);
        Ok(())
    }

    fn assert_typed_invalid<T>(
        mut registry: TypedContributionRegistry<T>,
        contribution: T,
    ) -> Result<(), Box<dyn Error>>
    where
        T: RegistryContribution + Clone,
    {
        let id = contribution.contribution_id().clone();
        let result = registry.register_entry(
            RegistryEntryKey::new(format!("invalid-{id}")),
            typed_metadata(&id),
            contribution,
        );
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn typed_registries_accept_valid_contributions() -> Result<(), Box<dyn Error>> {
        assert_typed_valid(
            ToolRegistry::tools(),
            ToolContribution {
                id: ContributionId::new("tool")?,
                description: "A model-visible tool".into(),
                input_schema: Value::Null,
                execution_mode: ToolExecutionMode::Parallel,
                handler: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            CommandRegistry::commands(),
            CommandContribution {
                id: ContributionId::new("command")?,
                description: "A slash command".into(),
                handler: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            KeymapRegistry::keymaps(),
            KeymapContribution {
                id: ContributionId::new("keymap")?,
                action: "app.test".into(),
                context: "global".into(),
                default_bindings: vec!["ctrl+x".into()],
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            HookRegistry::hooks(),
            HookContribution {
                id: ContributionId::new("hook")?,
                event: HookEventKind::ToolCall,
                priority: 0,
                mode: HookMode::Observe,
                handler: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            UiSurfaceRegistry::ui_surfaces(),
            UiSurfaceContribution {
                id: ContributionId::new("ui.sidebar")?,
                surface: UiSurfaceKind::Sidebar,
                title: "Sidebar".into(),
                state_schema: None,
                layout: UiLayoutPolicy::default(),
                visibility: UiVisibilityPolicy::default(),
                focus: UiFocusPolicy::default(),
                key_dispatch: UiKeyDispatchPolicy::default(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            SettingsPageRegistry::settings_pages(),
            SettingsPageContribution {
                id: ContributionId::new("settings.page")?,
                title: "Settings".into(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            ThemeRegistry::themes(),
            ThemeContribution {
                id: ContributionId::new("theme")?,
                path: "themes/dark.json".into(),
                tokens: BTreeMap::from([("accent".into(), "cyan".into())]),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            ProviderModelRegistry::providers_models(),
            ProviderContribution {
                id: ContributionId::new("provider")?,
                provider_id: "openrouter".into(),
                display_name: "OpenRouter".into(),
                model_ids: vec!["kr/claude-sonnet-4.5".into()],
                privacy: ProviderPrivacyPolicy {
                    can_receive_prompts: true,
                    can_receive_tools: true,
                    can_mutate_requests: false,
                },
                runtime: Some(ProviderRuntimeContribution {
                    protocol: ProviderRuntimeProtocol::OpenAiChatCompletions,
                    base_url: "http://localhost:20128/v1".into(),
                    models_url: None,
                    health_url: Some("http://localhost:20128/v1/models".into()),
                    api_key: ProviderRuntimeSecret::EnvVar {
                        name: "NINEROUTER_API_KEY".into(),
                    },
                    headers: BTreeMap::new(),
                    model_id: ProviderRuntimeModelIdPolicy::StripProviderPrefix,
                    config: ProviderRuntimeConfigContribution::default(),
                }),
                hook: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            ResourceRegistry::resources(),
            ResourceContribution {
                id: ContributionId::new("skill.resource")?,
                kind: ResourceKind::Skill,
                path: "skills/test/SKILL.md".into(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            PersistenceRegistry::persistence(),
            PersistenceContribution {
                id: ContributionId::new("state.processes")?,
                scope: PersistenceScope::Project,
                key: "processes".into(),
                schema_version: 1,
                schema: Some("object".into()),
                max_bytes: 4096,
                migration: PersistenceMigrationPolicy::HostCopyForward,
                cleanup: PersistenceCleanupPolicy::DeleteOnUninstall,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            AutosuggestRegistry::autosuggest_providers(),
            AutosuggestContribution {
                id: ContributionId::new("autosuggest")?,
                trigger: "@".into(),
                label: "Files".into(),
                items: vec![AutosuggestItem {
                    label: "README.md".into(),
                    replacement: "@README.md".into(),
                    detail: "Project file".into(),
                }],
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            TranscriptRendererRegistry::transcript_renderers(),
            RendererContribution {
                id: ContributionId::new("transcript.renderer")?,
                target: RendererTarget::TranscriptMessage,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            MessageRendererRegistry::message_renderers(),
            RendererContribution {
                id: ContributionId::new("message.renderer")?,
                target: RendererTarget::MarkdownBlock,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            ToolRendererRegistry::tool_renderers(),
            RendererContribution {
                id: ContributionId::new("tool.renderer")?,
                target: RendererTarget::ToolResult,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            DiagnosticRegistry::diagnostics(),
            DiagnosticContribution {
                id: ContributionId::new("diagnostic")?,
                title: "Diagnostics".into(),
                default_severity: DiagnosticSeverity::Warning,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_valid(
            HealthRegistry::health(),
            HealthContribution {
                id: ContributionId::new("health")?,
                title: "Health".into(),
                default_state: HealthState::Healthy,
                conflict: ConflictPolicy::default(),
            },
        )?;
        Ok(())
    }

    #[test]
    fn typed_registries_reject_invalid_contributions() -> Result<(), Box<dyn Error>> {
        assert_typed_invalid(
            ToolRegistry::tools(),
            ToolContribution {
                id: ContributionId::new("tool")?,
                description: String::new(),
                input_schema: Value::Null,
                execution_mode: ToolExecutionMode::Parallel,
                handler: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            CommandRegistry::commands(),
            CommandContribution {
                id: ContributionId::new("command")?,
                description: String::new(),
                handler: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            KeymapRegistry::keymaps(),
            KeymapContribution {
                id: ContributionId::new("keymap")?,
                action: String::new(),
                context: String::new(),
                default_bindings: Vec::new(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            HookRegistry::hooks(),
            HookContribution {
                id: ContributionId::new("hook")?,
                event: HookEventKind::ToolCall,
                priority: 0,
                mode: HookMode::Blocking,
                handler: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            UiSurfaceRegistry::ui_surfaces(),
            UiSurfaceContribution {
                id: ContributionId::new("ui.sidebar")?,
                surface: UiSurfaceKind::Sidebar,
                title: String::new(),
                state_schema: None,
                layout: UiLayoutPolicy::default(),
                visibility: UiVisibilityPolicy::default(),
                focus: UiFocusPolicy::default(),
                key_dispatch: UiKeyDispatchPolicy::default(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            SettingsPageRegistry::settings_pages(),
            SettingsPageContribution {
                id: ContributionId::new("settings.page")?,
                title: String::new(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            ThemeRegistry::themes(),
            ThemeContribution {
                id: ContributionId::new("theme")?,
                path: String::new(),
                tokens: BTreeMap::new(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            ProviderModelRegistry::providers_models(),
            ProviderContribution {
                id: ContributionId::new("provider")?,
                provider_id: String::new(),
                display_name: String::new(),
                model_ids: Vec::new(),
                privacy: ProviderPrivacyPolicy::default(),
                runtime: None,
                hook: None,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            ResourceRegistry::resources(),
            ResourceContribution {
                id: ContributionId::new("skill.resource")?,
                kind: ResourceKind::Skill,
                path: String::new(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            PersistenceRegistry::persistence(),
            PersistenceContribution {
                id: ContributionId::new("state.processes")?,
                scope: PersistenceScope::Project,
                key: String::new(),
                schema_version: 0,
                schema: None,
                max_bytes: 0,
                migration: PersistenceMigrationPolicy::None,
                cleanup: PersistenceCleanupPolicy::DeleteOnUninstall,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            AutosuggestRegistry::autosuggest_providers(),
            AutosuggestContribution {
                id: ContributionId::new("autosuggest")?,
                trigger: String::new(),
                label: String::new(),
                items: Vec::new(),
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            TranscriptRendererRegistry::transcript_renderers(),
            RendererContribution {
                id: ContributionId::new("transcript.renderer")?,
                target: RendererTarget::ToolCall,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            MessageRendererRegistry::message_renderers(),
            RendererContribution {
                id: ContributionId::new("message.renderer")?,
                target: RendererTarget::ToolResult,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            ToolRendererRegistry::tool_renderers(),
            RendererContribution {
                id: ContributionId::new("tool.renderer")?,
                target: RendererTarget::TranscriptMessage,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            DiagnosticRegistry::diagnostics(),
            DiagnosticContribution {
                id: ContributionId::new("diagnostic")?,
                title: String::new(),
                default_severity: DiagnosticSeverity::Warning,
                conflict: ConflictPolicy::default(),
            },
        )?;
        assert_typed_invalid(
            HealthRegistry::health(),
            HealthContribution {
                id: ContributionId::new("health")?,
                title: String::new(),
                default_state: HealthState::Healthy,
                conflict: ConflictPolicy::default(),
            },
        )?;
        Ok(())
    }

    fn active_ui_surface(
        id: &str,
        owner: &str,
        surface: UiSurfaceKind,
        slot: &str,
        schema: Option<&str>,
    ) -> Result<ActiveContribution<UiSurfaceContribution>, Box<dyn Error>> {
        let contribution_id = ContributionId::new(id)?;
        let extension_id = ExtensionId::new(owner)?;
        let mut key_scopes = BTreeSet::new();
        key_scopes.insert("extension.surface".into());
        Ok(ActiveContribution {
            effective_id: contribution_id.clone(),
            entry: RegistryEntry::new(
                RegistryEntryKey::new(id),
                ContributionMetadata::new(
                    contribution_id.clone(),
                    registry_source(SourceScope::Project, SourceKind::LocalPackage, id),
                )
                .with_extension_id(extension_id),
                UiSurfaceContribution {
                    id: contribution_id,
                    surface,
                    title: format!("{surface:?}"),
                    state_schema: schema.map(str::to_string),
                    layout: UiLayoutPolicy {
                        slot: slot.into(),
                        min_width: 30,
                        min_height: 8,
                        tiny_terminal: UiTinyTerminalFallback::StatusLine,
                        ..UiLayoutPolicy::default()
                    },
                    visibility: UiVisibilityPolicy::Visible,
                    focus: UiFocusPolicy::Focusable,
                    key_dispatch: UiKeyDispatchPolicy {
                        scopes: key_scopes,
                        pass_through: false,
                    },
                    conflict: ConflictPolicy::default(),
                },
            ),
        })
    }

    #[test]
    fn ui_surface_updates_validate_owner_state_shape_and_key_scopes() -> Result<(), Box<dyn Error>>
    {
        let surface = active_ui_surface(
            "ui.processes",
            "process-manager",
            UiSurfaceKind::Sidebar,
            "sidebar:right",
            Some("object"),
        )?;
        let valid_update = UiSurfaceStateUpdate {
            surface_id: ContributionId::new("ui.processes")?,
            owner_extension_id: ExtensionId::new("process-manager")?,
            state: serde_json::json!({ "rows": ["cargo test"] }),
            actions: vec![UiSurfaceAction {
                id: "stop".into(),
                label: "Stop".into(),
                key_scope: Some("extension.surface".into()),
            }],
        };
        assert!(validate_ui_surface_update(&surface, &valid_update).is_ok());

        let mut wrong_owner = valid_update.clone();
        wrong_owner.owner_extension_id = ExtensionId::new("other-extension")?;
        assert!(matches!(
            validate_ui_surface_update(&surface, &wrong_owner),
            Err(UiSurfaceValidationError::InvalidOwner { .. })
        ));

        let mut bad_shape = valid_update.clone();
        bad_shape.state = Value::String("not an object".into());
        assert!(matches!(
            validate_ui_surface_update(&surface, &bad_shape),
            Err(UiSurfaceValidationError::BadStateShape { .. })
        ));

        let mut bad_scope = valid_update;
        bad_scope.actions[0].key_scope = Some("undeclared.scope".into());
        assert!(matches!(
            validate_ui_surface_update(&surface, &bad_scope),
            Err(UiSurfaceValidationError::UndeclaredKeyScope { .. })
        ));
        Ok(())
    }

    #[test]
    fn ui_surface_conflicts_are_reported_by_surface_and_slot() -> Result<(), Box<dyn Error>> {
        let first = active_ui_surface(
            "ui.first",
            "first-ext",
            UiSurfaceKind::Footer,
            "footer:status",
            None,
        )?;
        let second = active_ui_surface(
            "ui.second",
            "second-ext",
            UiSurfaceKind::Footer,
            "footer:status",
            None,
        )?;
        let third = active_ui_surface(
            "ui.third",
            "third-ext",
            UiSurfaceKind::Sidebar,
            "sidebar:right",
            None,
        )?;

        let conflicts = detect_ui_surface_conflicts(&[first, second, third]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].surface, UiSurfaceKind::Footer);
        assert_eq!(conflicts[0].slot, "footer:status");
        assert_eq!(conflicts[0].owners.len(), 2);
        Ok(())
    }

    #[test]
    fn ui_surface_layout_decision_honors_tiny_terminal_fallbacks() -> Result<(), Box<dyn Error>> {
        let surface = active_ui_surface(
            "ui.status",
            "status-ext",
            UiSurfaceKind::Status,
            "status:footer",
            None,
        )?;
        assert_eq!(
            ui_surface_layout_decision(&surface.entry.contribution, 120, 30),
            UiSurfaceLayoutDecision::Render
        );
        assert_eq!(
            ui_surface_layout_decision(&surface.entry.contribution, 10, 5),
            UiSurfaceLayoutDecision::StatusLine
        );
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
