#![doc = r#"Author-facing SDK, devkit validators, examples, and test harness helpers
for Oino extension authors.

The crate intentionally reuses the same contracts consumed by Oino core:
`oino-extension-core` for manifests/contributions, `oino-extension-runtime`
for the JSON-v1 WASM boundary, and `oino-extension-manager` for package
validation and persistence helpers. The goal is to keep author tooling from
inventing a second schema.
"#]
#![forbid(unsafe_code)]

use oino_extension_core::{
    AutosuggestContribution, AutosuggestItem, ChangelogEntry, CommandContribution,
    CommunityPackageMetadata, ContributionId, ContributionMetadata, ExtensionContributions,
    ExtensionCoreError, ExtensionId, ExtensionManifest, ExtensionPermissions, HookContribution,
    HookEventKind, HookMode, KeymapContribution, OinoCompatibility, PackageAssetRef,
    PackageExtensionRef, PackageId, PackageManifest, PersistenceContribution, PersistenceRecord,
    PersistenceScope, ProviderContribution, ProviderPrivacyPolicy, RegistryEntryKey,
    RegistryFamily, RegistryPolicy, RuntimeDescriptor, RuntimeKind, SourceDescriptor, SourceKind,
    SourceScope, ThemeContribution, ToolContribution, ToolExecutionMode, TrustMetadata,
    UiFocusPolicy, UiKeyDispatchPolicy, UiLayoutPolicy, UiSurfaceAction, UiSurfaceContribution,
    UiSurfaceKind, UiSurfaceRegistry, UiSurfaceStateUpdate, UiSurfaceValidationError,
    MANIFEST_FILE, PACKAGE_MANIFEST_FILE,
};
use oino_extension_manager::{ExtensionPersistenceStore, PackageLifecycleService};
use oino_extension_runtime::{
    CapabilityBroker, CapabilityError, CapabilityRequest, CapabilityResponse, ExtensionRuntime,
    FixtureHandlerBehavior, FixtureWasmModule, JsonWasmRuntime, RuntimeCapabilityPolicy,
    RuntimeError, RuntimeInitialize, RuntimeInvocation, RuntimeProgress, RuntimeResultValue,
    WASM_JSON_V1_ABI,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthoringError {
    #[error(transparent)]
    Core(#[from] ExtensionCoreError),
    #[error(transparent)]
    Semver(#[from] semver::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Ui(#[from] UiSurfaceValidationError),
    #[error("validation failed: {0}")]
    Validation(String),
}

pub type AuthoringResult<T> = Result<T, AuthoringError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExampleExtensionTemplate {
    pub package_id: PackageId,
    pub extension_id: ExtensionId,
    pub version: Version,
}

impl ExampleExtensionTemplate {
    pub fn new(
        package_id: impl AsRef<str>,
        extension_id: impl AsRef<str>,
        version: impl AsRef<str>,
    ) -> AuthoringResult<Self> {
        Ok(Self {
            package_id: PackageId::new(package_id.as_ref())?,
            extension_id: ExtensionId::new(extension_id.as_ref())?,
            version: Version::parse(version.as_ref())?,
        })
    }

    pub fn complete_example() -> AuthoringResult<Self> {
        Self::new("acme.example_extension", "acme.example_extension", "1.0.0")
    }

    pub fn extension_manifest(&self) -> AuthoringResult<ExtensionManifest> {
        let tool_id = ContributionId::new("example_tool")?;
        let command_id = ContributionId::new("example_command")?;
        let sidebar_id = ContributionId::new("example_sidebar")?;
        let floating_id = ContributionId::new("example_floating_panel")?;
        let footer_id = ContributionId::new("example_footer")?;
        let theme_id = ContributionId::new("example_theme")?;
        let autosuggest_id = ContributionId::new("example_autosuggest")?;
        let provider_id = ContributionId::new("example_provider")?;
        let hook_id = ContributionId::new("example_tool_hook")?;
        let persistence_id = ContributionId::new("example_persistence")?;
        let keymap_id = ContributionId::new("example_keymap")?;

        let mut permissions = ExtensionPermissions::default();
        permissions.tools.insert(tool_id.to_string());
        permissions.commands.insert(command_id.to_string());
        permissions
            .host_capabilities
            .insert("host.test.echo".into());
        permissions
            .host_capabilities
            .insert("host.persistence.read".into());
        permissions
            .host_capabilities
            .insert("host.persistence.write".into());
        permissions
            .host_capabilities
            .insert("host.persistence.delete".into());
        permissions
            .ui
            .insert(UiSurfaceKind::Sidebar.as_permission_name().into());
        permissions
            .ui
            .insert(UiSurfaceKind::FloatingPanel.as_permission_name().into());
        permissions
            .ui
            .insert(UiSurfaceKind::Footer.as_permission_name().into());
        permissions
            .session_persistence
            .insert(PersistenceScope::Project);

        let mut theme_tokens = BTreeMap::new();
        theme_tokens.insert("accent".into(), "#7dd3fc".into());
        theme_tokens.insert("success".into(), "#86efac".into());

        let manifest = ExtensionManifest {
            id: self.extension_id.clone(),
            package_id: Some(self.package_id.clone()),
            display_name: "Oino Rust WASM example".into(),
            version: self.version.clone(),
            oino: OinoCompatibility::parse("^0.1")?,
            protocol: oino_extension_core::CURRENT_PROTOCOL_VERSION,
            runtime: RuntimeDescriptor {
                kind: RuntimeKind::Wasm,
                entry: Some("plugin.wasm".into()),
                abi: Some(WASM_JSON_V1_ABI.into()),
            },
            permissions,
            contributes: ExtensionContributions {
                tools: vec![ToolContribution {
                    id: tool_id,
                    description: "Echo a message through the extension runtime".into(),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": { "message": { "type": "string" } },
                        "required": ["message"]
                    }),
                    execution_mode: ToolExecutionMode::Parallel,
                    handler: Some("handle_tool".into()),
                    conflict: Default::default(),
                }],
                commands: vec![CommandContribution {
                    id: command_id,
                    description: "Run the example extension command".into(),
                    handler: Some("handle_command".into()),
                    conflict: Default::default(),
                }],
                keymaps: vec![KeymapContribution {
                    id: keymap_id,
                    action: "extension.example.open".into(),
                    context: "global".into(),
                    default_bindings: vec!["ctrl-o x".into()],
                    conflict: Default::default(),
                }],
                hooks: vec![HookContribution {
                    id: hook_id,
                    event: HookEventKind::ToolCall,
                    priority: 10,
                    mode: HookMode::Mutable,
                    handler: Some("handle_tool_hook".into()),
                    conflict: Default::default(),
                }],
                ui_surfaces: vec![
                    ui_surface(
                        sidebar_id,
                        UiSurfaceKind::Sidebar,
                        "Example Sidebar",
                        "object",
                    ),
                    ui_surface(
                        floating_id,
                        UiSurfaceKind::FloatingPanel,
                        "Example Floating Panel",
                        "object",
                    ),
                    ui_surface(footer_id, UiSurfaceKind::Footer, "Example Footer", "string"),
                ],
                settings_pages: Vec::new(),
                themes: vec![ThemeContribution {
                    id: theme_id,
                    path: "themes/example-theme.json".into(),
                    tokens: theme_tokens,
                    conflict: Default::default(),
                }],
                providers: vec![ProviderContribution {
                    id: provider_id,
                    provider_id: "example-provider".into(),
                    display_name: "Example Provider Metadata".into(),
                    model_ids: vec!["example-provider/example-model".into()],
                    privacy: ProviderPrivacyPolicy::default(),
                    hook: None,
                    conflict: Default::default(),
                }],
                resources: Vec::new(),
                persistence: vec![PersistenceContribution {
                    id: persistence_id,
                    scope: PersistenceScope::Project,
                    key: "example_state".into(),
                    schema_version: 1,
                    schema: Some("object".into()),
                    max_bytes: 4096,
                    migration: Default::default(),
                    cleanup: Default::default(),
                    conflict: Default::default(),
                }],
                autosuggest_providers: vec![AutosuggestContribution {
                    id: autosuggest_id,
                    trigger: "@example".into(),
                    label: "Example completions".into(),
                    items: vec![AutosuggestItem {
                        label: "example greeting".into(),
                        replacement: "Hello from an Oino extension".into(),
                        detail: "static SDK example suggestion".into(),
                    }],
                    conflict: Default::default(),
                }],
                renderers: Vec::new(),
            },
            source: None,
            provenance: None,
            metadata: serde_json::json!({
                "sdk": "oino-extension-sdk",
                "languages": ["rust", "typescript-plan", "go-plan", "python-plan"]
            }),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn package_manifest(&self) -> AuthoringResult<PackageManifest> {
        let manifest = PackageManifest {
            id: self.package_id.clone(),
            display_name: "Oino Rust WASM example package".into(),
            version: self.version.clone(),
            oino: OinoCompatibility::parse("^0.1")?,
            publisher: Some("acme".into()),
            description: Some(
                "Fixture package covering tools, commands, keymaps, UI, themes, providers, hooks, and persistence".into(),
            ),
            source: None,
            extensions: vec![PackageExtensionRef {
                manifest: format!("extensions/example/{MANIFEST_FILE}"),
                enabled_by_default: true,
            }],
            resources: Vec::new(),
            assets: vec![PackageAssetRef {
                path: "plugin.wasm".into(),
                checksum: None,
            }],
            examples: vec![PackageAssetRef {
                path: "examples/basic.rs".into(),
                checksum: None,
            }],
            docs: vec![PackageAssetRef {
                path: "docs/README.md".into(),
                checksum: None,
            }],
            dependencies: Vec::new(),
            permissions: self.extension_manifest()?.permissions,
            trust: TrustMetadata {
                reviewed: true,
                publisher: Some("acme".into()),
                checksum: None,
                signature: None,
                advisories: Vec::new(),
            },
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn registry_metadata(
        &self,
        package_path: Option<PathBuf>,
    ) -> AuthoringResult<CommunityPackageMetadata> {
        Ok(CommunityPackageMetadata {
            id: self.package_id.clone(),
            version: self.version.clone(),
            publisher: "acme".into(),
            display_name: "Oino Rust WASM example package".into(),
            description: "Local fixture registry metadata for author testing".into(),
            categories: vec!["examples".into(), "tools".into(), "ui".into()],
            license: Some("MIT OR Apache-2.0".into()),
            source_link: Some("https://example.invalid/oino/example-extension".into()),
            package_path,
            assets: Vec::new(),
            oino: OinoCompatibility::parse("^0.1")?,
            dependencies: Vec::new(),
            permissions: self.extension_manifest()?.permissions,
            trust: TrustMetadata {
                reviewed: true,
                publisher: Some("acme".into()),
                checksum: Some("fixture-checksum".into()),
                signature: None,
                advisories: Vec::new(),
            },
            update_policy: Default::default(),
            changelog: vec![ChangelogEntry {
                version: self.version.clone(),
                notes: "Initial SDK example".into(),
            }],
            deprecated: false,
            deprecation_message: None,
            advisories: Vec::new(),
        })
    }
}

fn ui_surface(
    id: ContributionId,
    surface: UiSurfaceKind,
    title: impl Into<String>,
    schema: impl Into<String>,
) -> UiSurfaceContribution {
    let mut scopes = BTreeSet::new();
    scopes.insert("example".into());
    UiSurfaceContribution {
        id,
        surface,
        title: title.into(),
        state_schema: Some(schema.into()),
        layout: UiLayoutPolicy {
            slot: surface.default_slot().into(),
            priority: 0,
            min_width: 20,
            min_height: 3,
            max_width: None,
            tiny_terminal: Default::default(),
        },
        visibility: Default::default(),
        focus: UiFocusPolicy::Focusable,
        key_dispatch: UiKeyDispatchPolicy {
            scopes,
            pass_through: true,
        },
        conflict: Default::default(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageValidationReport {
    pub package: PackageManifest,
    pub extensions: Vec<ExtensionManifest>,
    pub warnings: Vec<String>,
}

pub fn validate_extension_manifest_json(text: &str) -> AuthoringResult<ExtensionManifest> {
    let manifest = serde_json::from_str::<ExtensionManifest>(text)?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn validate_package_manifest_json(text: &str) -> AuthoringResult<PackageManifest> {
    let manifest = serde_json::from_str::<PackageManifest>(text)?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn validate_package_dir(path: impl AsRef<Path>) -> AuthoringResult<PackageValidationReport> {
    let path = path.as_ref();
    let package_path = path.join(PACKAGE_MANIFEST_FILE);
    let package = validate_package_manifest_json(&fs::read_to_string(&package_path)?)?;
    let mut extensions = Vec::new();
    let mut warnings = Vec::new();
    for extension_ref in &package.extensions {
        let manifest_path = path.join(&extension_ref.manifest);
        let extension = validate_extension_manifest_json(&fs::read_to_string(&manifest_path)?)?;
        if extension.package_id.as_ref() != Some(&package.id) {
            warnings.push(format!(
                "extension `{}` is not explicitly owned by package `{}`",
                extension.id, package.id
            ));
        }
        extensions.push(extension);
    }
    Ok(PackageValidationReport {
        package,
        extensions,
        warnings,
    })
}

pub fn write_example_package(
    root: impl AsRef<Path>,
    template: &ExampleExtensionTemplate,
) -> AuthoringResult<PathBuf> {
    let package_dir = root.as_ref().join(template.package_id.as_str());
    let extension_dir = package_dir.join("extensions/example");
    fs::create_dir_all(&extension_dir)?;
    fs::create_dir_all(package_dir.join("docs"))?;
    fs::create_dir_all(package_dir.join("examples"))?;
    fs::create_dir_all(package_dir.join("themes"))?;

    fs::write(
        extension_dir.join(MANIFEST_FILE),
        serde_json::to_string_pretty(&template.extension_manifest()?)?,
    )?;
    fs::write(
        package_dir.join(PACKAGE_MANIFEST_FILE),
        serde_json::to_string_pretty(&template.package_manifest()?)?,
    )?;
    fs::write(
        package_dir.join("plugin.wasm"),
        b"fixture wasm-json-v1 module",
    )?;
    fs::write(
        package_dir.join("docs/README.md"),
        "# Oino extension fixture\n\nCovers tools, commands, keymaps, UI, themes, autosuggest, providers, hooks, and persistence.\n",
    )?;
    fs::write(
        package_dir.join("examples/basic.rs"),
        r#"use oino_extension_sdk::WasmSdk;
use serde_json::json;

fn main() {
    let output = WasmSdk::tool_success(json!({ "message": "hello" }));
    println!("{}", output.output);
}
"#,
    )?;
    fs::write(
        package_dir.join("themes/example-theme.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "accent": "#7dd3fc",
            "success": "#86efac"
        }))?,
    )?;
    Ok(package_dir)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmSdk {
    extension_id: ExtensionId,
}

impl WasmSdk {
    #[must_use]
    pub fn new(extension_id: ExtensionId) -> Self {
        Self { extension_id }
    }

    #[must_use]
    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    #[must_use]
    pub fn tool_success(output: Value) -> RuntimeResultValue {
        RuntimeResultValue {
            output,
            progress: Vec::new(),
        }
    }

    #[must_use]
    pub fn command_success(output: Value) -> RuntimeResultValue {
        Self::tool_success(output)
    }

    #[must_use]
    pub fn with_progress(
        mut result: RuntimeResultValue,
        message: impl Into<String>,
    ) -> RuntimeResultValue {
        result.progress.push(RuntimeProgress {
            message: message.into(),
            details: None,
        });
        result
    }

    #[must_use]
    pub fn capability_request(
        &self,
        contribution_id: Option<ContributionId>,
        capability: impl Into<String>,
        payload: Value,
    ) -> CapabilityRequest {
        let mut request = CapabilityRequest::new(self.extension_id.clone(), capability, payload);
        request.contribution_id = contribution_id;
        request
    }

    #[must_use]
    pub fn ui_state_update(
        &self,
        surface_id: ContributionId,
        state: Value,
        actions: Vec<UiSurfaceAction>,
    ) -> UiSurfaceStateUpdate {
        UiSurfaceStateUpdate {
            surface_id,
            owner_extension_id: self.extension_id.clone(),
            state,
            actions,
        }
    }

    #[must_use]
    pub fn persistence_read_payload(scope: PersistenceScope, key: impl Into<String>) -> Value {
        serde_json::json!({ "scope": scope_name(scope), "key": key.into() })
    }

    #[must_use]
    pub fn persistence_write_payload(
        scope: PersistenceScope,
        key: impl Into<String>,
        value: Value,
    ) -> Value {
        serde_json::json!({ "scope": scope_name(scope), "key": key.into(), "value": value })
    }

    #[must_use]
    pub fn persistence_delete_payload(scope: PersistenceScope, key: impl Into<String>) -> Value {
        serde_json::json!({ "scope": scope_name(scope), "key": key.into() })
    }
}

fn scope_name(scope: PersistenceScope) -> &'static str {
    match scope {
        PersistenceScope::Session => "session",
        PersistenceScope::Project => "project",
        PersistenceScope::Global => "global",
    }
}

#[derive(Debug)]
pub struct ExtensionTestHarness {
    manifest: ExtensionManifest,
    runtime: JsonWasmRuntime,
    broker: CapabilityBroker,
    store: ExtensionPersistenceStore,
    ui_surfaces: oino_extension_core::RegistrySnapshot<UiSurfaceContribution>,
}

impl ExtensionTestHarness {
    pub fn new(
        manifest: ExtensionManifest,
        module: FixtureWasmModule,
        state_root: impl Into<PathBuf>,
    ) -> AuthoringResult<Self> {
        let runtime = JsonWasmRuntime::new(module).with_capabilities(
            RuntimeCapabilityPolicy::default()
                .allow("host.test.echo")
                .allow("host.persistence.read")
                .allow("host.persistence.write")
                .allow("host.persistence.delete"),
        );
        let mut broker = CapabilityBroker::new();
        broker.register_permissions(manifest.id.clone(), manifest.permissions.clone());
        let ui_surfaces = compose_ui_surfaces(&manifest)?;
        Ok(Self {
            manifest,
            runtime,
            broker,
            store: ExtensionPersistenceStore::new(state_root.into()),
            ui_surfaces,
        })
    }

    pub fn from_example(state_root: impl Into<PathBuf>) -> AuthoringResult<Self> {
        let template = ExampleExtensionTemplate::complete_example()?;
        let manifest = template.extension_manifest()?;
        let module = FixtureWasmModule::default()
            .with_handler(
                "handle_tool",
                FixtureHandlerBehavior::Success {
                    output: serde_json::json!({ "tool": "ok" }),
                    progress: vec![RuntimeProgress {
                        message: "tool progressed".into(),
                        details: None,
                    }],
                },
            )
            .with_handler(
                "handle_command",
                FixtureHandlerBehavior::Success {
                    output: serde_json::json!({ "command": "ok" }),
                    progress: Vec::new(),
                },
            )
            .with_handler(
                "handle_tool_hook",
                FixtureHandlerBehavior::Success {
                    output: serde_json::json!({ "hook": "ok" }),
                    progress: Vec::new(),
                },
            );
        Self::new(manifest, module, state_root)
    }

    #[must_use]
    pub fn manifest(&self) -> &ExtensionManifest {
        &self.manifest
    }

    #[must_use]
    pub fn ui_surfaces(&self) -> &oino_extension_core::RegistrySnapshot<UiSurfaceContribution> {
        &self.ui_surfaces
    }

    pub fn initialize_runtime(&mut self) -> Result<Vec<RuntimeProgress>, RuntimeError> {
        self.runtime.initialize(RuntimeInitialize {
            extension_id: self.manifest.id.clone(),
            abi: self
                .manifest
                .runtime
                .abi
                .clone()
                .unwrap_or_else(|| WASM_JSON_V1_ABI.into()),
            entry: self
                .manifest
                .runtime
                .entry
                .clone()
                .unwrap_or_else(|| "plugin.wasm".into()),
            metadata: self.manifest.metadata.clone(),
        })
    }

    pub fn invoke_tool(
        &mut self,
        contribution_id: &ContributionId,
        payload: Value,
    ) -> AuthoringResult<RuntimeResultValue> {
        let tool = self
            .manifest
            .contributes
            .tools
            .iter()
            .find(|tool| &tool.id == contribution_id)
            .ok_or_else(|| {
                AuthoringError::Validation(format!("unknown tool `{contribution_id}`"))
            })?;
        let mut invocation = RuntimeInvocation::new(
            self.manifest.id.clone(),
            tool.handler.clone().unwrap_or_else(|| tool.id.to_string()),
            payload,
        );
        invocation.contribution_id = Some(contribution_id.clone());
        Ok(self.runtime.invoke(invocation)?)
    }

    pub fn invoke_command(
        &mut self,
        contribution_id: &ContributionId,
        payload: Value,
    ) -> AuthoringResult<RuntimeResultValue> {
        let command = self
            .manifest
            .contributes
            .commands
            .iter()
            .find(|command| &command.id == contribution_id)
            .ok_or_else(|| {
                AuthoringError::Validation(format!("unknown command `{contribution_id}`"))
            })?;
        let mut invocation = RuntimeInvocation::new(
            self.manifest.id.clone(),
            command
                .handler
                .clone()
                .unwrap_or_else(|| command.id.to_string()),
            payload,
        );
        invocation.contribution_id = Some(contribution_id.clone());
        Ok(self.runtime.invoke(invocation)?)
    }

    pub fn call_capability(
        &mut self,
        contribution_id: Option<ContributionId>,
        capability: impl Into<String>,
        payload: Value,
    ) -> Result<CapabilityResponse, CapabilityError> {
        let mut request = CapabilityRequest::new(self.manifest.id.clone(), capability, payload);
        request.contribution_id = contribution_id;
        self.broker.call(request)
    }

    pub fn deny_future_capabilities(&mut self) {
        self.broker = CapabilityBroker::new();
    }

    pub fn ui_state_update(
        &self,
        surface_id: ContributionId,
        state: Value,
    ) -> AuthoringResult<UiSurfaceStateUpdate> {
        let active = self
            .ui_surfaces
            .active
            .iter()
            .find(|surface| surface.effective_id == surface_id)
            .ok_or_else(|| {
                AuthoringError::Validation(format!("unknown UI surface `{surface_id}`"))
            })?;
        let update = UiSurfaceStateUpdate {
            surface_id,
            owner_extension_id: self.manifest.id.clone(),
            state,
            actions: vec![UiSurfaceAction {
                id: "refresh".into(),
                label: "Refresh".into(),
                key_scope: Some("example".into()),
            }],
        };
        oino_extension_core::validate_ui_surface_update(active, &update)?;
        Ok(update)
    }

    pub fn write_persistence(
        &self,
        scope: PersistenceScope,
        key: impl Into<String>,
        payload: Value,
    ) -> AuthoringResult<()> {
        let record = PersistenceRecord {
            owner_extension_id: self.manifest.id.clone(),
            scope,
            key: key.into(),
            schema_version: 1,
            schema: Some("object".into()),
            payload,
            provenance: None,
            updated_at_unix_ms: 0,
            tombstoned: false,
        };
        self.store
            .write(&record, &self.manifest.permissions, Some(4096))
            .map_err(|err| AuthoringError::Validation(err.to_string()))
    }

    pub fn read_persistence(
        &self,
        scope: PersistenceScope,
        key: &str,
    ) -> AuthoringResult<PersistenceRecord> {
        self.store
            .read(&self.manifest.id, scope, key, &self.manifest.permissions)
            .map_err(|err| AuthoringError::Validation(err.to_string()))
    }
}

fn compose_ui_surfaces(
    manifest: &ExtensionManifest,
) -> AuthoringResult<oino_extension_core::RegistrySnapshot<UiSurfaceContribution>> {
    let mut registry = UiSurfaceRegistry::new(RegistryFamily::UiSurface);
    let source = SourceDescriptor {
        scope: SourceScope::Development,
        kind: SourceKind::LocalExtension,
        path: None,
        registry: None,
    };
    for surface in &manifest.contributes.ui_surfaces {
        let metadata = ContributionMetadata::new(surface.id.clone(), source.clone())
            .with_extension_id(manifest.id.clone());
        registry
            .register_entry(
                RegistryEntryKey::new(format!("sdk:{}", surface.id)),
                metadata,
                surface.clone(),
            )
            .map_err(|err| AuthoringError::Validation(err.to_string()))?;
    }
    Ok(registry.compose(&RegistryPolicy::default()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDecision {
    Implemented,
    Deferred,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGate {
    pub capability: &'static str,
    pub decision: CoverageDecision,
    pub evidence: &'static str,
}

pub const REQUIRED_COVERAGE_GATES: &[CoverageGate] = &[
    CoverageGate {
        capability: "Global/project extension discovery",
        decision: CoverageDecision::Implemented,
        evidence: "manager discovery tests",
    },
    CoverageGate {
        capability: "Hot reload",
        decision: CoverageDecision::Implemented,
        evidence: "reload diff tests",
    },
    CoverageGate {
        capability: "Custom model-visible tools",
        decision: CoverageDecision::Implemented,
        evidence: "runtime tool bridge tests",
    },
    CoverageGate {
        capability: "Tool-call block/patch hooks",
        decision: CoverageDecision::Implemented,
        evidence: "hook runner tests",
    },
    CoverageGate {
        capability: "Provider/model registration",
        decision: CoverageDecision::Implemented,
        evidence: "provider registry tests",
    },
    CoverageGate {
        capability: "Custom slash commands",
        decision: CoverageDecision::Implemented,
        evidence: "command bridge tests",
    },
    CoverageGate {
        capability: "Extension shortcuts/keybindings",
        decision: CoverageDecision::Implemented,
        evidence: "keymap conflict tests",
    },
    CoverageGate {
        capability: "TUI custom components",
        decision: CoverageDecision::Implemented,
        evidence: "UI surface state/render tests",
    },
    CoverageGate {
        capability: "Themes",
        decision: CoverageDecision::Implemented,
        evidence: "theme token tests",
    },
    CoverageGate {
        capability: "Session persistence",
        decision: CoverageDecision::Implemented,
        evidence: "persistence store/session tests",
    },
    CoverageGate {
        capability: "Package install/remove/update",
        decision: CoverageDecision::Implemented,
        evidence: "package lifecycle tests",
    },
    CoverageGate {
        capability: "Community package gallery",
        decision: CoverageDecision::Implemented,
        evidence: "fixture registry tests",
    },
    CoverageGate {
        capability: "Arbitrary filesystem/process/network imports",
        decision: CoverageDecision::Implemented,
        evidence: "capability denial tests",
    },
    CoverageGate {
        capability: "Extension examples/devkit",
        decision: CoverageDecision::Implemented,
        evidence: "oino-extension-sdk tests",
    },
    CoverageGate {
        capability: "Dynamic tool registration",
        decision: CoverageDecision::Deferred,
        evidence: "manifest-first policy",
    },
    CoverageGate {
        capability: "OAuth/login provider flows",
        decision: CoverageDecision::Deferred,
        evidence: "auth UX follow-up",
    },
    CoverageGate {
        capability: "npm/git package compatibility",
        decision: CoverageDecision::Rejected,
        evidence: "Oino-owned package format",
    },
    CoverageGate {
        capability: "Direct Ratatui rendering from extension code",
        decision: CoverageDecision::Rejected,
        evidence: "render safety policy",
    },
    CoverageGate {
        capability: "Pi TypeScript extension API",
        decision: CoverageDecision::Rejected,
        evidence: "semantic parity only",
    },
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    pub missing_capabilities: Vec<String>,
    pub missing_rejected_rows: Vec<String>,
}

impl CoverageReport {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.missing_capabilities.is_empty() && self.missing_rejected_rows.is_empty()
    }
}

#[must_use]
pub fn validate_parity_matrix(markdown: &str) -> CoverageReport {
    let mut report = CoverageReport::default();
    for gate in REQUIRED_COVERAGE_GATES {
        if !markdown.contains(&format!("| {} |", gate.capability)) {
            report.missing_capabilities.push(gate.capability.into());
        }
        if gate.decision == CoverageDecision::Rejected {
            let rejected_token = format!("| {} |", gate.capability);
            let row_rejected = markdown
                .lines()
                .find(|line| line.contains(&rejected_token))
                .is_some_and(|line| line.contains("| rejected |"));
            if !row_rejected {
                report.missing_rejected_rows.push(gate.capability.into());
            }
        }
    }
    report
}

#[must_use]
pub fn default_fixture_module() -> FixtureWasmModule {
    FixtureWasmModule::default()
        .with_handler(
            "handle_tool",
            FixtureHandlerBehavior::Success {
                output: serde_json::json!({ "ok": true }),
                progress: vec![RuntimeProgress {
                    message: "fixture progress".into(),
                    details: None,
                }],
            },
        )
        .with_handler(
            "handle_command",
            FixtureHandlerBehavior::Success {
                output: serde_json::json!({ "command": true }),
                progress: Vec::new(),
            },
        )
}

#[must_use]
pub fn package_service_for_layout(
    layout: oino_extension_manager::ExtensionLayoutPaths,
    current_version: Version,
) -> PackageLifecycleService {
    PackageLifecycleService::new(layout, current_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_extension_core::{
        ConflictPolicy, ConflictStrategy, SourceScopePolicy, UnknownContributionPolicy,
    };
    use oino_extension_manager::{
        ExtensionDiscovery, ExtensionLayoutPaths, ExtensionManager, ExtensionManagerConfig,
        PackageInstallScope,
    };
    use std::error::Error;

    fn write_json(path: &Path, value: &ExtensionManifest) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(value)?)?;
        Ok(())
    }

    #[test]
    fn example_template_generates_valid_manifest_package_and_registry_metadata(
    ) -> Result<(), Box<dyn Error>> {
        let template = ExampleExtensionTemplate::complete_example()?;
        let manifest = template.extension_manifest()?;
        let package = template.package_manifest()?;
        let metadata = template.registry_metadata(Some(PathBuf::from("/tmp/example")))?;
        assert_eq!(manifest.id.as_str(), "acme.example_extension");
        assert_eq!(package.extensions.len(), 1);
        assert!(manifest
            .contributes
            .tools
            .iter()
            .any(|tool| tool.id.as_str() == "example_tool"));
        assert!(manifest
            .contributes
            .commands
            .iter()
            .any(|command| command.id.as_str() == "example_command"));
        assert!(manifest
            .contributes
            .keymaps
            .iter()
            .any(|keymap| keymap.id.as_str() == "example_keymap"));
        assert!(manifest
            .contributes
            .ui_surfaces
            .iter()
            .any(|surface| surface.surface == UiSurfaceKind::Sidebar));
        assert!(manifest
            .contributes
            .ui_surfaces
            .iter()
            .any(|surface| surface.surface == UiSurfaceKind::FloatingPanel));
        assert!(manifest
            .contributes
            .ui_surfaces
            .iter()
            .any(|surface| surface.surface == UiSurfaceKind::Footer));
        assert!(!manifest.contributes.themes.is_empty());
        assert!(!manifest.contributes.autosuggest_providers.is_empty());
        assert!(!manifest.contributes.providers.is_empty());
        assert!(!manifest.contributes.hooks.is_empty());
        assert!(!manifest.contributes.persistence.is_empty());
        assert!(metadata
            .categories
            .iter()
            .any(|category| category == "examples"));
        Ok(())
    }

    #[test]
    fn devkit_validates_written_example_package() -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let template = ExampleExtensionTemplate::complete_example()?;
        let package_dir = write_example_package(temp.path(), &template)?;
        let report = validate_package_dir(&package_dir)?;
        assert_eq!(report.package.id, template.package_id);
        assert_eq!(report.extensions.len(), 1);
        assert!(report.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn rust_wasm_sdk_and_harness_cover_tool_command_capability_ui_and_persistence(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let mut harness = ExtensionTestHarness::from_example(temp.path().join("state"))?;
        harness.initialize_runtime()?;

        let tool_output = harness.invoke_tool(
            &ContributionId::new("example_tool")?,
            serde_json::json!({ "message": "hello" }),
        )?;
        assert_eq!(tool_output.output["tool"], "ok");
        assert_eq!(tool_output.progress[0].message, "tool progressed");

        let command_output = harness.invoke_command(
            &ContributionId::new("example_command")?,
            serde_json::json!({}),
        )?;
        assert_eq!(command_output.output["command"], "ok");

        let capability = harness.call_capability(
            Some(ContributionId::new("example_tool")?),
            "host.test.echo",
            serde_json::json!({ "message": "echo" }),
        )?;
        assert_eq!(capability.output["message"], "echo");

        harness.deny_future_capabilities();
        assert!(matches!(
            harness.call_capability(None, "host.test.echo", serde_json::json!({})),
            Err(CapabilityError::PermissionDenied { .. })
        ));

        let sidebar_update = harness.ui_state_update(
            ContributionId::new("example_sidebar")?,
            serde_json::json!({ "items": ["one"] }),
        )?;
        assert_eq!(sidebar_update.actions[0].id, "refresh");

        harness.write_persistence(
            PersistenceScope::Project,
            "example_state",
            serde_json::json!({ "count": 1 }),
        )?;
        let record = harness.read_persistence(PersistenceScope::Project, "example_state")?;
        assert_eq!(record.payload["count"], 1);
        Ok(())
    }

    #[test]
    fn extension_kernel_e2e_smoke_covers_reload_safe_mode_and_conflicts(
    ) -> Result<(), Box<dyn Error>> {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let layout = ExtensionLayoutPaths::for_home_and_project(&home, &project);
        let template = ExampleExtensionTemplate::complete_example()?;
        let package_dir = write_example_package(temp.path().join("sources"), &template)?;

        let mut policy = RegistryPolicy::safe_defaults();
        policy
            .enabled_extensions
            .insert(template.extension_id.clone());
        let config = ExtensionManagerConfig::new(
            Version::parse("0.1.0")?,
            ExtensionDiscovery::from_layout(&layout),
        )
        .with_policy(policy.clone());
        let mut manager = ExtensionManager::new(config);
        manager.load();
        let service = PackageLifecycleService::new(layout.clone(), Version::parse("0.1.0")?);
        let install =
            service.install_local(&package_dir, PackageInstallScope::Project, &mut manager)?;
        assert_eq!(install.reload.diffs.tools.added.len(), 1);

        let mut conflicting = template.extension_manifest()?;
        conflicting.id = ExtensionId::new("acme.conflicting_extension")?;
        conflicting.package_id = None;
        if let Some(tool) = conflicting.contributes.tools.first_mut() {
            tool.conflict = ConflictPolicy {
                strategy: ConflictStrategy::Error,
                priority: 0,
                allow_user_override: false,
            };
        }
        write_json(
            &layout
                .project_extensions
                .join("conflict")
                .join(MANIFEST_FILE),
            &conflicting,
        )?;
        policy.enabled_extensions.insert(conflicting.id.clone());
        manager = ExtensionManager::new(
            ExtensionManagerConfig::new(
                Version::parse("0.1.0")?,
                ExtensionDiscovery::from_layout(&layout),
            )
            .with_policy(policy),
        );
        let conflict_snapshot = manager.load();
        assert_eq!(conflict_snapshot.registries.tools.active.len(), 0);
        assert!(conflict_snapshot.registries.tools.inactive.len() >= 2);
        assert!(conflict_snapshot
            .registries
            .tools
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate contribution id")));

        let safe = manager.set_safe_mode(true);
        assert!(safe.next.safe_mode);
        assert_eq!(safe.next.registries.tools.active.len(), 0);
        assert!(safe.previous.is_some());
        Ok(())
    }

    #[test]
    fn parity_coverage_gate_matches_tracked_matrix() -> Result<(), Box<dyn Error>> {
        let matrix = include_str!(
            "../../../.unipi/docs/research/2026-05-21-oino-pi-extension-parity-matrix.md"
        );
        let report = validate_parity_matrix(matrix);
        assert!(report.is_ok(), "parity coverage gaps: {report:?}");
        assert!(REQUIRED_COVERAGE_GATES
            .iter()
            .any(|gate| gate.capability == "Pi TypeScript extension API"
                && gate.decision == CoverageDecision::Rejected));
        Ok(())
    }

    #[test]
    fn authors_can_override_dev_source_review_for_harness_snapshots() -> Result<(), Box<dyn Error>>
    {
        let template = ExampleExtensionTemplate::complete_example()?;
        let manifest = template.extension_manifest()?;
        let snapshot = compose_ui_surfaces(&manifest)?;
        assert!(!snapshot.active.is_empty());

        let mut policy = RegistryPolicy::safe_defaults();
        policy.source_scopes.insert(
            SourceScope::Development,
            SourceScopePolicy {
                unknown_contributions: UnknownContributionPolicy::Enabled,
                precedence: Some(SourceScope::Development.precedence()),
            },
        );
        let mut registry = UiSurfaceRegistry::new(RegistryFamily::UiSurface);
        let source = SourceDescriptor {
            scope: SourceScope::Development,
            kind: SourceKind::LocalExtension,
            path: None,
            registry: None,
        };
        for surface in &manifest.contributes.ui_surfaces {
            registry.register_entry(
                RegistryEntryKey::new(format!("dev:{}", surface.id)),
                ContributionMetadata::new(surface.id.clone(), source.clone())
                    .with_extension_id(manifest.id.clone()),
                surface.clone(),
            )?;
        }
        assert_eq!(registry.compose(&policy).active.len(), 3);
        Ok(())
    }
}
