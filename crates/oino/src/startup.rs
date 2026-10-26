//! pi `main.ts` startup: services, session selection, model/tool options, runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oino_ai::{JsonValue, Model, ThinkingLevel};
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::extensions::api::{ExtensionMode, FlagType};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection};
use oino_core::services::{ResourceFlags, Services, ServicesOptions};
use oino_models::resolver::{
    CliModelRequest, ModelCatalog, ScopedModel, resolve_cli_model_in, resolve_model_scope_in,
};
use oino_session::SessionManager;

use crate::cli::args::{Args, Diagnostic, normalize_session_name};
use crate::cli::help::ExtensionFlag;
use crate::loader::{CliLoader, EXTENSION_LOAD_FAILURE_HINT, LoaderContext};

fn settings_snapshot(services: &Services) -> oino_settings::Settings {
    services
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot()
}

/// Resolved app mode (pi `resolveAppMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMode {
    /// Terminal UI.
    Interactive,
    /// `-p` / `--mode text`.
    Print,
    /// `--mode json`.
    Json,
    /// `--mode rpc`.
    Rpc,
    /// `--mode acp`.
    Acp,
}

impl AppMode {
    /// pi `resolveAppMode`.
    #[must_use]
    pub fn resolve(args: &Args, stdin_tty: bool, stdout_tty: bool) -> Self {
        use crate::cli::args::OutputMode;
        match args.mode {
            Some(OutputMode::Rpc) => Self::Rpc,
            Some(OutputMode::Json) => Self::Json,
            Some(OutputMode::Acp) => Self::Acp,
            _ if args.print || !stdin_tty || !stdout_tty => Self::Print,
            _ => Self::Interactive,
        }
    }

    const fn extension_mode(self) -> ExtensionMode {
        match self {
            Self::Interactive => ExtensionMode::Tui,
            Self::Print => ExtensionMode::Print,
            Self::Json => ExtensionMode::Json,
            Self::Rpc => ExtensionMode::Rpc,
            Self::Acp => ExtensionMode::Acp,
        }
    }
}

/// A startup failure: the lines to print to stderr, then exit 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupError(pub Vec<String>);

impl StartupError {
    fn one(message: impl Into<String>) -> Self {
        Self(vec![message.into()])
    }
}

/// Everything a mode needs.
pub struct Started {
    /// The runtime.
    pub runtime: Arc<AgentSessionRuntime>,
    /// Diagnostics to report before running (warnings only; errors already failed).
    pub diagnostics: Vec<Diagnostic>,
    /// Extension flags (for `--help`).
    pub extension_flags: Vec<ExtensionFlag>,
    /// Sessions directory (interactive session picker).
    pub sessions_root: PathBuf,
    /// Project trust decision (interactive settings writes).
    pub project_trusted: bool,
}

impl std::fmt::Debug for Started {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Started")
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

/// pi `validateForkFlags` + `validateSessionIdFlags`.
///
/// # Errors
///
/// pi's conflict messages.
pub fn validate_session_flags(args: &Args) -> Result<(), StartupError> {
    let flags = |pairs: &[(bool, &str)]| -> Vec<String> {
        pairs
            .iter()
            .filter(|(set, _)| *set)
            .map(|(_, name)| (*name).to_owned())
            .collect()
    };
    if args.fork.is_some() {
        let conflicts = flags(&[
            (args.session.is_some(), "--session"),
            (args.continue_session, "--continue"),
            (args.resume, "--resume"),
            (args.no_session, "--no-session"),
        ]);
        if !conflicts.is_empty() {
            return Err(StartupError::one(format!(
                "Error: --fork cannot be combined with {}",
                conflicts.join(", ")
            )));
        }
    }
    if let Some(id) = &args.session_id {
        let conflicts = flags(&[
            (args.session.is_some(), "--session"),
            (args.continue_session, "--continue"),
            (args.resume, "--resume"),
        ]);
        if !conflicts.is_empty() {
            return Err(StartupError::one(format!(
                "Error: --session-id cannot be combined with {}",
                conflicts.join(", ")
            )));
        }
        oino_session::manager::validate_session_id(id)
            .map_err(|e| StartupError::one(format!("Error: {e}")))?;
    }
    Ok(())
}

enum Resolved {
    Path(PathBuf),
    Global(PathBuf, String),
    NotFound,
}

/// pi `resolveSessionPath`.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "pi matches `.jsonl` case-sensitively"
)]
fn resolve_session_path(arg: &str, cwd: &Path, root: &Path) -> Resolved {
    if arg.contains('/') || arg.contains('\\') || arg.ends_with(".jsonl") {
        let home = oino_settings::paths::home_dir();
        return Resolved::Path(oino_settings::paths::resolve_path(
            arg,
            cwd,
            home.as_deref(),
            false,
        ));
    }
    if let Ok(Some(path)) = SessionManager::find_by_id(cwd, arg, root)
        && path.parent() == Some(&oino_session::manager::cwd_session_dir(root, cwd))
    {
        return Resolved::Path(path);
    }
    if let Ok(local) = SessionManager::list(cwd, root, None, None)
        && let Some(found) = local.iter().find(|s| s.id.starts_with(arg))
    {
        return Resolved::Path(found.path.clone());
    }
    if let Ok(all) = SessionManager::list_all(root, None, None)
        && let Some(found) = all
            .iter()
            .find(|s| s.id == arg)
            .or_else(|| all.iter().find(|s| s.id.starts_with(arg)))
    {
        return Resolved::Global(found.path.clone(), found.cwd.clone());
    }
    Resolved::NotFound
}

/// pi `createSessionManager` as a [`SessionSource`].
fn session_source(
    args: &Args,
    metadata_only: bool,
    cwd: &Path,
    root: &Path,
) -> Result<SessionSource, StartupError> {
    if args.no_session || metadata_only {
        return Ok(SessionSource::InMemory);
    }
    let not_found = |arg: &str| StartupError::one(format!("No session found matching '{arg}'"));
    if let Some(fork) = &args.fork {
        if args.session_id.is_some() {
            return Err(StartupError::one(
                "Error: --fork with --session-id is not supported yet",
            ));
        }
        return match resolve_session_path(fork, cwd, root) {
            Resolved::Path(path) | Resolved::Global(path, _) => Ok(SessionSource::Fork(path)),
            Resolved::NotFound => Err(not_found(fork)),
        };
    }
    if let Some(session) = &args.session {
        return match resolve_session_path(session, cwd, root) {
            Resolved::Path(path) => Ok(SessionSource::Open(path)),
            Resolved::Global(_, other) => Err(StartupError(vec![
                format!("Session found in different project: {other}"),
                "Use --fork to copy it into the current directory.".to_owned(),
            ])),
            Resolved::NotFound => Err(not_found(session)),
        };
    }
    if args.resume {
        return Err(StartupError::one(
            "Error: --resume needs the interactive session picker; use --session <id> or --continue",
        ));
    }
    if args.continue_session {
        return Ok(SessionSource::ContinueRecent);
    }
    if let Some(id) = &args.session_id {
        return match SessionManager::find_by_id(cwd, id, root) {
            Ok(Some(path)) => Ok(SessionSource::Open(path)),
            _ => Err(StartupError::one(format!(
                "Error: No project session found with id '{id}'; creating sessions with a given id is not supported yet"
            ))),
        };
    }
    Ok(SessionSource::New)
}

/// Whether the selected session already has messages (pi `hasExistingSession`).
fn has_existing_session(source: &SessionSource, cwd: &Path, root: &Path) -> bool {
    let path = match source {
        SessionSource::Open(path) | SessionSource::Fork(path) => Some(path.clone()),
        SessionSource::ContinueRecent => SessionManager::find_most_recent(cwd, root).ok().flatten(),
        SessionSource::Resume(_) | SessionSource::New | SessionSource::InMemory => None,
    };
    path.and_then(|p| SessionManager::open(&p, root, Some(cwd)).ok())
        .is_some_and(|m| !m.build_session_context().messages.is_empty())
}

struct ModelChoice {
    model: Option<Model>,
    thinking: Option<ThinkingLevel>,
    scoped: Vec<ScopedModel>,
}

/// pi `buildSessionOptions` model part.
fn choose_model(
    args: &Args,
    services: &Services,
    has_existing: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> ModelChoice {
    let models = &services.models;
    let settings = settings_snapshot(services);
    let mut choice = ModelChoice {
        model: None,
        thinking: None,
        scoped: Vec::new(),
    };
    if let (Some(provider), None) = (&args.provider, &args.model) {
        diagnostics.push(Diagnostic::error(format!(
            "--provider requires --model (for example: --provider {provider} --model <pattern>)"
        )));
    }
    if args.model.is_some() {
        let request = CliModelRequest {
            provider: args.provider.as_deref(),
            model: args.model.as_deref(),
            thinking: args.thinking,
            inline_api_key: args.api_key.is_some(),
        };
        match resolve_cli_model_in(&request, models) {
            Ok(Some(resolved)) => {
                if let Some(warning) = resolved.warning {
                    diagnostics.push(Diagnostic::warning(warning));
                }
                if args.thinking.is_none() {
                    choice.thinking = resolved.thinking_level;
                }
                choice.model = Some(resolved.model);
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(Diagnostic::error(error.0)),
        }
    }
    let patterns = args.models.clone().or_else(|| settings.enabled_models());
    if let Some(patterns) = patterns.filter(|p| !p.is_empty()) {
        let scope = resolve_model_scope_in(&patterns, models);
        diagnostics.extend(
            scope
                .diagnostics
                .into_iter()
                .map(|d| Diagnostic::warning(d.message)),
        );
        choice.scoped = scope.scoped_models;
    }
    if choice.model.is_none() && !choice.scoped.is_empty() && !has_existing {
        let saved = settings
            .default_provider()
            .zip(settings.default_model())
            .and_then(|(p, id)| ModelCatalog::find(models, p, id));
        let pick = saved
            .and_then(|saved| {
                choice
                    .scoped
                    .iter()
                    .find(|s| s.model.provider == saved.provider && s.model.id == saved.id)
            })
            .or_else(|| choice.scoped.first())
            .cloned();
        if let Some(pick) = pick {
            if args.thinking.is_none() {
                choice.thinking = pick.thinking_level;
            }
            choice.model = Some(pick.model);
        }
    }
    if args.thinking.is_some() {
        choice.thinking = args.thinking;
    }
    choice
}

/// The bundled docs directory (`{docs_root}` of the system prompt); the agent dir when the
/// docs cannot be extracted.
pub(crate) fn docs_root(agent_dir: &Path) -> PathBuf {
    oino_bundle::DOCS
        .extract(agent_dir)
        .unwrap_or_else(|_| agent_dir.to_path_buf())
}

/// pi resource flags from the command line.
pub(crate) fn resource_flags(args: &Args, cwd: &Path) -> ResourceFlags {
    ResourceFlags {
        no_skills: args.no_skills,
        no_prompt_templates: args.no_prompt_templates,
        no_themes: args.no_themes,
        no_context_files: args.no_context_files,
        skill_paths: resolve_cli_paths(cwd, args.skills.as_ref()),
        prompt_template_paths: resolve_cli_paths(cwd, args.prompt_templates.as_ref()),
        theme_paths: resolve_cli_paths(cwd, args.themes.as_ref()),
        system_prompt: args.system_prompt.clone(),
        append_system_prompt: args.append_system_prompt.clone(),
        no_extensions: args.no_extensions,
        offline: args.offline,
    }
}

/// Tool selection from `--tools`, `--no-tools`, `--no-builtin-tools`.
pub(crate) fn tool_selection(args: &Args) -> ToolSelection {
    match &args.tools {
        Some(tools) => ToolSelection::List(tools.clone()),
        None if args.no_tools => ToolSelection::None,
        None if args.no_builtin_tools => ToolSelection::NoBuiltin,
        None => ToolSelection::Default,
    }
}

fn resolve_cli_paths(cwd: &Path, paths: Option<&Vec<String>>) -> Vec<String> {
    let home = oino_settings::paths::home_dir();
    paths
        .into_iter()
        .flatten()
        .map(|value| {
            if value.contains("://") || value.starts_with("npm:") || value.starts_with("git:") {
                value.clone()
            } else {
                oino_settings::paths::resolve_path(value, cwd, home.as_deref(), false)
                    .display()
                    .to_string()
            }
        })
        .collect()
}

/// pi `applyExtensionFlagValues` checks over the loaded extensions.
fn check_extension_flags(
    args: &Args,
    services: &Services,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<ExtensionFlag>, Vec<String>) {
    let mut registered = Vec::new();
    for extension in services.extensions.extensions() {
        for flag in extension.state().flags.into_values() {
            registered.push(ExtensionFlag {
                extension_path: extension.instance.path(),
                flag,
            });
        }
    }
    let mut unknown = Vec::new();
    let mut booleans = Vec::new();
    for (name, value) in &args.unknown_flags {
        match registered.iter().find(|f| &f.flag.name == name) {
            None => unknown.push(format!("--{name}")),
            Some(flag) if flag.flag.kind == FlagType::Boolean => booleans.push(name.clone()),
            Some(_) if value.is_string() => {}
            Some(_) => diagnostics.push(Diagnostic::error(format!(
                "Extension flag \"--{name}\" requires a value"
            ))),
        }
    }
    if !unknown.is_empty() {
        diagnostics.push(Diagnostic::error(format!(
            "Unknown option{}: {}",
            if unknown.len() == 1 { "" } else { "s" },
            unknown.join(", ")
        )));
    }
    (registered, booleans)
}

/// Startup inputs that do not come from `argv`.
#[derive(Clone, Debug)]
pub struct Environment {
    /// Process cwd.
    pub cwd: PathBuf,
    /// Agent dir.
    pub agent_dir: PathBuf,
}

/// Creates services and the runtime (pi `createRuntime` + `createAgentSessionRuntime`).
///
/// # Errors
///
/// Startup diagnostics with errors, session resolution failures, missing models.
#[allow(
    clippy::too_many_lines,
    reason = "linear port of pi main.ts startup; splitting hides the order"
)]
pub async fn start(args: &Args, mode: AppMode, env: &Environment) -> Result<Started, StartupError> {
    let metadata_only = args.help || args.list_models.is_some();
    let loader = Arc::new(CliLoader::new(
        args.extensions.clone().unwrap_or_default(),
        &LoaderContext {
            agent_dir: env.agent_dir.clone(),
            cwd: env.cwd.clone(),
            no_extensions: args.no_extensions,
        },
    ));
    let mut diagnostics: Vec<Diagnostic> = loader.diagnostics().to_vec();
    let services = Services::create(ServicesOptions {
        cwd: env.cwd.clone(),
        agent_dir: env.agent_dir.clone(),
        docs_root: docs_root(&env.agent_dir),
        mode: mode.extension_mode(),
        has_ui: matches!(mode, AppMode::Rpc | AppMode::Interactive),
        flags: resource_flags(args, &env.cwd),
        extension_flag_values: args.unknown_flags.clone(),
        loader: Arc::clone(&loader) as Arc<dyn oino_core::services::ExtensionLoader>,
        approve: args.project_trust_override,
        models: None,
        settings: None,
    })
    .await
    .map_err(|e| StartupError::one(format!("Error: {e}")))?;
    diagnostics.extend(loader.load_errors());
    diagnostics.extend(
        services
            .diagnostics
            .iter()
            .map(|d| Diagnostic::warning(d.clone())),
    );
    let (extension_flags, boolean_flags) = check_extension_flags(args, &services, &mut diagnostics);
    let root = services.sessions_root(args.session_dir.as_deref().map(Path::new));
    let project_trusted = services.project_trusted;
    let source = session_source(args, metadata_only, &env.cwd, &root)?;
    let has_existing = has_existing_session(&source, &env.cwd, &root);
    let choice = choose_model(args, &services, has_existing, &mut diagnostics);
    if let Some(key) = &args.api_key {
        match &choice.model {
            Some(model) => services.models.set_runtime_api_key(&model.provider, key.clone()),
            None => diagnostics.push(Diagnostic::error(
                "--api-key requires a model to be specified via --model, --provider/--model, or --models",
            )),
        }
    }
    let name = match &args.name {
        Some(raw) => Some(
            normalize_session_name(raw)
                .ok_or_else(|| StartupError::one("Error: --name requires a non-empty value"))?,
        ),
        None => None,
    };
    let context = Arc::new(SessionToolContext::default());
    let settings = settings_snapshot(&services);
    let builtin_tools = builtin_tool_entries(
        &services.cwd,
        &services.agent_dir,
        &settings,
        Arc::clone(&context),
    );
    let options = SessionOptions {
        source,
        session_dir: args.session_dir.as_ref().map(PathBuf::from),
        name,
        model: choice.model,
        thinking_level: choice.thinking,
        scoped_models: choice.scoped,
        tools: tool_selection(args),
        exclude_tools: args.exclude_tools.clone().unwrap_or_default(),
        builtin_tools,
        custom_tools: Vec::new(),
    };
    let runtime = AgentSessionRuntime::create(Arc::clone(&services), options)
        .await
        .map_err(|e| StartupError::one(format!("Error: {e}")))?;
    let session = runtime.session();
    context.bind(&session);
    let runner = session.runner();
    for name in boolean_flags {
        runner.set_flag_value(&name, JsonValue::Bool(true));
    }
    if metadata_only {
        return Ok(Started {
            runtime: Arc::new(runtime),
            diagnostics,
            extension_flags,
            sessions_root: root,
            project_trusted,
        });
    }
    if diagnostics
        .iter()
        .any(|d| d.severity == crate::cli::args::Severity::Error)
    {
        let mut lines = crate::report_lines(&diagnostics);
        if diagnostics
            .iter()
            .any(|d| d.message.contains("Failed to load extension"))
        {
            lines.push(EXTENSION_LOAD_FAILURE_HINT.to_owned());
        }
        runtime.dispose().await;
        return Err(StartupError(lines));
    }
    if mode != AppMode::Interactive && session.model().is_none() {
        let message = runtime.model_fallback_message().map_or_else(
            || {
                oino_models::auth::guidance::no_models_available_message(
                    &services.docs_root.join("docs"),
                )
            },
            str::to_owned,
        );
        let mut lines = crate::report_lines(&diagnostics);
        lines.push(message);
        runtime.dispose().await;
        return Err(StartupError(lines));
    }
    Ok(Started {
        runtime: Arc::new(runtime),
        diagnostics,
        extension_flags,
        sessions_root: root,
        project_trusted,
    })
}
