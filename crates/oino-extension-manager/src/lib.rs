#![doc = r#"Discovery, loading, reload, and management snapshots for Oino extensions.

The manager is intentionally data-oriented. It finds Oino-owned extension manifests,
validates them, wires declared
contributions into typed registries, composes read-only snapshots, and reports
health/diagnostic state without executing extension code.
"#]
#![forbid(unsafe_code)]

use oino_extension_builtins::BuiltinRegistryCatalog;
use oino_extension_core::{
    ActiveContribution, AdvisorySeverity, AutosuggestContribution, AutosuggestRegistry,
    CommandContribution, CommandRegistry, CommunityPackageMetadata, CommunityRegistryIndex,
    ContributionId, ContributionMetadata, ContributionRegistry, DiagnosticContribution,
    DiagnosticPhase, DiagnosticRegistry, DiagnosticSeverity, ExtensionContributions,
    ExtensionCoreError, ExtensionDiagnostic, ExtensionId, ExtensionManifest, ExtensionPermissions,
    HealthContribution, HealthRegistry, HealthState, HookContribution, HookRegistry,
    InactiveContribution, InactiveReason, KeymapContribution, KeymapRegistry, LifecycleState,
    PackageId, PackageManifest, PermissionDecision, PersistenceCleanupPolicy,
    PersistenceContribution, PersistenceMigrationPolicy, PersistenceRecord, PersistenceRegistry,
    PersistenceScope, Provenance, ProviderContribution, ProviderModelRegistry,
    RegistryCompatibility, RegistryDiff, RegistryEntryKey, RegistryFamily, RegistryPolicy,
    RegistrySnapshot, RendererContribution, ResourceContribution, ResourceRegistry,
    SecurityAdvisory, SettingsPageContribution, SettingsPageRegistry, SourceDescriptor, SourceKind,
    SourceScope, ThemeContribution, ThemeRegistry, ToolContribution, ToolRegistry,
    TypedContributionRegistry, UiSurfaceContribution, UiSurfaceRegistry, MANIFEST_FILE,
    PACKAGE_MANIFEST_FILE,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionManagerConfig {
    pub current_version: Version,
    pub discovery: ExtensionDiscovery,
    pub policy: RegistryPolicy,
    pub safe_mode: bool,
    pub denied_permissions: BTreeMap<ExtensionId, String>,
    pub builtins: Option<BuiltinRegistryCatalog>,
}

impl ExtensionManagerConfig {
    #[must_use]
    pub fn new(current_version: Version, discovery: ExtensionDiscovery) -> Self {
        Self {
            current_version,
            discovery,
            policy: RegistryPolicy::safe_defaults(),
            safe_mode: false,
            denied_permissions: BTreeMap::new(),
            builtins: None,
        }
    }

    #[must_use]
    pub fn with_policy(mut self, policy: RegistryPolicy) -> Self {
        self.policy = policy;
        self
    }

    #[must_use]
    pub fn with_safe_mode(mut self, safe_mode: bool) -> Self {
        self.safe_mode = safe_mode;
        self
    }

    #[must_use]
    pub fn with_builtins(mut self, builtins: BuiltinRegistryCatalog) -> Self {
        self.builtins = Some(builtins);
        self
    }

    #[must_use]
    pub fn deny_permissions(
        mut self,
        extension_id: ExtensionId,
        reason: impl Into<String>,
    ) -> Self {
        self.denied_permissions.insert(extension_id, reason.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionLayoutPaths {
    pub home_oino: PathBuf,
    pub project_oino: PathBuf,
    pub global_extensions: PathBuf,
    pub global_installed_packages: PathBuf,
    pub global_registry_packages: PathBuf,
    pub project_extensions: PathBuf,
    pub project_installed_packages: PathBuf,
    pub project_wasm_extensions: PathBuf,
    pub session_extensions: PathBuf,
    pub development_extensions: PathBuf,
    pub global_extension_state: PathBuf,
    pub project_extension_state: PathBuf,
    pub package_assets: PathBuf,
}

impl ExtensionLayoutPaths {
    #[must_use]
    pub fn for_home_and_project(home: impl AsRef<Path>, project: impl AsRef<Path>) -> Self {
        let home_oino = home.as_ref().join(".oino");
        let project_oino = project.as_ref().join(".oino");
        Self {
            global_extensions: home_oino.join("extensions"),
            global_installed_packages: home_oino.join("extension-packages"),
            global_registry_packages: home_oino.join("extension-registry"),
            project_extensions: project_oino.join("extensions"),
            project_installed_packages: project_oino.join("extension-packages"),
            project_wasm_extensions: project_oino.join("wasm-extensions"),
            session_extensions: project_oino.join("session-extensions"),
            development_extensions: project_oino.join("dev/extensions"),
            global_extension_state: home_oino.join("extension-state"),
            project_extension_state: project_oino.join("extension-state"),
            package_assets: project_oino.join("extension-assets"),
            home_oino,
            project_oino,
        }
    }

    #[must_use]
    pub fn discovery_roots(&self) -> Vec<ExtensionDiscoveryRoot> {
        vec![
            ExtensionDiscoveryRoot::new(
                self.global_extensions.clone(),
                SourceScope::Global,
                SourceKind::LocalExtension,
            ),
            ExtensionDiscoveryRoot::new(
                self.global_installed_packages.clone(),
                SourceScope::Global,
                SourceKind::InstalledPackage,
            ),
            ExtensionDiscoveryRoot::new(
                self.global_registry_packages.clone(),
                SourceScope::Global,
                SourceKind::RegistryPackage,
            ),
            ExtensionDiscoveryRoot::new(
                self.project_extensions.clone(),
                SourceScope::Project,
                SourceKind::LocalExtension,
            ),
            ExtensionDiscoveryRoot::new(
                self.project_installed_packages.clone(),
                SourceScope::Project,
                SourceKind::LocalPackage,
            ),
            ExtensionDiscoveryRoot::new(
                self.project_wasm_extensions.clone(),
                SourceScope::Project,
                SourceKind::WasmModule,
            ),
            ExtensionDiscoveryRoot::new(
                self.session_extensions.clone(),
                SourceScope::Session,
                SourceKind::LocalExtension,
            ),
            ExtensionDiscoveryRoot::new(
                self.development_extensions.clone(),
                SourceScope::Development,
                SourceKind::LocalExtension,
            ),
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionDiscovery {
    pub roots: Vec<ExtensionDiscoveryRoot>,
}

impl ExtensionDiscovery {
    #[must_use]
    pub fn new(roots: Vec<ExtensionDiscoveryRoot>) -> Self {
        let mut roots = roots;
        roots.sort_by(|left, right| {
            left.source
                .scope
                .precedence()
                .cmp(&right.source.scope.precedence())
                .then(left.source.kind.cmp(&right.source.kind))
                .then(left.path.cmp(&right.path))
        });
        Self { roots }
    }

    #[must_use]
    pub fn from_home_and_project(home: impl AsRef<Path>, project: impl AsRef<Path>) -> Self {
        Self::from_layout(&ExtensionLayoutPaths::for_home_and_project(home, project))
    }

    #[must_use]
    pub fn from_layout(layout: &ExtensionLayoutPaths) -> Self {
        Self::new(layout.discovery_roots())
    }

    #[must_use]
    pub fn discover_manifest_files(&self) -> Vec<DiscoveredExtensionFile> {
        let mut files = Vec::new();
        let mut diagnostics = Vec::new();
        self.discover_into(&mut files, &mut diagnostics);
        files
    }

    fn discover_into(
        &self,
        files: &mut Vec<DiscoveredExtensionFile>,
        diagnostics: &mut Vec<ExtensionDiagnostic>,
    ) {
        for root in &self.roots {
            discover_root(root, files, diagnostics);
        }
        files.sort_by(|left, right| {
            left.source
                .scope
                .precedence()
                .cmp(&right.source.scope.precedence())
                .then(left.source.kind.cmp(&right.source.kind))
                .then(left.path.cmp(&right.path))
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionDiscoveryRoot {
    pub path: PathBuf,
    pub source: SourceDescriptor,
}

impl ExtensionDiscoveryRoot {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, scope: SourceScope, kind: SourceKind) -> Self {
        let path = path.into();
        Self {
            source: SourceDescriptor {
                scope,
                kind,
                path: Some(path.clone()),
                registry: None,
            },
            path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredExtensionFile {
    pub path: PathBuf,
    pub source: SourceDescriptor,
    pub file_kind: DiscoveredFileKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveredFileKind {
    ExtensionManifest,
    PackageManifest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionRegistries {
    pub tools: ToolRegistry,
    pub commands: CommandRegistry,
    pub keymaps: KeymapRegistry,
    pub hooks: HookRegistry,
    pub ui_surfaces: UiSurfaceRegistry,
    pub settings_pages: SettingsPageRegistry,
    pub themes: ThemeRegistry,
    pub providers: ProviderModelRegistry,
    pub resources: ResourceRegistry,
    pub persistence: PersistenceRegistry,
    pub autosuggest_providers: AutosuggestRegistry,
    pub transcript_renderers: TypedContributionRegistry<RendererContribution>,
    pub message_renderers: TypedContributionRegistry<RendererContribution>,
    pub tool_renderers: TypedContributionRegistry<RendererContribution>,
    pub diagnostics: DiagnosticRegistry,
    pub health: HealthRegistry,
}

impl Default for ExtensionRegistries {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionRegistries {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: ToolRegistry::tools(),
            commands: CommandRegistry::commands(),
            keymaps: KeymapRegistry::keymaps(),
            hooks: HookRegistry::hooks(),
            ui_surfaces: UiSurfaceRegistry::ui_surfaces(),
            settings_pages: SettingsPageRegistry::settings_pages(),
            themes: ThemeRegistry::themes(),
            providers: ProviderModelRegistry::providers_models(),
            resources: ResourceRegistry::resources(),
            persistence: PersistenceRegistry::persistence(),
            autosuggest_providers: AutosuggestRegistry::autosuggest_providers(),
            transcript_renderers: TypedContributionRegistry::transcript_renderers(),
            message_renderers: TypedContributionRegistry::message_renderers(),
            tool_renderers: TypedContributionRegistry::tool_renderers(),
            diagnostics: DiagnosticRegistry::diagnostics(),
            health: HealthRegistry::health(),
        }
    }

    #[must_use]
    pub fn with_builtins(builtins: BuiltinRegistryCatalog) -> Self {
        Self {
            tools: builtins.tools,
            commands: builtins.commands,
            keymaps: builtins.keymaps,
            hooks: builtins.hooks,
            settings_pages: builtins.settings_pages,
            themes: builtins.themes,
            providers: builtins.providers,
            resources: builtins.resources,
            ..Self::new()
        }
    }

    #[must_use]
    pub fn compose(&self, policy: &RegistryPolicy) -> RegistrySnapshotBundle {
        RegistrySnapshotBundle {
            tools: self.tools.compose(policy),
            commands: self.commands.compose(policy),
            keymaps: self.keymaps.compose(policy),
            hooks: self.hooks.compose(policy),
            ui_surfaces: self.ui_surfaces.compose(policy),
            settings_pages: self.settings_pages.compose(policy),
            themes: self.themes.compose(policy),
            providers: self.providers.compose(policy),
            resources: self.resources.compose(policy),
            persistence: self.persistence.compose(policy),
            autosuggest_providers: self.autosuggest_providers.compose(policy),
            transcript_renderers: self.transcript_renderers.compose(policy),
            message_renderers: self.message_renderers.compose(policy),
            tool_renderers: self.tool_renderers.compose(policy),
            diagnostics: self.diagnostics.compose(policy),
            health: self.health.compose(policy),
        }
    }

    fn external_entry_keys(&self) -> BTreeSet<RegistryEntryKey> {
        let mut keys = BTreeSet::new();
        collect_external_keys(self.tools.inner(), &mut keys);
        collect_external_keys(self.commands.inner(), &mut keys);
        collect_external_keys(self.keymaps.inner(), &mut keys);
        collect_external_keys(self.hooks.inner(), &mut keys);
        collect_external_keys(self.ui_surfaces.inner(), &mut keys);
        collect_external_keys(self.settings_pages.inner(), &mut keys);
        collect_external_keys(self.themes.inner(), &mut keys);
        collect_external_keys(self.providers.inner(), &mut keys);
        collect_external_keys(self.resources.inner(), &mut keys);
        collect_external_keys(self.persistence.inner(), &mut keys);
        collect_external_keys(self.autosuggest_providers.inner(), &mut keys);
        collect_external_keys(self.transcript_renderers.inner(), &mut keys);
        collect_external_keys(self.message_renderers.inner(), &mut keys);
        collect_external_keys(self.tool_renderers.inner(), &mut keys);
        collect_external_keys(self.diagnostics.inner(), &mut keys);
        collect_external_keys(self.health.inner(), &mut keys);
        keys
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistrySnapshotBundle {
    pub tools: RegistrySnapshot<ToolContribution>,
    pub commands: RegistrySnapshot<CommandContribution>,
    pub keymaps: RegistrySnapshot<KeymapContribution>,
    pub hooks: RegistrySnapshot<HookContribution>,
    pub ui_surfaces: RegistrySnapshot<UiSurfaceContribution>,
    pub settings_pages: RegistrySnapshot<SettingsPageContribution>,
    pub themes: RegistrySnapshot<ThemeContribution>,
    pub providers: RegistrySnapshot<ProviderContribution>,
    pub resources: RegistrySnapshot<ResourceContribution>,
    pub persistence: RegistrySnapshot<PersistenceContribution>,
    pub autosuggest_providers: RegistrySnapshot<AutosuggestContribution>,
    pub transcript_renderers: RegistrySnapshot<RendererContribution>,
    pub message_renderers: RegistrySnapshot<RendererContribution>,
    pub tool_renderers: RegistrySnapshot<RendererContribution>,
    pub diagnostics: RegistrySnapshot<DiagnosticContribution>,
    pub health: RegistrySnapshot<HealthContribution>,
}

impl Default for RegistrySnapshotBundle {
    fn default() -> Self {
        Self {
            tools: empty_snapshot(),
            commands: empty_snapshot(),
            keymaps: empty_snapshot(),
            hooks: empty_snapshot(),
            ui_surfaces: empty_snapshot(),
            settings_pages: empty_snapshot(),
            themes: empty_snapshot(),
            providers: empty_snapshot(),
            resources: empty_snapshot(),
            persistence: empty_snapshot(),
            autosuggest_providers: empty_snapshot(),
            transcript_renderers: empty_snapshot(),
            message_renderers: empty_snapshot(),
            tool_renderers: empty_snapshot(),
            diagnostics: empty_snapshot(),
            health: empty_snapshot(),
        }
    }
}

fn empty_snapshot<T>() -> RegistrySnapshot<T> {
    RegistrySnapshot {
        active: Vec::new(),
        inactive: Vec::new(),
        diagnostics: Vec::new(),
    }
}

impl RegistrySnapshotBundle {
    #[must_use]
    pub fn diff(&self, next: &Self) -> RegistryDiffBundle {
        RegistryDiffBundle {
            tools: self.tools.diff(&next.tools),
            commands: self.commands.diff(&next.commands),
            keymaps: self.keymaps.diff(&next.keymaps),
            hooks: self.hooks.diff(&next.hooks),
            ui_surfaces: self.ui_surfaces.diff(&next.ui_surfaces),
            settings_pages: self.settings_pages.diff(&next.settings_pages),
            themes: self.themes.diff(&next.themes),
            providers: self.providers.diff(&next.providers),
            resources: self.resources.diff(&next.resources),
            persistence: self.persistence.diff(&next.persistence),
            autosuggest_providers: self.autosuggest_providers.diff(&next.autosuggest_providers),
            transcript_renderers: self.transcript_renderers.diff(&next.transcript_renderers),
            message_renderers: self.message_renderers.diff(&next.message_renderers),
            tool_renderers: self.tool_renderers.diff(&next.tool_renderers),
            diagnostics: self.diagnostics.diff(&next.diagnostics),
            health: self.health.diff(&next.health),
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> Vec<ExtensionDiagnostic> {
        let mut diagnostics = Vec::new();
        diagnostics.extend(self.tools.diagnostics.clone());
        diagnostics.extend(self.commands.diagnostics.clone());
        diagnostics.extend(self.keymaps.diagnostics.clone());
        diagnostics.extend(self.hooks.diagnostics.clone());
        diagnostics.extend(self.ui_surfaces.diagnostics.clone());
        diagnostics.extend(self.settings_pages.diagnostics.clone());
        diagnostics.extend(self.themes.diagnostics.clone());
        diagnostics.extend(self.providers.diagnostics.clone());
        diagnostics.extend(self.resources.diagnostics.clone());
        diagnostics.extend(self.persistence.diagnostics.clone());
        diagnostics.extend(self.autosuggest_providers.diagnostics.clone());
        diagnostics.extend(self.transcript_renderers.diagnostics.clone());
        diagnostics.extend(self.message_renderers.diagnostics.clone());
        diagnostics.extend(self.tool_renderers.diagnostics.clone());
        diagnostics.extend(self.diagnostics.diagnostics.clone());
        diagnostics.extend(self.health.diagnostics.clone());
        diagnostics
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistryDiffBundle {
    pub tools: RegistryDiff<ToolContribution>,
    pub commands: RegistryDiff<CommandContribution>,
    pub keymaps: RegistryDiff<KeymapContribution>,
    pub hooks: RegistryDiff<HookContribution>,
    pub ui_surfaces: RegistryDiff<UiSurfaceContribution>,
    pub settings_pages: RegistryDiff<SettingsPageContribution>,
    pub themes: RegistryDiff<ThemeContribution>,
    pub providers: RegistryDiff<ProviderContribution>,
    pub resources: RegistryDiff<ResourceContribution>,
    pub persistence: RegistryDiff<PersistenceContribution>,
    pub autosuggest_providers: RegistryDiff<AutosuggestContribution>,
    pub transcript_renderers: RegistryDiff<RendererContribution>,
    pub message_renderers: RegistryDiff<RendererContribution>,
    pub tool_renderers: RegistryDiff<RendererContribution>,
    pub diagnostics: RegistryDiff<DiagnosticContribution>,
    pub health: RegistryDiff<HealthContribution>,
}

impl Default for RegistryDiffBundle {
    fn default() -> Self {
        Self {
            tools: empty_diff(),
            commands: empty_diff(),
            keymaps: empty_diff(),
            hooks: empty_diff(),
            ui_surfaces: empty_diff(),
            settings_pages: empty_diff(),
            themes: empty_diff(),
            providers: empty_diff(),
            resources: empty_diff(),
            persistence: empty_diff(),
            autosuggest_providers: empty_diff(),
            transcript_renderers: empty_diff(),
            message_renderers: empty_diff(),
            tool_renderers: empty_diff(),
            diagnostics: empty_diff(),
            health: empty_diff(),
        }
    }
}

fn empty_diff<T>() -> RegistryDiff<T> {
    RegistryDiff {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
    }
}

impl RegistryDiffBundle {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        diff_empty(&self.tools)
            && diff_empty(&self.commands)
            && diff_empty(&self.keymaps)
            && diff_empty(&self.hooks)
            && diff_empty(&self.ui_surfaces)
            && diff_empty(&self.settings_pages)
            && diff_empty(&self.themes)
            && diff_empty(&self.providers)
            && diff_empty(&self.resources)
            && diff_empty(&self.persistence)
            && diff_empty(&self.autosuggest_providers)
            && diff_empty(&self.transcript_renderers)
            && diff_empty(&self.message_renderers)
            && diff_empty(&self.tool_renderers)
            && diff_empty(&self.diagnostics)
            && diff_empty(&self.health)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionManagerSnapshot {
    pub safe_mode: bool,
    pub registries: RegistrySnapshotBundle,
    pub extensions: Vec<ExtensionRecord>,
    pub packages: Vec<PackageRecord>,
    pub contributions: Vec<ContributionRecord>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
    pub diagnostic_groups: Vec<DiagnosticGroup>,
}

impl ExtensionManagerSnapshot {
    #[must_use]
    pub fn extension(&self, id: &ExtensionId) -> Option<&ExtensionRecord> {
        self.extensions.iter().find(|record| &record.id == id)
    }

    pub fn apply_health_event(&mut self, event: ExtensionHealthEvent) {
        let diagnostic = event.to_diagnostic();
        if let Some(extension_id) = &event.extension_id {
            if let Some(record) = self
                .extensions
                .iter_mut()
                .find(|record| &record.id == extension_id)
            {
                record.health = event.health;
                record.lifecycle = lifecycle_for_health(event.health);
            }
        }
        if let Some(contribution_id) = &event.contribution_id {
            for record in self
                .contributions
                .iter_mut()
                .filter(|record| &record.id == contribution_id)
            {
                record.health = event.health;
                record.state = contribution_state_for_health(event.health);
            }
        }
        self.diagnostics.push(diagnostic);
        self.diagnostic_groups = group_diagnostics(&self.diagnostics);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRecord {
    pub id: ExtensionId,
    pub display_name: String,
    pub version: Version,
    pub source: SourceDescriptor,
    pub lifecycle: LifecycleState,
    pub health: HealthState,
    pub permissions: ExtensionPermissions,
    pub package_id: Option<PackageId>,
    pub provenance: Option<Provenance>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageRecord {
    pub id: PackageId,
    pub display_name: String,
    pub version: Version,
    pub source: SourceDescriptor,
    pub lifecycle: LifecycleState,
    pub health: HealthState,
    pub diagnostics: Vec<ExtensionDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionRecord {
    pub family: RegistryFamily,
    pub id: ContributionId,
    pub canonical_id: ContributionId,
    pub entry_key: RegistryEntryKey,
    pub source: SourceDescriptor,
    pub extension_id: Option<ExtensionId>,
    pub package_id: Option<PackageId>,
    pub state: ContributionState,
    pub lifecycle: LifecycleState,
    pub health: HealthState,
    pub permission: PermissionDecision,
    pub compatibility: RegistryCompatibility,
    pub provenance: Option<Provenance>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionState {
    Active,
    Disabled,
    PendingReview,
    Blocked,
    Shadowed,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiagnosticGroupKey {
    pub package_id: Option<PackageId>,
    pub extension_id: Option<ExtensionId>,
    pub contribution_id: Option<ContributionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticGroup {
    pub key: DiagnosticGroupKey,
    pub health: HealthState,
    pub diagnostics: Vec<ExtensionDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionHealthEvent {
    pub phase: DiagnosticPhase,
    pub severity: DiagnosticSeverity,
    pub health: HealthState,
    pub message: String,
    pub remediation: Option<String>,
    pub package_id: Option<PackageId>,
    pub extension_id: Option<ExtensionId>,
    pub contribution_id: Option<ContributionId>,
}

impl ExtensionHealthEvent {
    #[must_use]
    pub fn runtime_crash(extension_id: ExtensionId, message: impl Into<String>) -> Self {
        Self {
            phase: DiagnosticPhase::RuntimeExecute,
            severity: DiagnosticSeverity::Error,
            health: HealthState::Unhealthy,
            message: message.into(),
            remediation: Some("disable the extension or reload it after fixing the runtime".into()),
            package_id: None,
            extension_id: Some(extension_id),
            contribution_id: None,
        }
    }

    #[must_use]
    pub fn permission_denied(
        extension_id: ExtensionId,
        contribution_id: ContributionId,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase: DiagnosticPhase::Permission,
            severity: DiagnosticSeverity::Error,
            health: HealthState::Blocked,
            message: message.into(),
            remediation: Some("grant permission or keep the contribution disabled".into()),
            package_id: None,
            extension_id: Some(extension_id),
            contribution_id: Some(contribution_id),
        }
    }

    #[must_use]
    pub fn hook_timeout(extension_id: ExtensionId, contribution_id: ContributionId) -> Self {
        Self {
            phase: DiagnosticPhase::RuntimeExecute,
            severity: DiagnosticSeverity::Warning,
            health: HealthState::Degraded,
            message: "hook timed out".into(),
            remediation: Some("increase timeout, fix the hook, or disable the contribution".into()),
            package_id: None,
            extension_id: Some(extension_id),
            contribution_id: Some(contribution_id),
        }
    }

    #[must_use]
    pub fn invalid_ui_update(extension_id: ExtensionId, contribution_id: ContributionId) -> Self {
        Self {
            phase: DiagnosticPhase::UiUpdate,
            severity: DiagnosticSeverity::Warning,
            health: HealthState::Degraded,
            message: "extension emitted an invalid UI update".into(),
            remediation: Some("validate UI payloads against the registered schema".into()),
            package_id: None,
            extension_id: Some(extension_id),
            contribution_id: Some(contribution_id),
        }
    }

    fn to_diagnostic(&self) -> ExtensionDiagnostic {
        ExtensionDiagnostic {
            severity: self.severity,
            phase: self.phase,
            package_id: self.package_id.clone(),
            extension_id: self.extension_id.clone(),
            contribution_id: self.contribution_id.clone(),
            source_path: None,
            message: self.message.clone(),
            remediation: self.remediation.clone(),
            health: self.health,
        }
    }
}

#[derive(Debug, Error)]
pub enum PersistenceStoreError {
    #[error("persistence key `{0}` is invalid")]
    InvalidKey(String),
    #[error("extension `{actual}` cannot access state owned by `{expected}`")]
    OwnerMismatch {
        expected: ExtensionId,
        actual: ExtensionId,
    },
    #[error("extension `{extension_id}` lacks persistence permission for `{scope:?}`")]
    PermissionDenied {
        extension_id: ExtensionId,
        scope: PersistenceScope,
    },
    #[error("persistence payload is {actual} bytes, max is {max}")]
    Oversized { actual: usize, max: usize },
    #[error("persistence record is corrupted: {0}")]
    Corrupted(String),
    #[error("persistence record not found")]
    NotFound,
    #[error("persistence migration from schema {from} to {to} is not available")]
    MigrationUnavailable { from: u32, to: u32 },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionPersistenceStore {
    root: PathBuf,
    default_max_bytes: usize,
}

impl ExtensionPersistenceStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            default_max_bytes: 64 * 1024,
        }
    }

    #[must_use]
    pub fn with_default_max_bytes(mut self, default_max_bytes: usize) -> Self {
        self.default_max_bytes = default_max_bytes;
        self
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write(
        &self,
        record: &PersistenceRecord,
        permissions: &ExtensionPermissions,
        max_bytes: Option<usize>,
    ) -> Result<(), PersistenceStoreError> {
        validate_persistence_key(&record.key)?;
        ensure_persistence_permission(&record.owner_extension_id, record.scope, permissions)?;
        let bytes = serde_json::to_vec(record)?;
        let max = max_bytes.unwrap_or(self.default_max_bytes);
        if bytes.len() > max {
            return Err(PersistenceStoreError::Oversized {
                actual: bytes.len(),
                max,
            });
        }
        let path = self.record_path(&record.owner_extension_id, record.scope, &record.key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
        Ok(())
    }

    pub fn read(
        &self,
        extension_id: &ExtensionId,
        scope: PersistenceScope,
        key: &str,
        permissions: &ExtensionPermissions,
    ) -> Result<PersistenceRecord, PersistenceStoreError> {
        validate_persistence_key(key)?;
        ensure_persistence_permission(extension_id, scope, permissions)?;
        let path = self.record_path(extension_id, scope, key)?;
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(PersistenceStoreError::NotFound)
            }
            Err(err) => return Err(err.into()),
        };
        let record = serde_json::from_str::<PersistenceRecord>(&text)
            .map_err(|err| PersistenceStoreError::Corrupted(err.to_string()))?;
        if &record.owner_extension_id != extension_id {
            return Err(PersistenceStoreError::OwnerMismatch {
                expected: record.owner_extension_id,
                actual: extension_id.clone(),
            });
        }
        Ok(record)
    }

    pub fn delete(
        &self,
        extension_id: &ExtensionId,
        scope: PersistenceScope,
        key: &str,
        permissions: &ExtensionPermissions,
    ) -> Result<(), PersistenceStoreError> {
        validate_persistence_key(key)?;
        ensure_persistence_permission(extension_id, scope, permissions)?;
        let path = self.record_path(extension_id, scope, key)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn migrate(
        &self,
        extension_id: &ExtensionId,
        scope: PersistenceScope,
        key: &str,
        target_version: u32,
        policy: &PersistenceMigrationPolicy,
        permissions: &ExtensionPermissions,
    ) -> Result<PersistenceRecord, PersistenceStoreError> {
        let mut record = self.read(extension_id, scope, key, permissions)?;
        if record.schema_version >= target_version {
            return Ok(record);
        }
        match policy {
            PersistenceMigrationPolicy::HostCopyForward => {
                record.schema_version = target_version;
                record.updated_at_unix_ms = current_unix_ms();
                self.write(&record, permissions, None)?;
                Ok(record)
            }
            PersistenceMigrationPolicy::None | PersistenceMigrationPolicy::ExtensionHook { .. } => {
                Err(PersistenceStoreError::MigrationUnavailable {
                    from: record.schema_version,
                    to: target_version,
                })
            }
        }
    }

    pub fn cleanup_extension(
        &self,
        extension_id: &ExtensionId,
        policy: PersistenceCleanupPolicy,
    ) -> Result<(), PersistenceStoreError> {
        match policy {
            PersistenceCleanupPolicy::Retain => Ok(()),
            PersistenceCleanupPolicy::DeleteOnUninstall => {
                for scope in [
                    PersistenceScope::Session,
                    PersistenceScope::Project,
                    PersistenceScope::Global,
                ] {
                    let path = self.extension_scope_dir(extension_id, scope);
                    match fs::remove_dir_all(path) {
                        Ok(()) => {}
                        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                        Err(err) => return Err(err.into()),
                    }
                }
                Ok(())
            }
            PersistenceCleanupPolicy::RetainWithTombstone => {
                for mut record in self.list_extension(extension_id)? {
                    record.tombstoned = true;
                    record.updated_at_unix_ms = current_unix_ms();
                    let path =
                        self.record_path(&record.owner_extension_id, record.scope, &record.key)?;
                    fs::write(path, serde_json::to_vec(&record)?)?;
                }
                Ok(())
            }
        }
    }

    pub fn list_extension(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<PersistenceRecord>, PersistenceStoreError> {
        let mut records = Vec::new();
        for scope in [
            PersistenceScope::Session,
            PersistenceScope::Project,
            PersistenceScope::Global,
        ] {
            let dir = self.extension_scope_dir(extension_id, scope);
            if !dir.exists() {
                continue;
            }
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let text = fs::read_to_string(path)?;
                let record = serde_json::from_str::<PersistenceRecord>(&text)
                    .map_err(|err| PersistenceStoreError::Corrupted(err.to_string()))?;
                records.push(record);
            }
        }
        records.sort_by(|left, right| {
            left.scope
                .cmp(&right.scope)
                .then(left.owner_extension_id.cmp(&right.owner_extension_id))
                .then(left.key.cmp(&right.key))
        });
        Ok(records)
    }

    fn record_path(
        &self,
        extension_id: &ExtensionId,
        scope: PersistenceScope,
        key: &str,
    ) -> Result<PathBuf, PersistenceStoreError> {
        validate_persistence_key(key)?;
        Ok(self
            .extension_scope_dir(extension_id, scope)
            .join(format!("{key}.json")))
    }

    fn extension_scope_dir(&self, extension_id: &ExtensionId, scope: PersistenceScope) -> PathBuf {
        self.root
            .join(scope_slug(scope))
            .join(extension_id.as_str())
    }
}

fn ensure_persistence_permission(
    extension_id: &ExtensionId,
    scope: PersistenceScope,
    permissions: &ExtensionPermissions,
) -> Result<(), PersistenceStoreError> {
    if permissions.allows_persistence_scope(scope) {
        Ok(())
    } else {
        Err(PersistenceStoreError::PermissionDenied {
            extension_id: extension_id.clone(),
            scope,
        })
    }
}

fn validate_persistence_key(key: &str) -> Result<(), PersistenceStoreError> {
    let valid = !key.trim().is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(PersistenceStoreError::InvalidKey(key.into()))
    }
}

fn scope_slug(scope: PersistenceScope) -> &'static str {
    match scope {
        PersistenceScope::Session => "session",
        PersistenceScope::Project => "project",
        PersistenceScope::Global => "global",
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageInstallScope {
    Global,
    Project,
}

impl PackageInstallScope {
    #[must_use]
    pub const fn source_scope(self) -> SourceScope {
        match self {
            Self::Global => SourceScope::Global,
            Self::Project => SourceScope::Project,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageLifecycleOperation {
    Install,
    Update,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackagePermissionPrompt {
    pub package_id: PackageId,
    pub permissions: ExtensionPermissions,
    pub trust: oino_extension_core::TrustMetadata,
    pub operation: PackageLifecycleOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageLifecycleReport {
    pub operation: PackageLifecycleOperation,
    pub package_id: PackageId,
    pub version: Version,
    pub destination: PathBuf,
    pub permission_prompt: PackagePermissionPrompt,
    pub diagnostics: Vec<ExtensionDiagnostic>,
    pub reload: ExtensionReload,
}

#[derive(Debug, Error)]
pub enum PackageLifecycleError {
    #[error("package manifest not found at {0}")]
    MissingManifest(PathBuf),
    #[error("package `{0}` is already installed")]
    AlreadyInstalled(PackageId),
    #[error("package `{0}` is not installed")]
    NotInstalled(PackageId),
    #[error("package `{package_id}` is incompatible with Oino {current_version}; requires {requirement}")]
    Incompatible {
        package_id: PackageId,
        current_version: Version,
        requirement: String,
    },
    #[error("dependency `{dependency}` is missing or incompatible")]
    DependencyConflict { dependency: PackageId },
    #[error("package `{package_id}` requires install scope `{required:?}`, got `{actual:?}`")]
    InvalidInstallScope {
        package_id: PackageId,
        required: SourceScope,
        actual: PackageInstallScope,
    },
    #[error("package checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("package signature `{0}` is not trusted by local policy")]
    SignatureRejected(String),
    #[error("package validation failed: {0}")]
    Validation(ExtensionCoreError),
    #[error("package metadata parse failed: {0}")]
    Parse(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageLifecycleService {
    layout: ExtensionLayoutPaths,
    current_version: Version,
}

impl PackageLifecycleService {
    #[must_use]
    pub fn new(layout: ExtensionLayoutPaths, current_version: Version) -> Self {
        Self {
            layout,
            current_version,
        }
    }

    #[must_use]
    pub fn layout(&self) -> &ExtensionLayoutPaths {
        &self.layout
    }

    pub fn install_local(
        &self,
        source_dir: impl AsRef<Path>,
        scope: PackageInstallScope,
        manager: &mut ExtensionManager,
    ) -> Result<PackageLifecycleReport, PackageLifecycleError> {
        let source_dir = source_dir.as_ref();
        let manifest = read_package_manifest_from_dir(source_dir)?;
        self.preflight_package(&manifest, source_dir, scope)?;
        let destination = self.destination_for(&manifest.id, scope);
        if destination.exists() {
            return Err(PackageLifecycleError::AlreadyInstalled(manifest.id));
        }
        copy_dir_rollback(source_dir, &destination)?;
        let reload = manager.reload();
        Ok(self.report(
            PackageLifecycleOperation::Install,
            manifest,
            destination,
            reload,
        ))
    }

    pub fn update_local(
        &self,
        source_dir: impl AsRef<Path>,
        scope: PackageInstallScope,
        manager: &mut ExtensionManager,
    ) -> Result<PackageLifecycleReport, PackageLifecycleError> {
        let source_dir = source_dir.as_ref();
        let manifest = read_package_manifest_from_dir(source_dir)?;
        self.preflight_package(&manifest, source_dir, scope)?;
        let destination = self.destination_for(&manifest.id, scope);
        if !destination.exists() {
            return Err(PackageLifecycleError::NotInstalled(manifest.id));
        }
        replace_dir_with_backup(source_dir, &destination)?;
        let reload = manager.reload();
        Ok(self.report(
            PackageLifecycleOperation::Update,
            manifest,
            destination,
            reload,
        ))
    }

    pub fn remove(
        &self,
        package_id: PackageId,
        scope: PackageInstallScope,
        manager: &mut ExtensionManager,
    ) -> Result<PackageLifecycleReport, PackageLifecycleError> {
        let destination = self.destination_for(&package_id, scope);
        if !destination.exists() {
            return Err(PackageLifecycleError::NotInstalled(package_id));
        }
        let manifest = read_package_manifest_from_dir(&destination)?;
        fs::remove_dir_all(&destination)?;
        let reload = manager.reload();
        Ok(self.report(
            PackageLifecycleOperation::Remove,
            manifest,
            destination,
            reload,
        ))
    }

    pub fn install_from_registry(
        &self,
        registry: &FixtureRegistryClient,
        package_id: &PackageId,
        scope: PackageInstallScope,
        manager: &mut ExtensionManager,
    ) -> Result<PackageLifecycleReport, PackageLifecycleError> {
        let metadata = registry
            .latest_package(package_id)
            .ok_or_else(|| PackageLifecycleError::NotInstalled(package_id.clone()))?;
        let Some(path) = &metadata.package_path else {
            return Err(PackageLifecycleError::MissingManifest(PathBuf::from(
                format!("registry:{package_id}"),
            )));
        };
        self.install_local(path, scope, manager)
    }

    fn preflight_package(
        &self,
        manifest: &PackageManifest,
        source_dir: &Path,
        scope: PackageInstallScope,
    ) -> Result<(), PackageLifecycleError> {
        manifest
            .validate()
            .map_err(PackageLifecycleError::Validation)?;
        if !manifest.compatible_with(&self.current_version) {
            return Err(PackageLifecycleError::Incompatible {
                package_id: manifest.id.clone(),
                current_version: self.current_version.clone(),
                requirement: manifest.oino.to_string(),
            });
        }
        if let Some(source) = &manifest.source {
            let actual_scope = scope.source_scope();
            if source.scope != actual_scope {
                return Err(PackageLifecycleError::InvalidInstallScope {
                    package_id: manifest.id.clone(),
                    required: source.scope,
                    actual: scope,
                });
            }
        }
        for dependency in &manifest.dependencies {
            if dependency.optional {
                continue;
            }
            let Some(installed) =
                self.installed_package_version_in_scope_or_global(&dependency.id, scope)
            else {
                return Err(PackageLifecycleError::DependencyConflict {
                    dependency: dependency.id.clone(),
                });
            };
            if !dependency.version.matches(&installed) {
                return Err(PackageLifecycleError::DependencyConflict {
                    dependency: dependency.id.clone(),
                });
            }
        }
        if let Some(expected) = manifest.trust.checksum.as_deref() {
            let actual = package_directory_checksum(source_dir)?;
            if expected != actual {
                return Err(PackageLifecycleError::ChecksumMismatch {
                    expected: expected.into(),
                    actual,
                });
            }
        }
        if let Some(signature) = manifest.trust.signature.as_deref() {
            if !manifest.trust.reviewed {
                return Err(PackageLifecycleError::SignatureRejected(signature.into()));
            }
        }
        Ok(())
    }

    fn installed_package_version_in_scope_or_global(
        &self,
        package_id: &PackageId,
        scope: PackageInstallScope,
    ) -> Option<Version> {
        self.installed_package_version(package_id, scope)
            .or_else(|| match scope {
                PackageInstallScope::Project => {
                    self.installed_package_version(package_id, PackageInstallScope::Global)
                }
                PackageInstallScope::Global => None,
            })
    }

    fn installed_package_version(
        &self,
        package_id: &PackageId,
        scope: PackageInstallScope,
    ) -> Option<Version> {
        let path = self
            .destination_for(package_id, scope)
            .join(PACKAGE_MANIFEST_FILE);
        read_json::<PackageManifest>(&path)
            .ok()
            .map(|manifest| manifest.version)
    }

    fn destination_for(&self, package_id: &PackageId, scope: PackageInstallScope) -> PathBuf {
        match scope {
            PackageInstallScope::Global => &self.layout.global_installed_packages,
            PackageInstallScope::Project => &self.layout.project_installed_packages,
        }
        .join(package_id.as_str())
    }

    fn report(
        &self,
        operation: PackageLifecycleOperation,
        manifest: PackageManifest,
        destination: PathBuf,
        reload: ExtensionReload,
    ) -> PackageLifecycleReport {
        let diagnostics = reload.next.diagnostics.clone();
        let permission_prompt = PackagePermissionPrompt {
            package_id: manifest.id.clone(),
            permissions: manifest.permissions.clone(),
            trust: manifest.trust.clone(),
            operation,
        };
        PackageLifecycleReport {
            operation,
            package_id: manifest.id,
            version: manifest.version,
            destination,
            permission_prompt,
            diagnostics,
            reload,
        }
    }
}

fn read_package_manifest_from_dir(dir: &Path) -> Result<PackageManifest, PackageLifecycleError> {
    let manifest_path = dir.join(PACKAGE_MANIFEST_FILE);
    if !manifest_path.exists() {
        return Err(PackageLifecycleError::MissingManifest(manifest_path));
    }
    read_json::<PackageManifest>(&manifest_path).map_err(PackageLifecycleError::Parse)
}

fn copy_dir_rollback(source: &Path, destination: &Path) -> Result<(), io::Error> {
    if let Err(err) = copy_dir_recursive(source, destination) {
        let _ = fs::remove_dir_all(destination);
        return Err(err);
    }
    Ok(())
}

fn replace_dir_with_backup(source: &Path, destination: &Path) -> Result<(), io::Error> {
    let backup = destination.with_extension("oino-backup");
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    fs::rename(destination, &backup)?;
    match copy_dir_recursive(source, destination) {
        Ok(()) => {
            fs::remove_dir_all(backup)?;
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_dir_all(destination);
            let _ = fs::rename(&backup, destination);
            Err(err)
        }
    }
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

pub fn package_directory_checksum(path: &Path) -> Result<String, io::Error> {
    let mut files = Vec::new();
    collect_package_files(path, path, &mut files)?;
    files.sort();
    let mut hash = 0xcbf29ce484222325u64;
    for relative in files {
        let absolute = path.join(&relative);
        update_checksum(&mut hash, relative.to_string_lossy().as_bytes());
        update_checksum(&mut hash, &checksum_file_bytes(&relative, &absolute)?);
    }
    Ok(format!("oino-fnv64:{hash:016x}"))
}

fn collect_package_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), io::Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_package_files(root, &path, files)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn checksum_file_bytes(relative: &Path, absolute: &Path) -> Result<Vec<u8>, io::Error> {
    let bytes = fs::read(absolute)?;
    if relative == Path::new(PACKAGE_MANIFEST_FILE) {
        if let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(trust) = value
                .get_mut("trust")
                .and_then(serde_json::Value::as_object_mut)
            {
                trust.remove("checksum");
                trust.remove("signature");
            }
            if let Ok(normalized) = serde_json::to_vec(&value) {
                return Ok(normalized);
            }
        }
    }
    Ok(bytes)
}

fn update_checksum(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryTrustPolicy {
    #[serde(default)]
    pub require_review: bool,
    #[serde(default)]
    pub require_checksum: bool,
    #[serde(default)]
    pub require_signature: bool,
    #[serde(default)]
    pub reject_deprecated: bool,
    #[serde(default)]
    pub reject_high_advisories: bool,
}

impl Default for RegistryTrustPolicy {
    fn default() -> Self {
        Self {
            require_review: true,
            require_checksum: true,
            require_signature: false,
            reject_deprecated: true,
            reject_high_advisories: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishingValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl PublishingValidation {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureRegistryClient {
    index: CommunityRegistryIndex,
}

impl FixtureRegistryClient {
    #[must_use]
    pub fn new(index: CommunityRegistryIndex) -> Self {
        Self { index }
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, PackageLifecycleError> {
        Ok(Self::new(
            read_json::<CommunityRegistryIndex>(path.as_ref())
                .map_err(PackageLifecycleError::Parse)?,
        ))
    }

    #[must_use]
    pub fn index(&self) -> &CommunityRegistryIndex {
        &self.index
    }

    #[must_use]
    pub fn latest_package(&self, package_id: &PackageId) -> Option<&CommunityPackageMetadata> {
        self.index
            .packages
            .iter()
            .filter(|package| &package.id == package_id)
            .max_by(|left, right| left.version.cmp(&right.version))
    }

    #[must_use]
    pub fn search(
        &self,
        query: &str,
        category: Option<&str>,
        current_version: &Version,
    ) -> Vec<&CommunityPackageMetadata> {
        let query = query.trim().to_ascii_lowercase();
        let category = category.map(str::to_ascii_lowercase);
        let mut packages = self
            .index
            .packages
            .iter()
            .filter(|package| package.oino.matches(current_version))
            .filter(|package| {
                category.as_ref().is_none_or(|category| {
                    package
                        .categories
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(category))
                })
            })
            .filter(|package| {
                query.is_empty()
                    || package.id.as_str().contains(&query)
                    || package.display_name.to_ascii_lowercase().contains(&query)
                    || package.description.to_ascii_lowercase().contains(&query)
            })
            .collect::<Vec<_>>();
        packages.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then(right.version.cmp(&left.version))
        });
        packages
    }

    #[must_use]
    pub fn advisories_for(&self, package_id: &PackageId) -> Vec<&SecurityAdvisory> {
        self.index
            .advisories
            .iter()
            .filter(|advisory| &advisory.package_id == package_id && !advisory.withdrawn)
            .collect()
    }
}

#[must_use]
pub fn validate_registry_package_metadata(
    package: &CommunityPackageMetadata,
    current_version: &Version,
    advisories: &[SecurityAdvisory],
    policy: &RegistryTrustPolicy,
) -> PublishingValidation {
    let mut validation = PublishingValidation::default();
    if package.publisher.trim().is_empty() {
        validation.errors.push("publisher is required".into());
    }
    if package.description.trim().is_empty() {
        validation.warnings.push("description is empty".into());
    }
    if package.categories.is_empty() {
        validation
            .warnings
            .push("at least one category is recommended".into());
    }
    if !package.oino.matches(current_version) {
        validation.errors.push(format!(
            "package is incompatible with Oino {current_version}"
        ));
    }
    if policy.require_review && !package.trust.reviewed {
        validation
            .errors
            .push("package has not been reviewed".into());
    }
    if policy.require_checksum
        && package
            .trust
            .checksum
            .as_deref()
            .unwrap_or_default()
            .is_empty()
    {
        validation
            .errors
            .push("package checksum is required".into());
    }
    if policy.require_signature
        && package
            .trust
            .signature
            .as_deref()
            .unwrap_or_default()
            .is_empty()
    {
        validation
            .errors
            .push("package signature is required".into());
    }
    if policy.reject_deprecated && package.deprecated {
        validation.errors.push("package is deprecated".into());
    }
    let active_advisories = advisories
        .iter()
        .filter(|advisory| advisory.package_id == package.id && !advisory.withdrawn)
        .collect::<Vec<_>>();
    if policy.reject_high_advisories
        && active_advisories
            .iter()
            .any(|advisory| advisory.severity >= AdvisorySeverity::High)
    {
        validation
            .errors
            .push("package has high or critical active security advisories".into());
    }
    if !active_advisories.is_empty() {
        validation.warnings.push(format!(
            "{} active security advisory/advisories apply",
            active_advisories.len()
        ));
    }
    validation
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionReload {
    pub previous: Option<ExtensionManagerSnapshot>,
    pub next: ExtensionManagerSnapshot,
    pub diffs: RegistryDiffBundle,
}

#[derive(Debug, Clone)]
pub struct ExtensionManager {
    config: ExtensionManagerConfig,
    current: Option<ExtensionManagerSnapshot>,
}

impl ExtensionManager {
    #[must_use]
    pub fn new(config: ExtensionManagerConfig) -> Self {
        Self {
            config,
            current: None,
        }
    }

    #[must_use]
    pub fn current(&self) -> Option<&ExtensionManagerSnapshot> {
        self.current.as_ref()
    }

    pub fn load(&mut self) -> ExtensionManagerSnapshot {
        let snapshot = load_snapshot(&self.config);
        self.current = Some(snapshot.clone());
        snapshot
    }

    pub fn reload(&mut self) -> ExtensionReload {
        let previous = self.current.clone();
        let next = load_snapshot(&self.config);
        let diffs = previous
            .as_ref()
            .map_or_else(RegistryDiffBundle::default, |previous| {
                previous.registries.diff(&next.registries)
            });
        self.current = Some(next.clone());
        ExtensionReload {
            previous,
            next,
            diffs,
        }
    }

    pub fn set_safe_mode(&mut self, safe_mode: bool) -> ExtensionReload {
        self.config.safe_mode = safe_mode;
        self.reload()
    }
}

fn load_snapshot(config: &ExtensionManagerConfig) -> ExtensionManagerSnapshot {
    let mut registries = config
        .builtins
        .clone()
        .map_or_else(ExtensionRegistries::new, ExtensionRegistries::with_builtins);
    let mut diagnostics = Vec::new();
    let mut files = Vec::new();
    config.discovery.discover_into(&mut files, &mut diagnostics);

    let mut packages = Vec::new();
    let mut extensions = Vec::new();
    for file in files {
        match file.file_kind {
            DiscoveredFileKind::PackageManifest => {
                packages.push(load_package_record(
                    &file,
                    &config.current_version,
                    &mut diagnostics,
                ));
            }
            DiscoveredFileKind::ExtensionManifest => {
                if let Some(record) = load_extension_record(
                    &file,
                    &config.current_version,
                    &config.denied_permissions,
                    &mut registries,
                    &mut diagnostics,
                ) {
                    extensions.push(record);
                }
            }
        }
    }

    let mut effective_policy = config.policy.clone();
    if config.safe_mode {
        effective_policy
            .disabled_entries
            .extend(registries.external_entry_keys());
        diagnostics.push(ExtensionDiagnostic {
            severity: DiagnosticSeverity::Info,
            phase: DiagnosticPhase::RegistryComposition,
            package_id: None,
            extension_id: None,
            contribution_id: None,
            source_path: None,
            message: "safe mode is enabled; all non-built-in extension contributions are disabled"
                .into(),
            remediation: Some("turn off safe mode after resolving extension failures".into()),
            health: HealthState::Disabled,
        });
        for extension in extensions
            .iter_mut()
            .filter(|extension| extension.source.scope != SourceScope::BuiltIn)
        {
            extension.lifecycle = LifecycleState::Disabled;
            extension.health = HealthState::Disabled;
        }
    }

    let registry_snapshots = registries.compose(&effective_policy);
    diagnostics.extend(registry_snapshots.diagnostics());
    diagnostics.sort_by_key(ExtensionDiagnostic::format_message);
    let diagnostic_groups = group_diagnostics(&diagnostics);
    assign_record_diagnostics(&mut extensions, &mut packages, &diagnostics);
    let contributions = contribution_records(&registry_snapshots, &diagnostics);

    ExtensionManagerSnapshot {
        safe_mode: config.safe_mode,
        registries: registry_snapshots,
        extensions,
        packages,
        contributions,
        diagnostics,
        diagnostic_groups,
    }
}

fn discover_root(
    root: &ExtensionDiscoveryRoot,
    files: &mut Vec<DiscoveredExtensionFile>,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) {
    if !root.path.exists() {
        return;
    }
    discover_dir(root, &root.path, files, diagnostics);
}

fn discover_dir(
    root: &ExtensionDiscoveryRoot,
    dir: &Path,
    files: &mut Vec<DiscoveredExtensionFile>,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) {
    let mut entries = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(err) => {
            diagnostics.push(io_diagnostic(dir, DiagnosticPhase::Discovery, err));
            return;
        }
    };
    entries.sort();
    for path in entries {
        if path.is_dir() {
            discover_dir(root, &path, files, diagnostics);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let file_kind = match name {
            MANIFEST_FILE => DiscoveredFileKind::ExtensionManifest,
            PACKAGE_MANIFEST_FILE => DiscoveredFileKind::PackageManifest,
            _ => continue,
        };
        let mut source = root.source.clone();
        source.path = Some(path.clone());
        files.push(DiscoveredExtensionFile {
            path,
            source,
            file_kind,
        });
    }
}

fn load_package_record(
    file: &DiscoveredExtensionFile,
    current_version: &Version,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) -> PackageRecord {
    match read_json::<PackageManifest>(&file.path) {
        Ok(manifest) => {
            let mut health = HealthState::Healthy;
            let mut lifecycle = LifecycleState::Validated;
            if let Err(err) = manifest.validate() {
                health = HealthState::Blocked;
                lifecycle = LifecycleState::Blocked;
                diagnostics.push(package_diagnostic(
                    &manifest,
                    &file.path,
                    DiagnosticPhase::ManifestParse,
                    DiagnosticSeverity::Error,
                    err.to_string(),
                    HealthState::Blocked,
                ));
            } else if !manifest.compatible_with(current_version) {
                health = HealthState::Degraded;
                lifecycle = LifecycleState::Blocked;
                diagnostics.push(package_diagnostic(
                    &manifest,
                    &file.path,
                    DiagnosticPhase::Compatibility,
                    DiagnosticSeverity::Warning,
                    format!(
                        "package requires Oino `{}` but current version is {current_version}",
                        manifest.oino
                    ),
                    HealthState::Degraded,
                ));
            }
            PackageRecord {
                id: manifest.id,
                display_name: if manifest.display_name.trim().is_empty() {
                    String::new()
                } else {
                    manifest.display_name
                },
                version: manifest.version,
                source: manifest.source.unwrap_or_else(|| file.source.clone()),
                lifecycle,
                health,
                diagnostics: Vec::new(),
            }
        }
        Err(err) => {
            diagnostics.push(path_diagnostic(
                &file.path,
                DiagnosticPhase::ManifestParse,
                DiagnosticSeverity::Error,
                format!("failed to parse package manifest: {err}"),
                HealthState::Blocked,
            ));
            PackageRecord {
                id: fallback_package_id(&file.path),
                display_name: String::new(),
                version: Version::new(0, 0, 0),
                source: file.source.clone(),
                lifecycle: LifecycleState::Blocked,
                health: HealthState::Blocked,
                diagnostics: Vec::new(),
            }
        }
    }
}

fn load_extension_record(
    file: &DiscoveredExtensionFile,
    current_version: &Version,
    denied_permissions: &BTreeMap<ExtensionId, String>,
    registries: &mut ExtensionRegistries,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) -> Option<ExtensionRecord> {
    let manifest = match read_json::<ExtensionManifest>(&file.path) {
        Ok(manifest) => manifest,
        Err(err) => {
            diagnostics.push(path_diagnostic(
                &file.path,
                DiagnosticPhase::ManifestParse,
                DiagnosticSeverity::Error,
                format!("failed to parse extension manifest: {err}"),
                HealthState::Blocked,
            ));
            return None;
        }
    };

    let source = manifest
        .source
        .clone()
        .unwrap_or_else(|| file.source.clone());
    let mut health = HealthState::Healthy;
    let mut lifecycle = LifecycleState::Validated;
    let mut can_register = true;

    if let Err(err) = manifest.validate() {
        can_register = false;
        health = HealthState::Blocked;
        lifecycle = LifecycleState::Blocked;
        diagnostics.push(extension_diagnostic(
            &manifest,
            &file.path,
            DiagnosticPhase::ManifestParse,
            DiagnosticSeverity::Error,
            err.to_string(),
            HealthState::Blocked,
        ));
    }
    if !manifest.compatible_with(current_version) {
        health = HealthState::Degraded;
        lifecycle = LifecycleState::Blocked;
        diagnostics.push(extension_diagnostic(
            &manifest,
            &file.path,
            DiagnosticPhase::Compatibility,
            DiagnosticSeverity::Warning,
            format!(
                "extension requires Oino `{}` but current version is {current_version}",
                manifest.oino
            ),
            HealthState::Degraded,
        ));
    }
    if let Some(reason) = denied_permissions.get(&manifest.id) {
        health = HealthState::Blocked;
        lifecycle = LifecycleState::Blocked;
        diagnostics.push(extension_diagnostic(
            &manifest,
            &file.path,
            DiagnosticPhase::Permission,
            DiagnosticSeverity::Error,
            reason.clone(),
            HealthState::Blocked,
        ));
    }

    if can_register {
        register_manifest_contributions(
            registries,
            &manifest,
            source.clone(),
            &file.path,
            current_version,
            denied_permissions.get(&manifest.id),
            diagnostics,
        );
    }

    let display_name = manifest.display_label().to_string();
    Some(ExtensionRecord {
        id: manifest.id,
        display_name,
        version: manifest.version,
        source,
        lifecycle,
        health,
        permissions: manifest.permissions,
        package_id: manifest.package_id,
        provenance: manifest.provenance,
        diagnostics: Vec::new(),
    })
}

fn register_manifest_contributions(
    registries: &mut ExtensionRegistries,
    manifest: &ExtensionManifest,
    source: SourceDescriptor,
    path: &Path,
    current_version: &Version,
    permission_denied: Option<&String>,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) {
    let metadata = manifest_metadata(manifest, source, current_version, permission_denied);
    let contributions = manifest.contributes.clone();
    register_all(registries, metadata, contributions, path, diagnostics);
}

fn register_all(
    registries: &mut ExtensionRegistries,
    metadata: ContributionMetadata,
    contributions: ExtensionContributions,
    path: &Path,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) {
    for contribution in contributions.tools {
        register_contribution(
            &mut registries.tools,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.commands {
        register_contribution(
            &mut registries.commands,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.keymaps {
        register_contribution(
            &mut registries.keymaps,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.hooks {
        register_contribution(
            &mut registries.hooks,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.ui_surfaces {
        register_contribution(
            &mut registries.ui_surfaces,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.settings_pages {
        register_contribution(
            &mut registries.settings_pages,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.themes {
        register_contribution(
            &mut registries.themes,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.providers {
        register_contribution(
            &mut registries.providers,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.resources {
        register_contribution(
            &mut registries.resources,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.persistence {
        register_contribution(
            &mut registries.persistence,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.autosuggest_providers {
        register_contribution(
            &mut registries.autosuggest_providers,
            metadata.clone(),
            contribution,
            path,
            diagnostics,
        );
    }
    for contribution in contributions.renderers {
        match contribution.target {
            oino_extension_core::RendererTarget::TranscriptMessage => register_contribution(
                &mut registries.transcript_renderers,
                metadata.clone(),
                contribution,
                path,
                diagnostics,
            ),
            oino_extension_core::RendererTarget::MarkdownBlock => register_contribution(
                &mut registries.message_renderers,
                metadata.clone(),
                contribution,
                path,
                diagnostics,
            ),
            oino_extension_core::RendererTarget::ToolCall
            | oino_extension_core::RendererTarget::ToolResult => register_contribution(
                &mut registries.tool_renderers,
                metadata.clone(),
                contribution,
                path,
                diagnostics,
            ),
        }
    }
}

fn register_contribution<T>(
    registry: &mut TypedContributionRegistry<T>,
    mut metadata: ContributionMetadata,
    contribution: T,
    path: &Path,
    diagnostics: &mut Vec<ExtensionDiagnostic>,
) where
    T: oino_extension_core::RegistryContribution,
{
    let id = contribution.contribution_id().clone();
    metadata.id = id.clone();
    let entry_key = registry_entry_key(&metadata, registry.family());
    if let Err(err) = registry.register_entry(entry_key, metadata.clone(), contribution) {
        diagnostics.push(ExtensionDiagnostic {
            severity: DiagnosticSeverity::Error,
            phase: DiagnosticPhase::RegistryComposition,
            package_id: metadata.package_id,
            extension_id: metadata.extension_id,
            contribution_id: Some(id),
            source_path: Some(path.to_path_buf()),
            message: err.to_string(),
            remediation: Some("fix the contribution schema or disable the extension".into()),
            health: HealthState::Blocked,
        });
    }
}

fn manifest_metadata(
    manifest: &ExtensionManifest,
    source: SourceDescriptor,
    current_version: &Version,
    permission_denied: Option<&String>,
) -> ContributionMetadata {
    let mut metadata = ContributionMetadata::new(fallback_contribution_id(), source)
        .with_extension_id(manifest.id.clone())
        .with_lifecycle(LifecycleState::Validated);
    if let Some(package_id) = &manifest.package_id {
        metadata.package_id = Some(package_id.clone());
    }
    metadata.provenance = manifest.provenance.clone();
    if !manifest.compatible_with(current_version) {
        metadata.compatibility = RegistryCompatibility::Incompatible(format!(
            "requires Oino `{}` but current version is {current_version}",
            manifest.oino
        ));
    }
    if let Some(reason) = permission_denied {
        metadata.permission = PermissionDecision::Denied(reason.clone());
    }
    metadata
}

fn registry_entry_key(metadata: &ContributionMetadata, family: RegistryFamily) -> RegistryEntryKey {
    let source_path = metadata
        .source
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| metadata.source.scope.slug().into());
    let owner = metadata
        .extension_id
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| metadata.package_id.as_ref().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".into());
    RegistryEntryKey::new(format!(
        "{}:{}:{}:{}",
        family.label(),
        owner,
        metadata.id,
        source_path
    ))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&text).map_err(|err| err.to_string())
}

fn collect_external_keys<T>(
    registry: &ContributionRegistry<T>,
    keys: &mut BTreeSet<RegistryEntryKey>,
) {
    for entry in registry.entries() {
        if entry.metadata.source.scope != SourceScope::BuiltIn {
            keys.insert(entry.key.clone());
        }
    }
}

fn contribution_records(
    snapshots: &RegistrySnapshotBundle,
    diagnostics: &[ExtensionDiagnostic],
) -> Vec<ContributionRecord> {
    let mut records = Vec::new();
    collect_active_records(
        RegistryFamily::Tool,
        &snapshots.tools.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Tool,
        &snapshots.tools.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Command,
        &snapshots.commands.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Command,
        &snapshots.commands.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Keymap,
        &snapshots.keymaps.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Keymap,
        &snapshots.keymaps.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Hook,
        &snapshots.hooks.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Hook,
        &snapshots.hooks.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::UiSurface,
        &snapshots.ui_surfaces.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::UiSurface,
        &snapshots.ui_surfaces.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::SettingsPage,
        &snapshots.settings_pages.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::SettingsPage,
        &snapshots.settings_pages.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Theme,
        &snapshots.themes.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Theme,
        &snapshots.themes.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::ProviderModel,
        &snapshots.providers.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::ProviderModel,
        &snapshots.providers.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Resource,
        &snapshots.resources.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Resource,
        &snapshots.resources.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Persistence,
        &snapshots.persistence.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Persistence,
        &snapshots.persistence.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Autosuggest,
        &snapshots.autosuggest_providers.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Autosuggest,
        &snapshots.autosuggest_providers.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::TranscriptRenderer,
        &snapshots.transcript_renderers.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::TranscriptRenderer,
        &snapshots.transcript_renderers.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::MessageRenderer,
        &snapshots.message_renderers.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::MessageRenderer,
        &snapshots.message_renderers.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::ToolRenderer,
        &snapshots.tool_renderers.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::ToolRenderer,
        &snapshots.tool_renderers.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Diagnostic,
        &snapshots.diagnostics.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Diagnostic,
        &snapshots.diagnostics.inactive,
        diagnostics,
        &mut records,
    );
    collect_active_records(
        RegistryFamily::Health,
        &snapshots.health.active,
        diagnostics,
        &mut records,
    );
    collect_inactive_records(
        RegistryFamily::Health,
        &snapshots.health.inactive,
        diagnostics,
        &mut records,
    );
    records.sort_by(|left, right| {
        left.family
            .label()
            .cmp(right.family.label())
            .then(left.id.cmp(&right.id))
            .then(left.entry_key.cmp(&right.entry_key))
    });
    records
}

fn collect_active_records<T>(
    family: RegistryFamily,
    active: &[ActiveContribution<T>],
    diagnostics: &[ExtensionDiagnostic],
    records: &mut Vec<ContributionRecord>,
) {
    for contribution in active {
        let metadata = &contribution.entry.metadata;
        records.push(ContributionRecord {
            family,
            id: contribution.effective_id.clone(),
            canonical_id: metadata.id.clone(),
            entry_key: contribution.entry.key.clone(),
            source: metadata.source.clone(),
            extension_id: metadata.extension_id.clone(),
            package_id: metadata.package_id.clone(),
            state: ContributionState::Active,
            lifecycle: LifecycleState::Active,
            health: metadata.health,
            permission: metadata.permission.clone(),
            compatibility: metadata.compatibility.clone(),
            provenance: metadata.provenance.clone(),
            diagnostics: diagnostics_for_contribution(diagnostics, metadata),
        });
    }
}

fn collect_inactive_records<T>(
    family: RegistryFamily,
    inactive: &[InactiveContribution<T>],
    diagnostics: &[ExtensionDiagnostic],
    records: &mut Vec<ContributionRecord>,
) {
    for contribution in inactive {
        let metadata = &contribution.entry.metadata;
        records.push(ContributionRecord {
            family,
            id: metadata.id.clone(),
            canonical_id: metadata.id.clone(),
            entry_key: contribution.entry.key.clone(),
            source: metadata.source.clone(),
            extension_id: metadata.extension_id.clone(),
            package_id: metadata.package_id.clone(),
            state: state_for_inactive(&contribution.reason),
            lifecycle: lifecycle_for_inactive(&contribution.reason),
            health: contribution.reason.health(),
            permission: metadata.permission.clone(),
            compatibility: metadata.compatibility.clone(),
            provenance: metadata.provenance.clone(),
            diagnostics: diagnostics_for_contribution(diagnostics, metadata),
        });
    }
}

fn diagnostics_for_contribution(
    diagnostics: &[ExtensionDiagnostic],
    metadata: &ContributionMetadata,
) -> Vec<ExtensionDiagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.contribution_id.as_ref() == Some(&metadata.id)
                || (diagnostic.contribution_id.is_none()
                    && diagnostic.extension_id == metadata.extension_id)
        })
        .cloned()
        .collect()
}

fn assign_record_diagnostics(
    extensions: &mut [ExtensionRecord],
    packages: &mut [PackageRecord],
    diagnostics: &[ExtensionDiagnostic],
) {
    for extension in extensions {
        extension.diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.extension_id.as_ref() == Some(&extension.id))
            .cloned()
            .collect();
    }
    for package in packages {
        package.diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.package_id.as_ref() == Some(&package.id))
            .cloned()
            .collect();
    }
}

fn group_diagnostics(diagnostics: &[ExtensionDiagnostic]) -> Vec<DiagnosticGroup> {
    let mut groups = BTreeMap::<DiagnosticGroupKey, Vec<ExtensionDiagnostic>>::new();
    for diagnostic in diagnostics {
        groups
            .entry(DiagnosticGroupKey {
                package_id: diagnostic.package_id.clone(),
                extension_id: diagnostic.extension_id.clone(),
                contribution_id: diagnostic.contribution_id.clone(),
            })
            .or_default()
            .push(diagnostic.clone());
    }
    groups
        .into_iter()
        .map(|(key, diagnostics)| {
            let health = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.health)
                .max_by_key(|health| health_rank(*health))
                .unwrap_or(HealthState::Healthy);
            DiagnosticGroup {
                key,
                health,
                diagnostics,
            }
        })
        .collect()
}

fn state_for_inactive(reason: &InactiveReason) -> ContributionState {
    match reason {
        InactiveReason::DisabledByPolicy(_) | InactiveReason::OverriddenByUser(_) => {
            ContributionState::Disabled
        }
        InactiveReason::PermissionPending(_) => ContributionState::PendingReview,
        InactiveReason::Incompatible(_)
        | InactiveReason::PermissionDenied(_)
        | InactiveReason::Unhealthy(_)
        | InactiveReason::ConflictError => ContributionState::Blocked,
        InactiveReason::ConflictShadowed => ContributionState::Shadowed,
        InactiveReason::Removed => ContributionState::Removed,
    }
}

fn lifecycle_for_inactive(reason: &InactiveReason) -> LifecycleState {
    match reason {
        InactiveReason::DisabledByPolicy(_) | InactiveReason::OverriddenByUser(_) => {
            LifecycleState::Disabled
        }
        InactiveReason::Removed => LifecycleState::Removed,
        InactiveReason::PermissionPending(_) | InactiveReason::ConflictShadowed => {
            LifecycleState::Validated
        }
        InactiveReason::Incompatible(_)
        | InactiveReason::PermissionDenied(_)
        | InactiveReason::Unhealthy(_)
        | InactiveReason::ConflictError => LifecycleState::Blocked,
    }
}

fn lifecycle_for_health(health: HealthState) -> LifecycleState {
    match health {
        HealthState::Healthy => LifecycleState::Active,
        HealthState::Degraded => LifecycleState::Unhealthy,
        HealthState::Unhealthy | HealthState::Blocked => LifecycleState::Blocked,
        HealthState::Disabled => LifecycleState::Disabled,
    }
}

fn contribution_state_for_health(health: HealthState) -> ContributionState {
    match health {
        HealthState::Healthy => ContributionState::Active,
        HealthState::Degraded => ContributionState::Active,
        HealthState::Unhealthy | HealthState::Blocked => ContributionState::Blocked,
        HealthState::Disabled => ContributionState::Disabled,
    }
}

fn health_rank(health: HealthState) -> u8 {
    match health {
        HealthState::Healthy => 0,
        HealthState::Disabled => 1,
        HealthState::Degraded => 2,
        HealthState::Unhealthy => 3,
        HealthState::Blocked => 4,
    }
}

fn diff_empty<T>(diff: &RegistryDiff<T>) -> bool {
    diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty()
}

fn fallback_contribution_id() -> ContributionId {
    match ContributionId::new("unknown") {
        Ok(id) => id,
        Err(err) => unreachable!("hardcoded fallback contribution id is valid: {err}"),
    }
}

fn fallback_package_id(path: &Path) -> PackageId {
    let stem = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(slug)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "invalid-package".into());
    PackageId::new(stem).unwrap_or_else(|_| match PackageId::new("invalid-package") {
        Ok(id) => id,
        Err(err) => unreachable!("hardcoded fallback package id is valid: {err}"),
    })
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut previous_separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            output.push(character);
            previous_separator = false;
        } else if !previous_separator && !output.is_empty() {
            output.push('-');
            previous_separator = true;
        }
    }
    while output.ends_with('-') {
        output.pop();
    }
    output
}

fn package_diagnostic(
    manifest: &PackageManifest,
    path: &Path,
    phase: DiagnosticPhase,
    severity: DiagnosticSeverity,
    message: String,
    health: HealthState,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity,
        phase,
        package_id: Some(manifest.id.clone()),
        extension_id: None,
        contribution_id: None,
        source_path: Some(path.to_path_buf()),
        message,
        remediation: Some("fix the package manifest or remove the package".into()),
        health,
    }
}

fn extension_diagnostic(
    manifest: &ExtensionManifest,
    path: &Path,
    phase: DiagnosticPhase,
    severity: DiagnosticSeverity,
    message: String,
    health: HealthState,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity,
        phase,
        package_id: manifest.package_id.clone(),
        extension_id: Some(manifest.id.clone()),
        contribution_id: None,
        source_path: Some(path.to_path_buf()),
        message,
        remediation: Some("fix the extension manifest or disable the extension".into()),
        health,
    }
}

fn path_diagnostic(
    path: &Path,
    phase: DiagnosticPhase,
    severity: DiagnosticSeverity,
    message: String,
    health: HealthState,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity,
        phase,
        package_id: None,
        extension_id: None,
        contribution_id: None,
        source_path: Some(path.to_path_buf()),
        message,
        remediation: Some("fix or remove the invalid extension file".into()),
        health,
    }
}

fn io_diagnostic(path: &Path, phase: DiagnosticPhase, err: io::Error) -> ExtensionDiagnostic {
    path_diagnostic(
        path,
        phase,
        DiagnosticSeverity::Warning,
        err.to_string(),
        HealthState::Degraded,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    fn write_json(path: &Path, json: &str) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
        Ok(())
    }

    fn manifest_json(id: &str, tool_id: &str) -> String {
        format!(
            r#"{{
              "id": "{id}",
              "version": "1.0.0",
              "oino": "^0.1",
              "runtime": {{ "kind": "wasm", "entry": "plugin.wasm" }},
              "permissions": {{ "tools": ["{tool_id}"] }},
              "contributes": {{
                "tools": [{{ "id": "{tool_id}", "description": "test tool" }}]
              }}
            }}"#
        )
    }

    #[test]
    fn discovery_order_is_deterministic_across_scopes_and_kinds() -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        write_json(
            &project.join(".oino/dev/extensions/dev/oino.extension.json"),
            &manifest_json("acme.dev", "dev_tool"),
        )?;
        write_json(
            &home.join(".oino/extensions/global/oino.extension.json"),
            &manifest_json("acme.global", "global_tool"),
        )?;
        write_json(
            &project.join(".oino/extensions/project/oino.extension.json"),
            &manifest_json("acme.project", "project_tool"),
        )?;
        write_json(
            &project.join(".oino/session-extensions/session/oino.extension.json"),
            &manifest_json("acme.session", "session_tool"),
        )?;
        write_json(
            &home.join(".oino/extension-registry/registry/oino.package.json"),
            r#"{
              "id": "acme.registry",
              "version": "1.0.0",
              "extensions": [{ "manifest": "extensions/acme/oino.extension.json" }]
            }"#,
        )?;

        let files =
            ExtensionDiscovery::from_home_and_project(&home, &project).discover_manifest_files();
        let scopes = files
            .iter()
            .map(|file| (file.source.scope, file.source.kind, file.file_kind))
            .collect::<Vec<_>>();
        assert_eq!(
            scopes,
            vec![
                (
                    SourceScope::Global,
                    SourceKind::LocalExtension,
                    DiscoveredFileKind::ExtensionManifest,
                ),
                (
                    SourceScope::Global,
                    SourceKind::RegistryPackage,
                    DiscoveredFileKind::PackageManifest,
                ),
                (
                    SourceScope::Project,
                    SourceKind::LocalExtension,
                    DiscoveredFileKind::ExtensionManifest,
                ),
                (
                    SourceScope::Session,
                    SourceKind::LocalExtension,
                    DiscoveredFileKind::ExtensionManifest,
                ),
                (
                    SourceScope::Development,
                    SourceKind::LocalExtension,
                    DiscoveredFileKind::ExtensionManifest,
                ),
            ]
        );
        Ok(())
    }

    #[test]
    fn layout_paths_are_oino_owned_and_ignore_implicit_foreign_files() -> Result<(), Box<dyn Error>>
    {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let layout = ExtensionLayoutPaths::for_home_and_project(&home, &project);
        assert!(layout
            .global_installed_packages
            .ends_with(".oino/extension-packages"));
        assert!(layout
            .project_extension_state
            .ends_with(".oino/extension-state"));
        assert!(layout.package_assets.ends_with(".oino/extension-assets"));

        fs::create_dir_all(&project)?;
        fs::write(project.join("CLAUDE.md"), "not an Oino extension")?;
        fs::write(project.join("AGENTS.md"), "not an Oino extension")?;
        write_json(
            &layout
                .project_installed_packages
                .join("pkg/oino.package.json"),
            r#"{
              "id": "acme.project-package",
              "version": "1.0.0",
              "extensions": [{ "manifest": "extensions/acme/oino.extension.json" }],
              "examples": [{ "path": "examples/basic" }],
              "docs": [{ "path": "docs/README.md" }]
            }"#,
        )?;
        let files = ExtensionDiscovery::from_layout(&layout).discover_manifest_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_kind, DiscoveredFileKind::PackageManifest);
        assert_eq!(files[0].source.scope, SourceScope::Project);
        assert_eq!(files[0].source.kind, SourceKind::LocalPackage);
        Ok(())
    }

    #[test]
    fn persistence_store_enforces_permissions_migrates_and_cleans_up() -> Result<(), Box<dyn Error>>
    {
        let temp = tempfile::tempdir()?;
        let store = ExtensionPersistenceStore::new(temp.path().join(".oino/extension-state"));
        let extension_id = ExtensionId::new("acme.persist")?;
        let mut permissions = ExtensionPermissions::default();
        permissions
            .session_persistence
            .insert(PersistenceScope::Project);
        let record = PersistenceRecord {
            owner_extension_id: extension_id.clone(),
            scope: PersistenceScope::Project,
            key: "processes".into(),
            schema_version: 1,
            schema: Some("object".into()),
            payload: serde_json::json!({ "running": ["cargo test"] }),
            provenance: None,
            updated_at_unix_ms: 1,
            tombstoned: false,
        };
        store.write(&record, &permissions, Some(4096))?;
        let loaded = store.read(
            &extension_id,
            PersistenceScope::Project,
            "processes",
            &permissions,
        )?;
        assert_eq!(loaded.payload["running"][0], "cargo test");

        let migrated = store.migrate(
            &extension_id,
            PersistenceScope::Project,
            "processes",
            2,
            &PersistenceMigrationPolicy::HostCopyForward,
            &permissions,
        )?;
        assert_eq!(migrated.schema_version, 2);

        let mut denied = ExtensionPermissions::default();
        denied.session_persistence.insert(PersistenceScope::Session);
        assert!(matches!(
            store.read(
                &extension_id,
                PersistenceScope::Project,
                "processes",
                &denied
            ),
            Err(PersistenceStoreError::PermissionDenied { .. })
        ));

        store.cleanup_extension(&extension_id, PersistenceCleanupPolicy::RetainWithTombstone)?;
        let tombstoned = store.read(
            &extension_id,
            PersistenceScope::Project,
            "processes",
            &permissions,
        )?;
        assert!(tombstoned.tombstoned);
        store.cleanup_extension(&extension_id, PersistenceCleanupPolicy::DeleteOnUninstall)?;
        assert!(matches!(
            store.read(
                &extension_id,
                PersistenceScope::Project,
                "processes",
                &permissions
            ),
            Err(PersistenceStoreError::NotFound)
        ));
        Ok(())
    }

    #[test]
    fn persistence_store_reports_corrupted_state() -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let store = ExtensionPersistenceStore::new(temp.path().join("state"));
        let extension_id = ExtensionId::new("acme.persist")?;
        let mut permissions = ExtensionPermissions::default();
        permissions
            .session_persistence
            .insert(PersistenceScope::Project);
        let path = store
            .root()
            .join("project")
            .join(extension_id.as_str())
            .join("bad.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, "not-json")?;
        assert!(matches!(
            store.read(
                &extension_id,
                PersistenceScope::Project,
                "bad",
                &permissions
            ),
            Err(PersistenceStoreError::Corrupted(_))
        ));
        Ok(())
    }

    #[test]
    fn manager_loads_manifests_collects_errors_and_composes_snapshots() -> Result<(), Box<dyn Error>>
    {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        write_json(
            &project.join(".oino/extensions/good/oino.extension.json"),
            &manifest_json("acme.good", "good_tool"),
        )?;
        write_json(
            &project.join(".oino/extensions/missing-runtime/oino.extension.json"),
            r#"{
              "id": "acme.bad_runtime",
              "version": "1.0.0",
              "runtime": { "kind": "wasm" }
            }"#,
        )?;
        write_json(
            &project.join(".oino/extensions/future/oino.extension.json"),
            r#"{
              "id": "acme.future",
              "version": "1.0.0",
              "oino": ">=9.0.0",
              "runtime": { "kind": "wasm", "entry": "plugin.wasm" },
              "permissions": { "tools": ["future_tool"] },
              "contributes": { "tools": [{ "id": "future_tool", "description": "future" }] }
            }"#,
        )?;
        write_json(
            &project.join(".oino/extensions/denied/oino.extension.json"),
            &manifest_json("acme.denied", "denied_tool"),
        )?;
        let denied_id = ExtensionId::new("acme.denied")?;
        let config = ExtensionManagerConfig::new(
            Version::parse("0.1.0")?,
            ExtensionDiscovery::from_home_and_project(&home, &project),
        )
        .deny_permissions(denied_id.clone(), "permission denied by test policy");
        let mut manager = ExtensionManager::new(config);
        let snapshot = manager.load();

        assert_eq!(snapshot.extensions.len(), 4);
        assert_eq!(snapshot.registries.tools.active.len(), 0);
        assert_eq!(snapshot.registries.tools.inactive.len(), 3);
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::ManifestParse
                && diagnostic.message.contains("runtime entry")
        }));
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Compatibility
                && diagnostic
                    .extension_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == "acme.future")
        }));
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::Permission
                && diagnostic.extension_id.as_ref() == Some(&denied_id)
        }));
        assert!(snapshot.contributions.iter().any(|record| {
            record.id.as_str() == "good_tool" && record.state == ContributionState::PendingReview
        }));
        Ok(())
    }

    #[test]
    fn safe_mode_disables_external_contributions_and_reload_diffs_changes(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let manifest_path = project.join(".oino/extensions/good/oino.extension.json");
        write_json(&manifest_path, &manifest_json("acme.good", "good_tool"))?;

        let mut policy = RegistryPolicy::safe_defaults();
        policy
            .enabled_extensions
            .insert(ExtensionId::new("acme.good")?);
        let config = ExtensionManagerConfig::new(
            Version::parse("0.1.0")?,
            ExtensionDiscovery::from_home_and_project(&home, &project),
        )
        .with_policy(policy);
        let mut manager = ExtensionManager::new(config);
        let first = manager.load();
        assert_eq!(first.registries.tools.active.len(), 1);

        let safe_reload = manager.set_safe_mode(true);
        assert!(safe_reload.next.safe_mode);
        assert_eq!(safe_reload.next.registries.tools.active.len(), 0);
        assert_eq!(safe_reload.next.registries.tools.inactive.len(), 1);
        assert!(!safe_reload.diffs.is_empty());

        fs::remove_file(&manifest_path)?;
        let reload = manager.set_safe_mode(false);
        assert_eq!(reload.next.registries.tools.active.len(), 0);
        assert_eq!(reload.diffs.tools.removed.len(), 0);
        assert_eq!(reload.diffs.tools.added.len(), 0);
        assert!(reload.previous.is_some());
        Ok(())
    }

    #[test]
    fn management_snapshot_groups_diagnostics_and_applies_health_events(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        write_json(
            &project.join(".oino/extensions/good/oino.extension.json"),
            &manifest_json("acme.good", "good_tool"),
        )?;
        let mut policy = RegistryPolicy::safe_defaults();
        policy
            .enabled_extensions
            .insert(ExtensionId::new("acme.good")?);
        let config = ExtensionManagerConfig::new(
            Version::parse("0.1.0")?,
            ExtensionDiscovery::from_home_and_project(&home, &project),
        )
        .with_policy(policy);
        let mut snapshot = ExtensionManager::new(config).load();
        let extension_id = ExtensionId::new("acme.good")?;
        let contribution_id = ContributionId::new("good_tool")?;
        assert_eq!(
            snapshot
                .extension(&extension_id)
                .map(|record| record.health),
            Some(HealthState::Healthy)
        );

        snapshot.apply_health_event(ExtensionHealthEvent::permission_denied(
            extension_id.clone(),
            contribution_id.clone(),
            "shell/process permission denied",
        ));
        assert_eq!(
            snapshot
                .extension(&extension_id)
                .map(|record| record.health),
            Some(HealthState::Blocked)
        );
        assert!(snapshot.contributions.iter().any(|record| {
            record.id == contribution_id && record.state == ContributionState::Blocked
        }));
        assert!(snapshot.diagnostic_groups.iter().any(|group| {
            group.key.extension_id.as_ref() == Some(&extension_id)
                && group.key.contribution_id.as_ref() == Some(&contribution_id)
                && group.health == HealthState::Blocked
        }));

        snapshot.apply_health_event(ExtensionHealthEvent::runtime_crash(
            extension_id.clone(),
            "runtime crashed",
        ));
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::RuntimeExecute
                && diagnostic.extension_id.as_ref() == Some(&extension_id)
        }));
        snapshot.apply_health_event(ExtensionHealthEvent::hook_timeout(
            extension_id.clone(),
            contribution_id.clone(),
        ));
        snapshot.apply_health_event(ExtensionHealthEvent::invalid_ui_update(
            extension_id,
            contribution_id,
        ));
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.phase == DiagnosticPhase::UiUpdate
                && diagnostic.message.contains("invalid UI update")
        }));
        Ok(())
    }

    fn write_package_fixture(
        root: &Path,
        package_id: &str,
        version: &str,
        tool_id: &str,
        dependency: Option<&str>,
        checksum: bool,
    ) -> Result<PathBuf, Box<dyn Error>> {
        let package_dir = root.join(package_id.replace('.', "-"));
        let extension_id = format!("{package_id}.extension");
        let extension_manifest = format!(
            r#"{{
              "id": "{extension_id}",
              "package_id": "{package_id}",
              "version": "{version}",
              "oino": "^0.1",
              "runtime": {{ "kind": "wasm", "entry": "plugin.wasm" }},
              "permissions": {{ "tools": ["{tool_id}"] }},
              "contributes": {{
                "tools": [{{ "id": "{tool_id}", "description": "lifecycle tool" }}]
              }}
            }}"#,
        );
        write_json(
            &package_dir.join("extensions/main/oino.extension.json"),
            &extension_manifest,
        )?;
        fs::write(package_dir.join("plugin.wasm"), b"wasm fixture")?;
        let dependency_json = dependency
            .map(|id| format!(r#", "dependencies": [{{ "id": "{id}", "version": "^1" }}]"#))
            .unwrap_or_default();
        let package_manifest = format!(
            r#"{{
              "id": "{package_id}",
              "display_name": "{package_id}",
              "version": "{version}",
              "oino": "^0.1",
              "publisher": "acme",
              "description": "Package fixture",
              "extensions": [{{ "manifest": "extensions/main/oino.extension.json" }}],
              "permissions": {{ "tools": ["{tool_id}"] }},
              "trust": {{ "reviewed": true }}{dependency_json}
            }}"#,
        );
        write_json(&package_dir.join(PACKAGE_MANIFEST_FILE), &package_manifest)?;
        if checksum {
            let checksum = package_directory_checksum(&package_dir)?;
            let mut value = serde_json::from_str::<serde_json::Value>(&package_manifest)?;
            value["trust"]["checksum"] = serde_json::Value::String(checksum);
            write_json(
                &package_dir.join(PACKAGE_MANIFEST_FILE),
                &serde_json::to_string_pretty(&value)?,
            )?;
        }
        Ok(package_dir)
    }

    fn package_metadata(
        id: &str,
        version: &str,
        category: &str,
        package_path: Option<&Path>,
    ) -> Result<CommunityPackageMetadata, Box<dyn Error>> {
        let mut value = serde_json::json!({
            "id": id,
            "version": version,
            "publisher": "acme",
            "display_name": id,
            "description": format!("{id} registry package"),
            "categories": [category],
            "license": "MIT",
            "source_link": "https://example.com/acme",
            "oino": "^0.1",
            "assets": [{ "path": "plugin.wasm", "size_bytes": 12 }],
            "permissions": { "tools": ["registry_tool"] },
            "trust": { "reviewed": true, "checksum": "fixture-checksum", "signature": "sig:fixture" },
            "update_policy": "compatible",
            "changelog": [{ "version": version, "notes": "Initial release" }]
        });
        if let Some(path) = package_path {
            value["package_path"] = serde_json::Value::String(path.display().to_string());
        }
        Ok(serde_json::from_value(value)?)
    }

    #[test]
    fn package_lifecycle_installs_updates_removes_and_reloads() -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let layout = ExtensionLayoutPaths::for_home_and_project(&home, &project);
        let package_v1 = write_package_fixture(
            &temp.path().join("sources/v1"),
            "acme.lifecycle",
            "1.0.0",
            "lifecycle_tool",
            None,
            true,
        )?;
        let package_v2 = write_package_fixture(
            &temp.path().join("sources/v2"),
            "acme.lifecycle",
            "1.1.0",
            "lifecycle_tool",
            None,
            true,
        )?;

        let mut policy = RegistryPolicy::safe_defaults();
        policy
            .enabled_extensions
            .insert(ExtensionId::new("acme.lifecycle.extension")?);
        let config = ExtensionManagerConfig::new(
            Version::parse("0.1.0")?,
            ExtensionDiscovery::from_layout(&layout),
        )
        .with_policy(policy);
        let mut manager = ExtensionManager::new(config);
        manager.load();
        let service = PackageLifecycleService::new(layout.clone(), Version::parse("0.1.0")?);

        let install =
            service.install_local(&package_v1, PackageInstallScope::Project, &mut manager)?;
        assert_eq!(install.operation, PackageLifecycleOperation::Install);
        assert_eq!(install.version, Version::parse("1.0.0")?);
        assert!(install
            .permission_prompt
            .permissions
            .tools
            .contains("lifecycle_tool"));
        assert!(install
            .reload
            .next
            .packages
            .iter()
            .any(|package| package.id.as_str() == "acme.lifecycle"));
        assert_eq!(install.reload.diffs.tools.added.len(), 1);

        let update =
            service.update_local(&package_v2, PackageInstallScope::Project, &mut manager)?;
        assert_eq!(update.operation, PackageLifecycleOperation::Update);
        assert_eq!(update.version, Version::parse("1.1.0")?);

        let remove = service.remove(
            PackageId::new("acme.lifecycle")?,
            PackageInstallScope::Project,
            &mut manager,
        )?;
        assert_eq!(remove.operation, PackageLifecycleOperation::Remove);
        assert!(!remove.destination.exists());
        assert!(!remove
            .reload
            .next
            .packages
            .iter()
            .any(|package| package.id.as_str() == "acme.lifecycle"));
        Ok(())
    }

    #[test]
    fn package_lifecycle_blocks_dependencies_and_preserves_existing_on_failure(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let layout = ExtensionLayoutPaths::for_home_and_project(
            temp.path().join("home"),
            temp.path().join("project"),
        );
        let mut manager = ExtensionManager::new(ExtensionManagerConfig::new(
            Version::parse("0.1.0")?,
            ExtensionDiscovery::from_layout(&layout),
        ));
        manager.load();
        let service = PackageLifecycleService::new(layout.clone(), Version::parse("0.1.0")?);
        let dependent = write_package_fixture(
            &temp.path().join("sources/dependent"),
            "acme.dependent",
            "1.0.0",
            "dependent_tool",
            Some("acme.missing"),
            false,
        )?;
        assert!(matches!(
            service.install_local(&dependent, PackageInstallScope::Project, &mut manager),
            Err(PackageLifecycleError::DependencyConflict { .. })
        ));

        let package_v1 = write_package_fixture(
            &temp.path().join("sources/original"),
            "acme.rollback",
            "1.0.0",
            "rollback_tool",
            None,
            true,
        )?;
        service.install_local(&package_v1, PackageInstallScope::Project, &mut manager)?;
        let bad_update = write_package_fixture(
            &temp.path().join("sources/bad-update"),
            "acme.rollback",
            "2.0.0",
            "rollback_tool",
            None,
            false,
        )?;
        let mut value = serde_json::from_str::<serde_json::Value>(&fs::read_to_string(
            bad_update.join(PACKAGE_MANIFEST_FILE),
        )?)?;
        value["trust"]["checksum"] = serde_json::Value::String("oino-fnv64:bad".into());
        write_json(
            &bad_update.join(PACKAGE_MANIFEST_FILE),
            &serde_json::to_string_pretty(&value)?,
        )?;
        assert!(matches!(
            service.update_local(&bad_update, PackageInstallScope::Project, &mut manager),
            Err(PackageLifecycleError::ChecksumMismatch { .. })
        ));
        let installed: PackageManifest = read_json(
            &layout
                .project_installed_packages
                .join("acme.rollback")
                .join(PACKAGE_MANIFEST_FILE),
        )?;
        assert_eq!(installed.version, Version::parse("1.0.0")?);
        Ok(())
    }

    #[test]
    fn fixture_registry_searches_validates_and_installs_with_diff_output(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let layout = ExtensionLayoutPaths::for_home_and_project(
            temp.path().join("home"),
            temp.path().join("project"),
        );
        let package_dir = write_package_fixture(
            &temp.path().join("sources/registry"),
            "acme.registry",
            "1.0.0",
            "registry_tool",
            None,
            true,
        )?;
        let registry = FixtureRegistryClient::new(CommunityRegistryIndex {
            schema_version: 1,
            packages: vec![
                package_metadata("acme.registry", "1.0.0", "tools", Some(&package_dir))?,
                serde_json::from_value(serde_json::json!({
                    "id": "acme.future",
                    "version": "1.0.0",
                    "publisher": "acme",
                    "description": "future only",
                    "categories": ["tools"],
                    "oino": ">=9.0.0",
                    "permissions": {},
                    "trust": { "reviewed": true, "checksum": "future" }
                }))?,
            ],
            advisories: Vec::new(),
        });
        let current = Version::parse("0.1.0")?;
        let search = registry.search("registry", Some("tools"), &current);
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].id.as_str(), "acme.registry");

        let mut policy = RegistryPolicy::safe_defaults();
        policy
            .enabled_extensions
            .insert(ExtensionId::new("acme.registry.extension")?);
        let mut manager = ExtensionManager::new(
            ExtensionManagerConfig::new(current.clone(), ExtensionDiscovery::from_layout(&layout))
                .with_policy(policy),
        );
        manager.load();
        let service = PackageLifecycleService::new(layout, current);
        let report = service.install_from_registry(
            &registry,
            &PackageId::new("acme.registry")?,
            PackageInstallScope::Project,
            &mut manager,
        )?;
        assert_eq!(report.operation, PackageLifecycleOperation::Install);
        assert_eq!(report.reload.diffs.tools.added.len(), 1);
        Ok(())
    }

    #[test]
    fn registry_metadata_validation_flags_deprecation_advisories_and_trust(
    ) -> Result<(), Box<dyn Error>> {
        let current = Version::parse("0.1.0")?;
        let package = package_metadata("acme.secure", "1.0.0", "security", None)?;
        let clean = validate_registry_package_metadata(
            &package,
            &current,
            &[],
            &RegistryTrustPolicy::default(),
        );
        assert!(clean.is_ok(), "unexpected errors: {:?}", clean.errors);

        let mut deprecated = package.clone();
        deprecated.deprecated = true;
        deprecated.deprecation_message = Some("superseded by acme.secure2".into());
        let deprecated_result = validate_registry_package_metadata(
            &deprecated,
            &current,
            &[],
            &RegistryTrustPolicy::default(),
        );
        assert!(deprecated_result
            .errors
            .iter()
            .any(|error| error.contains("deprecated")));

        let advisory = SecurityAdvisory {
            id: "OINO-2026-0001".into(),
            package_id: PackageId::new("acme.secure")?,
            affected: None,
            severity: AdvisorySeverity::High,
            title: "unsafe default".into(),
            description: "test advisory".into(),
            patched_versions: Vec::new(),
            withdrawn: false,
        };
        let advisory_result = validate_registry_package_metadata(
            &package,
            &current,
            &[advisory],
            &RegistryTrustPolicy::default(),
        );
        assert!(advisory_result
            .errors
            .iter()
            .any(|error| error.contains("advisories")));

        let mut unsigned = package;
        unsigned.trust.signature = None;
        let signature_required = RegistryTrustPolicy {
            require_signature: true,
            ..RegistryTrustPolicy::default()
        };
        let unsigned_result =
            validate_registry_package_metadata(&unsigned, &current, &[], &signature_required);
        assert!(unsigned_result
            .errors
            .iter()
            .any(|error| error.contains("signature")));
        Ok(())
    }
}
