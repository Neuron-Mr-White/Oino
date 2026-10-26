//! Per-cwd services (pi `agent-session-services.ts`): settings, resources, models + auth,
//! loaded extensions, the provider stream and the host router extensions call back into.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oino_ai::JsonValue;
use oino_models::{ModelRegistry, ModelRegistryOptions, ProviderRegistration};
use oino_resources::trust::{TrustRequest, TrustResolution, TrustStore, resolve_project_trust};
use oino_resources::{ResourceLoader, ResourceLoaderOptions};
use oino_settings::SettingsManager;
use serde_json::json;

use crate::extensions::api::{
    CallContext, CallKind, ExtensionInstance, ExtensionKind, ExtensionMode,
};
use crate::extensions::host::{HostApi, HostError, HostOp};
use crate::extensions::runner::{ExtensionSet, LoadedExtension};
use crate::extensions::settings::{ExtensionSettings, ExtensionSettingsError, SettingsChange};
use crate::provider::ProviderStream;
use crate::session::{AgentSession, Inner, lock};

/// Loads extension instances (WASM host, bundled components, test doubles).
pub trait ExtensionLoader: Send + Sync {
    /// Loads every enabled instance in discovery order. `host` is what the instances call back
    /// into; `project_trusted` gates `.oino/extensions`.
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>>;
}

/// A loader with a fixed instance list.
pub struct StaticLoader(pub Vec<Arc<dyn ExtensionInstance>>);

impl std::fmt::Debug for StaticLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticLoader")
            .field("instances", &self.0.len())
            .finish()
    }
}

impl ExtensionLoader for StaticLoader {
    fn load(
        &self,
        _host: Arc<dyn HostApi>,
        _project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        let instances = self.0.clone();
        Box::pin(async move { instances })
    }
}

/// CLI resource flags (pi `-ns`, `-np`, `-nc`, `--skill`, …).
#[allow(clippy::struct_excessive_bools, reason = "one field per pi CLI switch")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceFlags {
    /// `--no-skills`.
    pub no_skills: bool,
    /// `--no-prompt-templates`.
    pub no_prompt_templates: bool,
    /// `--no-themes`.
    pub no_themes: bool,
    /// `--no-context-files`.
    pub no_context_files: bool,
    /// `--skill` paths.
    pub skill_paths: Vec<String>,
    /// `--prompt-template` paths.
    pub prompt_template_paths: Vec<String>,
    /// `--theme` paths.
    pub theme_paths: Vec<String>,
    /// `--system-prompt`.
    pub system_prompt: Option<String>,
    /// `--append-system-prompt` values.
    pub append_system_prompt: Option<Vec<String>>,
    /// `-ne`: disable `kind: "extension"` discovery (providers stay).
    pub no_extensions: bool,
    /// `--offline` / `OINO_OFFLINE`: no startup network operations (dynamic model lists).
    pub offline: bool,
}

/// `OINO_OFFLINE` is `1`, `true` or `yes`.
#[must_use]
pub fn offline_env() -> bool {
    std::env::var("OINO_OFFLINE")
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// Inputs of [`Services::create`].
pub struct ServicesOptions {
    /// Working directory.
    pub cwd: PathBuf,
    /// Agent dir.
    pub agent_dir: PathBuf,
    /// Bundled docs root (system prompt docs section).
    pub docs_root: PathBuf,
    /// Run mode.
    pub mode: ExtensionMode,
    /// Dialog UI available (trust prompts, `hasUI`).
    pub has_ui: bool,
    /// CLI resource flags.
    pub flags: ResourceFlags,
    /// CLI values of extension flags.
    pub extension_flag_values: IndexMap<String, JsonValue>,
    /// Extension loader.
    pub loader: Arc<dyn ExtensionLoader>,
    /// `-a` / `-na`.
    pub approve: Option<bool>,
    /// Model registry override (tests); default reads `<agent>/models.json` and `auth.json`.
    pub models: Option<ModelRegistry>,
    /// In-memory settings (tests); default reads the settings files.
    pub settings: Option<SettingsManager>,
}

/// Failure creating services or sessions.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServicesError {
    /// Trust store failure.
    #[error("{0}")]
    Trust(String),
    /// Session file failure.
    #[error("{0}")]
    Session(String),
    /// Anything else.
    #[error("{0}")]
    Failed(String),
}

/// Routes host calls to the session named by the call context.
#[derive(Default)]
pub struct HostRouter {
    sessions: Mutex<HashMap<String, Weak<Inner>>>,
    extensions: OnceLock<Arc<ExtensionSet>>,
    models: OnceLock<ModelRegistry>,
    settings: OnceLock<Arc<ExtensionSettings>>,
    base_cx: OnceLock<CallContext>,
}

/// Kind of the event announcing a settings write to the owning extension.
pub const SETTINGS_CHANGED_EVENT: &str = "oino_settings_changed";

fn settings_error(error: ExtensionSettingsError) -> HostError {
    match error {
        ExtensionSettingsError::Settings(_) | ExtensionSettingsError::Secrets(_) => {
            HostError::Failed(error.to_string())
        }
        other => HostError::Invalid(other.to_string()),
    }
}

impl std::fmt::Debug for HostRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostRouter").finish_non_exhaustive()
    }
}

impl HostRouter {
    /// Registers a live session.
    pub fn attach(&self, session: &AgentSession) {
        lock(&self.sessions).insert(session.session_id(), Arc::downgrade(&session.inner));
    }

    /// Removes a session.
    pub fn detach(&self, session_id: &str) {
        lock(&self.sessions).remove(session_id);
    }

    fn session(&self, id: &str) -> Option<AgentSession> {
        let mut sessions = lock(&self.sessions);
        if let Some(inner) = sessions.get(id).and_then(Weak::upgrade) {
            Some(AgentSession::from_inner(inner))
        } else {
            sessions.remove(id);
            None
        }
    }

    /// The extension settings registry (set once by [`Services::create`]).
    #[must_use]
    pub fn extension_settings(&self) -> Option<&Arc<ExtensionSettings>> {
        self.settings.get()
    }

    /// Serves the settings operations ([`HostOp::is_settings_op`]) for `extension`.
    ///
    /// # Errors
    ///
    /// Registry failures; [`HostError::Failed`] before the registry is attached.
    pub fn settings_call(&self, extension: &str, op: HostOp) -> Result<JsonValue, HostError> {
        let registry = self
            .settings
            .get()
            .ok_or_else(|| HostError::Failed("extension settings are not available".into()))?;
        match op {
            HostOp::RegisterSettings(schema) => registry
                .register(extension, schema)
                .map(|()| JsonValue::Null)
                .map_err(settings_error),
            HostOp::GetExtensionSettings { namespace, layer } => {
                Ok(registry.values_for(extension, &namespace, layer))
            }
            HostOp::SetExtensionSetting {
                namespace,
                key,
                value,
                scope,
            } => registry
                .set_as(extension, &namespace, &key, value, scope)
                .map(|()| JsonValue::Null)
                .map_err(settings_error),
            HostOp::GetAllExtensionSettings => Ok(registry.all_for(extension)),
            HostOp::GetSettings => Ok(JsonValue::Object(registry.merged_settings())),
            other => Err(HostError::Invalid(format!(
                "not a settings operation: {other:?}"
            ))),
        }
    }

    /// Delivers `oino_settings_changed` to the namespace owner when it subscribed.
    async fn deliver_settings_change(&self, change: SettingsChange) {
        let Some(owner) = change.extension_id.as_deref() else {
            return;
        };
        let Some(extension) = self
            .extensions
            .get()
            .and_then(|set| set.get(owner).cloned())
        else {
            return;
        };
        if !extension.state().events.contains(SETTINGS_CHANGED_EVENT) {
            return;
        }
        let mut cx = self.base_cx.get().cloned().unwrap_or_else(|| CallContext {
            session_id: String::new(),
            mode: ExtensionMode::Print,
            has_ui: false,
            cwd: PathBuf::new(),
            is_child: false,
            child_role: None,
            call_kind: CallKind::Event,
        });
        if let Some(id) = self.any_root_session() {
            cx.session_id = id;
        }
        let payload = json!({
            "type": SETTINGS_CHANGED_EVENT,
            "namespace": change.namespace,
            "keys": change.keys,
            "scope": change.scope,
        });
        let deadline = crate::extensions::runner::deadline_for(SETTINGS_CHANGED_EVENT);
        match tokio::time::timeout(
            deadline,
            extension
                .instance
                .on_event(SETTINGS_CHANGED_EVENT, payload, cx),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(
                    extension = owner,
                    "{SETTINGS_CHANGED_EVENT}: {}",
                    error.message
                );
            }
            Err(_) => tracing::warn!(extension = owner, "{SETTINGS_CHANGED_EVENT} timed out"),
        }
    }

    /// Id of a live root session (event context for host-produced events).
    fn any_root_session(&self) -> Option<String> {
        let sessions = lock(&self.sessions);
        sessions
            .iter()
            .filter_map(|(id, weak)| weak.upgrade().map(|inner| (id, inner)))
            .find(|(_, inner)| inner.agents.link.is_none())
            .map(|(id, _)| id.clone())
    }

    /// Registry operations that do not need a session (calls during `init` or after the
    /// session ended).
    fn without_session(&self, extension: &str, op: HostOp) -> Result<JsonValue, HostError> {
        let loaded = self
            .extensions
            .get()
            .and_then(|set| set.get(extension).cloned());
        let models = self.models.get();
        let need_loaded =
            || HostError::StaleContext(crate::extensions::runner::STALE_CONTEXT_MESSAGE.to_owned());
        match op {
            HostOp::Subscribe(kinds) => loaded
                .ok_or_else(need_loaded)?
                .update(|s| s.events.extend(kinds)),
            HostOp::RegisterTool(spec) => {
                loaded
                    .ok_or_else(need_loaded)?
                    .update(|s| s.tools.insert(spec.name.clone(), spec));
            }
            HostOp::RegisterCommand(spec) => loaded
                .ok_or_else(need_loaded)?
                .update(|s| s.commands.push(spec)),
            HostOp::SetArbiterRoles(roles) => loaded
                .ok_or_else(need_loaded)?
                .update(|s| s.arbiter = roles),
            HostOp::RegisterProvider(config) => {
                let registration: ProviderRegistration = serde_json::from_value(config)
                    .map_err(|e| HostError::Invalid(e.to_string()))?;
                models
                    .ok_or_else(need_loaded)?
                    .register_provider(extension, registration)
                    .map_err(|e| HostError::Failed(e.to_string()))?;
            }
            _ => return Err(need_loaded()),
        }
        Ok(JsonValue::Null)
    }
}

impl HostApi for HostRouter {
    fn call<'a>(
        &'a self,
        extension: &'a str,
        cx: &'a CallContext,
        op: HostOp,
    ) -> BoxFuture<'a, Result<JsonValue, HostError>> {
        Box::pin(async move {
            if op.is_settings_op() {
                return self.settings_call(extension, op);
            }
            match self.session(&cx.session_id) {
                Some(session) => session.host_call(extension, cx, op).await,
                None => self.without_session(extension, op),
            }
        })
    }

    fn model_stream<'a>(
        &'a self,
        _extension: &'a str,
        cx: &'a CallContext,
        request: crate::extensions::host::ModelRequest,
    ) -> BoxFuture<'a, Result<futures::stream::BoxStream<'static, JsonValue>, HostError>> {
        Box::pin(async move {
            let session = self.session(&cx.session_id).ok_or_else(|| {
                HostError::StaleContext(crate::extensions::runner::STALE_CONTEXT_MESSAGE.into())
            })?;
            session.model_stream(request).await
        })
    }

    fn spawn_agent<'a>(
        &'a self,
        extension: &'a str,
        cx: &'a CallContext,
        spec: JsonValue,
    ) -> BoxFuture<'a, Result<Arc<crate::agents::ChildAgent>, HostError>> {
        Box::pin(async move {
            let session = self.session(&cx.session_id).ok_or_else(|| {
                HostError::StaleContext(crate::extensions::runner::STALE_CONTEXT_MESSAGE.into())
            })?;
            let spec: crate::agents::AgentSpec = serde_json::from_value(spec)
                .map_err(|e| HostError::Invalid(format!("AgentSpec: {e}")))?;
            session.spawn_agent(extension, spec).await
        })
    }
}

/// Services of one `(cwd, agent_dir)`.
pub struct Services {
    /// Working directory.
    pub cwd: PathBuf,
    /// Agent dir.
    pub agent_dir: PathBuf,
    /// Bundled docs root.
    pub docs_root: PathBuf,
    /// Run mode.
    pub mode: ExtensionMode,
    /// Dialog UI available.
    pub has_ui: bool,
    /// Project trust decision.
    pub project_trusted: bool,
    /// Settings.
    pub settings: Arc<Mutex<SettingsManager>>,
    /// Extension settings registry over [`Self::settings`] (runtime §10.3).
    pub extension_settings: Arc<ExtensionSettings>,
    /// Resources.
    pub resources: Arc<RwLock<ResourceLoader>>,
    /// Models and auth.
    pub models: ModelRegistry,
    /// Loaded extensions.
    pub extensions: Arc<ExtensionSet>,
    /// Provider dispatch.
    pub stream: Arc<ProviderStream>,
    /// Host router.
    pub host: Arc<HostRouter>,
    /// CLI extension flag values.
    pub flag_values: IndexMap<String, JsonValue>,
    /// Startup diagnostics.
    pub diagnostics: Vec<String>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("cwd", &self.cwd)
            .field("extensions", &self.extensions.len())
            .finish_non_exhaustive()
    }
}

fn resolve_trust(
    options: &ServicesOptions,
    settings: &oino_settings::Settings,
) -> Result<bool, ServicesError> {
    let store = TrustStore::new(&options.agent_dir);
    let home = oino_settings::paths::home_dir();
    let request = TrustRequest {
        cwd: &options.cwd,
        home: home.as_deref(),
        cli_override: options.approve,
        extension_decision: None,
        default_trust: settings.default_project_trust(),
        // Interactive trust prompts are a frontend concern (raised before services start).
        has_ui: false,
    };
    match resolve_project_trust(&request, &store)
        .map_err(|e| ServicesError::Trust(e.to_string()))?
    {
        TrustResolution::Decided { trusted, .. } => Ok(trusted),
        TrustResolution::Ask { .. } => Ok(false),
    }
}

impl Services {
    /// Startup (runtime §4): trust → settings/resources → extensions (`init`) → providers.
    ///
    /// # Errors
    ///
    /// Trust store failures.
    pub async fn create(mut options: ServicesOptions) -> Result<Arc<Self>, ServicesError> {
        let mut diagnostics = Vec::new();
        let mut settings = options
            .settings
            .take()
            .unwrap_or_else(|| SettingsManager::create(&options.cwd, &options.agent_dir, false));
        let trusted = resolve_trust(&options, settings.settings())?;
        settings.set_project_trusted(trusted);
        diagnostics.extend(settings.drain_errors().into_iter().map(|e| e.to_string()));
        let mut resources = ResourceLoader::new(ResourceLoaderOptions {
            cwd: options.cwd.clone(),
            agent_dir: options.agent_dir.clone(),
            home_dir: oino_settings::paths::home_dir(),
            no_skills: options.flags.no_skills,
            no_prompt_templates: options.flags.no_prompt_templates,
            no_themes: options.flags.no_themes,
            no_context_files: options.flags.no_context_files,
            skill_paths: options.flags.skill_paths.clone(),
            prompt_template_paths: options.flags.prompt_template_paths.clone(),
            theme_paths: options.flags.theme_paths.clone(),
            system_prompt: options.flags.system_prompt.clone(),
            append_system_prompt: options.flags.append_system_prompt.clone(),
        });
        resources.reload(settings.settings(), trusted);
        diagnostics.extend(resources.diagnostics().iter().map(|d| format!("{d:?}")));
        let models = options.models.clone().unwrap_or_else(|| {
            let auth = Arc::new(oino_models::AuthStorage::open(
                options.agent_dir.join("auth.json"),
            ));
            ModelRegistry::new(
                auth,
                ModelRegistryOptions::for_agent_dir(&options.agent_dir),
            )
        });
        let settings = Arc::new(Mutex::new(settings));
        let extension_settings = Arc::new(ExtensionSettings::new(
            Arc::clone(&settings),
            Some(oino_settings::SecretStore::new(&options.agent_dir)),
        ));
        let host = Arc::new(HostRouter::default());
        let _ = host.models.set(models.clone());
        let _ = host.settings.set(Arc::clone(&extension_settings));
        let instances = options
            .loader
            .load(Arc::clone(&host) as Arc<dyn HostApi>, trusted)
            .await;
        let cx = CallContext {
            session_id: String::new(),
            mode: options.mode,
            has_ui: options.has_ui,
            cwd: options.cwd.clone(),
            is_child: false,
            child_role: None,
            call_kind: CallKind::Event,
        };
        let _ = host.base_cx.set(cx.clone());
        let loaded = init_extensions(
            instances,
            &cx,
            &models,
            &extension_settings,
            options.flags.no_extensions,
            &mut diagnostics,
        )
        .await;
        let extensions = Arc::new(ExtensionSet::new(loaded));
        let _ = host.extensions.set(Arc::clone(&extensions));
        models.set_oauth_refresher(Arc::new(ComponentOAuth {
            extensions: Arc::downgrade(&extensions),
            models: models.clone(),
        }));
        forward_settings_changes(&extension_settings, &host);
        if !(options.flags.offline || offline_env()) {
            refresh_dynamic_models(&extensions, &models).await;
        }
        Ok(Arc::new(Self {
            cwd: options.cwd,
            agent_dir: options.agent_dir,
            docs_root: options.docs_root,
            mode: options.mode,
            has_ui: options.has_ui,
            project_trusted: trusted,
            settings,
            extension_settings,
            resources: Arc::new(RwLock::new(resources)),
            models,
            stream: Arc::new(ProviderStream::new(Arc::clone(&extensions))),
            extensions,
            host,
            flag_values: options.extension_flag_values,
            diagnostics,
        }))
    }

    /// Sessions root: `--session-dir` > `OINO_SESSION_DIR` > setting `sessionDir` >
    /// `<agent>/sessions`.
    #[must_use]
    pub fn sessions_root(&self, cli: Option<&Path>) -> PathBuf {
        if let Some(dir) = cli {
            return dir.to_path_buf();
        }
        if let Some(dir) = std::env::var_os("OINO_SESSION_DIR").filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        let setting = lock(&self.settings).settings().session_dir();
        setting.map_or_else(
            || self.agent_dir.join("sessions"),
            |dir| {
                if dir.is_absolute() {
                    dir
                } else {
                    self.cwd.join(dir)
                }
            },
        )
    }
}

/// Initialises instances in load order, registering their providers, APIs and virtual models.
async fn init_extensions(
    instances: Vec<Arc<dyn ExtensionInstance>>,
    cx: &CallContext,
    models: &ModelRegistry,
    settings: &ExtensionSettings,
    no_extensions: bool,
    diagnostics: &mut Vec<String>,
) -> Vec<Arc<LoadedExtension>> {
    let mut loaded: Vec<Arc<LoadedExtension>> = Vec::new();
    for instance in instances {
        if no_extensions && instance.kind() == ExtensionKind::Extension {
            continue;
        }
        if loaded.iter().any(|l| l.instance.id() == instance.id()) {
            diagnostics.push(format!("Duplicate extension id {} ignored", instance.id()));
            continue;
        }
        match instance.init(cx.clone()).await {
            Ok(registration) => {
                for api in &registration.apis {
                    models.register_api_implementor(api, instance.id());
                }
                for provider in &registration.providers {
                    match serde_json::from_value::<ProviderRegistration>(provider.clone()) {
                        Ok(registration) => {
                            if let Err(error) =
                                models.register_provider(instance.id(), registration)
                            {
                                diagnostics.push(format!("{}: {error}", instance.id()));
                            }
                        }
                        Err(error) => diagnostics
                            .push(format!("{}: invalid provider: {error}", instance.id())),
                    }
                }
                for namespace in &registration.settings {
                    if let Err(error) = settings.register(instance.id(), namespace.clone()) {
                        diagnostics.push(format!("{}: settings: {error}", instance.id()));
                    }
                }
                for definition in &registration.virtual_models {
                    match serde_json::from_value(definition.clone()) {
                        Ok(definition) => {
                            if let Err(error) =
                                models.register_virtual_model(instance.id(), definition)
                            {
                                diagnostics.push(format!("{}: {error}", instance.id()));
                            }
                        }
                        Err(error) => diagnostics
                            .push(format!("{}: invalid virtual model: {error}", instance.id())),
                    }
                }
                loaded.push(Arc::new(LoadedExtension::new(instance, registration)));
            }
            Err(error) => {
                diagnostics.push(format!("{}: init failed: {}", instance.id(), error.message));
            }
        }
    }
    loaded
}

/// Forwards registry writes to the owning extensions (`oino_settings_changed`) until the
/// registry or the router is gone.
fn forward_settings_changes(registry: &ExtensionSettings, router: &Arc<HostRouter>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let mut changes = registry.subscribe();
    let router = Arc::downgrade(router);
    handle.spawn(async move {
        loop {
            match changes.recv().await {
                Ok(change) => {
                    let Some(router) = router.upgrade() else {
                        break;
                    };
                    router.deliver_settings_change(change).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!("{missed} settings change notifications dropped");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Bridges `auth.json` OAuth refreshes to the registering component's `oauth-refresh`
/// export (the component implementing the provider registration's `api`).
struct ComponentOAuth {
    extensions: Weak<ExtensionSet>,
    models: ModelRegistry,
}

/// Deadline of one `oauth-refresh` call (pi: 15 s).
const OAUTH_REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

impl oino_models::OAuthRefresher for ComponentOAuth {
    fn refresh<'a>(
        &'a self,
        provider: &'a str,
        credential: JsonValue,
    ) -> BoxFuture<'a, Result<JsonValue, String>> {
        Box::pin(async move {
            let extensions = self
                .extensions
                .upgrade()
                .ok_or_else(|| "extensions are shut down".to_owned())?;
            let registration = self
                .models
                .registration(provider)
                .ok_or_else(|| format!("Unknown provider: {provider}"))?;
            let component = provider_component(&extensions, &registration.api)
                .ok_or_else(|| format!("No provider component for {}", registration.api))?;
            let provider_instance = component
                .instance
                .provider()
                .ok_or_else(|| format!("{provider}: not a provider"))?;
            match tokio::time::timeout(
                OAUTH_REFRESH_TIMEOUT,
                provider_instance.oauth_refresh(provider, credential),
            )
            .await
            {
                Ok(result) => result.map_err(|e| e.message),
                Err(_) => Err(format!("OAuth refresh for {provider} timed out")),
            }
        })
    }
}

/// The provider component implementing `api` (first in load order).
fn provider_component(extensions: &ExtensionSet, api: &str) -> Option<Arc<LoadedExtension>> {
    extensions
        .extensions()
        .iter()
        .find(|e| e.instance.provider().is_some() && e.state().apis.iter().any(|a| a == api))
        .cloned()
}

/// Re-fetches the model list of one `models: "dynamic"` provider with `credential` (after
/// `/login` or a new API key); other providers are left alone.
pub(crate) async fn refresh_provider_models(
    extensions: &ExtensionSet,
    models: &ModelRegistry,
    provider_id: &str,
    credential: Option<JsonValue>,
) {
    let Some(registration) = models.registration(provider_id) else {
        return;
    };
    if !matches!(registration.models, oino_models::ProviderModels::Dynamic) {
        return;
    }
    let Some(component) = provider_component(extensions, &registration.api) else {
        return;
    };
    let Some(provider) = component.instance.provider() else {
        return;
    };
    match tokio::time::timeout(
        LIST_MODELS_TIMEOUT,
        provider.list_models(provider_id, credential),
    )
    .await
    {
        Ok(Ok(list)) => {
            if let Err(error) = models.set_provider_models(provider_id, &list) {
                tracing::warn!(provider = %provider_id, "storing models failed: {error}");
            }
        }
        Ok(Err(error)) => {
            tracing::warn!(provider = %provider_id, "list-models failed: {}", error.message);
        }
        Err(_) => tracing::warn!(provider = %provider_id, "list-models timed out"),
    }
}

/// Fetches `list-models` for providers registered with dynamic models.
/// Upper bound for one provider's `list-models` call at startup.
const LIST_MODELS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Fetches the model lists of `models: "dynamic"` providers concurrently. Providers that need
/// an API key and have none configured are skipped (their models would be unavailable anyway);
/// keyless providers (local servers) are probed with a short timeout.
async fn refresh_dynamic_models(extensions: &ExtensionSet, models: &ModelRegistry) {
    let mut calls = Vec::new();
    for provider_id in models.registered_providers() {
        let Some(registration) = models.registration(&provider_id) else {
            continue;
        };
        if !matches!(registration.models, oino_models::ProviderModels::Dynamic) {
            continue;
        }
        let Some(component) = extensions
            .extensions()
            .iter()
            .find(|e| e.instance.provider().is_some() && e.state().apis.contains(&registration.api))
            .cloned()
        else {
            continue;
        };
        let key = models.get_api_key(&provider_id).await;
        let needs_key = !registration.auth_env.is_empty() || registration.oauth;
        if needs_key && key.is_none() && !models.has_configured_auth(&provider_id) {
            continue;
        }
        let credential = key.map(|key| json!({"type": "api_key", "key": key}));
        calls.push(async move {
            let provider = component.instance.provider()?;
            let result = tokio::time::timeout(
                LIST_MODELS_TIMEOUT,
                provider.list_models(&provider_id, credential),
            )
            .await;
            Some((provider_id, result))
        });
    }
    for (provider_id, result) in futures::future::join_all(calls).await.into_iter().flatten() {
        match result {
            Ok(Ok(list)) => {
                if let Err(error) = models.set_provider_models(&provider_id, &list) {
                    tracing::warn!(provider = %provider_id, "storing models failed: {error}");
                }
            }
            Ok(Err(error)) => {
                tracing::warn!(provider = %provider_id, "list-models failed: {}", error.message);
            }
            Err(_) => tracing::warn!(provider = %provider_id, "list-models timed out"),
        }
    }
}
