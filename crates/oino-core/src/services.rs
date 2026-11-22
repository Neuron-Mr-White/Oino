//! Per-cwd services: settings, resources, models + auth,
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
use crate::safe_mode::{SafeModePlan, banner as safe_mode_banner};
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

    /// Loads instances with filesystem preopens rooted at this services project's cwd.
    /// Fixed/test loaders may ignore the project; filesystem loaders must override it.
    fn load_for<'a>(
        &'a self,
        host: Arc<dyn HostApi>,
        cwd: &'a Path,
        project_trusted: bool,
        disabled: &'a [String],
    ) -> BoxFuture<'a, Vec<Arc<dyn ExtensionInstance>>> {
        let _ = cwd;
        self.load_enabled(host, project_trusted, disabled)
    }

    /// [`ExtensionLoader::load`] without the extensions listed in `disabledExtensions`.
    /// Loaders that can skip packages before instantiating them override this; the default
    /// loads everything and [`Services::create`] drops the disabled instances before `init`.
    fn load_enabled<'a>(
        &'a self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
        disabled: &'a [String],
    ) -> BoxFuture<'a, Vec<Arc<dyn ExtensionInstance>>> {
        let _ = disabled;
        self.load(host, project_trusted)
    }
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

/// CLI resource flags (`-ns`, `-np`, `-nc`, `--skill`, …).
#[allow(clippy::struct_excessive_bools, reason = "one field per CLI switch")]
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
    /// Safe-mode plan (X-SAFE): set by the embedding from [`crate::safe_mode::SafeMode::load`].
    /// `disable_extensions` ORs into `-ne` (providers stay) and adds the banner diagnostic.
    pub safe_mode: Option<SafeModePlan>,
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
    /// Docs root (system prompt docs section). Empty = absent: the section is omitted
    /// instead of pointing at a missing path (P-DOCS).
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
    extensions: OnceLock<Weak<ExtensionSet>>,
    models: OnceLock<ModelRegistry>,
    settings: OnceLock<Arc<ExtensionSettings>>,
    base_cx: OnceLock<CallContext>,
    topics: crate::topics::TopicStore,
    sessions_root: Mutex<Option<PathBuf>>,
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

    /// Keep extension index reads on the same sessions root as the host runtime.
    pub fn set_sessions_root(&self, root: PathBuf) {
        *lock(&self.sessions_root) = Some(root);
    }

    /// Removes a session.
    pub fn detach(&self, session_id: &str) {
        lock(&self.sessions).remove(session_id);
        for value in self.topics.detach(session_id) {
            for session in self.root_sessions() {
                session.offer_topic(&value);
            }
        }
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

    /// Delivers `oino_settings_changed` to the namespace owner when it subscribed and
    /// publishes the change on the bus topic `oino:settings:<namespace>`.
    async fn deliver_settings_change(&self, change: SettingsChange) {
        // Frontends (settings hub, StatusPanel) follow every write, including core namespaces
        // that no extension owns (`tui.*`).
        self.publish_topic(
            crate::topics::CORE_PUBLISHER,
            &crate::topics::settings_topic(&change.namespace),
            json!({
                "namespace": change.namespace.clone(),
                "keys": change.keys,
                "scope": change.scope,
                "extensionId": change.extension_id,
            }),
            true,
        );
        let Some(owner) = change.extension_id.as_deref() else {
            return;
        };
        let Some(extension) = self
            .extensions
            .get()
            .and_then(Weak::upgrade)
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

    /// The sticky topic store (`_oino/topics`).
    #[must_use]
    pub fn topics(&self) -> &crate::topics::TopicStore {
        &self.topics
    }

    /// Live root sessions.
    fn root_sessions(&self) -> Vec<AgentSession> {
        lock(&self.sessions)
            .values()
            .filter_map(Weak::upgrade)
            .filter(|inner| inner.agents.link.is_none())
            .map(AgentSession::from_inner)
            .collect()
    }

    /// Every live session (root and child).
    fn live_sessions(&self) -> Vec<AgentSession> {
        lock(&self.sessions)
            .values()
            .filter_map(Weak::upgrade)
            .map(AgentSession::from_inner)
            .collect()
    }

    /// Records a topic and offers it to every root session's frontend feed.
    pub fn publish_topic(&self, publisher: &str, topic: &str, data: JsonValue, sticky: bool) {
        if let Some(value) = self.topics.publish(publisher, topic, data, sticky) {
            for session in self.root_sessions() {
                session.offer_topic(&value);
            }
        }
    }

    /// Serves [`HostOp::is_bus_op`] operations for `extension`.
    ///
    /// # Errors
    ///
    /// [`HostError::Invalid`] for other operations.
    pub fn bus_call(&self, extension: &str, op: HostOp) -> Result<JsonValue, HostError> {
        match op {
            HostOp::BusPublish {
                topic,
                data,
                sticky,
            } => {
                self.publish_topic(extension, &topic, data, sticky);
                Ok(JsonValue::Null)
            }
            HostOp::BusGet(topic) => Ok(self
                .topics
                .get(&topic)
                .map_or(JsonValue::Null, |value| value.data)),
            other => Err(HostError::Invalid(format!(
                "not a bus operation: {other:?}"
            ))),
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
            .and_then(Weak::upgrade)
            .and_then(|set| set.get(extension).cloned());
        let models = self.models.get();
        let need_loaded =
            || HostError::StaleContext(crate::extensions::runner::STALE_CONTEXT_MESSAGE.to_owned());
        match op {
            HostOp::IsProjectTrusted => {
                return Ok(json!(
                    self.settings
                        .get()
                        .is_some_and(|settings| settings.project_trusted())
                ));
            }
            HostOp::Subscribe(kinds) => loaded
                .ok_or_else(need_loaded)?
                .update(|s| s.events.extend(kinds)),
            HostOp::RegisterTool(spec) => {
                loaded
                    .ok_or_else(need_loaded)?
                    .update(|s| s.tools.insert(spec.name.clone(), spec));
                // A re-registration after init (a guest's registry calls carry no session):
                // every live session of this set rebuilds its tool list, as the session path
                // does, so a changed schema reaches the next request.
                for session in self.live_sessions() {
                    session.refresh_tool_registry(None, false);
                }
            }
            HostOp::UnregisterTool(name) => {
                loaded
                    .ok_or_else(need_loaded)?
                    .update(|s| s.tools.shift_remove(&name));
                for session in self.live_sessions() {
                    session.refresh_tool_registry(None, false);
                }
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
    fn approval_pending(&self, cx: &CallContext) -> bool {
        self.session(&cx.session_id)
            .is_some_and(|s| s.has_pending_approval())
    }

    fn call<'a>(
        &'a self,
        extension: &'a str,
        cx: &'a CallContext,
        op: HostOp,
    ) -> BoxFuture<'a, Result<JsonValue, HostError>> {
        Box::pin(async move {
            if let HostOp::SessionsList(scope) = &op {
                let root = lock(&self.sessions_root)
                    .clone()
                    .ok_or_else(|| HostError::Failed("sessions index not attached".into()))?;
                let cwd = self.base_cx.get().map(|cx| cx.cwd.clone());
                let scope = *scope;
                return tokio::task::spawn_blocking(move || {
                    let cards = oino_session::SessionManager::list_all_cards(&root)
                        .into_iter()
                        .filter(|s| {
                            scope == oino_extension_api::ops::SessionsScope::All
                                || cwd
                                    .as_ref()
                                    .is_some_and(|cwd| s.cwd == cwd.to_string_lossy())
                        })
                        .map(|s| {
                            serde_json::json!({
                                "id": s.id, "name": s.name, "path": s.path,
                                "cwd": s.cwd, "firstMessage": s.first_message,
                            })
                        })
                        .collect::<Vec<_>>();
                    serde_json::to_value(cards).map_err(|e| HostError::Failed(e.to_string()))
                })
                .await
                .map_err(|e| HostError::Failed(e.to_string()))?;
            }
            if op.is_settings_op() {
                return self.settings_call(extension, op);
            }
            if let HostOp::BusPublish { topic, data, .. } = &op
                && topic == "oino:permission:background"
                && let Some(session) = self.session(&cx.session_id)
                && let Some(id) = data.get("agentId").and_then(JsonValue::as_str)
            {
                for child in lock(&session.inner.agents.children).iter() {
                    if child.id() == id
                        && let Some(child) = child.session()
                    {
                        child.set_unattended();
                    }
                }
            }
            if let HostOp::BusPublish { topic, data, .. } = &op
                && extension == "unipi.subagents"
                && topic == "unipi:subagents:list"
            {
                // The payload identifies the registry contribution, which can differ from
                // the ambient call context during graph reconciliation.
                let source = data
                    .get("parentSessionId")
                    .and_then(JsonValue::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| HostError::Invalid("missing subagent session".into()))?;
                let session = self
                    .session(source)
                    .ok_or_else(|| HostError::StaleContext("subagent session ended".into()))?;
                let root =
                    AgentSession::from_inner(crate::agents::root_of(&session.inner)).session_id();
                if let Some(value) = self
                    .topics
                    .publish_subagents(&root, source, extension, data)
                {
                    for session in self.root_sessions() {
                        session.offer_topic(&value);
                    }
                }
                return Ok(JsonValue::Null);
            }
            if op.is_bus_op() {
                return self.bus_call(extension, op);
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

    fn detach_bash<'a>(
        &'a self,
        _extension: &'a str,
        cx: &'a CallContext,
        tool_call_id: &'a str,
        task_id: &'a str,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<crate::session::DetachedBash, HostError>> {
        Box::pin(async move {
            let session = self.session(&cx.session_id).ok_or_else(|| {
                HostError::StaleContext(crate::extensions::runner::STALE_CONTEXT_MESSAGE.into())
            })?;
            session.detach_bash(tool_call_id, task_id, reason).await
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
    /// Initial project trust decision (CLI startup compatibility). Live trust is in
    /// `settings.is_project_trusted()` and is re-resolved on reload.
    pub project_trusted: bool,
    trust_override: Option<bool>,
    mcp_mtimes: Mutex<Vec<Option<(std::time::SystemTime, u64)>>>,
    reload_lock: tokio::sync::Mutex<()>,
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
    /// Safe-mode plan of this start (X-SAFE; `None` when the embedding does not participate).
    /// [`SafeModePlan::disable_extensions`] is what the `_oino/hello` reply reports as
    /// `safeMode`.
    pub safe_mode: Option<SafeModePlan>,
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

/// X-SAFE: the plan carried in [`ResourceFlags::safe_mode`] — its `-ne` effect (providers
/// stay), whether extensions are off, and the banner diagnostic for a tripped plan.
fn safe_mode_start(
    flags: &ResourceFlags,
    diagnostics: &mut Vec<String>,
) -> (Option<SafeModePlan>, bool) {
    let safe_mode = flags.safe_mode.clone();
    let no_extensions = flags.no_extensions
        || safe_mode
            .as_ref()
            .is_some_and(|plan| plan.disable_extensions);
    if let Some(plan) = &safe_mode
        && plan.disable_extensions
    {
        diagnostics.push(safe_mode_banner(plan));
    }
    (safe_mode, no_extensions)
}

/// Project MCP definitions and the legacy enabled-flags sidecar. Metadata only: no
/// untrusted config content is read while checking freshness.
fn mcp_mtimes(cwd: &Path) -> Vec<Option<(std::time::SystemTime, u64)>> {
    [
        ".oino/mcp.json",
        ".pi/mcp.json",
        ".unipi/config/mcp/mcp-config.json",
        ".unipi/config/mcp/config.json",
        ".mcp.json",
    ]
    .iter()
    .map(|file| {
        std::fs::metadata(cwd.join(file))
            .ok()
            .and_then(|m| m.modified().ok().map(|time| (time, m.len())))
    })
    .collect()
}

impl Services {
    /// Calls MCP Settings without allocating a session runtime. Executable transports
    /// still require a real chat context for permission checks.
    ///
    /// # Errors
    /// Returns an error if MCP is unavailable, its service fails, or the call times out.
    pub async fn mcp_servers(&self) -> Result<JsonValue, ServicesError> {
        let extension = self
            .extensions
            .get("unipi.mcp")
            .ok_or_else(|| ServicesError::Failed("Unknown service: mcp.servers".into()))?;
        let mut cx = self
            .host
            .base_cx
            .get()
            .cloned()
            .ok_or_else(|| ServicesError::Failed("services context unavailable".into()))?;
        if let Some(id) = self.host.any_root_session() {
            cx.session_id = id;
        }
        cx.call_kind = CallKind::Service;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            extension
                .instance
                .handle_service("mcp.servers", json!({}), "oino.frontend", cx),
        )
        .await
        .map_err(|_| ServicesError::Failed("MCP status timed out".into()))?
        .map_err(|e| ServicesError::Failed(e.message))
    }

    /// Re-resolve saved trust and refresh resources and the live MCP registry in place.
    /// Session objects (including children) retain their shared settings/resources.
    ///
    /// # Errors
    /// Returns an error for trust-store failures or a failed or timed-out MCP reload.
    pub async fn reload_project(&self) -> Result<(), ServicesError> {
        let _reload = self.reload_lock.lock().await;
        let trusted = {
            let settings = lock(&self.settings);
            let home = oino_settings::paths::home_dir();
            let request = TrustRequest {
                cwd: &self.cwd,
                home: home.as_deref(),
                cli_override: self.trust_override,
                extension_decision: None,
                default_trust: settings.settings().default_project_trust(),
                has_ui: false,
            };
            matches!(
                resolve_project_trust(&request, &TrustStore::new(&self.agent_dir))
                    .map_err(|e| ServicesError::Trust(e.to_string()))?,
                TrustResolution::Decided { trusted: true, .. }
            )
        };
        let snapshot = {
            let mut settings = lock(&self.settings);
            settings.set_project_trusted(trusted);
            settings.reload();
            settings.snapshot()
        };
        self.resources
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reload(&snapshot, trusted);
        // Use a real session context so permission and trust queries resolve against
        // this services set, never whichever project happened to load most recently.
        if let Some(extension) = self.extensions.get("unipi.mcp") {
            let mut cx = self
                .host
                .base_cx
                .get()
                .cloned()
                .ok_or_else(|| ServicesError::Failed("services context unavailable".into()))?;
            if let Some(id) = self.host.any_root_session() {
                cx.session_id = id;
            }
            cx.call_kind = CallKind::Service;
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                extension.instance.handle_service(
                    "mcp.server-action",
                    json!({"action":"reload", "cwd":self.cwd}),
                    "oino.frontend",
                    cx,
                ),
            )
            .await
            .map_err(|_| ServicesError::Failed("MCP reload timed out".into()))?
            .map_err(|e| ServicesError::Failed(e.message))?;
        }
        // ResourceLoader::reload drops extension contributions: restore them once
        // through the normal resources_discover hook, then rebuild every prompt.
        if let Some(session) = self.host.root_sessions().into_iter().next() {
            session.extend_resources_from_extensions("reload").await;
        }
        for session in self.host.live_sessions() {
            session.refresh_tool_registry(None, false);
        }
        *lock(&self.mcp_mtimes) = mcp_mtimes(&self.cwd);
        Ok(())
    }

    /// On chat open, detect hand-edited project files and saved trust changes.
    ///
    /// # Errors
    /// Returns an error if trust cannot be read or the project reload fails.
    pub async fn refresh_project(&self) -> Result<(), ServicesError> {
        let changed = *lock(&self.mcp_mtimes) != mcp_mtimes(&self.cwd);
        let saved = TrustStore::new(&self.agent_dir)
            .get(&self.cwd)
            .map_err(|e| ServicesError::Trust(e.to_string()))?;
        let trust_changed =
            saved.is_some_and(|value| value != lock(&self.settings).is_project_trusted());
        if changed || trust_changed {
            self.reload_project().await?;
        }
        Ok(())
    }

    /// Startup (runtime §4): trust → settings/resources → extensions (`init`) → providers.
    ///
    /// # Errors
    ///
    /// Trust store failures.
    pub async fn create(mut options: ServicesOptions) -> Result<Arc<Self>, ServicesError> {
        let mut diagnostics = Vec::new();
        let (safe_mode, no_extensions) = safe_mode_start(&options.flags, &mut diagnostics);
        let mut settings = options
            .settings
            .take()
            .unwrap_or_else(|| SettingsManager::create(&options.cwd, &options.agent_dir, false));
        let trusted = resolve_trust(&options, settings.settings())?;
        settings.set_project_trusted(trusted);
        diagnostics.extend(settings.drain_errors().into_iter().map(|e| e.to_string()));
        let mut resources = project_resources(&options);
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
        set_sessions_root(&host, &settings, &options);
        let _ = host.models.set(models.clone());
        let _ = host.settings.set(Arc::clone(&extension_settings));
        let disabled = lock(&settings).settings().disabled_extensions();
        let instances: Vec<Arc<dyn ExtensionInstance>> = options
            .loader
            .load_for(
                Arc::clone(&host) as Arc<dyn HostApi>,
                &options.cwd,
                trusted,
                &disabled,
            )
            .await
            .into_iter()
            .filter(|instance| !disabled.iter().any(|id| id == instance.id()))
            .collect();
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
            no_extensions,
            &mut diagnostics,
        )
        .await;
        let extensions = Arc::new(ExtensionSet::new(loaded));
        let _ = host.extensions.set(Arc::downgrade(&extensions));
        models.set_oauth_refresher(Arc::new(ComponentOAuth {
            extensions: Arc::downgrade(&extensions),
            models: models.clone(),
        }));
        forward_settings_changes(&extension_settings, &host);
        start_model_refresh(&options, &extensions, &models).await;
        Ok(Arc::new(Self {
            cwd: options.cwd.clone(),
            agent_dir: options.agent_dir,
            docs_root: options.docs_root,
            mode: options.mode,
            has_ui: options.has_ui,
            project_trusted: trusted,
            trust_override: options.approve,
            mcp_mtimes: Mutex::new(mcp_mtimes(&options.cwd)),
            reload_lock: tokio::sync::Mutex::new(()),
            settings,
            extension_settings,
            resources: Arc::new(RwLock::new(resources)),
            models,
            stream: Arc::new(ProviderStream::new(Arc::clone(&extensions))),
            extensions,
            host,
            safe_mode,
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

fn set_sessions_root(
    host: &HostRouter,
    settings: &Mutex<SettingsManager>,
    options: &ServicesOptions,
) {
    *lock(&host.sessions_root) = Some(
        std::env::var_os("OINO_SESSION_DIR")
            .filter(|v| !v.is_empty())
            .map_or_else(
                || {
                    lock(settings).settings().session_dir().map_or_else(
                        || options.agent_dir.join("sessions"),
                        |dir| {
                            if dir.is_absolute() {
                                dir
                            } else {
                                options.cwd.join(dir)
                            }
                        },
                    )
                },
                PathBuf::from,
            ),
    );
}

fn project_resources(options: &ServicesOptions) -> ResourceLoader {
    ResourceLoader::new(ResourceLoaderOptions {
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
    })
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
            Ok(mut registration) => {
                // Render-only overrides of built-in tools need `tools.override` (U1–U2);
                // specs for extension tools and custom messages are always allowed.
                let overrides = instance.tool_overrides();
                registration.renderers.retain(|spec| {
                    let allowed = crate::session::check_render_override(spec, &overrides).is_ok();
                    if !allowed {
                        diagnostics.push(format!(
                            "{}: render spec {spec} ignored (tools.override)",
                            instance.id()
                        ));
                    }
                    allowed
                });
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

/// Deadline of one `oauth-refresh` call (15 s).
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
/// `/login` or a new API key); other providers are left alone. Public for the embedding
/// host's hub-level `set_api_key` (app spec §3.14 `host_cmd`, WP-A24).
pub async fn refresh_provider_models(
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
    let credential = discovery_credential(models, provider_id, credential);
    // An explicit refresh (new key, `/login`) runs now and restarts the backoff.
    oino_models::ListSchedule::reset(provider_id);
    let result = tokio::time::timeout(
        LIST_MODELS_TIMEOUT,
        provider.list_models(provider_id, credential),
    )
    .await;
    let ok = apply_list_result(models, provider_id, result);
    oino_models::ListSchedule::finish(provider_id, ok, std::time::Instant::now());
}

/// Upper bound for one provider's `list-models` call.
const LIST_MODELS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How often an app host's background refresher looks for due providers.
const REFRESH_TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// The startup model refresh of [`Services::create`]: awaited for terminal modes (pi
/// parity). An in-process app host (desktop/mobile) never probes on services creation or
/// wake (v3-perf §4.5): the picker answers from the registry, filled from the on-disk
/// [`oino_models::ModelCache`], and a background refresher re-lists each dynamic provider
/// only when [`oino_models::ListSchedule`] says it is due (1 h after a success; 1, 5, then
/// 30 min after failures such as a dead ollama/lmstudio). The schedule is process-wide, so
/// many services sets probe a provider once.
async fn start_model_refresh(
    options: &ServicesOptions,
    extensions: &Arc<ExtensionSet>,
    models: &ModelRegistry,
) {
    if options.flags.offline || offline_env() {
        return;
    }
    if matches!(options.mode, ExtensionMode::Desktop | ExtensionMode::Mobile) {
        spawn_model_refresh(Arc::downgrade(extensions), models.clone());
    } else {
        refresh_dynamic_models(extensions, models, false).await;
    }
}

/// The app-host background refresher: ticks until the services set is dropped and lists
/// only the providers the schedule says are due.
fn spawn_model_refresh(extensions: Weak<ExtensionSet>, models: ModelRegistry) {
    tokio::spawn(async move {
        loop {
            let Some(set) = extensions.upgrade() else {
                break;
            };
            refresh_dynamic_models(&set, &models, true).await;
            drop(set);
            tokio::time::sleep(REFRESH_TICK).await;
        }
    });
}

/// Records one `list-models` outcome in the registry (and its on-disk cache).
fn apply_list_result(
    models: &ModelRegistry,
    provider_id: &str,
    result: Result<
        Result<Vec<oino_ai::Model>, crate::extensions::api::ExtensionCallError>,
        tokio::time::error::Elapsed,
    >,
) -> bool {
    match result {
        Ok(Ok(list)) => {
            models.set_last_list_error(provider_id, None);
            if let Err(error) = models.set_provider_models(provider_id, &list) {
                tracing::warn!(provider = %provider_id, "storing models failed: {error}");
            }
            true
        }
        Ok(Err(error)) => {
            models.set_last_list_error(provider_id, Some(error.message.clone()));
            tracing::warn!(provider = %provider_id, "list-models failed: {}", error.message);
            false
        }
        Err(_) => {
            models.set_last_list_error(provider_id, Some("timed out".to_owned()));
            tracing::warn!(provider = %provider_id, "list-models timed out");
            false
        }
    }
}

/// Fetches the model lists of `models: "dynamic"` providers concurrently. Providers that need
/// host credentials are skipped only for built-in components; other extensions may hold their own.
/// `scheduled`: only providers [`oino_models::ListSchedule`] says are due (app hosts).
async fn refresh_dynamic_models(
    extensions: &ExtensionSet,
    models: &ModelRegistry,
    scheduled: bool,
) {
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
        if !should_probe(
            component.instance.id(),
            needs_key,
            key.is_some(),
            models.has_configured_auth(&provider_id),
        ) {
            continue;
        }
        if scheduled
            && !oino_models::ListSchedule::claim(
                &provider_id,
                models.cached_models_age(&provider_id),
                std::time::Instant::now(),
            )
        {
            continue;
        }
        let credential = discovery_credential(
            models,
            &provider_id,
            key.map(|key| json!({"type": "api_key", "key": key})),
        );
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
        let ok = apply_list_result(models, &provider_id, result);
        if scheduled {
            oino_models::ListSchedule::finish(&provider_id, ok, std::time::Instant::now());
        }
    }
}

// Optional list-models credential metadata; no WIT change, old providers ignore it.
fn discovery_credential(
    models: &ModelRegistry,
    id: &str,
    credential: Option<JsonValue>,
) -> Option<JsonValue> {
    let Some(url) = models.discovery_base_url(id) else {
        return credential;
    };
    let mut value = credential.unwrap_or_else(|| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("baseUrl".into(), json!(url));
    }
    Some(value)
}

// Extension instances have no origin flag; `oino.` identifies built-in providers.
fn should_probe(extension_id: &str, needs_key: bool, has_key: bool, configured: bool) -> bool {
    !extension_id.starts_with("oino.") || !needs_key || has_key || configured
}

#[cfg(test)]
mod dynamic_probe_tests {
    use super::should_probe;

    #[test]
    fn dynamic_probe_respects_extension_credentials() {
        assert!(should_probe("omniroute.bridge", true, false, false));
        assert!(!should_probe("oino.openai", true, false, false));
        assert!(should_probe("oino.openai", true, true, false));
        assert!(should_probe("oino.openai", true, false, true));
        assert!(should_probe("oino.local", false, false, false));
    }

    #[test]
    fn dynamic_probe_error_is_reported_and_cleared() {
        let models = oino_models::ModelRegistry::in_memory(std::sync::Arc::new(
            oino_models::env::ProcessEnv,
        ));
        let clone = models.clone();
        models.set_last_list_error("omniroute", Some("connection refused".into()));
        assert_eq!(
            clone.with_list_error("Model not found.".into(), Some("omniroute")),
            "Model not found. list-models for omniroute failed: connection refused"
        );
        models.set_last_list_error("omniroute", None);
        assert_eq!(clone.last_list_error("omniroute"), None);
    }
}
