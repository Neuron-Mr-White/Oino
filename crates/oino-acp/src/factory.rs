//! Creating session runtimes for ACP sessions.
//!
//! The adapter asks a [`SessionFactory`] for one [`AgentSessionRuntime`] per ACP session.
//! [`CoreSessionFactory`] is the standard implementation: per-cwd [`Services`] (created once
//! from caller-supplied [`ServicesOptions`] and shared by every session of that cwd) and the
//! built-in tools wired to the session's [`ExecutionEnv`] (the ACP client's fs/terminal).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use oino_ai::{Model, ThinkingLevel};
use oino_core::builtin::SessionToolContext;
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection};
use oino_core::services::{Services, ServicesOptions};
use oino_core::session::ToolEntry;
use oino_tools::env::ExecutionEnv;
use oino_tools::{ImageOptions, ShellOptions, ToolConfig, builtin_tool_infos, builtin_tools};
use tokio::sync::Mutex;

use crate::error::AcpError;

/// What the adapter needs to start one session.
pub struct SessionRequest {
    /// Session working directory (`session/new` / `session/load` `cwd`).
    pub cwd: PathBuf,
    /// New session or a session file to open.
    pub source: SessionSource,
    /// File system and process host for the built-in tools.
    pub env: Arc<dyn ExecutionEnv>,
}

impl std::fmt::Debug for SessionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRequest")
            .field("cwd", &self.cwd)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// Creates runtimes and answers where sessions of a cwd are stored.
pub trait SessionFactory: Send + Sync {
    /// Creates the runtime of one ACP session.
    fn create(
        &self,
        request: SessionRequest,
    ) -> BoxFuture<'_, Result<Arc<AgentSessionRuntime>, AcpError>>;

    /// The sessions root used for `cwd` (`session/list`, `session/load`, `session/delete`).
    fn sessions_root<'a>(&'a self, cwd: &'a Path) -> BoxFuture<'a, Result<PathBuf, AcpError>>;

    /// The agent directory (`~/.oino`); deleted sessions move to `<agent_dir>/trash`.
    fn agent_dir<'a>(&'a self, cwd: &'a Path) -> BoxFuture<'a, Result<PathBuf, AcpError>>;
}

/// Session defaults applied to every ACP session (resolved CLI flags).
#[derive(Clone, Debug, Default)]
pub struct SessionDefaults {
    /// `--session-dir`.
    pub session_dir: Option<PathBuf>,
    /// Explicit model.
    pub model: Option<Model>,
    /// Explicit thinking level.
    pub thinking_level: Option<ThinkingLevel>,
    /// Tool selection.
    pub tools: ToolSelection,
    /// `--exclude-tools`.
    pub exclude_tools: Vec<String>,
    /// Resolves `--model` / `--provider` / `--models` against a cwd's model registry when
    /// [`Self::model`] is not set (the registry exists only once that cwd's services do).
    pub model_resolver: Option<ModelResolver>,
}

/// Model choice of a new session: model, thinking level and `--models` scope.
#[derive(Clone, Debug, Default)]
pub struct ModelChoice {
    /// Initial model.
    pub model: Option<Model>,
    /// Thinking level from `provider/model:level`.
    pub thinking_level: Option<ThinkingLevel>,
    /// Cycling scope (`--models`).
    pub scoped_models: Vec<oino_models::resolver::ScopedModel>,
}

/// Resolves the [`ModelChoice`] of sessions in a cwd (see [`SessionDefaults::model_resolver`]).
#[derive(Clone)]
pub struct ModelResolver(pub Arc<dyn Fn(&Services) -> ModelChoice + Send + Sync>);

impl std::fmt::Debug for ModelResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModelResolver(..)")
    }
}

/// Builds the [`ServicesOptions`] of a cwd.
pub type ServicesOptionsFn = dyn Fn(&Path) -> ServicesOptions + Send + Sync;

/// The standard [`SessionFactory`] over `oino-core`.
pub struct CoreSessionFactory {
    options: Box<ServicesOptionsFn>,
    defaults: SessionDefaults,
    services: Mutex<HashMap<PathBuf, Arc<Services>>>,
}

impl std::fmt::Debug for CoreSessionFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreSessionFactory")
            .field("defaults", &self.defaults)
            .finish_non_exhaustive()
    }
}

impl CoreSessionFactory {
    /// A factory creating services with `options(cwd)`; `mode` is forced to
    /// [`ExtensionMode::Acp`] and `has_ui` to `true` (dialogs reach the client).
    #[must_use]
    pub fn new(
        options: impl Fn(&Path) -> ServicesOptions + Send + Sync + 'static,
        defaults: SessionDefaults,
    ) -> Self {
        Self {
            options: Box::new(options),
            defaults,
            services: Mutex::new(HashMap::new()),
        }
    }

    /// The shared services of `cwd`, created on first use.
    ///
    /// # Errors
    ///
    /// [`AcpError::Factory`] when services cannot start (trust store, settings).
    pub async fn services(&self, cwd: &Path) -> Result<Arc<Services>, AcpError> {
        let mut cache = self.services.lock().await;
        if let Some(services) = cache.get(cwd) {
            return Ok(Arc::clone(services));
        }
        let mut options = (self.options)(cwd);
        options.cwd = cwd.to_path_buf();
        options.mode = ExtensionMode::Acp;
        options.has_ui = true;
        let services = Services::create(options)
            .await
            .map_err(|e| AcpError::Factory(e.to_string()))?;
        for diagnostic in &services.diagnostics {
            tracing::warn!("{diagnostic}");
        }
        cache.insert(cwd.to_path_buf(), Arc::clone(&services));
        Ok(services)
    }
}

/// Every built-in tool bound to `context`, running through `env`
/// (`oino_core::builtin::builtin_tool_entries` with a custom environment).
#[must_use]
pub fn builtin_tool_entries_with_env(
    services: &Services,
    context: Arc<SessionToolContext>,
    env: Arc<dyn ExecutionEnv>,
) -> Vec<ToolEntry> {
    let settings = services
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot();
    let mut config = ToolConfig::new(
        services.cwd.display().to_string(),
        services.agent_dir.display().to_string(),
    );
    config.env = env;
    config.context = context;
    config.images = ImageOptions {
        auto_resize: settings.image_settings().auto_resize,
        fallback_resize: None,
    };
    config.shell = ShellOptions {
        shell_path: settings.shell_path().map(|p| p.display().to_string()),
        command_prefix: settings.shell_command_prefix().map(str::to_owned),
        expose_session_environment: true,
    };
    let infos = builtin_tool_infos(&config);
    builtin_tools(&config)
        .into_iter()
        .zip(infos)
        .map(|(tool, info)| {
            ToolEntry::builtin(
                tool,
                info.prompt_snippet.map(str::to_owned),
                info.prompt_guidelines.unwrap_or_default(),
            )
        })
        .collect()
}

impl SessionFactory for CoreSessionFactory {
    fn create(
        &self,
        request: SessionRequest,
    ) -> BoxFuture<'_, Result<Arc<AgentSessionRuntime>, AcpError>> {
        Box::pin(async move {
            let services = self.services(&request.cwd).await?;
            let context = Arc::new(SessionToolContext::default());
            let builtin_tools =
                builtin_tool_entries_with_env(&services, Arc::clone(&context), request.env);
            let choice = match (&self.defaults.model, &self.defaults.model_resolver) {
                (None, Some(resolver)) => (resolver.0)(&services),
                (model, _) => ModelChoice {
                    model: model.clone(),
                    ..ModelChoice::default()
                },
            };
            let options = SessionOptions {
                source: request.source,
                session_dir: self.defaults.session_dir.clone(),
                name: None,
                model: choice.model,
                thinking_level: self.defaults.thinking_level.or(choice.thinking_level),
                scoped_models: choice.scoped_models,
                tools: self.defaults.tools.clone(),
                exclude_tools: self.defaults.exclude_tools.clone(),
                builtin_tools,
                custom_tools: Vec::new(),
            };
            let runtime = AgentSessionRuntime::create(services, options)
                .await
                .map_err(|e| AcpError::Factory(e.to_string()))?;
            context.bind(&runtime.session());
            Ok(Arc::new(runtime))
        })
    }

    fn sessions_root<'a>(&'a self, cwd: &'a Path) -> BoxFuture<'a, Result<PathBuf, AcpError>> {
        Box::pin(async move {
            let services = self.services(cwd).await?;
            Ok(services.sessions_root(self.defaults.session_dir.as_deref()))
        })
    }

    fn agent_dir<'a>(&'a self, cwd: &'a Path) -> BoxFuture<'a, Result<PathBuf, AcpError>> {
        Box::pin(async move { Ok(self.services(cwd).await?.agent_dir.clone()) })
    }
}
