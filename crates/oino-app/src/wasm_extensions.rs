//! WASM extension host integration: builds `WasmExtension` instances for
//! active wasm manifests, adapts their tools/commands/hooks/providers into the
//! app surfaces, and bridges host capabilities (secrets, UI, session) to real
//! app state.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use oino_agent_loop::{BeforeToolCallResult, StreamProvider, ToolCall};
use oino_auth::AuthStorage;
use oino_extension_core::{
    DiagnosticPhase, DiagnosticSeverity, ExtensionDiagnostic, ExtensionId, ExtensionManifest,
    ExtensionPermissions, ExtensionSessionEntry, HealthState, HookEventKind, HookMode,
    ProviderRuntimeProtocol, SourceScope, MANIFEST_FILE,
};
use oino_extension_manager::ExtensionManagerSnapshot;
use oino_harness::HookRegistry;
use oino_resource::ResourcePaths;
use oino_session::SessionManager;
use oino_types::Message;
use oino_wasm_host::{
    CallLimits, HostCaps, HostSession, HostUi, NoopSession, NoopUi, ProgressRouter, SecretResolver,
    WasmExtension, WasmHostEngine, WasmStreamProvider,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::extension_provider_runtime::extension_config_string;
use crate::TuiRuntimeEvent;

/// A hook contribution on a wasm extension, kept for handler registration.
struct WasmHookContribution {
    extension_id: ExtensionId,
    extension: Arc<WasmExtension>,
    event: HookEventKind,
    slug: String,
    mode: HookMode,
}

/// All wasm extension instances active for the current snapshot.
pub(crate) struct WasmExtensions {
    /// extension id -> instance.
    pub extensions: BTreeMap<ExtensionId, Arc<WasmExtension>>,
    /// provider_id -> streaming adapter.
    pub providers: BTreeMap<String, Arc<WasmStreamProvider>>,
    /// provider_id -> model ids from `list_models`.
    pub provider_models: BTreeMap<String, Vec<String>>,
    /// provider_id -> raw `list_models` entries (for cache warming
    /// `prompt_cache` lifetimes and cost data).
    pub provider_model_details: BTreeMap<String, Vec<Value>>,
    /// effective contribution id -> (extension, guest tool name).
    pub tools: BTreeMap<String, (Arc<WasmExtension>, String)>,
    /// effective contribution id -> (extension, guest command name).
    pub commands: BTreeMap<String, (Arc<WasmExtension>, String)>,
    hooks: Vec<WasmHookContribution>,
    pub progress: ProgressRouter,
    /// Diagnostics produced while loading/calling wasm extensions; surfaced
    /// via `/extensions`.
    pub diagnostics: Arc<Mutex<Vec<ExtensionDiagnostic>>>,
    session: Arc<Mutex<Option<Arc<tokio::sync::Mutex<SessionManager>>>>>,
}

impl WasmExtensions {
    /// Empty set — tests and flag-disabled paths.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            extensions: BTreeMap::new(),
            providers: BTreeMap::new(),
            provider_models: BTreeMap::new(),
            provider_model_details: BTreeMap::new(),
            tools: BTreeMap::new(),
            commands: BTreeMap::new(),
            hooks: Vec::new(),
            progress: ProgressRouter::default(),
            diagnostics: Arc::new(Mutex::new(Vec::new())),
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach the harness session handle once the harness exists (instances
    /// are built before the harness so providers can feed `build_runtime_provider`).
    pub fn set_session(&self, session: Arc<tokio::sync::Mutex<SessionManager>>) {
        if let Ok(mut slot) = self.session.lock() {
            *slot = Some(session);
        }
    }

    /// Stream providers keyed by provider id for `ProviderRouter`.
    pub fn provider_map(&self) -> BTreeMap<String, Arc<dyn StreamProvider>> {
        self.providers
            .iter()
            .map(|(id, provider)| (id.clone(), provider.clone() as Arc<dyn StreamProvider>))
            .collect()
    }

    pub fn provider_ids(&self) -> BTreeSet<String> {
        self.providers.keys().cloned().collect()
    }

    fn hooks_matching(&self, event: HookEventKind, mode: HookMode) -> Vec<&WasmHookContribution> {
        self.hooks
            .iter()
            .filter(|hook| hook.event == event && hook.mode == mode)
            .collect()
    }

    fn hooks_matching_kind(&self, event: Option<HookEventKind>) -> Vec<&WasmHookContribution> {
        let Some(event) = event else {
            return Vec::new();
        };
        self.hooks
            .iter()
            .filter(|hook| hook.event == event)
            .collect()
    }
}

fn diagnostic(
    extension_id: Option<&ExtensionId>,
    severity: DiagnosticSeverity,
    phase: DiagnosticPhase,
    message: impl Into<String>,
) -> ExtensionDiagnostic {
    ExtensionDiagnostic {
        severity,
        phase,
        package_id: None,
        extension_id: extension_id.cloned(),
        contribution_id: None,
        source_path: None,
        message: message.into(),
        remediation: None,
        health: HealthState::Degraded,
    }
}

/// `host.secret` resolver: extension config
/// `~/.oino/extensions/<id>/config.json` key first, then the environment
/// variable of the same name. The host only calls this for names declared in
/// `permissions.secrets`.
struct ExtensionSecretResolver {
    extension_id: String,
}

impl SecretResolver for ExtensionSecretResolver {
    fn secret(&self, name: &str) -> Option<String> {
        extension_config_string(&self.extension_id, name)
            .ok()
            .flatten()
            .or_else(|| std::env::var(name).ok())
    }
}

/// `host.notify`/`host.confirm` bridge. Notices go to the TUI runtime channel;
/// `confirm` returns `false` after emitting a notice — guest calls must never
/// block a tokio worker waiting on the TUI.
struct TuiHostUi {
    extension_id: String,
    tx: Option<mpsc::UnboundedSender<TuiRuntimeEvent>>,
}

impl HostUi for TuiHostUi {
    fn notify(&self, _level: u8, text: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(TuiRuntimeEvent::ExtensionNotice(format!(
                "[{}] {text}",
                self.extension_id
            )));
        }
    }
    fn confirm(&self, title: &str, body: &str) -> bool {
        self.notify(
            0,
            &format!("confirm `{title}` declined (interactive confirm not wired): {body}"),
        );
        false
    }
}

/// `host.session-append` → `ExtensionCustom` session entries.
struct AppHostSession {
    extension_id: ExtensionId,
    session: Arc<Mutex<Option<Arc<tokio::sync::Mutex<SessionManager>>>>>,
}

impl HostSession for AppHostSession {
    fn append(&self, kind: &str, payload: &str) -> Result<(), String> {
        let payload: Value =
            serde_json::from_str(payload).map_err(|err| format!("invalid payload JSON: {err}"))?;
        let session = self
            .session
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?
            .clone()
            .ok_or_else(|| "session not attached yet".to_string())?;
        let mut session = session
            .try_lock()
            .map_err(|_| "session busy; try again".to_string())?;
        session.append_extension_custom(ExtensionSessionEntry {
            owner_extension_id: self.extension_id.clone(),
            key: kind.to_string(),
            schema_version: 1,
            payload,
            provenance: None,
        });
        Ok(())
    }
}

fn hook_slug(kind: HookEventKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{kind:?}").to_lowercase())
}

fn manifest_dir(source_path: Option<&PathBuf>) -> Option<PathBuf> {
    let path = source_path?;
    if path.extension().is_some_and(|ext| ext == "json") {
        path.parent().map(PathBuf::from)
    } else {
        Some(path.clone())
    }
}

/// Build (or rebuild) wasm extension instances for the snapshot. Failures
/// become diagnostics — never crashes.
pub(crate) async fn build_wasm_extensions(
    snapshot: &ExtensionManagerSnapshot,
    paths: &ResourcePaths,
    ui_tx: Option<mpsc::UnboundedSender<TuiRuntimeEvent>>,
    auth: Option<&AuthStorage>,
    providers_runtime: &str,
) -> Arc<WasmExtensions> {
    let diagnostics: Arc<Mutex<Vec<ExtensionDiagnostic>>> = Arc::new(Mutex::new(Vec::new()));
    let progress = ProgressRouter::default();
    let session_slot: Arc<Mutex<Option<Arc<tokio::sync::Mutex<SessionManager>>>>> =
        Arc::new(Mutex::new(None));
    let push_diag = |diagnostic: ExtensionDiagnostic| {
        if let Ok(mut list) = diagnostics.lock() {
            list.push(diagnostic);
        }
    };

    let mut set = WasmExtensions {
        extensions: BTreeMap::new(),
        providers: BTreeMap::new(),
        provider_models: BTreeMap::new(),
        provider_model_details: BTreeMap::new(),
        tools: BTreeMap::new(),
        commands: BTreeMap::new(),
        hooks: Vec::new(),
        progress: progress.clone(),
        diagnostics: Arc::clone(&diagnostics),
        session: session_slot,
    };

    let cache_dir = paths.home_dir.join(".oino/cache/wasm");
    if let Err(err) = fs::create_dir_all(&cache_dir) {
        push_diag(diagnostic(
            None,
            DiagnosticSeverity::Error,
            DiagnosticPhase::RuntimeLoad,
            format!("wasm cache dir {}: {err}", cache_dir.display()),
        ));
    }
    let engine = match WasmHostEngine::new(&cache_dir) {
        Ok(engine) => engine,
        Err(err) => {
            push_diag(diagnostic(
                None,
                DiagnosticSeverity::Error,
                DiagnosticPhase::RuntimeLoad,
                format!("wasm engine unavailable: {err}"),
            ));
            return Arc::new(set);
        }
    };

    let record_for = |id: &ExtensionId| snapshot.extensions.iter().find(|record| &record.id == id);

    // Collect extension ids + manifest dirs from active contribution metadata.
    let mut wanted: BTreeMap<ExtensionId, PathBuf> = BTreeMap::new();
    let mut tool_contribs: Vec<(ExtensionId, String, String)> = Vec::new();
    let mut command_contribs: Vec<(ExtensionId, String, String)> = Vec::new();
    let mut hook_contribs: Vec<(ExtensionId, HookEventKind)> = Vec::new();
    let mut provider_contribs: Vec<(ExtensionId, String)> = Vec::new();

    for active in &snapshot.registries.tools.active {
        let Some(id) = &active.entry.metadata.extension_id else {
            continue;
        };
        let contribution = &active.entry.contribution;
        tool_contribs.push((
            id.clone(),
            active.effective_id.to_string(),
            contribution
                .handler
                .clone()
                .unwrap_or_else(|| contribution.id.to_string()),
        ));
    }
    for active in &snapshot.registries.commands.active {
        let Some(id) = &active.entry.metadata.extension_id else {
            continue;
        };
        let contribution = &active.entry.contribution;
        command_contribs.push((
            id.clone(),
            active.effective_id.to_string(),
            contribution
                .handler
                .clone()
                .unwrap_or_else(|| contribution.id.to_string()),
        ));
    }
    for active in &snapshot.registries.hooks.active {
        let Some(id) = &active.entry.metadata.extension_id else {
            continue;
        };
        hook_contribs.push((id.clone(), active.entry.contribution.event));
    }
    for active in &snapshot.registries.providers.active {
        let Some(id) = &active.entry.metadata.extension_id else {
            continue;
        };
        provider_contribs.push((id.clone(), active.entry.contribution.provider_id.clone()));
    }

    let mut collect_dir = |id: &ExtensionId| {
        if wanted.contains_key(id) {
            return;
        }
        let Some(record) = record_for(id) else { return };
        if let Some(dir) = manifest_dir(record.source.path.as_ref()) {
            wanted.insert(id.clone(), dir);
        }
    };
    for active in snapshot
        .registries
        .tools
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
    {
        if let Some(id) = &active.entry.metadata.extension_id {
            collect_dir(id);
        }
    }
    for active in snapshot
        .registries
        .commands
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
    {
        if let Some(id) = &active.entry.metadata.extension_id {
            collect_dir(id);
        }
    }
    for active in snapshot
        .registries
        .hooks
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
    {
        if let Some(id) = &active.entry.metadata.extension_id {
            collect_dir(id);
        }
    }
    for active in snapshot
        .registries
        .providers
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
    {
        if let Some(id) = &active.entry.metadata.extension_id {
            collect_dir(id);
        }
    }

    for (extension_id, dir) in &wanted {
        let manifest_path = dir.join(MANIFEST_FILE);
        let Some(manifest) = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|text| serde_json::from_str::<ExtensionManifest>(&text).ok())
        else {
            continue;
        };
        if !matches!(
            manifest.runtime.kind,
            oino_extension_core::RuntimeKind::Wasm
        ) {
            continue;
        }
        let Some(entry) = manifest.runtime.entry.clone() else {
            continue;
        };
        let bytes = match fs::read(dir.join(&entry)) {
            Ok(bytes) => bytes,
            Err(err) => {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Error,
                    DiagnosticPhase::RuntimeLoad,
                    format!("cannot read wasm entry `{entry}`: {err}"),
                ));
                continue;
            }
        };
        let mut permissions = record_for(extension_id)
            .map(|record| record.permissions.clone())
            .unwrap_or_else(|| manifest.permissions.clone());
        substitute_permission_paths(&mut permissions, paths);
        let kv_dir = paths
            .home_dir
            .join(".oino/extension-data")
            .join(extension_id.as_str())
            .join("kv");
        let _ = fs::create_dir_all(&kv_dir);
        let caps = HostCaps {
            permissions,
            secrets: Arc::new(ExtensionSecretResolver {
                extension_id: extension_id.to_string(),
            }),
            ui: Arc::new(TuiHostUi {
                extension_id: extension_id.to_string(),
                tx: ui_tx.clone(),
            }),
            session: Arc::new(AppHostSession {
                extension_id: extension_id.clone(),
                session: Arc::clone(&set.session),
            }),
            progress: progress.sink(),
            kv_dir,
        };
        let has_provider = provider_contribs.iter().any(|(id, _)| id == extension_id);
        let extension = match WasmExtension::new(
            engine.clone(),
            &bytes,
            caps,
            CallLimits::default(),
            has_provider,
        ) {
            Ok(extension) => Arc::new(extension),
            Err(err) => {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Error,
                    DiagnosticPhase::RuntimeLoad,
                    format!("component load failed: {err}"),
                ));
                continue;
            }
        };
        let mut config: Value = fs::read_to_string(
            paths
                .home_dir
                .join(".oino/extensions")
                .join(extension_id.as_str())
                .join("config.json"),
        )
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
        // Headless run overrides (e.g. `--max-cost` → oino.budget caps).
        for (env_name, target) in [
            ("OINO_BUDGET_CONFIG", "oino.budget"),
            ("OINO_PRESETS_CONFIG", "oino.presets"),
        ] {
            if extension_id.as_str() == target {
                if let Ok(overrides) = std::env::var(env_name)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .unwrap_or_else(|| json!({}))
                    .as_object()
                    .cloned()
                    .ok_or("not an object")
                {
                    if let Some(object) = config.as_object_mut() {
                        for (key, value) in overrides {
                            object.insert(key, value);
                        }
                    }
                }
            }
        }
        let registration = match extension.init(config).await {
            Ok(registration) => registration,
            Err(err) => {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Error,
                    DiagnosticPhase::RuntimeLoad,
                    format!("init failed: {err}"),
                ));
                continue;
            }
        };
        let names = |key: &str, field: &str| -> BTreeSet<String> {
            registration[key]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item[field].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let registered_tools = names("tools", "name");
        let registered_commands = names("commands", "name");
        let registered_providers: BTreeSet<String> = registration["providers"]
            .as_array()
            .map(|providers| {
                providers
                    .iter()
                    .filter_map(|provider| provider.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let registered_hooks: BTreeSet<String> = registration["hooks"]
            .as_array()
            .map(|hooks| {
                hooks
                    .iter()
                    .filter_map(|hook| hook.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        for (_, effective_id, guest_name) in
            tool_contribs.iter().filter(|(id, _, _)| id == extension_id)
        {
            if registered_tools.contains(guest_name) {
                set.tools.insert(
                    effective_id.clone(),
                    (extension.clone(), guest_name.clone()),
                );
            } else {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Warning,
                    DiagnosticPhase::RuntimeLoad,
                    format!(
                        "tool `{guest_name}` declared in manifest but missing from registration"
                    ),
                ));
            }
        }
        for (_, effective_id, guest_name) in command_contribs
            .iter()
            .filter(|(id, _, _)| id == extension_id)
        {
            if registered_commands.contains(guest_name) {
                set.commands.insert(
                    effective_id.clone(),
                    (extension.clone(), guest_name.clone()),
                );
            } else {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Warning,
                    DiagnosticPhase::RuntimeLoad,
                    format!(
                        "command `{guest_name}` declared in manifest but missing from registration"
                    ),
                ));
            }
        }
        for (_, event) in hook_contribs.iter().filter(|(id, _)| id == extension_id) {
            let slug = hook_slug(*event);
            if registered_hooks.contains(&slug) {
                let mode = manifest
                    .contributes
                    .hooks
                    .iter()
                    .find(|hook| hook.event == *event)
                    .map(|hook| hook.mode)
                    .unwrap_or(HookMode::Mutable);
                set.hooks.push(WasmHookContribution {
                    extension_id: extension_id.clone(),
                    extension: extension.clone(),
                    event: *event,
                    slug,
                    mode,
                });
            } else {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Warning,
                    DiagnosticPhase::RuntimeLoad,
                    format!("hook `{slug}` declared in manifest but missing from registration"),
                ));
            }
        }
        for name in registered_tools.iter().chain(registered_commands.iter()) {
            let known = tool_contribs
                .iter()
                .chain(command_contribs.iter())
                .any(|(id, _, guest)| id == extension_id && guest == name);
            if !known {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Info,
                    DiagnosticPhase::RuntimeLoad,
                    format!("registration exposes `{name}` not declared in the manifest; ignored"),
                ));
            }
        }

        for (_, provider_id) in provider_contribs
            .iter()
            .filter(|(id, _)| id == extension_id)
        {
            if !registered_providers.contains(provider_id) {
                push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Warning,
                    DiagnosticPhase::RuntimeLoad,
                    format!(
                        "provider `{provider_id}` declared in manifest but missing from registration"
                    ),
                ));
                continue;
            }
            match WasmStreamProvider::new(extension.clone()) {
                Ok(provider) => {
                    let provider = Arc::new(provider);
                    if let Ok(models) = provider.list_models().await {
                        let list = models["models"].as_array().or(models.as_array());
                        let ids = list
                            .map(|models| {
                                models
                                    .iter()
                                    .filter_map(|model| {
                                        model["id"]
                                            .as_str()
                                            .or(model["name"].as_str())
                                            .map(str::to_string)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        set.provider_models.insert(provider_id.clone(), ids);
                        if let Some(entries) = list {
                            set.provider_model_details
                                .insert(provider_id.clone(), entries.to_vec());
                        }
                    }
                    set.providers.insert(provider_id.clone(), provider);
                }
                Err(err) => push_diag(diagnostic(
                    Some(extension_id),
                    DiagnosticSeverity::Warning,
                    DiagnosticPhase::RuntimeLoad,
                    format!("provider `{provider_id}` adapter failed: {err}"),
                )),
            }
        }

        set.extensions.insert(extension_id.clone(), extension);
    }

    if providers_runtime == "wasm" {
        build_declarative_wasm_providers(snapshot, paths, auth, &engine, &mut set).await;
    } else {
        // Anthropic/Responses declarative providers only exist as WASM
        // components; surface a readiness diagnostic under the native runtime.
        for provider in crate::extension_provider_runtime::extension_runtime_providers(snapshot) {
            let protocol = provider
                .runtime
                .as_ref()
                .map_or(ProviderRuntimeProtocol::OpenAiChatCompletions, |runtime| {
                    runtime.protocol
                });
            if matches!(
                protocol,
                ProviderRuntimeProtocol::AnthropicMessages
                    | ProviderRuntimeProtocol::OpenAiResponses
            ) {
                set.diagnostics
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(diagnostic(
                        None,
                        DiagnosticSeverity::Warning,
                        DiagnosticPhase::RuntimeLoad,
                        format!(
                            "declarative provider `{}` uses the `{}` protocol and only works with providers.runtime = \"wasm\"",
                            provider.provider_id,
                            serde_json::to_value(protocol)
                                .ok()
                                .and_then(|v| v.as_str().map(str::to_string))
                                .unwrap_or_default()
                        ),
                    ));
            }
        }
    }

    Arc::new(set)
}

/// The bundled openai-compat component (provider-extension world). One
/// instance is created per declarative provider contribution when
/// `providers.runtime` is `wasm`; the host synthesizes permissions from the
/// resolved provider config (network hosts + `api_key` secret).
const OPENAI_COMPAT_WASM: &[u8] = include_bytes!("../assets/providers/openai_compat.wasm");

/// Bundled Anthropic Messages and OpenAI Responses components.
const ANTHROPIC_WASM: &[u8] = include_bytes!("../assets/providers/anthropic.wasm");
const OPENAI_RESPONSES_WASM: &[u8] = include_bytes!("../assets/providers/openai_responses.wasm");

fn component_for_protocol(protocol: ProviderRuntimeProtocol) -> Option<&'static [u8]> {
    match protocol {
        ProviderRuntimeProtocol::OpenAiChatCompletions => Some(OPENAI_COMPAT_WASM),
        ProviderRuntimeProtocol::AnthropicMessages => Some(ANTHROPIC_WASM),
        ProviderRuntimeProtocol::OpenAiResponses => Some(OPENAI_RESPONSES_WASM),
    }
}

/// Secret resolver serving exactly one resolved credential under `api_key`.
struct StaticSecretResolver(Option<String>);

impl SecretResolver for StaticSecretResolver {
    fn secret(&self, name: &str) -> Option<String> {
        if name == "api_key" {
            self.0.clone()
        } else {
            None
        }
    }
}

/// Host part of a URL (port ignored — matches `send_request` gating).
fn url_host(url: &str) -> String {
    let authority = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or_default();
    // strip optional port; bracketed IPv6 keeps its brackets off the port part
    let host = if let Some(end) = authority.strip_prefix('[').and_then(|a| a.find(']')) {
        &authority[1..=end]
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host.to_ascii_lowercase()
}

/// Instantiate `provider-openai-compat` once per declarative
/// (`runtime.protocol = open_ai_chat_completions`) provider contribution with
/// the same resolved config the native `OpenAiCompatibleProvider` would use.
async fn build_declarative_wasm_providers(
    snapshot: &ExtensionManagerSnapshot,
    paths: &ResourcePaths,
    auth: Option<&AuthStorage>,
    engine: &Arc<WasmHostEngine>,
    set: &mut WasmExtensions,
) {
    let contributions = crate::extension_provider_runtime::extension_runtime_providers(snapshot);
    for provider in contributions {
        let provider_id = provider.provider_id.clone();
        let config =
            match crate::extension_provider_runtime::extension_runtime_config_for_provider_id(
                &provider_id,
                std::slice::from_ref(&provider),
            ) {
                Ok(Some(config)) => config,
                Ok(None) => continue,
                Err(err) => {
                    set.diagnostics
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(diagnostic(
                            None,
                            DiagnosticSeverity::Error,
                            DiagnosticPhase::RuntimeLoad,
                            format!("wasm provider `{provider_id}` config failed: {err}"),
                        ));
                    continue;
                }
            };

        // Mirror `OpenAiCompatibleAuth` semantics host-side: required keys
        // fail the instance build, optional keys resolve best-effort.
        let (auth_kind, api_key_header, api_key) = match &config.auth {
            crate::provider_config::OpenAiCompatibleAuth::None => ("none", None, None),
            crate::provider_config::OpenAiCompatibleAuth::Bearer { spec } => {
                let key = match auth {
                    Some(auth) => match auth.resolve_api_key(spec).await {
                        Ok(key) => Some(key),
                        Err(err) => {
                            set.diagnostics
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(diagnostic(
                                    None,
                                    DiagnosticSeverity::Error,
                                    DiagnosticPhase::RuntimeLoad,
                                    format!("wasm provider `{provider_id}` api key: {err}"),
                                ));
                            continue;
                        }
                    },
                    None => None,
                };
                ("required", None, key)
            }
            crate::provider_config::OpenAiCompatibleAuth::OptionalBearer { spec } => {
                let key = match auth {
                    Some(auth) => auth
                        .resolve(spec)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|credential| credential.as_api_key().map(str::to_string)),
                    None => None,
                };
                ("optional", None, key)
            }
            crate::provider_config::OpenAiCompatibleAuth::ApiKeyHeader { spec, header_name } => {
                let key = match auth {
                    Some(auth) => match auth.resolve_api_key(spec).await {
                        Ok(key) => Some(key),
                        Err(err) => {
                            set.diagnostics
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(diagnostic(
                                    None,
                                    DiagnosticSeverity::Error,
                                    DiagnosticPhase::RuntimeLoad,
                                    format!("wasm provider `{provider_id}` api key: {err}"),
                                ));
                            continue;
                        }
                    },
                    None => None,
                };
                ("required", Some(header_name.clone()), key)
            }
        };

        let models_url = provider
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.models_url.clone());
        instantiate_declarative_wasm_provider(
            provider
                .runtime
                .as_ref()
                .map_or(ProviderRuntimeProtocol::OpenAiChatCompletions, |runtime| {
                    runtime.protocol
                }),
            &provider_id,
            &config,
            auth_kind,
            api_key_header,
            api_key,
            models_url,
            paths,
            engine,
            set,
        )
        .await;
    }
}

/// Instantiate the bundled openai-compat component for one provider id and
/// register it (plus `list_models` ids) on `set`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn instantiate_declarative_wasm_provider(
    protocol: ProviderRuntimeProtocol,
    provider_id: &str,
    config: &crate::provider_config::OpenAiCompatibleConfig,
    auth_kind: &str,
    api_key_header: Option<String>,
    api_key: Option<String>,
    models_url: Option<String>,
    paths: &ResourcePaths,
    engine: &Arc<WasmHostEngine>,
    set: &mut WasmExtensions,
) {
    let mut permissions = ExtensionPermissions::default();
    for url in [
        config.base_url.clone(),
        models_url
            .clone()
            .unwrap_or_else(|| config.models_endpoint()),
    ] {
        let host = url_host(&url);
        if !host.is_empty() {
            permissions.network.hosts.push(host);
        }
    }
    permissions.secrets.insert("api_key".into());

    let kv_dir = paths
        .home_dir
        .join(".oino/extension-data")
        .join(format!("provider-{provider_id}"));
    let _ = fs::create_dir_all(&kv_dir);
    let caps = HostCaps {
        permissions,
        secrets: Arc::new(StaticSecretResolver(api_key)),
        ui: Arc::new(NoopUi),
        session: Arc::new(NoopSession),
        progress: set.progress.sink(),
        kv_dir,
    };
    let Some(component) = component_for_protocol(protocol) else {
        set.diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(diagnostic(
                None,
                DiagnosticSeverity::Error,
                DiagnosticPhase::RuntimeLoad,
                format!("wasm provider `{provider_id}` has no bundled component for this protocol"),
            ));
        return;
    };
    let extension =
        match WasmExtension::new(engine.clone(), component, caps, CallLimits::default(), true) {
            Ok(extension) => Arc::new(extension),
            Err(err) => {
                set.diagnostics
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(diagnostic(
                        None,
                        DiagnosticSeverity::Error,
                        DiagnosticPhase::RuntimeLoad,
                        format!("wasm provider `{provider_id}` component load failed: {err}"),
                    ));
                return;
            }
        };
    let init = json!({
        "provider_id": provider_id,
        "display_name": config.display_name,
        "base_url": config.base_url,
        "models_url": models_url,
        "headers": config.headers,
        "auth": auth_kind,
        "api_key_header": api_key_header,
        "model_id_policy": "strip_provider_prefix",
        "compat": {
            "send_prompt_cache_key": config.send_prompt_cache_key,
            "session_affinity": config.session_affinity.map(|format| match format {
                crate::provider_config::SessionAffinityFormat::OpenAi => "openai",
                crate::provider_config::SessionAffinityFormat::OpenRouter => "openrouter",
            }),
            "cache_control_format": config.cache_control_format.map(|_| "anthropic"),
        },
    });
    if let Err(err) = extension.init(init).await {
        set.diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(diagnostic(
                None,
                DiagnosticSeverity::Error,
                DiagnosticPhase::RuntimeLoad,
                format!("wasm provider `{provider_id}` init failed: {err}"),
            ));
        return;
    }
    match WasmStreamProvider::new(extension) {
        Ok(provider_instance) => {
            let provider_instance = Arc::new(provider_instance);
            if let Ok(models) = provider_instance.list_models().await {
                let list = models["models"].as_array().or(models.as_array());
                let ids = list
                    .map(|models| {
                        models
                            .iter()
                            .filter_map(|model| {
                                model["id"]
                                    .as_str()
                                    .or(model["name"].as_str())
                                    .map(str::to_string)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                set.provider_models.insert(provider_id.to_string(), ids);
                if let Some(entries) = list {
                    set.provider_model_details
                        .insert(provider_id.to_string(), entries.to_vec());
                }
            }
            set.providers
                .insert(provider_id.to_string(), provider_instance);
        }
        Err(err) => set
            .diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(diagnostic(
                None,
                DiagnosticSeverity::Error,
                DiagnosticPhase::RuntimeLoad,
                format!("wasm provider `{provider_id}` adapter failed: {err}"),
            )),
    }
}

/// Holder for the current wasm set so hook handlers always dispatch to the
/// live instances (rebuilds replace the inner `Arc` instead of stacking
/// duplicate handlers).
pub(crate) type WasmHolder = Arc<std::sync::RwLock<Arc<WasmExtensions>>>;

/// Register one generic handler per harness hook point; each call iterates
/// the wasm hook contributions present in the holder's current set.
pub(crate) async fn register_wasm_hooks(hooks: &HookRegistry, holder: WasmHolder) {
    let holder1 = holder.clone();
    hooks
        .on_input(Arc::new(move |mut message: Message| {
            let holder = holder1.clone();
            Box::pin(async move {
                let wasm = holder.read().ok().map(|guard| guard.clone());
                let Some(wasm) = wasm else { return Ok(message) };
                let text = message.text_content().unwrap_or_default();
                if text.is_empty() {
                    return Ok(message);
                }
                for contribution in wasm.hooks_matching(HookEventKind::Input, HookMode::Mutable) {
                    match contribution
                        .extension
                        .handle_event(&contribution.slug, json!({ "text": text }))
                        .await
                    {
                        Ok(result) => {
                            if let Some(new_text) = result["payload"].as_str() {
                                message.replace_text(new_text);
                            }
                        }
                        Err(err) => hook_diag(
                            &wasm.diagnostics,
                            &contribution.extension_id,
                            &contribution.slug,
                            &err,
                        ),
                    }
                }
                Ok(message)
            })
        }))
        .await;

    let holder2 = holder.clone();
    hooks
        .on_context(Arc::new(move |mut messages: Vec<Message>| {
            let holder = holder2.clone();
            Box::pin(async move {
                let wasm = holder.read().ok().map(|guard| guard.clone());
                let Some(wasm) = wasm else {
                    return Ok(messages);
                };
                for contribution in wasm.hooks_matching(HookEventKind::Context, HookMode::Mutable) {
                    match contribution
                        .extension
                        .handle_event(&contribution.slug, json!({ "messages": messages }))
                        .await
                    {
                        Ok(result) => {
                            if let Ok(replacement) =
                                serde_json::from_value::<Vec<Message>>(result["payload"].clone())
                            {
                                messages = replacement;
                            }
                        }
                        Err(err) => hook_diag(
                            &wasm.diagnostics,
                            &contribution.extension_id,
                            &contribution.slug,
                            &err,
                        ),
                    }
                }
                Ok(messages)
            })
        }))
        .await;

    let holder3 = holder.clone();
    hooks
        .on_before_tool_call(Arc::new(move |mut call: ToolCall| {
            let holder = holder3.clone();
            Box::pin(async move {
                let wasm = holder.read().ok().map(|guard| guard.clone());
                let Some(wasm) = wasm else {
                    return Ok(BeforeToolCallResult::Allow(call));
                };
                for contribution in wasm.hooks_matching(HookEventKind::ToolCall, HookMode::Mutable)
                {
                    match contribution
                        .extension
                        .handle_event(&contribution.slug, json!({ "call": call }))
                        .await
                    {
                        Ok(result) => {
                            if let Some(block) = result.get("block") {
                                return Ok(BeforeToolCallResult::Block {
                                    reason: block["reason"]
                                        .as_str()
                                        .unwrap_or("blocked by extension")
                                        .to_string(),
                                    terminate: block["terminate"].as_bool().unwrap_or(false),
                                });
                            }
                            if let Ok(rewritten) =
                                serde_json::from_value::<ToolCall>(result["payload"].clone())
                            {
                                call = rewritten;
                            }
                        }
                        Err(err) => hook_diag(
                            &wasm.diagnostics,
                            &contribution.extension_id,
                            &contribution.slug,
                            &err,
                        ),
                    }
                }
                Ok(BeforeToolCallResult::Allow(call))
            })
        }))
        .await;

    let holder4 = holder.clone();
    hooks
        .on_after_tool_call(Arc::new(move |mut result: oino_agent_loop::ToolResult| {
            let holder = holder4.clone();
            Box::pin(async move {
                let wasm = holder.read().ok().map(|guard| guard.clone());
                let Some(wasm) = wasm else { return Ok(result) };
                for contribution in
                    wasm.hooks_matching(HookEventKind::ToolResult, HookMode::Mutable)
                {
                    match contribution
                        .extension
                        .handle_event(&contribution.slug, json!({ "result": result }))
                        .await
                    {
                        Ok(value) => {
                            if let Ok(replaced) =
                                serde_json::from_value::<oino_agent_loop::ToolResult>(
                                    value["result"].clone(),
                                )
                            {
                                result = replaced;
                            }
                        }
                        Err(err) => hook_diag(
                            &wasm.diagnostics,
                            &contribution.extension_id,
                            &contribution.slug,
                            &err,
                        ),
                    }
                }
                Ok(result)
            })
        }))
        .await;

    let holder5 = holder.clone();
    hooks
        .on_before_agent_start(Arc::new(move || {
            let holder = holder5.clone();
            Box::pin(async move {
                let wasm = holder.read().ok().map(|guard| guard.clone());
                let mut merged = oino_harness::BeforeAgentStartResult::default();
                let Some(wasm) = wasm else { return Ok(merged) };
                for contribution in
                    wasm.hooks_matching(HookEventKind::BeforeAgentStart, HookMode::Mutable)
                {
                    match contribution
                        .extension
                        .handle_event(&contribution.slug, json!({}))
                        .await
                    {
                        Ok(result) => {
                            if let Ok(result) = serde_json::from_value::<
                                oino_extension_runtime::BeforeAgentStartResult,
                            >(result)
                            {
                                merged.section_edits.extend(result.section_edits);
                                if result.active_tools.is_some() {
                                    merged.active_tools = result.active_tools;
                                }
                            }
                        }
                        Err(err) => hook_diag(
                            &wasm.diagnostics,
                            &contribution.extension_id,
                            &contribution.slug,
                            &err,
                        ),
                    }
                }
                Ok::<oino_harness::BeforeAgentStartResult, oino_agent_loop::LoopError>(merged)
            })
        }))
        .await;

    let holder6 = holder.clone();
    hooks
        .on_turn_end(Arc::new(move |context| {
            let holder = holder6.clone();
            Box::pin(async move {
                let wasm = holder.read().ok().map(|guard| guard.clone());
                let mut merged = oino_harness::TurnEndResult::default();
                let Some(wasm) = wasm else {
                    return Ok(Some(merged));
                };
                for contribution in wasm.hooks_matching(HookEventKind::TurnEnd, HookMode::Mutable) {
                    let payload = json!({
                        "message": context.message,
                        "tool_results": context.tool_results,
                    });
                    match contribution
                        .extension
                        .handle_event(&contribution.slug, payload)
                        .await
                    {
                        Ok(result) => {
                            if let Ok(result) = serde_json::from_value::<
                                oino_extension_runtime::TurnEndResult,
                            >(result)
                            {
                                merged.entries.extend(result.entries);
                                merged.continue_run |= result.continue_run;
                                merged.end_run |= result.end_run;
                            }
                        }
                        Err(err) => hook_diag(
                            &wasm.diagnostics,
                            &contribution.extension_id,
                            &contribution.slug,
                            &err,
                        ),
                    }
                }
                Ok::<Option<oino_harness::TurnEndResult>, oino_agent_loop::LoopError>(Some(merged))
            })
        }))
        .await;

    // Observe-only agent events → fire-and-forget `handle_event`.
    for (event, hook) in [
        (
            HookEventKind::AgentStart,
            oino_harness::NotificationHook::AgentStart,
        ),
        (
            HookEventKind::AgentEnd,
            oino_harness::NotificationHook::AgentEnd,
        ),
        (
            HookEventKind::AgentSettled,
            oino_harness::NotificationHook::Settled,
        ),
        (
            HookEventKind::MessageStart,
            oino_harness::NotificationHook::MessageStart,
        ),
        (
            HookEventKind::MessageEnd,
            oino_harness::NotificationHook::MessageEnd,
        ),
        (
            HookEventKind::TurnStart,
            oino_harness::NotificationHook::TurnStart,
        ),
        (
            HookEventKind::ToolExecutionStart,
            oino_harness::NotificationHook::ToolExecutionStart,
        ),
        (
            HookEventKind::ToolExecutionEnd,
            oino_harness::NotificationHook::ToolExecutionEnd,
        ),
    ] {
        let holder7 = holder.clone();
        hooks
            .on_notification(
                hook,
                Arc::new(move |event| {
                    let holder = holder7.clone();
                    Box::pin(async move {
                        let wasm = holder.read().ok().map(|guard| guard.clone());
                        let Some(wasm) = wasm else { return };
                        let json = serde_json::to_value(&event).unwrap_or_default();
                        for contribution in wasm.hooks_matching_kind(event_kind_for(&event)) {
                            if let Err(err) = contribution
                                .extension
                                .handle_event(&contribution.slug, json!({ "event": json }))
                                .await
                            {
                                hook_diag(
                                    &wasm.diagnostics,
                                    &contribution.extension_id,
                                    &contribution.slug,
                                    &err,
                                );
                            }
                        }
                    })
                }),
            )
            .await;
        let _ = event;
    }
}

fn event_kind_for(event: &oino_agent_loop::AgentEvent) -> Option<HookEventKind> {
    use oino_agent_loop::AgentEvent as A;
    Some(match event {
        A::AgentStart { .. } => HookEventKind::AgentStart,
        A::AgentEnd { .. } => HookEventKind::AgentEnd,
        A::Settled => HookEventKind::AgentSettled,
        A::MessageStart { .. } => HookEventKind::MessageStart,
        A::MessageEnd { .. } => HookEventKind::MessageEnd,
        A::TurnStart { .. } => HookEventKind::TurnStart,
        A::ToolExecutionStart { .. } => HookEventKind::ToolExecutionStart,
        A::ToolExecutionEnd { .. } => HookEventKind::ToolExecutionEnd,
        _ => return None,
    })
}

fn hook_diag(
    diagnostics: &Arc<Mutex<Vec<ExtensionDiagnostic>>>,
    extension_id: &ExtensionId,
    slug: &str,
    err: &oino_wasm_host::WasmHostError,
) {
    if let Ok(mut list) = diagnostics.lock() {
        list.push(diagnostic(
            Some(extension_id),
            DiagnosticSeverity::Warning,
            DiagnosticPhase::RuntimeLoad,
            format!("hook `{slug}` failed: {err}"),
        ));
    }
}

/// `cache_warming_decision` veto consult: mutable wasm hooks registered for
/// the event may veto via `{"veto": true}`.
pub(crate) fn wasm_cache_warming_veto(holder: WasmHolder) -> oino_harness::CacheWarmingVeto {
    Arc::new(move |payload: Value| {
        let holder = holder.clone();
        Box::pin(async move {
            let wasm = holder.read().ok().map(|guard| guard.clone());
            let Some(wasm) = wasm else { return false };
            for contribution in
                wasm.hooks_matching(HookEventKind::CacheWarmingDecision, HookMode::Mutable)
            {
                match contribution
                    .extension
                    .handle_event(&contribution.slug, payload.clone())
                    .await
                {
                    Ok(result) if result["veto"].as_bool() == Some(true) => return true,
                    Ok(_) => {}
                    Err(err) => hook_diag(
                        &wasm.diagnostics,
                        &contribution.extension_id,
                        &contribution.slug,
                        &err,
                    ),
                }
            }
            false
        }) as oino_agent_loop::BoxFuture<'static, bool>
    })
}

/// Catalog lookup for warming eligibility: resolves `prompt_cache` lifetimes
/// and prices from wasm `list_models` details.
pub(crate) fn wasm_model_cache_lookup(holder: WasmHolder) -> oino_harness::ModelCacheInfoLookup {
    Arc::new(move |model: &oino_types::Model| {
        let wasm = holder.read().ok().map(|guard| guard.clone())?;
        let entries = wasm.provider_model_details.get(&model.provider)?;
        let entry = entries
            .iter()
            .find(|e| e["id"].as_str() == Some(model.name.as_str()))?;
        let f64v = |v: &Value| v.as_f64().or_else(|| v.as_str()?.parse().ok());
        let u64v = |v: &Value| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64));
        let cache = &entry["prompt_cache"];
        let cost = &entry["cost"];
        Some(oino_harness::ModelCacheInfo {
            short_secs: u64v(&cache["short_secs"]),
            long_secs: u64v(&cache["long_secs"]),
            input_price: f64v(&cost["input"]),
            cache_read_price: f64v(&cost["cache_read"]),
        })
    })
}

/// Substitute `${project}` (project root) and `${home}` placeholders in
/// `permissions.filesystem[].paths` before preopens are built.
fn substitute_permission_paths(
    permissions: &mut oino_extension_core::ExtensionPermissions,
    paths: &ResourcePaths,
) {
    let project = paths.project_root.display().to_string();
    let home = paths.home_dir.display().to_string();
    for permission in &mut permissions.filesystem {
        for path in &mut permission.paths {
            *path = path
                .replace("${project}", &project)
                .replace("${home}", &home);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_paths_substitute_project_and_home() {
        let paths = ResourcePaths::from_home_and_cwd("/home/u", "/work/proj")
            .unwrap_or_else(|err| panic!("paths: {err}"));
        let mut permissions = oino_extension_core::ExtensionPermissions {
            filesystem: vec![oino_extension_core::FilesystemPermission {
                access: oino_extension_core::FilesystemAccess::Read,
                paths: vec!["${project}".into(), "${home}/shared".into(), "/abs".into()],
            }],
            ..oino_extension_core::ExtensionPermissions::default()
        };
        substitute_permission_paths(&mut permissions, &paths);
        assert_eq!(
            permissions.filesystem[0].paths,
            vec![
                "/work/proj".to_string(),
                "/home/u/shared".to_string(),
                "/abs".to_string()
            ]
        );
    }
}
