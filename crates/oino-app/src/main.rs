#![doc = r#"Oino binary runtime wiring.

This binary crate is the composition layer for the Oino executable. It owns
process-level concerns: CLI argument parsing, settings/auth/session/resource
bootstrap, OpenRouter provider construction, extension package lifecycle wiring,
terminal setup/teardown, TUI event dispatch, non-interactive command execution,
model-cache refresh, chat export, and platform file/URL opening.

## Startup map

- `CliArgs` and `AppConfig` read process arguments, environment overrides, and
  persisted user settings.
- `ResourcePaths`, `load_resource_catalog`, and `default_system_prompt` connect
  Oino-owned files to the provider-facing system prompt while leaving resource
  discovery rules in `oino-resource`.
- `load_or_create_session`, `new_tui_session`, and session open/list helpers wire
  JSONL sessions without owning the session tree format.
- `build_harness` binds model, provider, auth resolver, tools, resources, and
  session state through `oino-harness`.
- `load_extension_snapshot`, package lifecycle helpers, and `apply_*_to_state`
  functions translate extension-manager snapshots into TUI/runtime state.
- `run_tui` owns the crossterm/Ratatui lifecycle, event loop, stream hooks,
  queued-prompt scheduling, settings persistence, and terminal cleanup guard.
- `run_non_interactive` executes shell commands or one-shot prompts and prints
  shell-safe output.

## Contributor rules

Keep domain behavior in the owning crate: provider HTTP in provider crates,
session structure in `oino-session`, resource discovery in `oino-resource`,
headless agent/session orchestration in `oino-harness`, UI state/rendering in
`oino-tui`, and extension policy/package rules in extension crates. Changes in
this crate should usually be glue that keeps those layers synchronized. When
adding runtime behavior, update both TUI and non-interactive paths when relevant,
preserve terminal cleanup on every exit path, and add targeted tests around the
wiring edge that changed.
"#]
#![forbid(unsafe_code)]

mod ask_user;
mod auth_readiness;
mod extension_provider_runtime;
mod extension_readiness;
mod llm_compact;
mod model_catalog;
mod notify;
mod provider_runtime;
mod ralph_loop;
mod router;
mod usage;
mod user_settings;
mod vcc;

use ask_user::{AskUserRequester, AskUserTool, ASK_USER_TOOL_NAME};
use async_trait::async_trait;
use auth_readiness::{format_auth_status, quickstart as format_auth_quickstart};
use crossterm::{
    cursor::MoveTo,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEvent,
        MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute, queue,
    style::{
        Attribute as CAttribute, Color as CColor, Print, ResetColor, SetAttribute,
        SetForegroundColor,
    },
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
#[cfg(test)]
use extension_provider_runtime::{
    extension_config_dir_name, extension_config_string_from_value,
    extension_runtime_base_url_env_candidates,
};
use extension_provider_runtime::{extension_runtime_providers, ExtensionRuntimeHealth};
#[cfg(test)]
use extension_readiness::status_items as extension_readiness_status_items;
use extension_readiness::{
    provider_matches as extension_readiness_provider_matches,
    status_items_with_health as extension_readiness_status_items_with_health,
};
use model_catalog::ModelCatalogUpdate;
use oino_agent_loop::{
    AbortSignal, AgentEvent, BeforeToolCallResult, BoxFuture, LoopError, StreamProvider,
    StreamRequest, Tool, ToolCall, ToolDefinition, ToolResult, ToolUpdateCallback,
};
use oino_auth::{AuthError, AuthStorage, ProviderAuthSpec};
#[cfg(test)]
use oino_extension_core::ProviderContribution;
use oino_extension_core::{
    ActiveContribution, ContributionId, ContributionMetadata, DiagnosticContribution, ExtensionId,
    HealthContribution, PackageId, PackageManifest, PolicyToggle, RegistryEntry, RegistryEntryKey,
    RegistryPolicy, RendererContribution, RendererTarget, ResourceKind, SourceScope, UiFocusPolicy,
    UiKeyDispatchPolicy, UiLayoutPolicy, UiSurfaceContribution, UiSurfaceKind,
    UiTinyTerminalFallback, UiVisibilityPolicy,
};
use oino_extension_manager::{
    ExtensionDiscovery, ExtensionLayoutPaths, ExtensionManager, ExtensionManagerConfig,
    ExtensionManagerSnapshot, PackageInstallScope, PackageLifecycleError, PackageLifecycleService,
};
use oino_extension_runtime::{
    ExtensionCommandAdapter, ExtensionRuntime, ExtensionToolAdapter, FixtureHandlerBehavior,
    FixtureWasmModule, JsonWasmRuntime, RuntimeInitialize, RuntimeProgress, SharedRuntime,
    WASM_JSON_V1_ABI,
};
use oino_harness::{AuthResolver, Harness, HarnessConfig, HarnessError, NotificationHook};
use oino_provider_catalog::{provider_by_id, resolve_provider, ProviderDescriptor};
use oino_provider_openrouter::OpenRouterConfig;
use oino_resource::{PromptTemplate, ResourceCatalog, ResourcePaths, Skill};
use oino_session::{SessionHeader, SessionManager, SessionRepository};
use oino_tui::{
    collapse_mode_value, collapse_target_value, format_command_help, parse_command, render,
    terminal_cursor_position, transcript_click_targets, transcript_url_overlays,
    transcript_visible_lines, AgentMode, AskUserOutcome, AskUserRequest, AuthStatusItem,
    CollapseMode, CompactMethodOverride, ExtensionAutosuggestItem, ExtensionCommandSuggestion,
    ExtensionManagementItem, ExtensionManagementTarget, ExtensionShortcut, ExtensionThemeState,
    KeySequence, KeymapConfig, MessageView, ModelOption, NotifyEventKind as TuiNotifyEventKind,
    NotifyField, NotifyScopeSettings, ParsedCommand, PromptResource, RalphCommand,
    RalphRecordPromise, SessionListItem, SettingsCommand, SkillResource, TerminalClickTarget,
    TerminalUrlOverlay, ThemeCatalog, ThemeCatalogEntry, ThemeDocument, ThemeSource,
    ThemeSourceKind, ThemeSourceScope, ToolSettingsItem, ToolSettingsScope, TuiAction, TuiState,
    HELP_STATUS,
};
use oino_types::{AssistantStreamEvent, ContentBlock, Message, Model, OinoId, ThinkingLevel};
#[cfg(test)]
use provider_runtime::ProviderRouter;
use provider_runtime::{build_runtime_provider, provider_status_for_model_identifier};
use ratatui::{backend::CrosstermBackend, Terminal};
use router::{execute_router_command_input, load_router_config, resolved_router_base_url};
#[cfg(test)]
use router::{
    format_extension_readiness_detail as format_router_extension_readiness_detail,
    resolved_router_tag, router_managed_run_command, router_run_args, router_start_candidates,
    validate_router_tag, RouterConfig, RouterHealth, ROUTER_DEFAULT_BASE_URL,
    ROUTER_KNOWN_GOOD_TAG,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Stdout, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use usage::{account_usage_progress_placeholder, UsageReport};
use user_settings::UserSettings;

const DEFAULT_MODEL: &str = "router:kr/claude-sonnet-4.5";
#[cfg(test)]
const OPENAI_ACCESS_TOKEN_ENV: &str = "OPENAI_ACCESS_TOKEN";
#[cfg(test)]
const OPENAI_REFRESH_TOKEN_ENV: &str = "OPENAI_REFRESH_TOKEN";
#[cfg(test)]
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliArgs {
    show_help: bool,
    settings: bool,
    model: Option<String>,
    session: Option<OinoId>,
    input: Option<String>,
}

impl CliArgs {
    fn parse_from<I, S>(args: I) -> Result<Self, AppError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = args.into_iter().map(Into::into).peekable();
        let mut parsed = Self {
            show_help: false,
            settings: false,
            model: None,
            session: None,
            input: None,
        };
        let mut input_parts = Vec::new();
        while let Some(arg) = args.next() {
            if !input_parts.is_empty() {
                input_parts.push(arg);
                continue;
            }
            match arg.as_str() {
                "--help" | "-h" => parsed.show_help = true,
                "--settings" => parsed.settings = true,
                "--model" => {
                    let Some(value) = args.next() else {
                        return Err(AppError::InvalidArguments(
                            "--model requires a value".into(),
                        ));
                    };
                    parsed.model = Some(value);
                }
                "--session" => {
                    let Some(value) = args.next() else {
                        return Err(AppError::InvalidArguments(
                            "--session requires a uuid".into(),
                        ));
                    };
                    parsed.session = Some(value.parse().map_err(|_| {
                        AppError::InvalidArguments(format!("invalid session uuid `{value}`"))
                    })?);
                }
                value if value.starts_with('-') => {
                    return Err(AppError::InvalidArguments(format!(
                        "unknown flag `{value}`"
                    )));
                }
                value => input_parts.push(value.to_string()),
            }
        }
        if !input_parts.is_empty() {
            let input = input_parts.join(" ");
            parsed.input = Some(normalize_cli_command_input(&input));
        }
        if parsed.model.is_some() {
            parsed.settings = true;
        }
        Ok(parsed)
    }
}

fn normalize_cli_command_input(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with('/') {
        return trimmed.to_string();
    }
    let head = trimmed
        .split_once(char::is_whitespace)
        .map_or(trimmed, |(head, _)| head);
    match head {
        "settings" | "model" | "thinking" | "theme" | "extensions" | "auth" | "account"
        | "usage" | "prompts" | "skills" | "reload" | "inspect" | "compact" | "recall"
        | "ralph" | "mode" | "btw" | "new" | "sessions" | "help" | "title" | "router" => {
            format!("/{trimmed}")
        }
        _ => trimmed.to_string(),
    }
}

fn usage() -> &'static str {
    "Usage:\n  oino\n  oino --settings --model router:kr/claude-sonnet-4.5\n  oino --session <uuid> <message-or-command>\n  oino settings notify enabled true\n\nCommands:\n  /new\n  /btw | /btw new | /btw configure inherit|<provider:model>\n  /sessions\n  /settings\n  /theme\n  /extensions | /extensions update\n  /router setup|guide|status|models|stop|restart    (enabled builtin:router extension)\n  /router version list|pin <tag> | /router rollback [tag]\n  /auth [provider]   (extension readiness/status)\n  /account [provider]\n  /usage\n  /prompts\n  /skills\n  /reload                 (resources, extensions, tools, themes, file index)\n  /inspect\n  /compact                        (compact session with configured method)
  /compact vcc | /compact llm     (override method for one-shot)
  /compact threshold [pct]        (set/show auto-compact threshold %)
  /compact auto <on|off>           (enable/disable auto-compact)
  /compact model [inherit|<m>]     (set/show LLM compact model)
  /compact prompt [path]           (set/show LLM compact prompt)\n  /recall [query]\n  /ralph help | /ralph start <name> <task> | /ralph continue [name]\n  /mode <profile>\n  /prompt:<name>\n  /skill:<name>\n  /model [provider:model-id] | /model btw inherit|<provider:model> | /model notify-summary inherit|<provider:model>\n  /thinking [off|minimal|low|medium|high|xhigh]\n  /title <session-title>\n  /settings model <provider:model-id>\n  /settings thinking <off|minimal|low|medium|high|xhigh>\n  /settings collapse <thinking|tool> <full|truncate|collapse>\n  /settings chat-style <chat|agentic|minimal>\n  /settings tools\n  /settings auth\n  /settings keymaps\n  /settings theme\n  /settings extensions\n  /settings notify [project|global] <field> <value>\n    fields: enabled, server, topic, token, priority, tags, agent_end, tool_error, summary_enabled, summary_model, summary_prompt, summary_max_chars\n\nOptional built-ins: install from /extensions with builtin:router, builtin:footer-status, builtin:ralph-loop, builtin:mode-sandbox, builtin:notify, builtin:craft-skill, builtin:vcc, or builtin:ask-user"
}

#[derive(Debug, Error)]
enum AppError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Provider(#[from] oino_provider_openrouter::OpenRouterError),
    #[error(transparent)]
    Harness(#[from] HarnessError),
    #[error(transparent)]
    Session(#[from] oino_session::SessionError),
    #[error(transparent)]
    Resource(#[from] oino_resource::ResourceError),
    #[error(transparent)]
    Ralph(#[from] ralph_loop::RalphLoopError),
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("invalid model identifier `{0}`; expected provider:model-id")]
    InvalidModelIdentifier(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppConfig {
    model: String,
    thinking_level: ThinkingLevel,
    thinking_collapse_mode: CollapseMode,
    tool_collapse_mode: CollapseMode,
    chat_style: oino_tui::ChatStyle,
    keymap: KeymapConfig,
    referer: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Clone)]
struct TuiLaunchConfig {
    initial_model: String,
    initial_thinking_level: ThinkingLevel,
    initial_thinking_collapse_mode: CollapseMode,
    initial_tool_collapse_mode: CollapseMode,
    initial_chat_style: oino_tui::ChatStyle,
    initial_keymap: KeymapConfig,
    provider_config: OpenRouterConfig,
    session_path: PathBuf,
    resource_paths: ResourcePaths,
    resource_catalog: ResourceCatalog,
    open_settings: bool,
}

#[derive(Debug, Clone, Default)]
struct ToolSettingsSnapshot {
    global: UserSettings,
    project: UserSettings,
}

impl AppConfig {
    async fn load() -> Self {
        let saved_settings = UserSettings::load_default().await.unwrap_or_default();
        Self::from_saved_settings(saved_settings)
    }

    fn from_saved_settings(saved_settings: UserSettings) -> Self {
        Self::from_sources(
            saved_settings,
            non_empty_env("OINO_MODEL"),
            non_empty_env("OINO_OPENROUTER_REFERER"),
            non_empty_env("OINO_OPENROUTER_TITLE"),
        )
    }

    fn from_sources(
        saved_settings: UserSettings,
        model_override: Option<String>,
        referer: Option<String>,
        title: Option<String>,
    ) -> Self {
        Self {
            model: model_override
                .or(saved_settings.model)
                .unwrap_or_else(|| DEFAULT_MODEL.into()),
            thinking_level: saved_settings.thinking_level.unwrap_or_default(),
            thinking_collapse_mode: saved_settings.thinking_collapse_mode.unwrap_or_default(),
            tool_collapse_mode: saved_settings.tool_collapse_mode.unwrap_or_default(),
            chat_style: saved_settings.chat_style.unwrap_or_default(),
            keymap: saved_settings.keymap.unwrap_or_default(),
            referer,
            title,
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        Ok(_) | Err(_) => None,
    }
}

fn ensure_model_identifier(value: &str) -> Result<Model, AppError> {
    Model::from_identifier(value).ok_or_else(|| AppError::InvalidModelIdentifier(value.into()))
}

async fn load_or_create_session(
    session_id: Option<OinoId>,
    cwd: PathBuf,
) -> Result<(PathBuf, SessionManager), AppError> {
    let root = default_session_root()?;
    let repository = SessionRepository::new(root.clone());
    if let Some(session_id) = session_id {
        let path = root.join(format!("{session_id}.jsonl"));
        let session = repository.open(&path).await?;
        return Ok((path, session));
    }
    let session = SessionManager::new(SessionHeader::new("oino", cwd));
    let path = root.join(format!("{}.jsonl", session.header().session_id));
    Ok((path, session))
}

fn default_session_root() -> Result<PathBuf, AppError> {
    let Some(home) = dirs::home_dir() else {
        return Err(AppError::InvalidArguments(
            "home directory unavailable for session storage".into(),
        ));
    };
    Ok(home.join(".oino").join("sessions"))
}

async fn auth_status_items(
    auth: &AuthStorage,
    provider_filter: Option<&str>,
    current_model_identifier: &str,
) -> Result<Vec<AuthStatusItem>, AppError> {
    let current_provider = Model::from_identifier(current_model_identifier)
        .map(|model| model.provider)
        .unwrap_or_else(|| oino_auth::OPENROUTER_PROVIDER_ID.into());
    let selected_provider = provider_filter
        .map(|provider| {
            resolve_provider(provider).ok_or_else(|| {
                AppError::InvalidArguments(format!(
                    "unknown provider `{provider}`; use `/auth` to list known providers"
                ))
            })
        })
        .transpose()?;
    let _ = auth;
    Ok(selected_provider.map_or_else(Vec::new, |provider| {
        vec![removed_builtin_auth_status_item(
            *provider,
            provider.id == current_provider,
        )]
    }))
}

fn removed_builtin_auth_status_item(provider: ProviderDescriptor, current: bool) -> AuthStatusItem {
    AuthStatusItem {
        provider_id: provider.id.into(),
        display_name: provider.display_name.into(),
        auth_kind: "removed built-in auth".into(),
        runtime: provider.runtime.label().into(),
        state: "removed".into(),
        readiness: "use extension auth".into(),
        source: "core auth removed".into(),
        detail: auth_readiness::removed_builtin_auth_message("/auth status"),
        setup_url: None,
        current,
    }
}

async fn auth_status_items_with_extension_readiness(
    auth: &AuthStorage,
    provider_filter: Option<&str>,
    current_model_identifier: &str,
    snapshot: &ExtensionManagerSnapshot,
) -> Result<Vec<AuthStatusItem>, AppError> {
    let current_provider = Model::from_identifier(current_model_identifier)
        .map(|model| model.provider)
        .unwrap_or_else(|| oino_auth::OPENROUTER_PROVIDER_ID.into());
    let historical_provider_known = provider_filter.and_then(resolve_provider).is_some();
    let extension_known = provider_filter
        .is_some_and(|provider| extension_readiness_provider_matches(snapshot, provider));
    if provider_filter.is_some() && !historical_provider_known && !extension_known {
        let provider = provider_filter.unwrap_or_default();
        return Err(AppError::InvalidArguments(format!(
            "unknown provider `{provider}`; use `/auth` to list known providers"
        )));
    }

    let mut items = if provider_filter.is_none() || historical_provider_known {
        auth_status_items(auth, provider_filter, current_model_identifier).await?
    } else {
        Vec::new()
    };
    items.extend(
        extension_readiness_status_items_with_health(
            snapshot,
            provider_filter,
            &current_provider,
            &format_extension_runtime_health_detail,
        )
        .await,
    );
    Ok(items)
}

fn format_extension_runtime_health_detail(
    provider_id: &str,
    health: &ExtensionRuntimeHealth,
) -> String {
    if provider_id == "router" {
        router::format_extension_runtime_health_detail(health)
    } else {
        extension_provider_runtime::format_extension_runtime_health_detail(health)
    }
}

async fn usage_report_for_current_session(
    harness: &Harness,
    auth: &AuthStorage,
    current_model_identifier: &str,
) -> Result<UsageReport, AppError> {
    let messages = harness.build_context().await?;
    let mut report = UsageReport::from_messages(&messages);
    if let Some(provider) = current_model_provider(current_model_identifier)? {
        let progress =
            account_usage_progress_placeholder(auth, provider, report.generated_at_unix).await?;
        report.upsert_provider_progress(progress);
    }
    Ok(report)
}

fn current_model_provider(
    current_model_identifier: &str,
) -> Result<Option<ProviderDescriptor>, AppError> {
    let Some(model) = Model::from_identifier(current_model_identifier) else {
        return Ok(None);
    };
    Ok(provider_by_id(&model.provider).copied())
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let cli = CliArgs::parse_from(std::env::args().skip(1))?;
    if cli.show_help {
        println!("{}", usage());
        return Ok(());
    }

    let mut config = AppConfig::load().await;
    if let Some(model) = cli.model.clone() {
        ensure_model_identifier(&model)?;
        config.model = model;
    }

    let auth = AuthStorage::default_file()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_paths = ResourcePaths::for_cwd(&cwd)?;
    migrate_legacy_router_builtin(&resource_paths).await;
    let resource_catalog = load_resource_catalog(&resource_paths)?;
    let (session_path, session) = load_or_create_session(cli.session, cwd.clone()).await?;
    let session_context = session.build_session_context()?;
    if cli.model.is_none() {
        if let Some(model) = session_context.model {
            config.model = model.identifier();
        }
    }
    if let Some(thinking_level) = session_context.thinking_level {
        config.thinking_level = thinking_level;
    }

    let provider_config = OpenRouterConfig {
        referer: config.referer.clone(),
        title: config.title.clone(),
        ..OpenRouterConfig::default()
    };
    let tool_settings = load_tool_settings(&resource_paths).await;
    let extension_snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
    let provider = build_runtime_provider(
        auth.clone(),
        provider_config.clone(),
        extension_runtime_providers(&extension_snapshot),
    );
    let harness = build_harness(
        config.model.clone(),
        config.thinking_level,
        provider,
        auth.clone(),
        session,
        &resource_catalog,
    )?;
    apply_tool_settings_to_harness(
        &harness,
        &tool_settings,
        &resource_paths,
        &cwd,
        AgentMode::Work,
        None,
    )
    .await;

    if cli.settings || cli.input.is_some() {
        return run_non_interactive(
            cli,
            harness,
            auth,
            config,
            session_path,
            resource_catalog,
            extension_runtime_provider_ids_from_snapshot(&extension_snapshot),
        )
        .await;
    }

    // First-run onboarding hint
    let has_any_credentials = match auth.load().await {
        Ok(entries) => !entries.is_empty(),
        Err(_) => false,
    };
    if !has_any_credentials && std::env::var("OPENROUTER_API_KEY").is_err() {
        eprintln!("Welcome to Oino! No router/auth configured yet.");
        eprintln!("Recommended: run `/router setup` for extension-managed auth/routing.");
        eprintln!("Configure provider credentials in OmniRoute or an auth extension.\n");
    }

    run_tui(
        harness,
        auth,
        TuiLaunchConfig {
            initial_model: config.model,
            initial_thinking_level: config.thinking_level,
            initial_thinking_collapse_mode: config.thinking_collapse_mode,
            initial_tool_collapse_mode: config.tool_collapse_mode,
            initial_chat_style: config.chat_style,
            initial_keymap: config.keymap,
            provider_config,
            session_path,
            resource_paths,
            resource_catalog,
            open_settings: false,
        },
    )
    .await
}

fn build_auth_resolver(auth: AuthStorage) -> AuthResolver {
    Arc::new(move |provider: String| {
        let auth = auth.clone();
        let fut: BoxFuture<'static, oino_agent_loop::LoopResult<Option<String>>> =
            Box::pin(async move {
                let spec = provider_by_id(&provider)
                    .and_then(|descriptor| {
                        descriptor.credential_spec().env_var.map(|env_var| {
                            let credential = descriptor.credential_spec();
                            ProviderAuthSpec::new(
                                credential.provider_id,
                                credential.auth_key,
                                env_var,
                            )
                        })
                    })
                    .unwrap_or_else(|| {
                        ProviderAuthSpec::new(
                            provider.clone(),
                            provider.clone(),
                            format!("{}_API_KEY", provider.to_uppercase()),
                        )
                    });
                match auth.resolve_api_key(&spec).await {
                    Ok(key) => Ok(Some(key)),
                    Err(AuthError::MissingCredential { .. }) => Ok(None),
                    Err(err) => Err(LoopError::Stream(err.to_string())),
                }
            });
        fut
    })
}

fn build_harness(
    model_identifier: String,
    thinking_level: ThinkingLevel,
    provider: Arc<dyn StreamProvider>,
    auth: AuthStorage,
    session: SessionManager,
    resource_catalog: &ResourceCatalog,
) -> Result<Harness, AppError> {
    let cwd = session.header().cwd.clone();
    let model = Model::from_identifier(&model_identifier)
        .ok_or_else(|| AppError::InvalidModelIdentifier(model_identifier.clone()))?;
    let mut config = HarnessConfig::new(model, provider, session);
    config.tools = oino_tools::default_tools(Arc::clone(&config.env), cwd.clone());
    config.system_prompt = Some(default_system_prompt(&cwd, resource_catalog));
    config.thinking_level = thinking_level;
    config.auth_resolver = Some(build_auth_resolver(auth));
    Ok(Harness::new(config))
}

const BUILT_IN_TOOL_NAMES: &[&str] = &["bash", "edit", "read", "write"];

async fn load_tool_settings(paths: &ResourcePaths) -> ToolSettingsSnapshot {
    ToolSettingsSnapshot {
        global: user_settings::load_from_path(&paths.global_settings)
            .await
            .unwrap_or_default(),
        project: user_settings::load_from_path(&paths.project_settings)
            .await
            .unwrap_or_default(),
    }
}

fn known_tool_names(settings: &ToolSettingsSnapshot) -> BTreeSet<String> {
    let mut names = BUILT_IN_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();
    names.insert(oino_tools::SESSION_TITLE_TOOL_NAME.into());
    names.extend(settings.global.tools.keys().cloned());
    names.extend(settings.project.tools.keys().cloned());
    names
}

fn default_tool_enabled(name: &str) -> bool {
    name != oino_tools::SESSION_TITLE_TOOL_NAME
}

fn global_tool_enabled(settings: &ToolSettingsSnapshot, name: &str) -> bool {
    settings
        .global
        .tools
        .get(name)
        .copied()
        .unwrap_or_else(|| default_tool_enabled(name))
}

fn project_tool_enabled(settings: &ToolSettingsSnapshot, name: &str) -> bool {
    settings
        .project
        .tools
        .get(name)
        .copied()
        .unwrap_or_else(|| global_tool_enabled(settings, name))
}

fn tool_registry_policy_from_settings(
    settings: &ToolSettingsSnapshot,
    names: impl IntoIterator<Item = String>,
) -> RegistryPolicy {
    let mut policy = RegistryPolicy::default();
    for name in names {
        let Ok(id) = ContributionId::new(name.clone()) else {
            continue;
        };
        if project_tool_enabled(settings, &name) {
            policy.enabled_contributions.insert(id);
        } else {
            policy.disabled_contributions.insert(id);
        }
    }
    policy
}

fn tool_settings_items(settings: &ToolSettingsSnapshot) -> Vec<ToolSettingsItem> {
    known_tool_names(settings)
        .into_iter()
        .map(|name| {
            ToolSettingsItem::global(name.clone()).with_scopes(
                global_tool_enabled(settings, &name),
                project_tool_enabled(settings, &name),
            )
        })
        .collect()
}

fn tool_map_from_state(state: &TuiState, scope: ToolSettingsScope) -> BTreeMap<String, bool> {
    state
        .settings
        .tools
        .iter()
        .map(|tool| {
            let enabled = match scope {
                ToolSettingsScope::Global => tool.global_enabled,
                ToolSettingsScope::Project => tool.project_enabled,
            };
            (tool.name.clone(), enabled)
        })
        .collect()
}

fn set_tool_enabled(
    settings: &mut ToolSettingsSnapshot,
    scope: ToolSettingsScope,
    name: String,
    enabled: bool,
) {
    match scope {
        ToolSettingsScope::Global => settings.global.tools.insert(name, enabled),
        ToolSettingsScope::Project => settings.project.tools.insert(name, enabled),
    };
}

fn set_theme_active(settings: &mut ToolSettingsSnapshot, scope: ToolSettingsScope, id: String) {
    match scope {
        ToolSettingsScope::Global => settings.global.theme.set_active(id),
        ToolSettingsScope::Project => settings.project.theme.set_active(id),
    }
}

fn reset_theme(settings: &mut ToolSettingsSnapshot, scope: ToolSettingsScope) {
    match scope {
        ToolSettingsScope::Global => {
            settings.global.theme.clear_active();
            settings.global.theme.overrides.clear();
        }
        ToolSettingsScope::Project => {
            settings.project.theme.clear_active();
            settings.project.theme.overrides.clear();
        }
    }
}

fn notify_settings_mut(
    settings: &mut ToolSettingsSnapshot,
    scope: ToolSettingsScope,
) -> &mut notify::NotifySettings {
    match scope {
        ToolSettingsScope::Global => &mut settings.global.notify,
        ToolSettingsScope::Project => &mut settings.project.notify,
    }
}

fn set_notify_enabled(
    settings: &mut ToolSettingsSnapshot,
    scope: ToolSettingsScope,
    enabled: bool,
) {
    notify_settings_mut(settings, scope).enabled = Some(enabled);
}

fn set_notify_field(
    settings: &mut ToolSettingsSnapshot,
    scope: ToolSettingsScope,
    field: NotifyField,
    value: Option<String>,
) {
    let notify = notify_settings_mut(settings, scope);
    match field {
        NotifyField::Server => notify.ntfy.server = value,
        NotifyField::Topic => notify.ntfy.topic = value,
        NotifyField::Token => notify.ntfy.token = value,
        NotifyField::Priority => {
            notify.ntfy.priority = value.and_then(|value| notify::normalize_ntfy_priority(&value));
        }
        NotifyField::SummaryModel => notify.summarizer.model = value,
        NotifyField::SummaryPrompt => {
            if let Some(value) = value.as_deref() {
                if value == "__summary_enabled:true" || value == "__summary_enabled:false" {
                    notify.summarizer.enabled = Some(value.ends_with("true"));
                    return;
                }
            }
            notify.summarizer.prompt = value;
        }
        NotifyField::SummaryMaxChars => {
            notify.summarizer.max_chars = value
                .and_then(|value| value.parse::<usize>().ok())
                .map(|value| value.clamp(80, 2000));
        }
        NotifyField::Tags => {
            notify.ntfy.tags = value.map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            });
        }
    }
}

fn set_notify_event(
    settings: &mut ToolSettingsSnapshot,
    scope: ToolSettingsScope,
    event: TuiNotifyEventKind,
    enabled: bool,
) {
    let notify = notify_settings_mut(settings, scope);
    let mut events = notify.events.clone().unwrap_or_else(|| {
        std::collections::BTreeSet::from([
            notify::NotifyEvent::AgentEnd,
            notify::NotifyEvent::ToolError,
        ])
    });
    let event = app_notify_event_from_tui(event);
    if enabled {
        events.insert(event);
    } else {
        events.remove(&event);
    }
    notify.events = Some(events);
}

fn extension_enabled_global(settings: &ToolSettingsSnapshot, id: &ExtensionId) -> bool {
    extension_policy_enabled(settings.global.extensions.extensions.get(id), true)
}

fn extension_enabled_project(settings: &ToolSettingsSnapshot, id: &ExtensionId) -> bool {
    extension_policy_enabled(
        settings.project.extensions.extensions.get(id),
        extension_enabled_global(settings, id),
    )
}

fn package_enabled_global(settings: &ToolSettingsSnapshot, id: &PackageId) -> bool {
    extension_policy_enabled(settings.global.extensions.packages.get(id), true)
}

fn package_enabled_project(settings: &ToolSettingsSnapshot, id: &PackageId) -> bool {
    extension_policy_enabled(
        settings.project.extensions.packages.get(id),
        package_enabled_global(settings, id),
    )
}

fn contribution_enabled_global(settings: &ToolSettingsSnapshot, id: &ContributionId) -> bool {
    extension_policy_enabled(settings.global.extensions.contributions.get(id), true)
}

fn contribution_enabled_project(settings: &ToolSettingsSnapshot, id: &ContributionId) -> bool {
    extension_policy_enabled(
        settings.project.extensions.contributions.get(id),
        contribution_enabled_global(settings, id),
    )
}

fn contribution_override_global(
    settings: &ToolSettingsSnapshot,
    id: &ContributionId,
    entry_key: &RegistryEntryKey,
) -> bool {
    settings
        .global
        .extensions
        .overrides
        .get(id)
        .is_some_and(|key| key == entry_key)
}

fn contribution_override_project(
    settings: &ToolSettingsSnapshot,
    id: &ContributionId,
    entry_key: &RegistryEntryKey,
) -> bool {
    settings
        .project
        .extensions
        .overrides
        .get(id)
        .is_some_and(|key| key == entry_key)
}

fn extension_policy_enabled(toggle: Option<&PolicyToggle>, default: bool) -> bool {
    match toggle {
        Some(PolicyToggle::Enabled) => true,
        Some(PolicyToggle::Disabled) => false,
        None => default,
    }
}

fn set_extension_override(
    settings: &mut ToolSettingsSnapshot,
    contribution_id: String,
    entry_key: String,
    scope: ToolSettingsScope,
) {
    let Ok(contribution_id) = ContributionId::new(contribution_id) else {
        return;
    };
    let entry_key = RegistryEntryKey::new(entry_key);
    let settings = match scope {
        ToolSettingsScope::Global => &mut settings.global.extensions,
        ToolSettingsScope::Project => &mut settings.project.extensions,
    };
    settings.overrides.insert(contribution_id, entry_key);
}

fn clear_extension_override(
    settings: &mut ToolSettingsSnapshot,
    contribution_id: String,
    scope: ToolSettingsScope,
) {
    let Ok(contribution_id) = ContributionId::new(contribution_id) else {
        return;
    };
    let settings = match scope {
        ToolSettingsScope::Global => &mut settings.global.extensions,
        ToolSettingsScope::Project => &mut settings.project.extensions,
    };
    settings.overrides.remove(&contribution_id);
}

fn set_extension_enabled(
    settings: &mut ToolSettingsSnapshot,
    target: ExtensionManagementTarget,
    id: String,
    scope: ToolSettingsScope,
    enabled: bool,
) {
    let toggle = if enabled {
        PolicyToggle::Enabled
    } else {
        PolicyToggle::Disabled
    };
    let settings = match scope {
        ToolSettingsScope::Global => &mut settings.global.extensions,
        ToolSettingsScope::Project => &mut settings.project.extensions,
    };
    match target {
        ExtensionManagementTarget::Extension => {
            if let Ok(id) = ExtensionId::new(id) {
                settings.extensions.insert(id, toggle);
            }
        }
        ExtensionManagementTarget::Package => {
            if let Ok(id) = PackageId::new(id) {
                settings.packages.insert(id, toggle);
            }
        }
        ExtensionManagementTarget::Contribution => {
            if let Ok(id) = ContributionId::new(id) {
                settings.contributions.insert(id, toggle);
            }
        }
    }
}

async fn save_tool_settings_for_scope(
    settings: &ToolSettingsSnapshot,
    paths: &ResourcePaths,
    scope: ToolSettingsScope,
) -> Result<(), AppError> {
    match scope {
        ToolSettingsScope::Global => {
            user_settings::save_to_path(&settings.global, &paths.global_settings).await?
        }
        ToolSettingsScope::Project => {
            user_settings::save_to_path(&settings.project, &paths.project_settings).await?
        }
    }
    Ok(())
}

async fn save_tool_settings(
    settings: &ToolSettingsSnapshot,
    paths: &ResourcePaths,
    scope: ToolSettingsScope,
    state: &mut TuiState,
) {
    let result = match scope {
        ToolSettingsScope::Global => {
            user_settings::save_to_path(&settings.global, &paths.global_settings).await
        }
        ToolSettingsScope::Project => {
            user_settings::save_to_path(&settings.project, &paths.project_settings).await
        }
    };
    if let Err(err) = result {
        state.set_error(format!("Tool settings save failed: {err}"));
        state.status = HELP_STATUS.into();
    }
}

fn current_extension_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 1, 0))
}

fn extension_manager_with_current_policy(
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
) -> ExtensionManager {
    let policy = oino_extension_core::ExtensionPolicySettings::merged_registry_policy(
        &settings.global.extensions,
        &settings.project.extensions,
    );
    let config = ExtensionManagerConfig::new(
        current_extension_version(),
        ExtensionDiscovery::from_home_and_project(&paths.home_dir, &paths.project_root),
    )
    .with_policy(policy);
    ExtensionManager::new(config)
}

fn load_extension_snapshot(
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
) -> ExtensionManagerSnapshot {
    extension_manager_with_current_policy(paths, settings).load()
}

fn extension_layout_paths(paths: &ResourcePaths) -> ExtensionLayoutPaths {
    ExtensionLayoutPaths::for_home_and_project(&paths.home_dir, &paths.project_root)
}

const LEGACY_ROUTER_PACKAGE_ID: &str = "oino.9router";
const ROUTER_PACKAGE_ID: &str = "oino.router";

async fn migrate_legacy_router_builtin(paths: &ResourcePaths) {
    let layout = extension_layout_paths(paths);
    let scopes = [
        (
            ToolSettingsScope::Global,
            layout.global_installed_packages.clone(),
            paths.global_settings.clone(),
        ),
        (
            ToolSettingsScope::Project,
            layout.project_installed_packages.clone(),
            paths.project_settings.clone(),
        ),
    ];

    let settings = load_tool_settings(paths).await;
    let mut manager = extension_manager_with_current_policy(paths, &settings);
    manager.load();
    let service = PackageLifecycleService::new(layout, current_extension_version());

    for (scope, package_root, settings_path) in scopes {
        let legacy_dir = package_root.join(LEGACY_ROUTER_PACKAGE_ID);
        if legacy_dir.exists() {
            let router_dir = package_root.join(ROUTER_PACKAGE_ID);
            if !router_dir.exists() {
                if let Some(source) =
                    oino_extension_builtins::optional_builtin_package_path(ROUTER_PACKAGE_ID)
                {
                    if let Ok(report) =
                        service.install_local(source, package_install_scope(scope), &mut manager)
                    {
                        let _ = write_extension_package_source_record(
                            &report.destination,
                            "builtin:router",
                        );
                    }
                }
            }
            let _ = fs::remove_dir_all(&legacy_dir);
        }
        let _ = migrate_legacy_router_policy_settings(&settings_path).await;
    }
}

async fn migrate_legacy_router_policy_settings(settings_path: &Path) -> io::Result<()> {
    if !settings_path.exists() {
        return Ok(());
    }
    let mut settings = user_settings::load_from_path(settings_path).await?;
    let mut changed = false;

    let legacy_package = PackageId::new(LEGACY_ROUTER_PACKAGE_ID)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let router_package = PackageId::new(ROUTER_PACKAGE_ID)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if let Some(toggle) = settings.extensions.packages.remove(&legacy_package) {
        if toggle == PolicyToggle::Enabled {
            settings
                .extensions
                .packages
                .entry(router_package)
                .or_insert(PolicyToggle::Enabled);
        }
        changed = true;
    }

    let legacy_extension = ExtensionId::new(LEGACY_ROUTER_PACKAGE_ID)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let router_extension = ExtensionId::new(ROUTER_PACKAGE_ID)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if let Some(toggle) = settings.extensions.extensions.remove(&legacy_extension) {
        if toggle == PolicyToggle::Enabled {
            settings
                .extensions
                .extensions
                .entry(router_extension)
                .or_insert(PolicyToggle::Enabled);
        }
        changed = true;
    }

    if changed {
        user_settings::save_to_path(&settings, settings_path).await?;
    }
    Ok(())
}

fn package_install_scope(scope: ToolSettingsScope) -> PackageInstallScope {
    match scope {
        ToolSettingsScope::Global => PackageInstallScope::Global,
        ToolSettingsScope::Project => PackageInstallScope::Project,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExtensionInstallSource {
    Local(PathBuf),
    Git(GitInstallSource),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitInstallSource {
    clone_url: String,
    display: String,
    reference: Option<String>,
}

#[derive(Debug)]
struct PreparedExtensionInstallSource {
    path: PathBuf,
    display: String,
    update_source: String,
    _temp_checkout: Option<TemporaryDirectory>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ExtensionPackageSourceRecord {
    source: String,
}

const EXTENSION_PACKAGE_SOURCE_RECORD: &str = ".oino-install-source.json";

#[derive(Debug)]
struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn create(prefix: &str) -> Result<Self, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| format!("system clock error: {err}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path)
            .map_err(|err| format!("failed to create temp checkout `{}`: {err}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn resolve_install_source(source: &str, cwd: &Path, home_dir: &Path) -> ExtensionInstallSource {
    let source = source.trim();
    let local_path = resolve_local_install_path(source, cwd, home_dir);
    if let Some(git) = git_install_source(source, local_path.exists()) {
        ExtensionInstallSource::Git(git)
    } else {
        ExtensionInstallSource::Local(local_path)
    }
}

fn resolve_local_install_path(source: &str, cwd: &Path, home_dir: &Path) -> PathBuf {
    if let Some(rest) = source.strip_prefix("~/") {
        return home_dir.join(rest);
    }
    let path = PathBuf::from(source);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn prepare_install_source(
    source: &str,
    cwd: &Path,
    home_dir: &Path,
) -> Result<PreparedExtensionInstallSource, String> {
    if let Some(query) = optional_builtin_install_query(source) {
        let path =
            oino_extension_builtins::optional_builtin_package_path(query).ok_or_else(|| {
                format!(
                    "unknown optional built-in extension `{query}`; available: {}",
                    optional_builtin_install_choices()
                )
            })?;
        return Ok(PreparedExtensionInstallSource {
            display: format!("builtin:{query}"),
            update_source: format!("builtin:{query}"),
            path,
            _temp_checkout: None,
        });
    }

    match resolve_install_source(source, cwd, home_dir) {
        ExtensionInstallSource::Local(path) => Ok(PreparedExtensionInstallSource {
            display: path.display().to_string(),
            update_source: source.trim().to_string(),
            path,
            _temp_checkout: None,
        }),
        ExtensionInstallSource::Git(git) => clone_extension_git_source(git),
    }
}

fn optional_builtin_install_query(source: &str) -> Option<&str> {
    let source = source.trim();
    source
        .strip_prefix("builtin:")
        .or_else(|| source.strip_prefix("built-in:"))
        .map(str::trim)
        .filter(|query| !query.is_empty())
}

fn optional_builtin_install_choices() -> String {
    oino_extension_builtins::optional_builtin_packages()
        .iter()
        .map(|package| format!("{} (alias: {})", package.id, package.directory_name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn clone_extension_git_source(
    source: GitInstallSource,
) -> Result<PreparedExtensionInstallSource, String> {
    let checkout = TemporaryDirectory::create("oino-extension-install")?;
    let mut command = Command::new("git");
    command
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--recurse-submodules")
        .arg("--shallow-submodules");
    if let Some(reference) = &source.reference {
        command.arg("--branch").arg(reference);
    }
    command.arg(&source.clone_url).arg(checkout.path());
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("failed to run git clone: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let details = [stderr.trim(), stdout.trim()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(if details.is_empty() {
            format!("git clone failed for `{}`", source.display)
        } else {
            format!("git clone failed for `{}`: {details}", source.display)
        });
    }
    Ok(PreparedExtensionInstallSource {
        path: checkout.path().to_path_buf(),
        update_source: source.display.clone(),
        display: source.display,
        _temp_checkout: Some(checkout),
    })
}

fn write_extension_package_source_record(destination: &Path, source: &str) -> Result<(), AppError> {
    let record = ExtensionPackageSourceRecord {
        source: source.to_string(),
    };
    let text = serde_json::to_string_pretty(&record).map_err(|err| {
        AppError::InvalidArguments(format!(
            "could not serialize extension source record: {err}"
        ))
    })?;
    fs::write(
        destination.join(EXTENSION_PACKAGE_SOURCE_RECORD),
        format!("{text}\n"),
    )?;
    Ok(())
}

fn read_extension_package_source_record(package_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(package_dir.join(EXTENSION_PACKAGE_SOURCE_RECORD)).ok()?;
    let record = serde_json::from_str::<ExtensionPackageSourceRecord>(&text).ok()?;
    (!record.source.trim().is_empty()).then_some(record.source)
}

fn optional_builtin_source_for_package(package_id: &PackageId) -> Option<String> {
    oino_extension_builtins::optional_builtin_packages()
        .iter()
        .find(|package| package.id == package_id.as_str())
        .map(|package| format!("builtin:{}", package.directory_name))
}

fn installed_extension_package_dirs(paths: &ResourcePaths) -> Vec<(ToolSettingsScope, PathBuf)> {
    let layout = extension_layout_paths(paths);
    [
        (ToolSettingsScope::Global, layout.global_installed_packages),
        (
            ToolSettingsScope::Project,
            layout.project_installed_packages,
        ),
    ]
    .into_iter()
    .flat_map(|(scope, root)| {
        fs::read_dir(root)
            .ok()
            .into_iter()
            .flat_map(move |entries| {
                entries.filter_map(move |entry| {
                    let path = entry.ok()?.path();
                    path.is_dir().then_some((scope, path))
                })
            })
    })
    .collect()
}

fn read_package_manifest_from_installed_dir(path: &Path) -> Option<PackageManifest> {
    let text = fs::read_to_string(path.join("oino.package.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn git_install_source(source: &str, local_path_exists: bool) -> Option<GitInstallSource> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let source = source.strip_prefix("git+").unwrap_or(source);
    let (body, reference) = split_git_reference(source);
    if let Some(repo) = body
        .strip_prefix("github:")
        .or_else(|| body.strip_prefix("gh:"))
    {
        return github_shorthand_source(repo, reference);
    }
    if !local_path_exists {
        if let Some(github) = github_shorthand_source(body, reference.clone()) {
            return Some(github);
        }
    }
    if looks_like_git_url(body) {
        return Some(GitInstallSource {
            clone_url: body.to_string(),
            display: if let Some(reference) = &reference {
                format!("{body}#{reference}")
            } else {
                body.to_string()
            },
            reference,
        });
    }
    None
}

fn split_git_reference(source: &str) -> (&str, Option<String>) {
    match source.rsplit_once('#') {
        Some((body, reference)) if !body.is_empty() && !reference.trim().is_empty() => {
            (body, Some(reference.trim().to_string()))
        }
        _ => (source, None),
    }
}

fn github_shorthand_source(repo: &str, reference: Option<String>) -> Option<GitInstallSource> {
    let repo = repo.trim().trim_start_matches('/').trim_end_matches('/');
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let mut parts = repo.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() || !valid_github_owner(owner) || !valid_github_repo(name) {
        return None;
    }
    let clone_url = format!("https://github.com/{owner}/{name}.git");
    let display = if let Some(reference) = &reference {
        format!("github:{owner}/{name}#{reference}")
    } else {
        format!("github:{owner}/{name}")
    };
    Some(GitInstallSource {
        clone_url,
        display,
        reference,
    })
}

fn valid_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && owner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn valid_github_repo(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn looks_like_git_url(source: &str) -> bool {
    source.starts_with("https://")
        || source.starts_with("http://")
        || source.starts_with("ssh://")
        || source.starts_with("git://")
        || source.starts_with("git@")
        || source.ends_with(".git")
}

fn update_installed_extension_packages(
    paths: &ResourcePaths,
    cwd: &Path,
    settings: &ToolSettingsSnapshot,
) -> String {
    let mut manager = extension_manager_with_current_policy(paths, settings);
    manager.load();
    let service =
        PackageLifecycleService::new(extension_layout_paths(paths), current_extension_version());
    let mut updated = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for (scope, package_dir) in installed_extension_package_dirs(paths) {
        let Some(manifest) = read_package_manifest_from_installed_dir(&package_dir) else {
            skipped.push(format!(
                "{}: unreadable package at {}",
                scope.label(),
                package_dir.display()
            ));
            continue;
        };
        let source = read_extension_package_source_record(&package_dir)
            .or_else(|| optional_builtin_source_for_package(&manifest.id));
        let Some(source) = source else {
            skipped.push(format!(
                "{}:{} (no remembered local/GitHub/built-in source)",
                scope.label(),
                manifest.id
            ));
            continue;
        };
        let prepared = match prepare_install_source(&source, cwd, &paths.home_dir) {
            Ok(prepared) => prepared,
            Err(err) => {
                failed.push(format!(
                    "{}:{} from `{source}`: {err}",
                    scope.label(),
                    manifest.id
                ));
                continue;
            }
        };
        match service.update_local(&prepared.path, package_install_scope(scope), &mut manager) {
            Ok(report) => {
                if let Err(err) = write_extension_package_source_record(
                    &report.destination,
                    &prepared.update_source,
                ) {
                    failed.push(format!(
                        "{}:{} updated but source record failed: {err}",
                        scope.label(),
                        report.package_id
                    ));
                } else {
                    updated.push(format!(
                        "{}:{} from `{}`",
                        scope.label(),
                        report.package_id,
                        prepared.display
                    ));
                }
            }
            Err(err) => failed.push(format!(
                "{}:{} from `{}`: {err}",
                scope.label(),
                manifest.id,
                prepared.display
            )),
        }
    }

    let mut lines = vec![format!(
        "Extension update complete: {} updated, {} skipped, {} failed.",
        updated.len(),
        skipped.len(),
        failed.len()
    )];
    if !updated.is_empty() {
        lines.push(format!("Updated: {}", updated.join(", ")));
    }
    if !skipped.is_empty() {
        lines.push(format!("Skipped: {}", skipped.join(", ")));
    }
    if !failed.is_empty() {
        lines.push(format!("Failed: {}", failed.join(", ")));
    }
    lines.join("\n")
}

fn apply_extension_snapshot_to_tui_state(
    state: &mut TuiState,
    snapshot: &ExtensionManagerSnapshot,
    settings: &ToolSettingsSnapshot,
    paths: &ResourcePaths,
) {
    state.set_extension_ui_surfaces(extension_ui_surfaces(snapshot));
    state.set_extension_commands(extension_command_suggestions(snapshot));
    state.set_extension_shortcuts(extension_shortcuts(snapshot));
    state.set_extension_autosuggest_items(extension_autosuggest_items(snapshot));
    state.set_theme_catalog(
        theme_catalog_from_sources(snapshot, paths),
        &settings.global.theme,
        &settings.project.theme,
    );
    state.set_extension_theme(ExtensionThemeState::default());
    state.set_extension_management_items(extension_management_items(snapshot, settings));
    state
        .settings
        .set_notify_available(extension_settings_page_enabled(snapshot, "notify"));
    state.settings.set_notify_settings(
        notify_settings_to_tui(&settings.global.notify),
        notify_settings_to_tui(&settings.project.notify),
    );
}

fn extension_settings_page_enabled(snapshot: &ExtensionManagerSnapshot, id: &str) -> bool {
    snapshot
        .registries
        .settings_pages
        .active
        .iter()
        .any(|active| active.effective_id.as_str() == id)
}

fn notify_settings_to_tui(settings: &notify::NotifySettings) -> NotifyScopeSettings {
    NotifyScopeSettings {
        enabled: settings.enabled,
        server: settings.ntfy.server.clone(),
        topic: settings.ntfy.topic.clone(),
        token: settings.ntfy.token.clone(),
        priority: settings.ntfy.priority.clone(),
        tags: settings.ntfy.tags.clone(),
        events: settings.events.as_ref().map(|events| {
            events
                .iter()
                .copied()
                .map(tui_notify_event_from_app)
                .collect::<Vec<_>>()
        }),
        summary_enabled: settings.summarizer.enabled,
        summary_model: settings.summarizer.model.clone(),
        summary_prompt: settings.summarizer.prompt.clone(),
        summary_max_chars: settings.summarizer.max_chars,
    }
}

fn tui_notify_event_from_app(event: notify::NotifyEvent) -> TuiNotifyEventKind {
    match event {
        notify::NotifyEvent::AgentEnd => TuiNotifyEventKind::AgentEnd,
        notify::NotifyEvent::ToolError => TuiNotifyEventKind::ToolError,
    }
}

fn app_notify_event_from_tui(event: TuiNotifyEventKind) -> notify::NotifyEvent {
    match event {
        TuiNotifyEventKind::AgentEnd => notify::NotifyEvent::AgentEnd,
        TuiNotifyEventKind::ToolError => notify::NotifyEvent::ToolError,
    }
}

fn extension_ui_surfaces(
    snapshot: &ExtensionManagerSnapshot,
) -> Vec<ActiveContribution<UiSurfaceContribution>> {
    let mut surfaces = snapshot.registries.ui_surfaces.active.clone();
    surfaces.extend(
        snapshot
            .registries
            .settings_pages
            .active
            .iter()
            .map(|active| {
                synthetic_ui_surface(
                    &active.effective_id,
                    &active.entry.metadata,
                    UiSurfaceKind::SettingsPage,
                    &active.entry.contribution.title,
                    15,
                )
            }),
    );
    surfaces.extend(snapshot.registries.themes.active.iter().map(|active| {
        synthetic_ui_surface(
            &active.effective_id,
            &active.entry.metadata,
            UiSurfaceKind::Theme,
            &format!("Theme {}", active.entry.contribution.path),
            10,
        )
    }));
    surfaces.extend(
        snapshot
            .registries
            .autosuggest_providers
            .active
            .iter()
            .map(|active| {
                synthetic_ui_surface(
                    &active.effective_id,
                    &active.entry.metadata,
                    UiSurfaceKind::Autosuggest,
                    &format!("Autosuggest {}", active.entry.contribution.trigger),
                    5,
                )
            }),
    );
    surfaces.extend(
        snapshot
            .registries
            .transcript_renderers
            .active
            .iter()
            .map(|active| {
                synthetic_renderer_surface(
                    active,
                    UiSurfaceKind::TranscriptRenderer,
                    "Transcript renderer",
                )
            }),
    );
    surfaces.extend(
        snapshot
            .registries
            .message_renderers
            .active
            .iter()
            .map(|active| {
                synthetic_renderer_surface(
                    active,
                    UiSurfaceKind::MessageRenderer,
                    "Message renderer",
                )
            }),
    );
    surfaces.extend(
        snapshot
            .registries
            .tool_renderers
            .active
            .iter()
            .map(|active| {
                let kind = match active.entry.contribution.target {
                    RendererTarget::ToolCall => UiSurfaceKind::ToolCallRenderer,
                    RendererTarget::ToolResult => UiSurfaceKind::ToolResultRenderer,
                    RendererTarget::TranscriptMessage | RendererTarget::MarkdownBlock => {
                        UiSurfaceKind::ToolRenderer
                    }
                };
                synthetic_renderer_surface(active, kind, "Tool renderer")
            }),
    );
    surfaces.extend(
        snapshot
            .registries
            .diagnostics
            .active
            .iter()
            .map(synthetic_diagnostic_surface),
    );
    surfaces.extend(
        snapshot
            .registries
            .health
            .active
            .iter()
            .map(synthetic_health_surface),
    );
    surfaces
}

fn extension_command_suggestions(
    snapshot: &ExtensionManagerSnapshot,
) -> Vec<ExtensionCommandSuggestion> {
    let mut commands = snapshot
        .registries
        .commands
        .active
        .iter()
        .filter(|active| {
            active.entry.metadata.source.scope != SourceScope::BuiltIn
                || active
                    .entry
                    .metadata
                    .package_id
                    .as_ref()
                    .is_some_and(|package_id| package_id.as_str() == ROUTER_PACKAGE_ID)
        })
        .filter_map(|active| {
            let contribution = &active.entry.contribution;
            let id = contribution.id.as_str();
            let (label, replacement) =
                extension_command_label(id, contribution.handler.as_deref())?;
            Some(ExtensionCommandSuggestion::new(
                label,
                contribution.description.clone(),
                replacement,
            ))
        })
        .collect::<Vec<_>>();
    commands.sort_by(|left, right| left.label.cmp(&right.label));
    commands.dedup_by(|left, right| left.label == right.label);
    commands
}

fn extension_command_label(id: &str, handler: Option<&str>) -> Option<(String, String)> {
    let label = match (id, handler) {
        ("mode_read", _) | (_, Some("mode.read")) => return None,
        ("mode_create", _) | (_, Some("mode.create")) => return None,
        ("mode_plan", _) | (_, Some("mode.plan")) => "/mode plan".into(),
        ("mode_work", _) | (_, Some("mode.work")) => "/mode work".into(),
        ("mode_profile", _) | (_, Some("mode.profile")) => {
            return Some(("/mode".into(), "/mode ".into()))
        }
        ("notify", _) | (_, Some("notify.settings")) => "/settings notify".into(),
        _ if id.trim().is_empty() => return None,
        _ => format!("/{id}"),
    };
    Some((label.clone(), label))
}

fn extension_shortcuts(snapshot: &ExtensionManagerSnapshot) -> Vec<ExtensionShortcut> {
    snapshot
        .registries
        .keymaps
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
        .flat_map(|active| {
            active
                .entry
                .contribution
                .default_bindings
                .iter()
                .filter_map(|binding| {
                    let sequence = binding.parse::<KeySequence>().ok()?;
                    let source = active
                        .entry
                        .metadata
                        .extension_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| active.entry.metadata.source.scope.slug().into());
                    Some(ExtensionShortcut::new(
                        active.entry.contribution.action.clone(),
                        sequence,
                        source,
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn extension_autosuggest_items(
    snapshot: &ExtensionManagerSnapshot,
) -> Vec<ExtensionAutosuggestItem> {
    snapshot
        .registries
        .autosuggest_providers
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
        .flat_map(|active| {
            let trigger = active.entry.contribution.trigger.clone();
            let source = active
                .entry
                .metadata
                .extension_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| active.effective_id.to_string());
            let fallback_label = if active.entry.contribution.label.trim().is_empty() {
                active.effective_id.to_string()
            } else {
                active.entry.contribution.label.clone()
            };
            if active.entry.contribution.items.is_empty() {
                return vec![ExtensionAutosuggestItem {
                    label: fallback_label.clone(),
                    summary: format!("Extension autosuggest trigger `{trigger}`"),
                    replacement: trigger.clone(),
                    trigger,
                    source,
                }];
            }
            active
                .entry
                .contribution
                .items
                .iter()
                .map(|item| ExtensionAutosuggestItem {
                    label: item.label.clone(),
                    summary: if item.detail.trim().is_empty() {
                        fallback_label.clone()
                    } else {
                        item.detail.clone()
                    },
                    replacement: item.replacement.clone(),
                    trigger: trigger.clone(),
                    source: source.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn theme_catalog_from_sources(
    snapshot: &ExtensionManagerSnapshot,
    paths: &ResourcePaths,
) -> ThemeCatalog {
    let mut catalog = ThemeCatalog::builtins();
    register_theme_dir(
        &mut catalog,
        &paths.global_themes_dir,
        ThemeSource {
            kind: ThemeSourceKind::File,
            scope: ThemeSourceScope::Global,
        },
    );
    for active in snapshot
        .registries
        .themes
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
    {
        let document = extension_theme_document(active);
        catalog.register(ThemeCatalogEntry::new(
            extension_theme_source(active.entry.metadata.source.scope),
            document,
        ));
    }
    register_theme_dir(
        &mut catalog,
        &paths.project_themes_dir,
        ThemeSource {
            kind: ThemeSourceKind::File,
            scope: ThemeSourceScope::Project,
        },
    );
    catalog
}

fn register_theme_dir(catalog: &mut ThemeCatalog, dir: &Path, source: ThemeSource) {
    for path in discover_theme_files(dir) {
        if let Some(document) = read_theme_document_from_path(&path) {
            catalog.register(ThemeCatalogEntry::new(source, document));
        }
    }
}

fn discover_theme_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    discover_theme_files_into(dir, &mut out);
    out.sort();
    out
}

fn discover_theme_files_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            discover_theme_files_into(&path, out);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

fn read_theme_document_from_path(path: &Path) -> Option<ThemeDocument> {
    let text = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    if let Ok(mut document) = serde_json::from_value::<ThemeDocument>(value.clone()) {
        if document.normalized_id().is_none() {
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                document.id = stem.to_string();
            }
        }
        if document.display_name.trim().is_empty() {
            document.display_name = document.id.clone();
        }
        if document.inherits.is_none() {
            document.inherits = Some("system".into());
        }
        return Some(document);
    }
    let tokens = legacy_theme_token_map(&value)?;
    let id = path.file_stem()?.to_string_lossy().to_string();
    Some(ThemeDocument {
        schema_version: 1,
        display_name: id.clone(),
        id,
        description: Some(format!("Theme file {}", path.display())),
        mode: oino_tui::ThemeMode::Dark,
        inherits: Some("system".into()),
        palette: BTreeMap::new(),
        tokens,
    })
}

fn extension_theme_document(
    active: &ActiveContribution<oino_extension_core::ThemeContribution>,
) -> ThemeDocument {
    if let Some(path) = extension_theme_file_path(active) {
        if let Some(document) = read_extension_theme_document(&path, active) {
            return document;
        }
    }
    ThemeDocument {
        schema_version: 1,
        id: active.effective_id.to_string(),
        display_name: active.effective_id.to_string(),
        description: Some("Extension theme contribution".into()),
        mode: oino_tui::ThemeMode::Dark,
        inherits: Some("system".into()),
        palette: BTreeMap::new(),
        tokens: active.entry.contribution.tokens.clone(),
    }
}

fn read_extension_theme_document(
    path: &Path,
    active: &ActiveContribution<oino_extension_core::ThemeContribution>,
) -> Option<ThemeDocument> {
    let text = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    if let Some(tokens) = legacy_theme_token_map(&value) {
        return Some(ThemeDocument {
            schema_version: 1,
            id: active.effective_id.to_string(),
            display_name: active.effective_id.to_string(),
            description: Some(format!("Extension theme from {}", path.display())),
            mode: oino_tui::ThemeMode::Dark,
            inherits: Some("system".into()),
            palette: BTreeMap::new(),
            tokens: if tokens.is_empty() {
                active.entry.contribution.tokens.clone()
            } else {
                tokens
            },
        });
    }
    if let Ok(mut document) = serde_json::from_value::<ThemeDocument>(value) {
        if document.normalized_id().is_none() {
            document.id = active.effective_id.to_string();
        }
        if document.display_name.trim().is_empty() {
            document.display_name = active.effective_id.to_string();
        }
        if document.inherits.is_none() {
            document.inherits = Some("system".into());
        }
        return Some(document);
    }
    None
}

fn legacy_theme_token_map(value: &serde_json::Value) -> Option<BTreeMap<String, String>> {
    let object = value.as_object()?;
    if object.contains_key("schema_version")
        || object.contains_key("id")
        || object.contains_key("tokens")
        || object.contains_key("palette")
    {
        return None;
    }
    object
        .iter()
        .map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}

fn extension_theme_file_path(
    active: &ActiveContribution<oino_extension_core::ThemeContribution>,
) -> Option<PathBuf> {
    let path = Path::new(&active.entry.contribution.path);
    let manifest_path = active.entry.metadata.source.path.as_ref().or_else(|| {
        active
            .entry
            .metadata
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.manifest_path.as_ref())
    })?;
    let manifest_dir = manifest_path.parent()?;
    let base =
        package_root_for_manifest(manifest_dir).unwrap_or_else(|| manifest_dir.to_path_buf());
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    if !theme_path_is_within_base(&candidate, &base) {
        return None;
    }
    candidate.exists().then_some(candidate)
}

fn package_root_for_manifest(manifest_dir: &Path) -> Option<PathBuf> {
    let mut current = Some(manifest_dir);
    while let Some(dir) = current {
        if dir.join("oino.package.json").is_file() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn theme_path_is_within_base(path: &Path, base: &Path) -> bool {
    let Ok(base) = base.canonicalize() else {
        return false;
    };
    match path.canonicalize() {
        Ok(path) => path.starts_with(base),
        Err(_) => path
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .is_some_and(|parent| parent.starts_with(base)),
    }
}

const fn extension_theme_source(scope: SourceScope) -> ThemeSource {
    let scope = match scope {
        SourceScope::BuiltIn => ThemeSourceScope::BuiltIn,
        SourceScope::Global => ThemeSourceScope::Global,
        SourceScope::Project | SourceScope::Session | SourceScope::Development => {
            ThemeSourceScope::Project
        }
    };
    ThemeSource {
        kind: ThemeSourceKind::Extension,
        scope,
    }
}

fn extension_runtime_provider_ids_from_snapshot(
    snapshot: &ExtensionManagerSnapshot,
) -> BTreeSet<String> {
    snapshot
        .registries
        .providers
        .active
        .iter()
        .filter(|active| active.entry.contribution.runtime.is_some())
        .map(|active| active.entry.contribution.provider_id.clone())
        .collect()
}

fn validate_model_identifier_with_extensions(
    model_identifier: &str,
    extension_runtime_provider_ids: &BTreeSet<String>,
) -> Result<Model, String> {
    let Some(model) = Model::from_identifier(model_identifier) else {
        return Err(format!(
            "Invalid model identifier `{model_identifier}`; expected provider:model-id"
        ));
    };
    if extension_runtime_provider_ids.contains(&model.provider) {
        return Ok(model);
    }
    Err(format!(
        "Provider `{}` is not provided by an enabled extension runtime. Built-in provider runtime has been removed; run `/router setup` and select a `router:<model>` model, or install/enable an extension runtime provider for `{}`.",
        model.provider, model.provider
    ))
}

fn extension_model_options(snapshot: &ExtensionManagerSnapshot) -> Vec<ModelOption> {
    snapshot
        .registries
        .providers
        .active
        .iter()
        .filter(|active| active.entry.metadata.source.scope != SourceScope::BuiltIn)
        .filter(|active| {
            active.entry.contribution.runtime.is_some()
                || !active.entry.contribution.privacy.can_receive_prompts
        })
        .flat_map(|active| {
            let provider_id = &active.entry.contribution.provider_id;
            let display = if active.entry.contribution.display_name.trim().is_empty() {
                provider_id.as_str()
            } else {
                active.entry.contribution.display_name.as_str()
            };
            active
                .entry
                .contribution
                .model_ids
                .iter()
                .filter_map(move |model_id| {
                    let id = format!("{provider_id}:{model_id}");
                    Model::from_identifier(&id).map(|_| {
                        let mut option = ModelOption::new(id)
                            .with_display_name(format!("{display} {model_id}"))
                            .with_provider_label(format!("{display} extension"));
                        if provider_id.as_str() == model_catalog::ROUTER_PROVIDER_ID {
                            option = option.with_context_length(Some(200_000));
                        }
                        option
                    })
                })
        })
        .collect()
}

fn merge_extension_models(
    mut models: Vec<ModelOption>,
    extension_models: &[ModelOption],
) -> Vec<ModelOption> {
    let mut seen = models
        .iter()
        .map(|model| model.id.clone())
        .collect::<BTreeSet<_>>();
    for model in extension_models {
        if seen.insert(model.id.clone()) {
            models.push(model.clone());
        }
    }
    models
}

fn extension_management_items(
    snapshot: &ExtensionManagerSnapshot,
    settings: &ToolSettingsSnapshot,
) -> Vec<ExtensionManagementItem> {
    let conflicts = extension_conflict_map(snapshot);
    let mut items = Vec::new();
    items.extend(snapshot.extensions.iter().map(|record| {
        ExtensionManagementItem {
            target: ExtensionManagementTarget::Extension,
            id: record.id.to_string(),
            title: record.display_name.clone(),
            family: "extension".into(),
            scope: record.source.scope.slug().into(),
            health: format!("{:?}", record.health),
            state: format!("{:?}", record.lifecycle),
            permission: permission_summary(&record.permissions),
            provenance: provenance_summary(record.provenance.as_ref()),
            diagnostics: record
                .diagnostics
                .iter()
                .map(oino_extension_core::ExtensionDiagnostic::format_message)
                .collect(),
            conflicts: Vec::new(),
            entry_key: None,
            canonical_id: None,
            global_override: false,
            project_override: false,
            global_enabled: extension_enabled_global(settings, &record.id),
            project_enabled: extension_enabled_project(settings, &record.id),
        }
    }));
    items.extend(snapshot.packages.iter().map(|record| {
        ExtensionManagementItem {
            target: ExtensionManagementTarget::Package,
            id: record.id.to_string(),
            title: record.display_name.clone(),
            family: "package".into(),
            scope: record.source.scope.slug().into(),
            health: format!("{:?}", record.health),
            state: format!("{:?}", record.lifecycle),
            permission: "package".into(),
            provenance: String::new(),
            diagnostics: record
                .diagnostics
                .iter()
                .map(oino_extension_core::ExtensionDiagnostic::format_message)
                .collect(),
            conflicts: Vec::new(),
            entry_key: None,
            canonical_id: None,
            global_override: false,
            project_override: false,
            global_enabled: package_enabled_global(settings, &record.id),
            project_enabled: package_enabled_project(settings, &record.id),
        }
    }));
    items.extend(snapshot.contributions.iter().map(|record| {
        let contribution_conflicts = conflicts
            .get(&record.id.to_string())
            .cloned()
            .unwrap_or_default();
        ExtensionManagementItem {
            target: ExtensionManagementTarget::Contribution,
            id: record.id.to_string(),
            title: record.entry_key.to_string(),
            family: record.family.label().into(),
            scope: record.source.scope.slug().into(),
            health: format!("{:?}", record.health),
            state: format!("{:?}", record.state),
            permission: permission_decision_summary(&record.permission),
            provenance: provenance_summary(record.provenance.as_ref()),
            diagnostics: record
                .diagnostics
                .iter()
                .map(oino_extension_core::ExtensionDiagnostic::format_message)
                .collect(),
            conflicts: contribution_conflicts,
            entry_key: Some(record.entry_key.to_string()),
            canonical_id: Some(record.canonical_id.to_string()),
            global_override: contribution_override_global(
                settings,
                &record.canonical_id,
                &record.entry_key,
            ),
            project_override: contribution_override_project(
                settings,
                &record.canonical_id,
                &record.entry_key,
            ),
            global_enabled: contribution_enabled_global(settings, &record.canonical_id),
            project_enabled: contribution_enabled_project(settings, &record.canonical_id),
        }
    }));
    items.sort_by(|left, right| {
        left.target
            .label()
            .cmp(right.target.label())
            .then(left.family.cmp(&right.family))
            .then(left.id.cmp(&right.id))
    });
    items
}

fn extension_conflict_map(snapshot: &ExtensionManagerSnapshot) -> BTreeMap<String, Vec<String>> {
    let mut conflicts = BTreeMap::<String, Vec<String>>::new();
    for conflict in &snapshot.registries.ui_surfaces.diagnostics {
        if let Some(id) = &conflict.contribution_id {
            conflicts
                .entry(id.to_string())
                .or_default()
                .push(conflict.message.clone());
        }
    }
    for diagnostic in &snapshot.diagnostics {
        if let Some(id) = &diagnostic.contribution_id {
            if diagnostic.message.contains("conflict") || diagnostic.message.contains("duplicate") {
                conflicts
                    .entry(id.to_string())
                    .or_default()
                    .push(diagnostic.message.clone());
            }
        }
    }
    conflicts
}

fn permission_summary(permissions: &oino_extension_core::ExtensionPermissions) -> String {
    format!(
        "tools:{} commands:{} ui:{} host:{}",
        permissions.tools.len(),
        permissions.commands.len(),
        permissions.ui.len(),
        permissions.host_capabilities.len()
    )
}

fn permission_decision_summary(permission: &oino_extension_core::PermissionDecision) -> String {
    match permission {
        oino_extension_core::PermissionDecision::Granted => "granted".into(),
        oino_extension_core::PermissionDecision::PendingReview(reason) => {
            format!("pending: {reason}")
        }
        oino_extension_core::PermissionDecision::Denied(reason) => format!("denied: {reason}"),
    }
}

fn provenance_summary(provenance: Option<&oino_extension_core::Provenance>) -> String {
    provenance.map_or_else(String::new, |provenance| {
        let package = provenance
            .package_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let extension = provenance
            .extension_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        format!("{} {}", package, extension).trim().to_string()
    })
}

fn synthetic_renderer_surface(
    active: &ActiveContribution<RendererContribution>,
    surface: UiSurfaceKind,
    prefix: &str,
) -> ActiveContribution<UiSurfaceContribution> {
    synthetic_ui_surface(
        &active.effective_id,
        &active.entry.metadata,
        surface,
        &format!("{prefix} {:?}", active.entry.contribution.target),
        5,
    )
}

fn synthetic_diagnostic_surface(
    active: &ActiveContribution<DiagnosticContribution>,
) -> ActiveContribution<UiSurfaceContribution> {
    synthetic_ui_surface(
        &active.effective_id,
        &active.entry.metadata,
        UiSurfaceKind::Notification,
        &active.entry.contribution.title,
        5,
    )
}

fn synthetic_health_surface(
    active: &ActiveContribution<HealthContribution>,
) -> ActiveContribution<UiSurfaceContribution> {
    synthetic_ui_surface(
        &active.effective_id,
        &active.entry.metadata,
        UiSurfaceKind::Health,
        &active.entry.contribution.title,
        5,
    )
}

fn synthetic_ui_surface(
    id: &ContributionId,
    metadata: &ContributionMetadata,
    surface: UiSurfaceKind,
    title: &str,
    priority: i32,
) -> ActiveContribution<UiSurfaceContribution> {
    let mut scopes = BTreeSet::new();
    scopes.insert(format!("extension.{}", surface.as_permission_name()));
    ActiveContribution {
        effective_id: id.clone(),
        entry: RegistryEntry::new(
            RegistryEntryKey::new(format!("synthetic-ui:{surface:?}:{id}")),
            metadata.clone(),
            UiSurfaceContribution {
                id: id.clone(),
                surface,
                title: title.into(),
                state_schema: Some("object".into()),
                layout: UiLayoutPolicy {
                    slot: surface.default_slot().into(),
                    priority,
                    min_width: 24,
                    min_height: 8,
                    max_width: Some(34),
                    tiny_terminal: UiTinyTerminalFallback::CompactBadge,
                },
                visibility: UiVisibilityPolicy::Visible,
                focus: UiFocusPolicy::None,
                key_dispatch: UiKeyDispatchPolicy {
                    scopes,
                    pass_through: true,
                },
                conflict: Default::default(),
            },
        ),
    }
}

fn extension_tool_map(snapshot: &ExtensionManagerSnapshot) -> BTreeMap<String, Arc<dyn Tool>> {
    let mut tools = BTreeMap::new();
    for active in &snapshot.registries.tools.active {
        if active.entry.metadata.source.scope == SourceScope::BuiltIn
            || active
                .entry
                .metadata
                .package_id
                .as_ref()
                .is_some_and(|package_id| {
                    matches!(package_id.as_str(), VCC_PACKAGE_ID | ASK_USER_PACKAGE_ID)
                })
        {
            continue;
        }
        let Some(extension_id) = active.entry.metadata.extension_id.clone() else {
            continue;
        };
        let contribution = active.entry.contribution.clone();
        if let Some(runtime) = fixture_runtime_for_tool(&extension_id, &contribution) {
            tools.insert(
                contribution.id.to_string(),
                Arc::new(ExtensionToolAdapter::new(
                    contribution,
                    extension_id,
                    runtime,
                )) as Arc<dyn Tool>,
            );
        }
    }
    tools
}

fn fixture_runtime_for_tool(
    extension_id: &oino_extension_core::ExtensionId,
    contribution: &oino_extension_core::ToolContribution,
) -> Option<SharedRuntime> {
    let handler = contribution
        .handler
        .clone()
        .unwrap_or_else(|| contribution.id.to_string());
    let module = FixtureWasmModule::default().with_handler(
        handler,
        FixtureHandlerBehavior::Success {
            output: serde_json::json!({
                "status": "extension runtime bridge invoked",
                "tool": contribution.id.to_string(),
                "note": "wasm-json-v1 fixture runtime is active; replace with a real WASM handler when available"
            }),
            progress: vec![RuntimeProgress {
                message: format!("Invoked extension tool {}", contribution.id),
                details: None,
            }],
        },
    );
    let mut runtime = JsonWasmRuntime::new(module);
    runtime
        .initialize(RuntimeInitialize {
            extension_id: extension_id.clone(),
            abi: WASM_JSON_V1_ABI.into(),
            entry: "fixture-runtime".into(),
            metadata: serde_json::Value::Null,
        })
        .ok()?;
    Some(Arc::new(Mutex::new(Box::new(runtime))))
}

async fn execute_extension_command(
    input: &str,
    snapshot: &ExtensionManagerSnapshot,
) -> Option<Result<String, AppError>> {
    let trimmed = input.trim();
    let command_name = trimmed
        .strip_prefix('/')?
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let contribution_id = ContributionId::new(command_name).ok()?;
    let active = snapshot.registries.commands.active.iter().find(|active| {
        active.effective_id == contribution_id
            && (active.entry.metadata.source.scope != SourceScope::BuiltIn
                || active
                    .entry
                    .metadata
                    .package_id
                    .as_ref()
                    .is_some_and(|package_id| package_id.as_str() == ROUTER_PACKAGE_ID))
    })?;
    let contribution = &active.entry.contribution;
    let handler = contribution
        .handler
        .clone()
        .unwrap_or_else(|| contribution.id.to_string());
    if active
        .entry
        .metadata
        .package_id
        .as_ref()
        .is_some_and(|package_id| package_id.as_str() == ROUTER_PACKAGE_ID)
        || handler == "router.command"
    {
        return Some(execute_router_command_input(trimmed).await);
    }
    let extension_id = active.entry.metadata.extension_id.clone()?;
    let module = FixtureWasmModule::default().with_handler(
        handler.clone(),
        FixtureHandlerBehavior::Success {
            output: serde_json::json!({
                "status": "extension command bridge invoked",
                "command": contribution.id.to_string(),
                "input": trimmed,
            }),
            progress: Vec::new(),
        },
    );
    let mut runtime = JsonWasmRuntime::new(module);
    if let Err(err) = runtime.initialize(RuntimeInitialize {
        extension_id: extension_id.clone(),
        abi: WASM_JSON_V1_ABI.into(),
        entry: "fixture-runtime".into(),
        metadata: serde_json::Value::Null,
    }) {
        return Some(Err(AppError::InvalidArguments(err.to_string())));
    }
    let adapter = ExtensionCommandAdapter::new(
        contribution.id.clone(),
        contribution.description.clone(),
        extension_id,
        handler,
        Arc::new(Mutex::new(Box::new(runtime))),
    );
    Some(
        adapter
            .execute(serde_json::json!({ "input": trimmed }))
            .map(|output| output.to_string())
            .map_err(|err| AppError::InvalidArguments(err.to_string())),
    )
}

async fn apply_tool_settings_to_harness(
    harness: &Harness,
    settings: &ToolSettingsSnapshot,
    paths: &ResourcePaths,
    cwd: &Path,
    mode: AgentMode,
    ask_user_tx: Option<mpsc::UnboundedSender<TuiRuntimeEvent>>,
) {
    let mut available = oino_tools::default_tools(harness.env(), cwd.to_path_buf());
    available.insert(
        oino_tools::SESSION_TITLE_TOOL_NAME.into(),
        oino_tools::session_title_tool(harness.session_title_setter()),
    );
    let snapshot = load_extension_snapshot(paths, settings);
    if optional_package_tool_active(&snapshot, VCC_PACKAGE_ID, VCC_RECALL_TOOL_NAME) {
        available.insert(
            VCC_RECALL_TOOL_NAME.into(),
            Arc::new(VccRecallTool::new(harness.session_handle())) as Arc<dyn Tool>,
        );
    }
    if optional_package_tool_active(&snapshot, ASK_USER_PACKAGE_ID, ASK_USER_TOOL_NAME) {
        let requester = ask_user_tx.map(ask_user_requester);
        available.insert(
            ASK_USER_TOOL_NAME.into(),
            Arc::new(AskUserTool::new(requester)) as Arc<dyn Tool>,
        );
    }
    available.extend(extension_tool_map(&snapshot));
    let mode_profile = mode_sandbox_enabled_in_snapshot(&snapshot)
        .then(|| load_mode_sandbox_profile(paths, &mode));
    let active_tool_names = match oino_extension_builtins::tool_registry_from_tools(&available) {
        Ok(registry) => {
            let policy = tool_registry_policy_from_settings(settings, available.keys().cloned());
            registry
                .compose(&policy)
                .active
                .into_iter()
                .map(|entry| entry.effective_id.to_string())
                .collect::<BTreeSet<_>>()
        }
        Err(_) => available
            .keys()
            .filter(|name| project_tool_enabled(settings, name))
            .cloned()
            .collect::<BTreeSet<_>>(),
    };
    let tools = available
        .into_iter()
        .filter(|(name, tool)| {
            let active = active_tool_names.contains(name)
                || tool.definition().name == *name
                    && snapshot
                        .registries
                        .tools
                        .active
                        .iter()
                        .any(|active| active.effective_id.as_str() == name);
            active
                && mode_profile
                    .as_ref()
                    .is_none_or(|profile| profile.allows_tool(name))
        })
        .collect::<BTreeMap<String, Arc<dyn Tool>>>();
    harness.set_tools(tools).await;
}

fn ask_user_requester(tx: mpsc::UnboundedSender<TuiRuntimeEvent>) -> AskUserRequester {
    Arc::new(move |request| {
        let tx = tx.clone();
        let fut: BoxFuture<'static, oino_agent_loop::LoopResult<AskUserOutcome>> =
            Box::pin(async move {
                let (responder, response_rx) = oneshot::channel();
                if tx
                    .send(TuiRuntimeEvent::AskUserPrompt { request, responder })
                    .is_err()
                {
                    return Ok(no_ui_ask_user_outcome());
                }
                Ok(response_rx
                    .await
                    .unwrap_or_else(|_| no_ui_ask_user_outcome()))
            });
        fut
    })
}

fn no_ui_ask_user_outcome() -> AskUserOutcome {
    AskUserOutcome {
        answers: Vec::new(),
        cancelled: true,
        error: Some("no_ui".into()),
    }
}

fn load_resource_catalog(paths: &ResourcePaths) -> Result<ResourceCatalog, AppError> {
    paths.ensure_skeleton()?;
    Ok(paths.load_catalog())
}

fn export_chat_html(state: &TuiState, exports_dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(exports_dir)?;
    let path = unique_export_path(exports_dir);
    fs::write(&path, render_chat_export_html(state))?;
    Ok(path)
}

fn unique_export_path(exports_dir: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let stem = format!("chat-{timestamp}");
    for index in 0..1000usize {
        let file_name = if index == 0 {
            format!("{stem}.html")
        } else {
            format!("{stem}-{index}.html")
        };
        let path = exports_dir.join(file_name);
        if !path.exists() {
            return path;
        }
    }
    exports_dir.join(format!("{stem}-{}.html", std::process::id()))
}

fn render_chat_export_html(state: &TuiState) -> String {
    let title = if state.session_title.trim().is_empty() {
        "Oino Chat Export"
    } else {
        state.session_title.trim()
    };
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str(&format!("<title>{}</title>\n", html_escape(title)));
    html.push_str(
        "<style>\n:root{color-scheme:dark light;font-family:Inter,ui-sans-serif,system-ui,sans-serif;}\nbody{margin:0;padding:2rem;background:#0b1020;color:#e6edf3;}\nmain{max-width:920px;margin:0 auto;}\nh1{margin:0 0 .25rem;font-size:1.8rem;}\n.meta{color:#8b949e;margin:0 0 1.5rem;}\n.message{border:1px solid #30363d;border-radius:12px;padding:1rem;margin:1rem 0;background:#111827;}\n.message.user{background:#102a43;}\n.message.assistant{background:#1f2937;}\n.message.tool{background:#261f12;}\n.header{display:flex;gap:.75rem;align-items:baseline;margin-bottom:.75rem;}\n.role{font-weight:700;text-transform:uppercase;letter-spacing:.06em;font-size:.78rem;color:#58a6ff;}\n.title{color:#8b949e;font-size:.85rem;}\npre{white-space:pre-wrap;word-break:break-word;margin:.5rem 0 0;font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;}\ndetails{margin:.75rem 0;color:#c9d1d9;}\nsummary{cursor:pointer;color:#d29922;}\n.empty{color:#8b949e;}\n</style>\n</head>\n<body>\n<main>\n",
    );
    html.push_str(&format!("<h1>{}</h1>\n", html_escape(title)));
    html.push_str(&format!(
        "<p class=\"meta\">Exported from Oino • {} message{}</p>\n",
        state.messages.len(),
        if state.messages.len() == 1 { "" } else { "s" }
    ));
    if state.messages.is_empty() {
        html.push_str("<p class=\"empty\">No messages.</p>\n");
    } else {
        for message in &state.messages {
            html.push_str(&render_message_export_html(message));
        }
    }
    html.push_str("</main>\n</body>\n</html>\n");
    html
}

fn render_message_export_html(message: &MessageView) -> String {
    let class = message_role_class(&message.role);
    let mut html = String::new();
    html.push_str(&format!(
        "<section class=\"message {}\" data-message-id=\"{}\">\n<div class=\"header\"><span class=\"role\">{}</span>",
        class,
        html_escape(&message.id.to_string()),
        html_escape(&message.role)
    ));
    if let Some(title) = message.title.as_deref().filter(|title| !title.is_empty()) {
        html.push_str(&format!(
            "<span class=\"title\">{}</span>",
            html_escape(title)
        ));
    }
    html.push_str("</div>\n");
    if let Some(thinking) = message.thinking.as_deref() {
        let label = if message.thinking_redacted {
            "Thinking (redacted)"
        } else {
            "Thinking"
        };
        html.push_str(&format!(
            "<details><summary>{}</summary><pre>{}</pre></details>\n",
            label,
            html_escape(thinking)
        ));
    }
    if !message.content.trim().is_empty() {
        html.push_str(&format!("<pre>{}</pre>\n", html_escape(&message.content)));
    }
    for call in &message.tool_calls {
        let arguments = serde_json::to_string_pretty(&call.arguments)
            .unwrap_or_else(|_| call.arguments.to_string());
        html.push_str(&format!(
            "<details><summary>Tool call: {}</summary><pre>{}</pre></details>\n",
            html_escape(&call.name),
            html_escape(&arguments)
        ));
    }
    html.push_str("</section>\n");
    html
}

fn message_role_class(role: &str) -> String {
    if role.starts_with("tool:") {
        return "tool".into();
    }
    role.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn html_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn default_system_prompt(cwd: &std::path::Path, catalog: &ResourceCatalog) -> String {
    let mut sections = Vec::new();
    for section in catalog.system_prompt_sections() {
        sections.push(format!(
            "# {}\n\n<!-- {} -->\n{}",
            section.title,
            section.path.display(),
            section.content
        ));
    }
    sections.push(
        "# Oino Resource Inclusion Policy\n\nPrompt templates and skills are explicit composer resources. Only use prompt or skill content when it has been included in the user message via `/prompt:<name>` or `/skill:<name>`. Do not auto-load skills from discovery metadata.".into(),
    );
    sections.push(format!("Current working directory: {}", cwd.display()));
    sections.join("\n\n---\n\n")
}

async fn run_tui(
    harness: Harness,
    auth: AuthStorage,
    launch: TuiLaunchConfig,
) -> Result<(), AppError> {
    let TuiLaunchConfig {
        initial_model,
        initial_thinking_level,
        initial_thinking_collapse_mode,
        initial_tool_collapse_mode,
        initial_chat_style,
        initial_keymap,
        provider_config,
        mut session_path,
        resource_paths,
        resource_catalog,
        open_settings,
    } = launch;
    let mut terminal = TerminalGuard::enter()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut tool_settings = load_tool_settings(&resource_paths).await;
    apply_tool_settings_to_harness(
        &harness,
        &tool_settings,
        &resource_paths,
        &cwd,
        AgentMode::Work,
        None,
    )
    .await;
    let mut state = TuiState::with_settings(initial_model.clone(), initial_thinking_level);
    state.set_btw_configured_model(tool_settings.global.btw_model.clone(), &initial_model);
    state.set_working_directory(path_to_string(&cwd));
    state.set_git_branch(current_git_branch(&cwd));
    let agent_mode = Arc::new(Mutex::new(state.agent_mode.clone()));
    let extension_snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
    if mode_sandbox_enabled_in_snapshot(&extension_snapshot) {
        let _ = ensure_mode_sandbox_profiles(&resource_paths);
    }
    let mut extension_models = extension_model_options(&extension_snapshot);
    let mut extension_runtime_provider_ids =
        extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
    state.set_session_title(harness.session_title().await);
    state.set_tool_settings(tool_settings_items(&tool_settings));
    state.set_theme_settings(&tool_settings.global.theme, &tool_settings.project.theme);
    {
        let user_settings = user_settings::UserSettings::load_default()
            .await
            .unwrap_or_default();
        state.settings.compact.update_from_settings(
            user_settings.compact.threshold_pct,
            matches!(user_settings.compact.method, oino_types::CompactMethod::Llm),
            user_settings.compact.auto,
            user_settings.compact.model.clone(),
            user_settings.compact.prompt.clone(),
        );
    }
    apply_extension_snapshot_to_tui_state(
        &mut state,
        &extension_snapshot,
        &tool_settings,
        &resource_paths,
    );
    apply_resource_catalog_to_state(&mut state, &resource_catalog, &extension_snapshot);
    state.set_file_paths(scan_project_files(&cwd));
    state
        .settings
        .set_collapse_modes(initial_thinking_collapse_mode, initial_tool_collapse_mode);
    state.settings.set_chat_style(initial_chat_style);
    state.set_keymap(initial_keymap);
    if let Ok(messages) = harness.build_context().await {
        state.set_messages_from_oino(&messages);
    }
    if let Ok(report) = usage_report_for_current_session(&harness, &auth, &initial_model).await {
        state.set_usage_report(report.to_tui_report());
    }
    refresh_tui_context_status(&mut state, &harness, &cwd).await;
    if let Ok(items) =
        auth_status_items_with_extension_readiness(&auth, None, &initial_model, &extension_snapshot)
            .await
    {
        // First-run onboarding: if no credentials configured, show setup hint
        let has_configured = items.iter().any(|item| {
            item.readiness == "configured"
                || item.source == "stored"
                || item.source == "environment"
        });
        state.set_auth_status_items(items, None);
        if !has_configured {
            state.status = "No router/auth configured. Type /router setup to get started, or /auth quickstart for the extension setup guide.".into();
        }
    }
    if open_settings {
        state.open_settings();
    }
    let mut applied_thinking_level = initial_thinking_level;
    let harness = Arc::new(harness);
    let (tx, mut rx) = mpsc::unbounded_channel();
    apply_tool_settings_to_harness(
        &harness,
        &tool_settings,
        &resource_paths,
        &cwd,
        state.agent_mode.clone(),
        Some(tx.clone()),
    )
    .await;
    register_tui_stream_hooks(&harness, tx.clone()).await;
    register_mode_hooks(&harness, Arc::clone(&agent_mode), resource_paths.clone()).await;
    let btw_provider_config = provider_config.clone();
    let notify_stream = build_runtime_provider(
        auth.clone(),
        provider_config.clone(),
        extension_runtime_providers(&extension_snapshot),
    );
    register_notify_hooks(&harness, resource_paths.clone(), notify_stream).await;
    spawn_model_catalog_task(
        tx.clone(),
        auth.clone(),
        provider_config.clone(),
        initial_model,
    );
    let mut prompt_in_flight = false;
    let mut btw_harness: Option<Arc<Harness>> = None;
    let mut btw_in_flight = false;
    let mut ralph_controller = RalphRunController::default();
    let mut pending_ask_user: Option<oneshot::Sender<AskUserOutcome>> = None;
    loop {
        let mut prompt_finished = false;
        let mut finished_prompt_result: Option<Result<Vec<Message>, String>> = None;
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiRuntimeEvent::AskUserPrompt { request, responder } => {
                    if let Some(previous) = pending_ask_user.take() {
                        let _ = previous.send(AskUserOutcome {
                            answers: Vec::new(),
                            cancelled: true,
                            error: Some("replaced".into()),
                        });
                    }
                    pending_ask_user = Some(responder);
                    state.open_ask_user_overlay(request);
                }
                other => {
                    if let TuiRuntimeEvent::BtwFinished(result) = &other {
                        btw_in_flight = false;
                        match result {
                            Ok(messages) => {
                                state.set_btw_messages_from_oino(messages);
                                state.status = "BTW complete".into();
                            }
                            Err(message) => {
                                state.set_btw_error(message.clone());
                                state.status = HELP_STATUS.into();
                            }
                        }
                        continue;
                    }
                    if let TuiRuntimeEvent::PromptFinished(result) = &other {
                        prompt_finished = true;
                        finished_prompt_result = Some(result.clone());
                    }
                    apply_tui_runtime_event(
                        &mut state,
                        other,
                        &mut prompt_in_flight,
                        &extension_models,
                    );
                }
            }
        }
        if prompt_finished {
            if let Some(Ok(messages)) = finished_prompt_result {
                continue_ralph_after_prompt_if_needed(
                    &mut state,
                    &mut ralph_controller,
                    &auth,
                    &harness,
                    &tx,
                    &session_path,
                    &resource_paths,
                    &mut prompt_in_flight,
                    &extension_runtime_provider_ids,
                    &messages,
                )
                .await;
            }
            start_next_queued_prompt_if_idle(
                &mut state,
                &auth,
                &harness,
                &tx,
                &session_path,
                &mut prompt_in_flight,
                &extension_runtime_provider_ids,
            )
            .await;
            refresh_tui_context_status(&mut state, &harness, &cwd).await;
            try_auto_compact(
                &mut state,
                &harness,
                &auth,
                &provider_config,
                &extension_snapshot,
                &session_path,
                &cwd,
            )
            .await;
        }
        if applied_thinking_level != state.settings.selected_thinking_level {
            applied_thinking_level = state.settings.selected_thinking_level;
            if let Err(err) = harness.set_thinking_level(applied_thinking_level).await {
                state.set_error(err.to_string());
            }
        }
        terminal.draw(&state)?;
        if let Ok((width, height)) = terminal.size() {
            state.set_transcript_page_lines(transcript_visible_lines(&state, width, height));
        }
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let action = match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                state.handle_key(key)
            }
            Event::Paste(text) => {
                if let Some(inserted) = dropped_file_paths_to_mentions(&text, &cwd) {
                    state.insert_literal(&inserted)
                } else {
                    state.handle_paste(&text)
                }
            }
            Event::Mouse(mouse) => {
                if let Ok((width, height)) = terminal.size() {
                    handle_mouse_event(mouse, &mut state, &cwd, width, height);
                }
                continue;
            }
            _ => continue,
        };
        match action {
            TuiAction::None => {}
            TuiAction::Quit => break,
            TuiAction::OpenInspect => match harness.inspect_full_prompt().await {
                Ok(snapshot) => {
                    state.set_inspect_full_prompt(snapshot.content, snapshot.token_count)
                }
                Err(err) => {
                    state.inspect.loading = false;
                    state.set_error(format!("Inspect failed: {err}"));
                }
            },
            TuiAction::ExportChatHtml => {
                match export_chat_html(&state, &resource_paths.project_exports_dir) {
                    Ok(path) => {
                        let display = path
                            .strip_prefix(&resource_paths.project_root)
                            .unwrap_or(path.as_path())
                            .display();
                        state.set_inspect_export_message(format!("Exported chat to {display}"));
                    }
                    Err(err) => {
                        state.set_inspect_export_message(format!("Chat export failed: {err}"));
                        state.set_error(format!("Chat export failed: {err}"));
                    }
                }
            }
            TuiAction::SetModel(model) => {
                let thinking_level = state.settings.selected_thinking_level;
                let Some(parsed_model) = Model::from_identifier(&model) else {
                    state.set_error(format!(
                        "Invalid model identifier `{model}`; expected provider:model-id"
                    ));
                    continue;
                };
                if let Err(message) = validate_model_identifier_with_extensions(
                    &model,
                    &extension_runtime_provider_ids,
                ) {
                    state.set_error(message);
                    continue;
                }
                if let Err(err) = harness.set_model(parsed_model).await {
                    state.set_error(err.to_string());
                } else if let Err(err) = harness.set_thinking_level(thinking_level).await {
                    state.set_error(err.to_string());
                } else {
                    applied_thinking_level = thinking_level;
                    if state.btw.configured_model.is_none() {
                        let chat_model = state.settings.selected_model().to_string();
                        state.set_btw_configured_model(None, &chat_model);
                        btw_harness = None;
                    }
                    persist_current_settings(&mut state).await;
                    save_tui_session(&mut state, &harness, &session_path).await;
                }
            }
            TuiAction::SetThinkingLevel(level) => {
                if let Err(err) = harness.set_thinking_level(level).await {
                    state.set_error(err.to_string());
                } else {
                    applied_thinking_level = level;
                    persist_current_settings(&mut state).await;
                    save_tui_session(&mut state, &harness, &session_path).await;
                }
            }
            TuiAction::SetCollapseMode(_, _)
            | TuiAction::SetChatStyle(_)
            | TuiAction::SetKeymap(_) => {
                persist_current_settings(&mut state).await;
                save_tui_session(&mut state, &harness, &session_path).await;
            }
            TuiAction::SetToolEnabled {
                name,
                scope,
                enabled,
            } => {
                set_tool_enabled(&mut tool_settings, scope, name, enabled);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                apply_tool_settings_to_harness(
                    &harness,
                    &tool_settings,
                    &resource_paths,
                    &cwd,
                    state.agent_mode.clone(),
                    Some(tx.clone()),
                )
                .await;
                let extension_snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
                extension_models = extension_model_options(&extension_snapshot);
                extension_runtime_provider_ids =
                    extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
                state.set_tool_settings(tool_settings_items(&tool_settings));
                apply_extension_snapshot_to_tui_state(
                    &mut state,
                    &extension_snapshot,
                    &tool_settings,
                    &resource_paths,
                );
                apply_resource_catalog_to_state(&mut state, &resource_catalog, &extension_snapshot);
            }
            TuiAction::SetTheme { id, scope } => {
                set_theme_active(&mut tool_settings, scope, id.clone());
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                state.set_theme_settings(&tool_settings.global.theme, &tool_settings.project.theme);
                state.status = format!("{} theme set to `{id}`", scope.label());
            }
            TuiAction::ResetTheme { scope } => {
                reset_theme(&mut tool_settings, scope);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                state.set_theme_settings(&tool_settings.global.theme, &tool_settings.project.theme);
                state.status = format!("{} theme reset", scope.label());
            }
            TuiAction::SetNotifyEnabled { scope, enabled } => {
                set_notify_enabled(&mut tool_settings, scope, enabled);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                state.settings.set_notify_settings(
                    notify_settings_to_tui(&tool_settings.global.notify),
                    notify_settings_to_tui(&tool_settings.project.notify),
                );
            }
            TuiAction::SetNotifyField {
                scope,
                field,
                value,
            } => {
                set_notify_field(&mut tool_settings, scope, field, value);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                state.settings.set_notify_settings(
                    notify_settings_to_tui(&tool_settings.global.notify),
                    notify_settings_to_tui(&tool_settings.project.notify),
                );
            }
            TuiAction::SetNotifyEvent {
                scope,
                event,
                enabled,
            } => {
                set_notify_event(&mut tool_settings, scope, event, enabled);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                state.settings.set_notify_settings(
                    notify_settings_to_tui(&tool_settings.global.notify),
                    notify_settings_to_tui(&tool_settings.project.notify),
                );
            }
            TuiAction::SetCompactSettings {
                method_is_llm,
                auto_enabled,
                threshold_pct,
            } => {
                let mut settings = user_settings::UserSettings::load_default()
                    .await
                    .unwrap_or_default();
                settings.compact.method = if method_is_llm {
                    oino_types::CompactMethod::Llm
                } else {
                    oino_types::CompactMethod::Vcc
                };
                settings.compact.auto = auto_enabled;
                settings.compact.threshold_pct = threshold_pct;
                if let Err(err) = settings.save_default().await {
                    state.set_error(format!("Failed to save compaction settings: {err}"));
                } else {
                    state.settings.compact.method_is_llm = method_is_llm;
                    state.settings.compact.auto_enabled = auto_enabled;
                    state.settings.compact.threshold_pct = threshold_pct;
                    state.clear_error();
                }
            }
            TuiAction::RunExtensionUiAction {
                surface_id,
                action_id,
            } => {
                state.status = format!("Extension UI action `{surface_id}.{action_id}` queued");
            }
            TuiAction::RunExtensionAction { action } => {
                state.status = format!("Extension shortcut action `{action}` queued");
            }
            TuiAction::AnswerAskUser(outcome) => {
                if let Some(responder) = pending_ask_user.take() {
                    let _ = responder.send(outcome);
                } else {
                    state.status = "No ask-user request is waiting".into();
                }
            }
            TuiAction::RefreshAuthStatus { provider } => {
                match auth_status_items_with_extension_readiness(
                    &auth,
                    provider.as_deref(),
                    &state.settings.selected_model(),
                    &extension_snapshot,
                )
                .await
                {
                    Ok(items) => {
                        let count = items.len();
                        state.set_auth_status_items(items, provider.as_deref());
                        state.set_auth_status_message(format!(
                            "Loaded auth status for {count} provider(s)"
                        ));
                    }
                    Err(err) => state.set_auth_status_error(err.to_string()),
                }
            }
            TuiAction::AuthQuickstart => {
                let message = format_auth_quickstart();
                match auth_status_items_with_extension_readiness(
                    &auth,
                    None,
                    &state.settings.selected_model(),
                    &extension_snapshot,
                )
                .await
                {
                    Ok(items) => state.set_auth_status_items(items, None),
                    Err(err) => state.set_auth_status_error(err.to_string()),
                }
                state.set_auth_status_message(message);
            }
            TuiAction::Compact => {
                let user_settings = user_settings::UserSettings::load_default()
                    .await
                    .unwrap_or_default();
                match user_settings.compact.method {
                    oino_types::CompactMethod::Vcc => {
                        if vcc_command_enabled(&resource_paths, &tool_settings, "compact") {
                            match compact_session_with_vcc(&harness).await {
                                Ok((message, messages)) => {
                                    state.set_messages_from_oino(&messages);
                                    save_tui_session(&mut state, &harness, &session_path).await;
                                    state.clear_error();
                                    state.status = message;
                                    refresh_tui_context_status(&mut state, &harness, &cwd).await;
                                }
                                Err(err) => {
                                    state.set_error(err.to_string());
                                    state.status = HELP_STATUS.into();
                                }
                            }
                        } else {
                            state.set_error(
                                "VCC extension is not enabled; install `builtin:vcc` from `/extensions` before using `/compact`",
                            );
                            state.status = HELP_STATUS.into();
                        }
                    }
                    oino_types::CompactMethod::Llm => {
                        let model_id = user_settings
                            .compact
                            .model
                            .as_deref()
                            .filter(|m| *m != "inherit")
                            .unwrap_or(&state.settings.selected_model());
                        let model = match Model::from_identifier(model_id) {
                            Some(m) => m,
                            None => {
                                state.set_error(format!("Invalid compact model: {model_id}"));
                                state.status = HELP_STATUS.into();
                                return Ok(());
                            }
                        };
                        let compact_stream = build_runtime_provider(
                            auth.clone(),
                            provider_config.clone(),
                            extension_runtime_providers(&extension_snapshot),
                        );
                        match compact_session_with_llm(
                            &harness,
                            &compact_stream,
                            &model,
                            user_settings.compact.prompt.as_deref(),
                            &cwd,
                        )
                        .await
                        {
                            Ok((message, messages)) => {
                                state.set_messages_from_oino(&messages);
                                save_tui_session(&mut state, &harness, &session_path).await;
                                state.clear_error();
                                state.status = message;
                                refresh_tui_context_status(&mut state, &harness, &cwd).await;
                            }
                            Err(err) => {
                                state.set_error(err.to_string());
                                state.status = HELP_STATUS.into();
                            }
                        }
                    }
                }
            }
            TuiAction::Recall { query } => {
                if vcc_command_enabled(&resource_paths, &tool_settings, "recall") {
                    match recall_session_with_vcc(&harness, query, true).await {
                        Ok((output, Some(messages))) => {
                            state.set_messages_from_oino(&messages);
                            save_tui_session(&mut state, &harness, &session_path).await;
                            state.clear_error();
                            state.status = first_line_or_default(&output, "VCC recall complete");
                        }
                        Ok((output, None)) => {
                            state.clear_error();
                            state.status = first_line_or_default(&output, "VCC recall complete");
                        }
                        Err(err) => {
                            state.set_error(err.to_string());
                            state.status = HELP_STATUS.into();
                        }
                    }
                } else {
                    state.set_error(
                        "VCC extension is not enabled; install `builtin:vcc` from `/extensions` before using `/recall`",
                    );
                    state.status = HELP_STATUS.into();
                }
            }
            TuiAction::CompactMethodOverride { method } => {
                let method_name = match &method {
                    CompactMethodOverride::Vcc => "VCC",
                    CompactMethodOverride::Llm => "LLM",
                };
                match method {
                    CompactMethodOverride::Vcc => {
                        if vcc_command_enabled(&resource_paths, &tool_settings, "compact") {
                            match compact_session_with_vcc(&harness).await {
                                Ok((message, messages)) => {
                                    state.set_messages_from_oino(&messages);
                                    save_tui_session(&mut state, &harness, &session_path).await;
                                    state.clear_error();
                                    state.status = format!("{method_name}: {message}");
                                    refresh_tui_context_status(&mut state, &harness, &cwd).await;
                                }
                                Err(err) => {
                                    state.set_error(err.to_string());
                                    state.status = HELP_STATUS.into();
                                }
                            }
                        } else {
                            state.set_error("VCC extension is not enabled; install `builtin:vcc` from `/extensions`");
                            state.status = HELP_STATUS.into();
                        }
                    }
                    CompactMethodOverride::Llm => {
                        let user_settings = user_settings::UserSettings::load_default()
                            .await
                            .unwrap_or_default();
                        let model_id = user_settings
                            .compact
                            .model
                            .as_deref()
                            .filter(|m| *m != "inherit")
                            .unwrap_or(&state.settings.selected_model());
                        let model = match Model::from_identifier(model_id) {
                            Some(m) => m,
                            None => {
                                state.set_error(format!("Invalid compact model: {model_id}"));
                                state.status = HELP_STATUS.into();
                                return Ok(());
                            }
                        };
                        let compact_stream = build_runtime_provider(
                            auth.clone(),
                            provider_config.clone(),
                            extension_runtime_providers(&extension_snapshot),
                        );
                        match compact_session_with_llm(
                            &harness,
                            &compact_stream,
                            &model,
                            user_settings.compact.prompt.as_deref(),
                            &cwd,
                        )
                        .await
                        {
                            Ok((message, messages)) => {
                                state.set_messages_from_oino(&messages);
                                save_tui_session(&mut state, &harness, &session_path).await;
                                state.clear_error();
                                state.status = format!("{method_name}: {message}");
                                refresh_tui_context_status(&mut state, &harness, &cwd).await;
                            }
                            Err(err) => {
                                state.set_error(err.to_string());
                                state.status = HELP_STATUS.into();
                            }
                        }
                    }
                }
            }
            TuiAction::CompactThreshold { pct } => {
                let mut settings = user_settings::UserSettings::load_default()
                    .await
                    .unwrap_or_default();
                match pct {
                    Some(p) => {
                        settings.compact.threshold_pct = Some(p);
                        if let Err(err) = settings.save_default().await {
                            state.set_error(format!("Failed to save settings: {err}"));
                        } else {
                            state.clear_error();
                            state.status = format!("Auto-compact threshold set to {p}%");
                        }
                    }
                    None => {
                        let current = settings
                            .compact
                            .threshold_pct
                            .map(|p| format!("{p}%"))
                            .unwrap_or_else(|| "disabled".to_string());
                        state.clear_error();
                        state.status = format!("Current auto-compact threshold: {current}");
                    }
                }
            }
            TuiAction::CompactAuto { enabled } => {
                let mut settings = user_settings::UserSettings::load_default()
                    .await
                    .unwrap_or_default();
                settings.compact.auto = enabled;
                if let Err(err) = settings.save_default().await {
                    state.set_error(format!("Failed to save settings: {err}"));
                } else {
                    state.clear_error();
                    state.status = format!(
                        "Auto-compact {}",
                        if enabled { "enabled" } else { "disabled" }
                    );
                }
            }
            TuiAction::CompactModel { model } => {
                let mut settings = user_settings::UserSettings::load_default()
                    .await
                    .unwrap_or_default();
                match model {
                    Some(Some(m)) => {
                        settings.compact.model = Some(m.clone());
                        if let Err(err) = settings.save_default().await {
                            state.set_error(format!("Failed to save settings: {err}"));
                        } else {
                            state.clear_error();
                            state.status = format!("LLM compact model set to {m}");
                        }
                    }
                    Some(None) => {
                        settings.compact.model = None;
                        if let Err(err) = settings.save_default().await {
                            state.set_error(format!("Failed to save settings: {err}"));
                        } else {
                            state.clear_error();
                            state.status = "LLM compact model set to inherit".into();
                        }
                    }
                    None => {
                        let current = settings.compact.model.as_deref().unwrap_or("inherit");
                        state.clear_error();
                        state.status = format!("Current LLM compact model: {current}");
                    }
                }
            }
            TuiAction::CompactPrompt { path } => {
                let mut settings = user_settings::UserSettings::load_default()
                    .await
                    .unwrap_or_default();
                match path {
                    Some(p) => {
                        settings.compact.prompt = Some(p.clone());
                        if let Err(err) = settings.save_default().await {
                            state.set_error(format!("Failed to save settings: {err}"));
                        } else {
                            state.clear_error();
                            state.status = format!("LLM compact prompt set to {p}");
                        }
                    }
                    None => {
                        let current = settings.compact.prompt.as_deref().unwrap_or("default");
                        state.clear_error();
                        state.status = format!("Current LLM compact prompt: {current}");
                    }
                }
            }
            TuiAction::RefreshUsage => {
                match usage_report_for_current_session(
                    &harness,
                    &auth,
                    &state.settings.selected_model(),
                )
                .await
                {
                    Ok(report) => {
                        let status = report.status_line();
                        state.set_usage_report(report.to_tui_report());
                        state.clear_error();
                        state.status = status;
                    }
                    Err(err) => state.set_usage_error(format!("Usage refresh failed: {err}")),
                }
            }
            TuiAction::OpenBtw => {
                if btw_harness.is_none() {
                    match create_btw_harness(
                        &harness,
                        &auth,
                        &resource_catalog,
                        &resource_paths,
                        &tool_settings,
                        &cwd,
                        &btw_provider_config,
                        &extension_snapshot,
                        state.btw.configured_model.clone(),
                        &state.settings.selected_model(),
                        true,
                    )
                    .await
                    {
                        Ok(created) => {
                            btw_harness = Some(created);
                            let chat_model = state.settings.selected_model().to_string();
                            state.set_btw_configured_model(
                                tool_settings.global.btw_model.clone(),
                                &chat_model,
                            );
                        }
                        Err(err) => state.set_btw_error(err.to_string()),
                    }
                }
            }
            TuiAction::SubmitBtwPrompt(prompt) => {
                if btw_in_flight {
                    state.status = "BTW prompt is already running".into();
                } else {
                    if btw_harness.is_none() {
                        match create_btw_harness(
                            &harness,
                            &auth,
                            &resource_catalog,
                            &resource_paths,
                            &tool_settings,
                            &cwd,
                            &btw_provider_config,
                            &extension_snapshot,
                            state.btw.configured_model.clone(),
                            &state.settings.selected_model(),
                            true,
                        )
                        .await
                        {
                            Ok(created) => btw_harness = Some(created),
                            Err(err) => {
                                state.set_btw_error(err.to_string());
                                continue;
                            }
                        }
                    }
                    if let Some(created) = &btw_harness {
                        btw_in_flight = true;
                        start_btw_prompt(Arc::clone(created), tx.clone(), prompt);
                    }
                }
            }
            TuiAction::ResetBtwSession => {
                btw_in_flight = false;
                match create_btw_harness(
                    &harness,
                    &auth,
                    &resource_catalog,
                    &resource_paths,
                    &tool_settings,
                    &cwd,
                    &btw_provider_config,
                    &extension_snapshot,
                    state.btw.configured_model.clone(),
                    &state.settings.selected_model(),
                    false,
                )
                .await
                {
                    Ok(created) => btw_harness = Some(created),
                    Err(err) => state.set_btw_error(err.to_string()),
                }
            }
            TuiAction::ConfigureBtwModel(model) => {
                if let Some(model_id) = &model {
                    if Model::from_identifier(model_id).is_none() {
                        state.set_error(format!(
                            "Invalid BTW model `{model_id}`; expected provider:model-id"
                        ));
                        continue;
                    }
                }
                tool_settings.global.btw_model = model.clone();
                save_tool_settings(
                    &tool_settings,
                    &resource_paths,
                    ToolSettingsScope::Global,
                    &mut state,
                )
                .await;
                let chat_model = state.settings.selected_model().to_string();
                state.set_btw_configured_model(model.clone(), &chat_model);
                btw_harness = None;
                state.status = match model {
                    Some(model) => format!("BTW model set to `{model}`"),
                    None => "BTW model set to inherit current chat model".into(),
                };
            }
            TuiAction::Ralph(command) => {
                match handle_tui_ralph_command(
                    &mut state,
                    &mut ralph_controller,
                    &auth,
                    &harness,
                    &tx,
                    &session_path,
                    &resource_paths,
                    &tool_settings,
                    &mut prompt_in_flight,
                    &extension_runtime_provider_ids,
                    command,
                )
                .await
                {
                    Ok(message) => {
                        state.clear_error();
                        state.status = message;
                    }
                    Err(err) => {
                        state.set_error(err.to_string());
                        state.status = HELP_STATUS.into();
                    }
                }
            }
            TuiAction::RunExtensionCommand { input } => {
                let snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
                match execute_extension_command(&input, &snapshot).await {
                    Some(Ok(message)) => {
                        let refresh_models = input.trim_start().starts_with("/router models");
                        state.clear_error();
                        state.status = compact_status_line(&message);
                        state.append_command_output(input, message);
                        if refresh_models {
                            apply_cached_model_catalog_to_tui_state(&mut state, &extension_models)
                                .await;
                        }
                    }
                    Some(Err(err)) => {
                        state.set_error(err.to_string());
                        state.status = HELP_STATUS.into();
                    }
                    None => {
                        state.set_error(format!("Extension command is not enabled: {input}"));
                        state.status = HELP_STATUS.into();
                    }
                }
            }
            TuiAction::SetAgentMode(mode) => {
                if mode_sandbox_command_enabled(&mode, &resource_paths, &tool_settings) {
                    if let Err(err) = ensure_mode_sandbox_profiles(&resource_paths) {
                        state.set_error(format!("Mode profile setup failed: {err}"));
                    }
                    state.set_agent_mode(mode.clone());
                    if let Ok(mut guard) = agent_mode.lock() {
                        *guard = mode.clone();
                    }
                    apply_tool_settings_to_harness(
                        &harness,
                        &tool_settings,
                        &resource_paths,
                        &cwd,
                        mode,
                        Some(tx.clone()),
                    )
                    .await;
                } else {
                    state.set_error(format!(
                        "Mode sandbox extension is not enabled; install `builtin:mode-sandbox` from `/extensions` before using `/mode {}`",
                        mode.value()
                    ));
                    state.status = HELP_STATUS.into();
                }
            }
            TuiAction::SetExtensionEnabled {
                target,
                id,
                scope,
                enabled,
            } => {
                set_extension_enabled(&mut tool_settings, target, id, scope, enabled);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                apply_tool_settings_to_harness(
                    &harness,
                    &tool_settings,
                    &resource_paths,
                    &cwd,
                    state.agent_mode.clone(),
                    Some(tx.clone()),
                )
                .await;
                let extension_snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
                extension_models = extension_model_options(&extension_snapshot);
                extension_runtime_provider_ids =
                    extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
                state.set_tool_settings(tool_settings_items(&tool_settings));
                apply_extension_snapshot_to_tui_state(
                    &mut state,
                    &extension_snapshot,
                    &tool_settings,
                    &resource_paths,
                );
                apply_resource_catalog_to_state(&mut state, &resource_catalog, &extension_snapshot);
            }
            TuiAction::SetExtensionOverride {
                contribution_id,
                entry_key,
                scope,
            } => {
                set_extension_override(&mut tool_settings, contribution_id, entry_key, scope);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                apply_tool_settings_to_harness(
                    &harness,
                    &tool_settings,
                    &resource_paths,
                    &cwd,
                    state.agent_mode.clone(),
                    Some(tx.clone()),
                )
                .await;
                let extension_snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
                extension_models = extension_model_options(&extension_snapshot);
                extension_runtime_provider_ids =
                    extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
                state.set_tool_settings(tool_settings_items(&tool_settings));
                apply_extension_snapshot_to_tui_state(
                    &mut state,
                    &extension_snapshot,
                    &tool_settings,
                    &resource_paths,
                );
                apply_resource_catalog_to_state(&mut state, &resource_catalog, &extension_snapshot);
            }
            TuiAction::ClearExtensionOverride {
                contribution_id,
                scope,
            } => {
                clear_extension_override(&mut tool_settings, contribution_id, scope);
                save_tool_settings(&tool_settings, &resource_paths, scope, &mut state).await;
                apply_tool_settings_to_harness(
                    &harness,
                    &tool_settings,
                    &resource_paths,
                    &cwd,
                    state.agent_mode.clone(),
                    Some(tx.clone()),
                )
                .await;
                let extension_snapshot = load_extension_snapshot(&resource_paths, &tool_settings);
                extension_models = extension_model_options(&extension_snapshot);
                extension_runtime_provider_ids =
                    extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
                state.set_tool_settings(tool_settings_items(&tool_settings));
                apply_extension_snapshot_to_tui_state(
                    &mut state,
                    &extension_snapshot,
                    &tool_settings,
                    &resource_paths,
                );
                apply_resource_catalog_to_state(&mut state, &resource_catalog, &extension_snapshot);
            }
            TuiAction::InstallExtensionPackage { source, scope } => {
                let prepared_source =
                    prepare_install_source(&source, &cwd, &resource_paths.home_dir);
                let mut manager =
                    extension_manager_with_current_policy(&resource_paths, &tool_settings);
                manager.load();
                let service = PackageLifecycleService::new(
                    extension_layout_paths(&resource_paths),
                    current_extension_version(),
                );
                let lifecycle = prepared_source.and_then(|prepared| {
                    let lifecycle = service
                        .install_local(&prepared.path, package_install_scope(scope), &mut manager)
                        .or_else(|err| match err {
                            PackageLifecycleError::AlreadyInstalled(_) => service.update_local(
                                &prepared.path,
                                package_install_scope(scope),
                                &mut manager,
                            ),
                            other => Err(other),
                        })
                        .map_err(|err| err.to_string());
                    lifecycle.and_then(|report| {
                        write_extension_package_source_record(
                            &report.destination,
                            &prepared.update_source,
                        )
                        .map_err(|err| err.to_string())?;
                        Ok((report, prepared.display))
                    })
                });
                match lifecycle {
                    Ok((report, source_display)) => {
                        let package_id = report.package_id.to_string();
                        set_extension_enabled(
                            &mut tool_settings,
                            ExtensionManagementTarget::Package,
                            package_id.clone(),
                            scope,
                            true,
                        );
                        save_tool_settings(&tool_settings, &resource_paths, scope, &mut state)
                            .await;
                        if package_id == MODE_SANDBOX_PACKAGE_ID {
                            if let Err(err) = ensure_mode_sandbox_profiles(&resource_paths) {
                                state.set_error(format!("Mode profile setup failed: {err}"));
                            }
                        }
                        apply_tool_settings_to_harness(
                            &harness,
                            &tool_settings,
                            &resource_paths,
                            &cwd,
                            state.agent_mode.clone(),
                            Some(tx.clone()),
                        )
                        .await;
                        let extension_snapshot =
                            load_extension_snapshot(&resource_paths, &tool_settings);
                        extension_models = extension_model_options(&extension_snapshot);
                        extension_runtime_provider_ids =
                            extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
                        state.set_tool_settings(tool_settings_items(&tool_settings));
                        apply_extension_snapshot_to_tui_state(
                            &mut state,
                            &extension_snapshot,
                            &tool_settings,
                            &resource_paths,
                        );
                        apply_resource_catalog_to_state(
                            &mut state,
                            &resource_catalog,
                            &extension_snapshot,
                        );
                        state.status = format!(
                            "Installed {} package `{}` from `{}`",
                            scope.label(),
                            package_id,
                            source_display
                        );
                    }
                    Err(err) => state.set_error(format!("Extension install failed: {err}")),
                }
            }
            TuiAction::UpdateExtensionPackages => {
                let message =
                    update_installed_extension_packages(&resource_paths, &cwd, &tool_settings);
                reload_tui_everything(
                    &mut state,
                    &harness,
                    &resource_paths,
                    &cwd,
                    &mut tool_settings,
                    &mut extension_models,
                    &mut extension_runtime_provider_ids,
                    Some(tx.clone()),
                )
                .await;
                state.append_command_output("/extensions update", message.clone());
                state.status = compact_status_line(&message);
            }
            TuiAction::RemoveExtensionPackage { package_id, scope } => {
                let mut manager =
                    extension_manager_with_current_policy(&resource_paths, &tool_settings);
                manager.load();
                let service = PackageLifecycleService::new(
                    extension_layout_paths(&resource_paths),
                    current_extension_version(),
                );
                match PackageId::new(&package_id) {
                    Ok(id) => {
                        match service.remove(id, package_install_scope(scope), &mut manager) {
                            Ok(_) => {
                                set_extension_enabled(
                                    &mut tool_settings,
                                    ExtensionManagementTarget::Package,
                                    package_id.clone(),
                                    scope,
                                    false,
                                );
                                save_tool_settings(
                                    &tool_settings,
                                    &resource_paths,
                                    scope,
                                    &mut state,
                                )
                                .await;
                                apply_tool_settings_to_harness(
                                    &harness,
                                    &tool_settings,
                                    &resource_paths,
                                    &cwd,
                                    state.agent_mode.clone(),
                                    Some(tx.clone()),
                                )
                                .await;
                                let extension_snapshot =
                                    load_extension_snapshot(&resource_paths, &tool_settings);
                                extension_models = extension_model_options(&extension_snapshot);
                                extension_runtime_provider_ids =
                                    extension_runtime_provider_ids_from_snapshot(
                                        &extension_snapshot,
                                    );
                                state.set_tool_settings(tool_settings_items(&tool_settings));
                                apply_extension_snapshot_to_tui_state(
                                    &mut state,
                                    &extension_snapshot,
                                    &tool_settings,
                                    &resource_paths,
                                );
                                apply_resource_catalog_to_state(
                                    &mut state,
                                    &resource_catalog,
                                    &extension_snapshot,
                                );
                                state.status = format!(
                                    "Uninstalled {} package `{}`",
                                    scope.label(),
                                    package_id
                                );
                            }
                            Err(err) => {
                                state.set_error(format!("Extension uninstall failed: {err}"))
                            }
                        }
                    }
                    Err(err) => {
                        state.set_error(format!("Invalid package id `{package_id}`: {err}"))
                    }
                }
            }
            TuiAction::SetSessionTitle(title) => {
                if let Err(err) = harness.set_session_title(title.clone()).await {
                    state.set_error(err.to_string());
                } else {
                    state.set_session_title(title);
                    save_tui_session(&mut state, &harness, &session_path).await;
                }
            }
            TuiAction::AbortPrompt => {
                if prompt_in_flight {
                    harness.abort().await;
                    state.status = "Stopping response…".into();
                }
            }
            TuiAction::SubmitPrompt(prompt) => {
                start_prompt(
                    &mut state,
                    &auth,
                    &harness,
                    &tx,
                    &session_path,
                    &mut prompt_in_flight,
                    &extension_runtime_provider_ids,
                    prompt,
                )
                .await;
            }
            TuiAction::SteerPrompt(prompt) => {
                if prompt_in_flight {
                    let message = Message::user_text(prompt);
                    match harness.steer(message.clone()).await {
                        Ok(()) => materialize_accepted_steer(&mut state, &message),
                        Err(err) => state.set_error(user_facing_error(&err)),
                    }
                } else {
                    start_prompt(
                        &mut state,
                        &auth,
                        &harness,
                        &tx,
                        &session_path,
                        &mut prompt_in_flight,
                        &extension_runtime_provider_ids,
                        prompt,
                    )
                    .await;
                }
            }
            TuiAction::QueuePrompt(_) => {
                start_next_queued_prompt_if_idle(
                    &mut state,
                    &auth,
                    &harness,
                    &tx,
                    &session_path,
                    &mut prompt_in_flight,
                    &extension_runtime_provider_ids,
                )
                .await;
            }
            TuiAction::NewSession => {
                if prompt_in_flight {
                    state.status = "Cannot start a new session while a prompt is running".into();
                } else {
                    start_new_tui_session(&mut state, &harness, &mut session_path).await;
                }
            }
            TuiAction::ListSessions => {
                load_tui_sessions(&mut state, &session_path).await;
            }
            TuiAction::OpenSession(session_id) => {
                if prompt_in_flight {
                    state.status = "Cannot switch sessions while a prompt is running".into();
                } else if let Some(thinking_level) =
                    open_tui_session(&mut state, &harness, &mut session_path, &session_id).await
                {
                    applied_thinking_level = thinking_level;
                }
            }
            TuiAction::ReloadResources => {
                reload_tui_everything(
                    &mut state,
                    &harness,
                    &resource_paths,
                    &cwd,
                    &mut tool_settings,
                    &mut extension_models,
                    &mut extension_runtime_provider_ids,
                    Some(tx.clone()),
                )
                .await;
                apply_cached_model_catalog_to_tui_state(&mut state, &extension_models).await;
            }
        }
    }
    Ok(())
}

async fn start_new_tui_session(
    state: &mut TuiState,
    harness: &Arc<Harness>,
    session_path: &mut PathBuf,
) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = match default_session_root() {
        Ok(root) => root,
        Err(err) => {
            state.set_error(format!("New session failed: {err}"));
            return;
        }
    };
    let (path, session) = new_tui_session(root, cwd);
    harness.replace_session(session).await;
    *session_path = path;
    let session_id = session_id_from_path(session_path);
    state.reset_for_new_session(&session_id);
    state.set_session_title(harness.session_title().await);
}

fn new_tui_session(root: PathBuf, cwd: PathBuf) -> (PathBuf, SessionManager) {
    let session = SessionManager::new(SessionHeader::new("oino", cwd));
    let path = root.join(format!("{}.jsonl", session.header().session_id));
    (path, session)
}

fn apply_resource_catalog_to_state(
    state: &mut TuiState,
    catalog: &ResourceCatalog,
    snapshot: &ExtensionManagerSnapshot,
) {
    let mut prompts = catalog
        .prompts
        .iter()
        .map(|prompt| PromptResource {
            name: prompt.name.clone(),
            description: prompt.description.clone(),
            argument_hint: prompt.argument_hint.clone(),
            source: path_to_string(&prompt.path),
            scope: prompt.scope.label().into(),
            content: prompt.content.clone(),
        })
        .collect::<Vec<_>>();
    let mut skills = catalog
        .skills
        .iter()
        .map(|skill| SkillResource {
            name: skill.name.clone(),
            description: skill.description.clone(),
            source: path_to_string(&skill.path),
            scope: skill.scope.label().into(),
            content: skill.content.clone(),
        })
        .collect::<Vec<_>>();
    let (extension_prompts, extension_skills, extension_diagnostics) =
        extension_resource_items(snapshot);
    prompts.extend(extension_prompts);
    skills.extend(extension_skills);
    let diagnostics = catalog
        .diagnostics
        .iter()
        .map(oino_resource::ResourceDiagnostic::message)
        .chain(extension_diagnostics)
        .collect::<Vec<_>>();
    state.set_resources(prompts, skills, diagnostics);
}

fn extension_resource_items(
    snapshot: &ExtensionManagerSnapshot,
) -> (Vec<PromptResource>, Vec<SkillResource>, Vec<String>) {
    let mut prompts = Vec::new();
    let mut skills = Vec::new();
    let mut diagnostics = Vec::new();
    for active in &snapshot.registries.resources.active {
        let Some(path) = extension_resource_path(active) else {
            diagnostics.push(format!(
                "Extension resource `{}` has no resolvable source path",
                active.effective_id
            ));
            continue;
        };
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                diagnostics.push(format!(
                    "Extension resource `{}` could not be read from {}: {err}",
                    active.effective_id,
                    path.display()
                ));
                continue;
            }
        };
        let source = path_to_string(&path);
        let description = extension_resource_description(&content, &active.effective_id);
        match active.entry.contribution.kind {
            ResourceKind::Prompt
            | ResourceKind::SystemPrompt
            | ResourceKind::ProjectInstructions => {
                prompts.push(PromptResource {
                    name: active.effective_id.to_string(),
                    description,
                    argument_hint: None,
                    source,
                    scope: active.entry.metadata.source.scope.slug().into(),
                    content,
                });
            }
            ResourceKind::Skill => skills.push(SkillResource {
                name: active.effective_id.to_string(),
                description,
                source,
                scope: active.entry.metadata.source.scope.slug().into(),
                content,
            }),
            ResourceKind::Theme | ResourceKind::Asset => {}
        }
    }
    (prompts, skills, diagnostics)
}

fn extension_resource_path(
    active: &ActiveContribution<oino_extension_core::ResourceContribution>,
) -> Option<PathBuf> {
    let base = active.entry.metadata.source.path.as_ref()?;
    let base = if base.is_dir() {
        base.as_path()
    } else {
        base.parent()?
    };
    Some(base.join(&active.entry.contribution.path))
}

fn extension_resource_description(content: &str, id: &ContributionId) -> String {
    if let Some(description) = frontmatter_description(content) {
        return description;
    }
    content
        .lines()
        .skip(body_start_line(content))
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| format!("Extension resource `{id}`"))
}

fn frontmatter_description(content: &str) -> Option<String> {
    let mut lines = content.lines().map(str::trim);
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            return None;
        }
        let Some(description) = line.strip_prefix("description:") else {
            continue;
        };
        let description = description
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        if !description.is_empty() {
            return Some(description);
        }
    }
    None
}

fn body_start_line(content: &str) -> usize {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return 0;
    }
    for (index, line) in lines.enumerate() {
        if line.trim() == "---" {
            return index + 2;
        }
    }
    0
}

async fn apply_cached_model_catalog_to_tui_state(
    state: &mut TuiState,
    extension_models: &[ModelOption],
) {
    if let Some(update) =
        model_catalog::load_cached_update_with_historical_provider_catalog(false).await
    {
        state.set_model_catalog(
            merge_extension_models(update.models, extension_models),
            update.status,
        );
        state.set_model_catalog_refreshing(update.refreshing);
    }
}

async fn reload_tui_everything(
    state: &mut TuiState,
    harness: &Harness,
    resource_paths: &ResourcePaths,
    cwd: &Path,
    tool_settings: &mut ToolSettingsSnapshot,
    extension_models: &mut Vec<ModelOption>,
    extension_runtime_provider_ids: &mut BTreeSet<String>,
    ask_user_tx: Option<mpsc::UnboundedSender<TuiRuntimeEvent>>,
) {
    *tool_settings = load_tool_settings(resource_paths).await;
    apply_tool_settings_to_harness(
        harness,
        tool_settings,
        resource_paths,
        cwd,
        state.agent_mode.clone(),
        ask_user_tx,
    )
    .await;

    match load_resource_catalog(resource_paths) {
        Ok(catalog) => {
            harness
                .set_system_prompt(Some(default_system_prompt(cwd, &catalog)))
                .await;
            let extension_snapshot = load_extension_snapshot(resource_paths, tool_settings);
            *extension_models = extension_model_options(&extension_snapshot);
            *extension_runtime_provider_ids =
                extension_runtime_provider_ids_from_snapshot(&extension_snapshot);
            state.set_tool_settings(tool_settings_items(tool_settings));
            state.set_theme_settings(&tool_settings.global.theme, &tool_settings.project.theme);
            apply_extension_snapshot_to_tui_state(
                state,
                &extension_snapshot,
                tool_settings,
                resource_paths,
            );
            apply_resource_catalog_to_state(state, &catalog, &extension_snapshot);
            state.set_file_paths(scan_project_files(cwd));
            if let Some(summary) = catalog.diagnostics_summary() {
                state.set_error(format!("Resource warnings: {summary}"));
            } else {
                state.clear_error();
                state.status = format!(
                    "Reloaded {} prompts, {} skills, extensions, tools, themes, and file index",
                    catalog.prompts.len(),
                    catalog.skills.len()
                );
            }
        }
        Err(err) => state.set_error(format!("Reload failed: {err}")),
    }
}

async fn load_tui_sessions(state: &mut TuiState, current_session_path: &Path) {
    match session_list_items(
        current_session_path
            .file_stem()
            .and_then(|value| value.to_str()),
    )
    .await
    {
        Ok(sessions) => state.set_sessions(sessions),
        Err(err) => state.set_error(format!("Sessions load failed: {err}")),
    }
}

async fn open_tui_session(
    state: &mut TuiState,
    harness: &Arc<Harness>,
    session_path: &mut PathBuf,
    session_id: &str,
) -> Option<ThinkingLevel> {
    let root = match default_session_root() {
        Ok(root) => root,
        Err(err) => {
            state.set_error(format!("Open session failed: {err}"));
            return None;
        }
    };
    let path = root.join(format!("{session_id}.jsonl"));
    let repository = SessionRepository::new(root);
    match repository.open(&path).await {
        Ok(session) => {
            let context = match session.build_session_context() {
                Ok(context) => context,
                Err(err) => {
                    state.set_error(format!("Open session failed: {err}"));
                    return None;
                }
            };
            let model = context.model.as_ref().map(Model::identifier);
            let thinking_level = context.thinking_level;
            let session_title = session.get_session_name();
            harness.replace_session(session).await;
            *session_path = path;
            state.set_session_title(session_title);
            if let Some(model) = model {
                state.settings.select_model_identifier(&model);
            }
            if let Some(level) = thinking_level {
                state.settings.select_thinking_level(level);
            }
            state.switch_to_session(session_id, &context.messages);
            thinking_level
        }
        Err(err) => {
            state.set_error(format!("Open session failed: {err}"));
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_tui_ralph_command(
    state: &mut TuiState,
    controller: &mut RalphRunController,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
    prompt_in_flight: &mut bool,
    extension_runtime_provider_ids: &BTreeSet<String>,
    command: RalphCommand,
) -> Result<String, AppError> {
    if !ralph_loop_command_enabled(paths, settings) {
        return Err(AppError::InvalidArguments(
            "Ralph loop extension is not enabled; install `builtin:ralph-loop` from `/extensions` first".into(),
        ));
    }
    match command {
        RalphCommand::Start { name, task } => {
            let state_loop = ralph_loop::start_loop(
                &paths.project_root,
                ralph_loop::RalphLoopStart::new(name, task),
            )?;
            controller.active_loop = Some(state_loop.name.clone());
            controller.auto_continue = true;
            start_ralph_iteration_prompt(
                state,
                controller,
                auth,
                harness,
                tx,
                session_path,
                &paths.project_root,
                prompt_in_flight,
                extension_runtime_provider_ids,
                &state_loop,
                true,
            )
            .await?;
            Ok(format!(
                "Started Ralph loop `{}` and queued iteration {}/{}",
                state_loop.name,
                state_loop.iteration.saturating_add(1),
                state_loop.max_iterations
            ))
        }
        RalphCommand::Resume { name } => {
            let state_loop = ralph_loop::resume_loop(&paths.project_root, &name)?;
            controller.active_loop = Some(state_loop.name.clone());
            controller.auto_continue = true;
            start_ralph_iteration_prompt(
                state,
                controller,
                auth,
                harness,
                tx,
                session_path,
                &paths.project_root,
                prompt_in_flight,
                extension_runtime_provider_ids,
                &state_loop,
                true,
            )
            .await?;
            Ok(format!("Resumed Ralph loop `{}`", state_loop.name))
        }
        RalphCommand::Continue { name } => {
            let state_loop = load_ralph_target(&paths.project_root, name.as_deref())?;
            controller.active_loop = Some(state_loop.name.clone());
            controller.auto_continue = true;
            start_ralph_iteration_prompt(
                state,
                controller,
                auth,
                harness,
                tx,
                session_path,
                &paths.project_root,
                prompt_in_flight,
                extension_runtime_provider_ids,
                &state_loop,
                true,
            )
            .await?;
            Ok(format!("Continuing Ralph loop `{}`", state_loop.name))
        }
        RalphCommand::Once { name } => {
            let state_loop = load_ralph_target(&paths.project_root, name.as_deref())?;
            controller.active_loop = Some(state_loop.name.clone());
            controller.auto_continue = false;
            start_ralph_iteration_prompt(
                state,
                controller,
                auth,
                harness,
                tx,
                session_path,
                &paths.project_root,
                prompt_in_flight,
                extension_runtime_provider_ids,
                &state_loop,
                false,
            )
            .await?;
            Ok(format!(
                "Running one Ralph iteration for `{}`",
                state_loop.name
            ))
        }
        RalphCommand::Steer { name, note } => {
            let state_loop = ralph_loop::append_steering(&paths.project_root, &name, &note)?;
            Ok(format!(
                "Added steering to Ralph loop `{}` ({})",
                state_loop.name, state_loop.steering_file
            ))
        }
        RalphCommand::Pause { name } => {
            let state_loop = ralph_loop::pause_loop(&paths.project_root, &name)?;
            if controller.active_loop.as_deref() == Some(state_loop.name.as_str()) {
                controller.auto_continue = false;
                controller.prompt_loop_in_flight = None;
            }
            Ok(format!("Paused Ralph loop `{}`", state_loop.name))
        }
        RalphCommand::Cancel { name } => {
            let state_loop = ralph_loop::cancel_loop(&paths.project_root, &name)?;
            if controller.active_loop.as_deref() == Some(state_loop.name.as_str()) {
                controller.active_loop = None;
                controller.auto_continue = false;
                controller.prompt_loop_in_flight = None;
            }
            Ok(format!("Cancelled Ralph loop `{}`", state_loop.name))
        }
        other => execute_ralph_command_if_enabled(other, paths, settings),
    }
}

#[allow(clippy::too_many_arguments)]
async fn continue_ralph_after_prompt_if_needed(
    state: &mut TuiState,
    controller: &mut RalphRunController,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    paths: &ResourcePaths,
    prompt_in_flight: &mut bool,
    extension_runtime_provider_ids: &BTreeSet<String>,
    messages: &[Message],
) {
    let Some(loop_name) = controller.prompt_loop_in_flight.take() else {
        return;
    };
    let output = last_assistant_text(messages).unwrap_or_default();
    match ralph_loop::record_iteration_output(&paths.project_root, &loop_name, &output) {
        Ok(state_loop) => {
            state.status = format!(
                "Ralph `{}` recorded iteration {}/{} ({:?})",
                state_loop.name, state_loop.iteration, state_loop.max_iterations, state_loop.status
            );
            if state_loop.status == ralph_loop::RalphLoopStatus::Active && controller.auto_continue
            {
                if let Err(err) = start_ralph_iteration_prompt(
                    state,
                    controller,
                    auth,
                    harness,
                    tx,
                    session_path,
                    &paths.project_root,
                    prompt_in_flight,
                    extension_runtime_provider_ids,
                    &state_loop,
                    true,
                )
                .await
                {
                    state.set_error(format!("Ralph continue failed: {err}"));
                    controller.auto_continue = false;
                }
            } else if state_loop.status != ralph_loop::RalphLoopStatus::Active {
                controller.auto_continue = false;
                controller.active_loop = Some(state_loop.name);
            }
        }
        Err(err) => {
            controller.auto_continue = false;
            state.set_error(format!("Ralph record failed: {err}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn start_ralph_iteration_prompt(
    state: &mut TuiState,
    controller: &mut RalphRunController,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    project_root: &Path,
    prompt_in_flight: &mut bool,
    extension_runtime_provider_ids: &BTreeSet<String>,
    loop_state: &ralph_loop::RalphLoopState,
    auto_continue: bool,
) -> Result<(), AppError> {
    if loop_state.status != ralph_loop::RalphLoopStatus::Active {
        return Err(AppError::InvalidArguments(format!(
            "Ralph loop `{}` is {:?}; resume it before continuing",
            loop_state.name, loop_state.status
        )));
    }
    let prompt = ralph_loop::build_iteration_prompt(project_root, loop_state)?;
    if start_prompt(
        state,
        auth,
        harness,
        tx,
        session_path,
        prompt_in_flight,
        extension_runtime_provider_ids,
        prompt,
    )
    .await
    {
        controller.active_loop = Some(loop_state.name.clone());
        controller.auto_continue = auto_continue;
        controller.prompt_loop_in_flight = Some(loop_state.name.clone());
        Ok(())
    } else {
        Err(AppError::InvalidArguments(
            "Could not start Ralph iteration because another prompt is running or credentials are missing".into(),
        ))
    }
}

fn load_ralph_target(
    project_root: &Path,
    name: Option<&str>,
) -> Result<ralph_loop::RalphLoopState, AppError> {
    if let Some(name) = name {
        return Ok(ralph_loop::load_state(project_root, name)?);
    }
    let active = ralph_loop::list_states(project_root)?
        .into_iter()
        .find(|state| state.status == ralph_loop::RalphLoopStatus::Active)
        .ok_or_else(|| {
            AppError::InvalidArguments(
                "No active Ralph loop found; pass a loop name or start one with `/ralph start <name> <task>`".into(),
            )
        })?;
    Ok(active)
}

fn last_assistant_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::Assistant { content, .. } => Some(content_text(content)),
            _ => None,
        })
        .filter(|text| !text.trim().is_empty())
}

fn content_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_line_or_default(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

async fn start_next_queued_prompt_if_idle(
    state: &mut TuiState,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    prompt_in_flight: &mut bool,
    extension_runtime_provider_ids: &BTreeSet<String>,
) {
    if *prompt_in_flight {
        return;
    }
    let Some(prompt) = state.next_queued_prompt().map(ToOwned::to_owned) else {
        return;
    };
    if start_prompt(
        state,
        auth,
        harness,
        tx,
        session_path,
        prompt_in_flight,
        extension_runtime_provider_ids,
        prompt,
    )
    .await
    {
        let _ = state.pop_next_queued_prompt();
    }
}

async fn create_btw_harness(
    main_harness: &Arc<Harness>,
    auth: &AuthStorage,
    resource_catalog: &ResourceCatalog,
    resource_paths: &ResourcePaths,
    tool_settings: &ToolSettingsSnapshot,
    cwd: &Path,
    provider_config: &OpenRouterConfig,
    extension_snapshot: &ExtensionManagerSnapshot,
    configured_model: Option<String>,
    current_model: &str,
    inherit_history: bool,
) -> Result<Arc<Harness>, AppError> {
    let effective_model = configured_model.unwrap_or_else(|| current_model.to_string());
    let mut session = if inherit_history {
        main_harness.session_handle().lock().await.clone()
    } else {
        SessionManager::new(SessionHeader::new("btw", cwd.to_path_buf()))
    };
    session.append_model(
        Model::from_identifier(&effective_model)
            .ok_or_else(|| AppError::InvalidModelIdentifier(effective_model.clone()))?,
    );
    let provider = build_runtime_provider(
        auth.clone(),
        provider_config.clone(),
        extension_runtime_providers(extension_snapshot),
    );
    let harness = Arc::new(build_harness(
        effective_model,
        ThinkingLevel::Off,
        provider,
        auth.clone(),
        session,
        resource_catalog,
    )?);
    apply_tool_settings_to_harness(
        &harness,
        tool_settings,
        resource_paths,
        cwd,
        AgentMode::Plan,
        None,
    )
    .await;
    let mode = Arc::new(Mutex::new(AgentMode::Plan));
    register_mode_hooks(&harness, mode, resource_paths.clone()).await;
    Ok(harness)
}

fn start_btw_prompt(
    harness: Arc<Harness>,
    tx: mpsc::UnboundedSender<TuiRuntimeEvent>,
    prompt: String,
) {
    tokio::spawn(async move {
        let result = harness
            .prompt(Message::user_text(prompt))
            .await
            .map_err(|err| user_facing_error(&err));
        let _ = tx.send(TuiRuntimeEvent::BtwFinished(result));
    });
}

async fn start_prompt(
    state: &mut TuiState,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    prompt_in_flight: &mut bool,
    extension_runtime_provider_ids: &BTreeSet<String>,
    prompt: String,
) -> bool {
    if *prompt_in_flight {
        state.status =
            "A prompt is already running. Use Enter to steer or Ctrl-O q then q to queue.".into();
        return false;
    }
    if let Err(message) = preflight_model_credentials(
        auth,
        &state.settings.selected_model(),
        extension_runtime_provider_ids,
    )
    .await
    {
        state.set_error(message);
        state.status = HELP_STATUS.into();
        return false;
    }
    state.set_working(true);
    *prompt_in_flight = true;
    let prompt_message = Message::user_text(prompt);
    let task_harness = Arc::clone(harness);
    let task_tx = tx.clone();
    let task_session_path = session_path.to_path_buf();
    let task_auth = auth.clone();
    let task_model_identifier;
    {
        task_model_identifier = state.settings.selected_model().to_string();
    }
    tokio::spawn(async move {
        let result = match task_harness.prompt(prompt_message).await {
            Ok(messages) => match task_harness.save_session_jsonl(&task_session_path).await {
                Ok(()) => Ok(messages),
                Err(err) => Err(err.to_string()),
            },
            Err(err) => Err(user_facing_error(&err)),
        };
        if result.is_ok() {
            let _ = task_tx.send(TuiRuntimeEvent::SessionTitle(
                task_harness.session_title().await,
            ));
        }
        let usage_report = match result.as_ref().ok() {
            Some(messages) => {
                let mut report = UsageReport::from_messages(messages);
                if let Ok(Some(provider)) = current_model_provider(&task_model_identifier) {
                    if let Ok(progress) = account_usage_progress_placeholder(
                        &task_auth,
                        provider,
                        report.generated_at_unix,
                    )
                    .await
                    {
                        report.upsert_provider_progress(progress);
                    }
                }
                Some(report)
            }
            None => None,
        };
        let _ = task_tx.send(TuiRuntimeEvent::PromptFinished(result));
        if let Some(report) = usage_report {
            let _ = task_tx.send(TuiRuntimeEvent::UsageProgress(report));
        }
    });
    true
}

fn handle_mouse_event(
    mouse: MouseEvent,
    state: &mut TuiState,
    cwd: &Path,
    width: u16,
    height: u16,
) {
    if !is_external_open_mouse_event(&mouse) {
        return;
    }
    let targets = transcript_click_targets(state, width, height);
    let Some(target) = targets
        .iter()
        .find(|target| click_hits_target(mouse.column, mouse.row, target))
    else {
        return;
    };

    match open_external_target(&target.target, cwd) {
        Ok(()) => state.status = format!("Opened {}", target.target),
        Err(err) => state.set_error(format!("Open failed for {}: {err}", target.target)),
    }
}

fn click_hits_target(column: u16, row: u16, target: &TerminalClickTarget) -> bool {
    row == target.y && column >= target.x && column < target.x.saturating_add(target.width)
}

fn is_external_open_mouse_event(mouse: &MouseEvent) -> bool {
    matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        && mouse.modifiers.contains(KeyModifiers::CONTROL)
}

fn open_external_target(target: &str, cwd: &Path) -> io::Result<()> {
    let target = resolve_external_target(target, cwd);
    if is_external_url(&target) {
        return open_external_url(&target);
    }

    let browser_env = std::env::var("BROWSER").ok();
    open_resolved_target(&target, browser_env.as_deref())
}

fn open_external_url(target: &str) -> io::Result<()> {
    match webbrowser::open(target) {
        Ok(()) => Ok(()),
        Err(webbrowser_err) => {
            let browser_env = std::env::var("BROWSER").ok();
            open_resolved_target(target, browser_env.as_deref()).map_err(|fallback_err| {
                io::Error::new(
                    fallback_err.kind(),
                    format!(
                        "webbrowser failed for {target}: {webbrowser_err}; fallback failed: {fallback_err}"
                    ),
                )
            })
        }
    }
}

fn resolve_external_target(target: &str, cwd: &Path) -> String {
    if is_external_url(target) || target.starts_with("file://") {
        return target.to_string();
    }
    let path = PathBuf::from(target);
    if path.is_absolute() {
        path_to_string(&path)
    } else {
        path_to_string(&cwd.join(path))
    }
}

fn open_resolved_target(target: &str, browser_env: Option<&str>) -> io::Result<()> {
    let candidates = opener_candidates(target, browser_env);
    let mut failures = Vec::new();

    for candidate in candidates {
        match candidate.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => failures.push(format!("{} exited with {status}", candidate.display())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                failures.push(format!("{} not found", candidate.program))
            }
            Err(err) => failures.push(format!("{} failed: {err}", candidate.display())),
        }
    }

    let detail = if failures.is_empty() {
        "no opener candidates configured".to_string()
    } else {
        format!("tried {}", failures.join("; "))
    };
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no system opener succeeded for {target}; {detail}"),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenCommand {
    program: String,
    args: Vec<String>,
}

impl OpenCommand {
    fn new<I, S>(program: &str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    fn with_target(program: &str, target: &str) -> Self {
        Self::new(program, [target])
    }

    fn status(&self) -> io::Result<std::process::ExitStatus> {
        Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    }

    fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn opener_candidates(target: &str, browser_env: Option<&str>) -> Vec<OpenCommand> {
    let mut candidates = browser_env_candidates(target, browser_env);

    #[cfg(target_os = "macos")]
    {
        candidates.push(OpenCommand::with_target("open", target));
    }

    #[cfg(target_os = "windows")]
    {
        candidates.push(OpenCommand::new(
            "rundll32",
            ["url.dll,FileProtocolHandler", target],
        ));
        candidates.push(OpenCommand::new("cmd", ["/C", "start", "", target]));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if running_under_wsl() {
            candidates.extend(wsl_interop_candidates(target));
        }
        candidates.push(OpenCommand::with_target("xdg-open", target));
        candidates.push(OpenCommand::new("gio", ["open", target]));
        candidates.push(OpenCommand::with_target("kde-open", target));
        candidates.push(OpenCommand::with_target("kde-open5", target));
        candidates.push(OpenCommand::with_target("gnome-open", target));
        candidates.push(OpenCommand::with_target("sensible-browser", target));
        candidates.push(OpenCommand::with_target("wslview", target));
    }

    candidates
}

fn wsl_interop_candidates(target: &str) -> Vec<OpenCommand> {
    let target = wsl_host_target(target);
    vec![
        OpenCommand::new("cmd.exe", ["/C", "start", "", target.as_str()]),
        OpenCommand::new(
            "powershell.exe",
            [
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process -FilePath $args[0]",
                target.as_str(),
            ],
        ),
        OpenCommand::with_target("explorer.exe", &target),
        OpenCommand::new(
            "rundll32.exe",
            ["url.dll,FileProtocolHandler", target.as_str()],
        ),
    ]
}

fn wsl_host_target(target: &str) -> String {
    if is_external_url(target) || target.starts_with("file://") || !target.starts_with('/') {
        return target.to_string();
    }

    Command::new("wslpath")
        .args(["-w", target])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| target.to_string())
}

fn running_under_wsl() -> bool {
    std::env::var_os("WSL_INTEROP").is_some() || std::env::var_os("WSL_DISTRO_NAME").is_some()
}

fn browser_env_candidates(target: &str, browser_env: Option<&str>) -> Vec<OpenCommand> {
    let separator = if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    };
    browser_env
        .into_iter()
        .flat_map(|env| env.split(separator))
        .filter_map(|spec| browser_env_candidate(target, spec))
        .collect()
}

fn browser_env_candidate(target: &str, spec: &str) -> Option<OpenCommand> {
    let mut parts = spec.split_whitespace();
    let program = parts.next()?.trim();
    if program.is_empty() {
        return None;
    }

    let mut args = Vec::new();
    let mut inserted_target = false;
    for part in parts {
        if part.contains("%s") || part.contains("%u") {
            args.push(part.replace("%s", target).replace("%u", target));
            inserted_target = true;
        } else {
            args.push(part.to_string());
        }
    }
    if !inserted_target {
        args.push(target.to_string());
    }
    Some(OpenCommand::new(program, args))
}

fn is_external_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn scan_project_files(root: &Path) -> Vec<String> {
    const MAX_FILES: usize = 5000;
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if path.is_dir() {
                if matches!(
                    file_name.as_ref(),
                    ".git" | "target" | "node_modules" | ".direnv" | ".cache"
                ) {
                    continue;
                }
                stack.push(path);
            } else if path.is_file() {
                if let Some(relative) = relative_path_string(root, &path) {
                    files.push(relative);
                    if files.len() >= MAX_FILES {
                        files.sort();
                        return files;
                    }
                }
            }
        }
    }
    files.sort();
    files
}

fn dropped_file_paths_to_mentions(text: &str, cwd: &Path) -> Option<String> {
    let candidates = dropped_file_path_candidates(text);
    if candidates.is_empty() {
        return None;
    }
    let mut mentions = Vec::new();
    for candidate in candidates {
        let path = normalize_dropped_path(&candidate);
        if !path.exists() {
            return None;
        }
        let mention = relative_path_string(cwd, &path).unwrap_or_else(|| path_to_string(&path));
        mentions.push(format!("@{mention}"));
    }
    Some(format!("{} ", mentions.join(" ")))
}

fn dropped_file_path_candidates(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.lines().count() > 1 {
        trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(trim_shell_path)
            .collect()
    } else {
        vec![trim_shell_path(trimmed)]
    }
}

fn normalize_dropped_path(candidate: &str) -> PathBuf {
    let candidate = candidate
        .strip_prefix("file://")
        .map(percent_decode_file_url)
        .unwrap_or_else(|| candidate.to_string());
    PathBuf::from(candidate)
}

fn trim_shell_path(value: &str) -> String {
    value
        .trim_matches(|ch| matches!(ch, '\'' | '"'))
        .to_string()
}

fn percent_decode_file_url(value: &str) -> String {
    value.replace("%20", " ")
}

fn relative_path_string(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root).ok().map(path_to_string)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn current_git_branch(cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("branch")
        .arg("--show-current")
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

async fn run_non_interactive(
    cli: CliArgs,
    harness: Harness,
    auth: AuthStorage,
    mut config: AppConfig,
    session_path: PathBuf,
    resource_catalog: ResourceCatalog,
    extension_runtime_provider_ids: BTreeSet<String>,
) -> Result<(), AppError> {
    if let Some(model) = cli.model.clone() {
        let command =
            ParsedCommand::Settings(SettingsCommand::SetModel(ensure_model_identifier(&model)?));
        let message = execute_runtime_command(
            command,
            &harness,
            &auth,
            &mut config,
            &session_path,
            &resource_catalog,
            &extension_runtime_provider_ids,
        )
        .await?;
        if cli.input.is_none() {
            println!("{message}");
            return Ok(());
        }
    } else if cli.settings && cli.input.is_none() {
        run_tui(
            harness,
            auth,
            TuiLaunchConfig {
                initial_model: config.model,
                initial_thinking_level: config.thinking_level,
                initial_thinking_collapse_mode: config.thinking_collapse_mode,
                initial_tool_collapse_mode: config.tool_collapse_mode,
                initial_chat_style: config.chat_style,
                initial_keymap: config.keymap,
                provider_config: OpenRouterConfig {
                    referer: config.referer,
                    title: config.title,
                    ..OpenRouterConfig::default()
                },
                session_path,
                resource_paths: resource_catalog.paths.clone(),
                resource_catalog,
                open_settings: true,
            },
        )
        .await?;
        return Ok(());
    }

    let Some(input) = cli.input else {
        return Ok(());
    };

    if input.trim_start().starts_with('/') && !contains_resource_reference(&input) {
        if let Some(command) = parse_command(&input) {
            let message = execute_runtime_command(
                command,
                &harness,
                &auth,
                &mut config,
                &session_path,
                &resource_catalog,
                &extension_runtime_provider_ids,
            )
            .await?;
            println!("{message}");
            return Ok(());
        }
        let tool_settings = load_tool_settings(&resource_catalog.paths).await;
        let snapshot = load_extension_snapshot(&resource_catalog.paths, &tool_settings);
        if let Some(message) = execute_extension_command(&input, &snapshot).await {
            println!("{}", message?);
            return Ok(());
        }
        return Err(AppError::InvalidArguments(format!(
            "unknown command `{input}`"
        )));
    }

    let input = expand_resource_references(&input, &resource_catalog)?;
    preflight_model_credentials(&auth, &config.model, &extension_runtime_provider_ids)
        .await
        .map_err(AppError::InvalidArguments)?;
    let messages = harness.prompt(Message::user_text(input)).await?;
    harness.save_session_jsonl(&session_path).await?;
    if let Some(text) = last_assistant_text(&messages) {
        println!("{text}");
    }
    eprintln!("session: {}", session_id_from_path(&session_path));
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CliResourceReferences {
    prompts: Vec<String>,
    skills: Vec<String>,
    incomplete: Vec<String>,
    user_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliResourceReferenceKind {
    Prompt,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliByteToken<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn contains_resource_reference(input: &str) -> bool {
    cli_resource_references(input).is_some()
}

fn expand_resource_references(input: &str, catalog: &ResourceCatalog) -> Result<String, AppError> {
    let Some(references) = cli_resource_references(input) else {
        return Ok(input.to_string());
    };
    if let Some(token) = references.incomplete.first() {
        return Err(AppError::InvalidArguments(format!(
            "incomplete resource reference `{token}`"
        )));
    }

    let mut prompts = Vec::new();
    for name in &references.prompts {
        let prompt = catalog.prompt_by_name(name).ok_or_else(|| {
            AppError::InvalidArguments(format!("unknown prompt `/prompt:{name}`"))
        })?;
        prompts.push(prompt);
    }

    let mut skills = Vec::new();
    for name in &references.skills {
        let skill = catalog
            .skill_by_name(name)
            .ok_or_else(|| AppError::InvalidArguments(format!("unknown skill `/skill:{name}`")))?;
        skills.push(skill);
    }

    Ok(build_cli_resource_augmented_prompt(
        &prompts,
        &skills,
        &references.user_input,
    ))
}

fn cli_resource_references(input: &str) -> Option<CliResourceReferences> {
    let mut references = CliResourceReferences::default();
    let mut found = false;
    let mut stripped = String::new();
    let mut copied_until = 0;

    for token in cli_byte_tokens(input) {
        let Some((kind, name)) = cli_resource_reference_token(token.text) else {
            continue;
        };
        found = true;
        stripped.push_str(&input[copied_until..token.start]);
        copied_until = token.end;

        if name.is_empty() {
            references.incomplete.push(token.text.to_string());
            continue;
        }
        match kind {
            CliResourceReferenceKind::Prompt => push_unique_resource(&mut references.prompts, name),
            CliResourceReferenceKind::Skill => push_unique_resource(&mut references.skills, name),
        }
    }

    if !found {
        return None;
    }

    stripped.push_str(&input[copied_until..]);
    references.user_input = clean_cli_resource_user_input(&stripped);
    Some(references)
}

fn cli_resource_reference_token(token: &str) -> Option<(CliResourceReferenceKind, String)> {
    if let Some(name) = token.strip_prefix("/prompt:") {
        return Some((CliResourceReferenceKind::Prompt, name.to_string()));
    }
    token
        .strip_prefix("/skill:")
        .map(|name| (CliResourceReferenceKind::Skill, name.to_string()))
}

fn push_unique_resource(items: &mut Vec<String>, value: String) {
    if !items.iter().any(|item| item == &value) {
        items.push(value);
    }
}

fn cli_byte_tokens(input: &str) -> Vec<CliByteToken<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, ch) in input.char_indices() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                tokens.push(CliByteToken {
                    text: &input[token_start..index],
                    start: token_start,
                    end: index,
                });
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(token_start) = start {
        tokens.push(CliByteToken {
            text: &input[token_start..],
            start: token_start,
            end: input.len(),
        });
    }
    tokens
}

fn clean_cli_resource_user_input(input: &str) -> String {
    input
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn build_cli_resource_augmented_prompt(
    prompts: &[&PromptTemplate],
    skills: &[&Skill],
    user_input: &str,
) -> String {
    if skills.is_empty() {
        return prompts
            .iter()
            .map(|prompt| prompt.expand(user_input))
            .collect::<Vec<_>>()
            .join("\n\n");
    }

    let mut output = String::from("Use the following Oino resources for this request.");
    if !prompts.is_empty() {
        output.push_str("\n\n# Included Prompt Templates");
        for prompt in prompts {
            output.push_str("\n\n");
            output.push_str(&markdown_resource_block(
                "Prompt",
                &prompt.name,
                &prompt.path.display().to_string(),
                &prompt.expand(user_input),
            ));
        }
    }
    if !skills.is_empty() {
        output.push_str("\n\n# Included Skills");
        for skill in skills {
            output.push_str("\n\n");
            output.push_str(&markdown_resource_block(
                "Skill",
                &skill.name,
                &skill.path.display().to_string(),
                &skill.content,
            ));
        }
    }
    if !user_input.is_empty() {
        output.push_str("\n\n# User Request\n\n");
        output.push_str(user_input);
    }
    output
}

fn markdown_resource_block(kind: &str, name: &str, source: &str, content: &str) -> String {
    format!(
        "## Included {kind}: `{name}`\nSource: `{source}`\n\n{}",
        fenced_markdown(content)
    )
}

fn fenced_markdown(content: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(content).saturating_add(1).max(4));
    format!("{fence}markdown\n{}\n{fence}", content.trim_end())
}

fn longest_backtick_run(content: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in content.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

const RALPH_LOOP_PACKAGE_ID: &str = "oino.ralph_loop";
const RALPH_LOOP_COMMAND_ID: &str = "ralph";
const MODE_SANDBOX_PACKAGE_ID: &str = "oino.mode_sandbox";
const NOTIFY_PACKAGE_ID: &str = "oino.notify";
const VCC_PACKAGE_ID: &str = "oino.vcc";
const VCC_RECALL_TOOL_NAME: &str = "vcc_recall";
const ASK_USER_PACKAGE_ID: &str = "oino.ask_user";

#[derive(Clone)]
struct VccRecallTool {
    session: Arc<tokio::sync::Mutex<SessionManager>>,
}

impl VccRecallTool {
    fn new(session: Arc<tokio::sync::Mutex<SessionManager>>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for VccRecallTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: VCC_RECALL_TOOL_NAME.into(),
            description: "Search raw Oino session history outside the compacted model context. Use this when prior details may have been compacted away.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "description": "Words to search for. Omit or leave empty to browse recent history."},
                    "scope": {"type": "string", "enum": ["active", "all"], "description": "Search the active branch by default, or all session entries."},
                    "offset": {"type": "number", "description": "Pagination offset, zero-based."},
                    "limit": {"type": "number", "description": "Maximum results to return, capped at 20."},
                    "expand": {"type": "boolean", "description": "Return full matching entries instead of snippets."}
                }
            }),
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> oino_agent_loop::LoopResult<ToolResult> {
        if signal.is_aborted() {
            return Err(LoopError::Aborted);
        }
        let options = recall_options_from_value(&call.arguments);
        let session = self.session.lock().await;
        let branch = session
            .get_branch(session.get_leaf_id())
            .map_err(|err| LoopError::Tool(err.to_string()))?;
        let all_entries = session.get_entries();
        drop(session);
        if signal.is_aborted() {
            return Err(LoopError::Aborted);
        }
        let result = vcc::recall(&branch, &all_entries, options);
        let mut tool_result = ToolResult::text(&call, result.output);
        tool_result.details = Some(serde_json::json!({
            "total": result.total,
            "offset": result.offset,
            "limit": result.limit
        }));
        Ok(tool_result)
    }
}

fn recall_options_from_value(value: &serde_json::Value) -> vcc::VccRecallOptions {
    let mut options = vcc::VccRecallOptions::default();
    if let Some(query) = value.get("query").and_then(serde_json::Value::as_str) {
        if !query.trim().is_empty() {
            options.query = Some(query.trim().to_string());
        }
    }
    if value
        .get("scope")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|scope| scope.eq_ignore_ascii_case("all"))
    {
        options.scope_all = true;
    }
    if let Some(offset) = value.get("offset").and_then(serde_json::Value::as_u64) {
        options.offset = offset as usize;
    }
    if let Some(limit) = value.get("limit").and_then(serde_json::Value::as_u64) {
        options.limit = limit as usize;
    }
    if let Some(expand) = value.get("expand").and_then(serde_json::Value::as_bool) {
        options.expand = expand;
    }
    options
}

fn package_contribution_active<T>(
    active: &ActiveContribution<T>,
    package: &str,
    contribution: &str,
) -> bool {
    active.effective_id.as_str() == contribution
        && active
            .entry
            .metadata
            .package_id
            .as_ref()
            .is_some_and(|package_id| package_id.as_str() == package)
}

fn optional_package_command_enabled(
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
    package: &str,
    command: &str,
) -> bool {
    let snapshot = load_extension_snapshot(paths, settings);
    snapshot
        .registries
        .commands
        .active
        .iter()
        .any(|active| package_contribution_active(active, package, command))
}

fn optional_package_tool_active(
    snapshot: &ExtensionManagerSnapshot,
    package: &str,
    tool: &str,
) -> bool {
    snapshot
        .registries
        .tools
        .active
        .iter()
        .any(|active| package_contribution_active(active, package, tool))
}

fn ralph_loop_command_enabled(paths: &ResourcePaths, settings: &ToolSettingsSnapshot) -> bool {
    optional_package_command_enabled(
        paths,
        settings,
        RALPH_LOOP_PACKAGE_ID,
        RALPH_LOOP_COMMAND_ID,
    )
}

fn vcc_command_enabled(
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
    command: &str,
) -> bool {
    optional_package_command_enabled(paths, settings, VCC_PACKAGE_ID, command)
}

fn notify_extension_enabled(paths: &ResourcePaths, settings: &ToolSettingsSnapshot) -> bool {
    let snapshot = load_extension_snapshot(paths, settings);
    snapshot.registries.hooks.active.iter().any(|active| {
        active
            .entry
            .metadata
            .package_id
            .as_ref()
            .is_some_and(|package_id| package_id.as_str() == NOTIFY_PACKAGE_ID)
    })
}

const MODE_SANDBOX_DIR: &str = "sandbox-mode";
const LEGACY_PLAN_SANDBOX_PROMPT: &str = "Sandbox mode: PLAN. Treat the workspace as read-only planning context. Use read freely, use bash only for inspection, and do not perform mutating bash/edit/write actions unless the user switches to work mode or changes this profile.";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct ModeSandboxProfile {
    allowed_tools: Vec<String>,
    prompt: String,
}

impl ModeSandboxProfile {
    fn allows_tool(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|allowed| {
            let allowed = allowed.trim();
            allowed == "*" || allowed == tool
        })
    }
}

impl Default for ModeSandboxProfile {
    fn default() -> Self {
        default_mode_sandbox_profile(&AgentMode::Work)
    }
}

fn default_mode_sandbox_profile(mode: &AgentMode) -> ModeSandboxProfile {
    match mode {
        AgentMode::Plan => ModeSandboxProfile {
            allowed_tools: vec!["read".into(), "bash".into()],
            prompt: "Sandbox mode: PLAN. Treat the workspace as read-only planning context. Use read freely and bash only for inspection. Do not edit/write files or run mutating shell commands unless the user switches to work mode or changes this profile.".into(),
        },
        AgentMode::Work => ModeSandboxProfile {
            allowed_tools: vec!["*".into()],
            prompt: "Sandbox mode: WORK. Normal enabled Oino tools are available; still follow project instructions and ask before risky or destructive actions.".into(),
        },
        AgentMode::Custom(name) => ModeSandboxProfile {
            allowed_tools: vec!["read".into(), "bash".into()],
            prompt: format!(
                "Sandbox mode: {}. Custom profile; edit allowed_tools and prompt in sandbox-mode/{}.json to change this mode. Default is read plus inspection-only bash.",
                mode.label(),
                name
            ),
        },
    }
}

fn normalize_loaded_mode_sandbox_profile(
    mode: &AgentMode,
    profile: ModeSandboxProfile,
) -> ModeSandboxProfile {
    if is_legacy_autogenerated_mode_sandbox_profile(mode, &profile) {
        default_mode_sandbox_profile(mode)
    } else {
        profile
    }
}

fn is_legacy_autogenerated_mode_sandbox_profile(
    mode: &AgentMode,
    profile: &ModeSandboxProfile,
) -> bool {
    matches!(mode, AgentMode::Plan)
        && profile
            .allowed_tools
            .iter()
            .map(String::as_str)
            .eq(["read", "bash", "edit"])
        && profile.prompt == LEGACY_PLAN_SANDBOX_PROMPT
}

fn write_mode_sandbox_profile(path: &Path, profile: &ModeSandboxProfile) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(profile).map_err(io::Error::other)?;
    fs::write(path, format!("{text}\n"))
}

fn sandbox_mode_dir_for_scope(paths: &ResourcePaths, scope: ToolSettingsScope) -> PathBuf {
    match scope {
        ToolSettingsScope::Global => paths.global_dir.join(MODE_SANDBOX_DIR),
        ToolSettingsScope::Project => paths.project_dir.join(MODE_SANDBOX_DIR),
    }
}

fn mode_sandbox_profile_path(
    paths: &ResourcePaths,
    scope: ToolSettingsScope,
    mode: &AgentMode,
) -> PathBuf {
    sandbox_mode_dir_for_scope(paths, scope).join(format!("{}.json", mode.value()))
}

fn load_mode_sandbox_profile(paths: &ResourcePaths, mode: &AgentMode) -> ModeSandboxProfile {
    for scope in [ToolSettingsScope::Project, ToolSettingsScope::Global] {
        let path = mode_sandbox_profile_path(paths, scope, mode);
        if let Ok(text) = fs::read_to_string(path) {
            return serde_json::from_str(&text)
                .map(|profile| normalize_loaded_mode_sandbox_profile(mode, profile))
                .unwrap_or_else(|_| default_mode_sandbox_profile(mode));
        }
    }
    default_mode_sandbox_profile(mode)
}

fn ensure_mode_sandbox_profiles(paths: &ResourcePaths) -> io::Result<()> {
    let dir = sandbox_mode_dir_for_scope(paths, ToolSettingsScope::Global);
    fs::create_dir_all(&dir)?;
    for mode in [AgentMode::Plan, AgentMode::Work] {
        let default_profile = default_mode_sandbox_profile(&mode);
        let global_path = mode_sandbox_profile_path(paths, ToolSettingsScope::Global, &mode);
        if global_path.exists() {
            upgrade_legacy_mode_sandbox_profile(&global_path, &mode)?;
        } else {
            write_mode_sandbox_profile(&global_path, &default_profile)?;
        }

        let project_path = mode_sandbox_profile_path(paths, ToolSettingsScope::Project, &mode);
        if project_path.exists() {
            upgrade_legacy_mode_sandbox_profile(&project_path, &mode)?;
        }
    }
    Ok(())
}

fn upgrade_legacy_mode_sandbox_profile(path: &Path, mode: &AgentMode) -> io::Result<()> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(profile) = serde_json::from_str::<ModeSandboxProfile>(&text) else {
        return Ok(());
    };
    if is_legacy_autogenerated_mode_sandbox_profile(mode, &profile) {
        write_mode_sandbox_profile(path, &default_mode_sandbox_profile(mode))?;
    }
    Ok(())
}

fn mode_sandbox_enabled_in_snapshot(snapshot: &ExtensionManagerSnapshot) -> bool {
    snapshot.registries.commands.active.iter().any(|active| {
        active
            .entry
            .metadata
            .package_id
            .as_ref()
            .is_some_and(|package_id| package_id.as_str() == MODE_SANDBOX_PACKAGE_ID)
    })
}

fn mode_sandbox_context_message(paths: &ResourcePaths, mode: &AgentMode) -> Option<Message> {
    let profile = load_mode_sandbox_profile(paths, mode);
    let prompt = profile.prompt.trim();
    if prompt.is_empty() {
        return None;
    }
    Some(Message::CompactionSummary {
        id: OinoId::nil(),
        summary: format!(
            "# Oino sandbox mode\n\nMode: {}\nProfile: {}\nAllowed tools: {}\n\n{}",
            mode.label(),
            mode.value(),
            profile.allowed_tools.join(", "),
            prompt
        ),
    })
}

fn mode_sandbox_command_enabled(
    _mode: &AgentMode,
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
) -> bool {
    optional_package_command_enabled(paths, settings, MODE_SANDBOX_PACKAGE_ID, "mode_profile")
}

fn mode_before_tool_call_result(
    mode: &AgentMode,
    profile: &ModeSandboxProfile,
    call: ToolCall,
) -> BeforeToolCallResult {
    if profile.allows_tool(&call.name) {
        return BeforeToolCallResult::Allow(call);
    }
    BeforeToolCallResult::Block(format!(
        "{} mode blocked tool `{}`. Allowed tools for this profile: {}. Edit `~/.oino/sandbox-mode/{}.json`, add a project override at `.oino/sandbox-mode/{}.json`, or switch modes before using this tool.",
        mode.label(),
        call.name,
        profile.allowed_tools.join(", "),
        mode.value(),
        mode.value()
    ))
}

fn execute_ralph_command_if_enabled(
    command: RalphCommand,
    paths: &ResourcePaths,
    settings: &ToolSettingsSnapshot,
) -> Result<String, AppError> {
    if !ralph_loop_command_enabled(paths, settings) {
        return Err(AppError::InvalidArguments(
            "Ralph loop extension is not enabled; install `builtin:ralph-loop` from `/extensions` first".into(),
        ));
    }
    execute_ralph_command(command, &paths.project_root)
}

fn execute_ralph_command(command: RalphCommand, project_root: &Path) -> Result<String, AppError> {
    match command {
        RalphCommand::Help => Ok(ralph_help_text()),
        RalphCommand::List => {
            let states = ralph_loop::list_states(project_root)?;
            if states.is_empty() {
                Ok("No Ralph loops found; start one with `/ralph start <name> <task>`".into())
            } else {
                Ok(states
                    .iter()
                    .map(ralph_loop::status_line)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        RalphCommand::Status { name } => {
            if let Some(name) = name {
                let state = ralph_loop::load_state(project_root, &name)?;
                Ok(ralph_loop::status_line(&state))
            } else {
                execute_ralph_command(RalphCommand::List, project_root)
            }
        }
        RalphCommand::Start { name, task } => {
            let state =
                ralph_loop::start_loop(project_root, ralph_loop::RalphLoopStart::new(name, task))?;
            Ok(format!(
                "Started Ralph loop `{}` at {}",
                state.name, state.task_file
            ))
        }
        RalphCommand::Pause { name } => {
            let state = ralph_loop::pause_loop(project_root, &name)?;
            Ok(format!("Paused Ralph loop `{}`", state.name))
        }
        RalphCommand::Resume { name } => {
            let state = ralph_loop::resume_loop(project_root, &name)?;
            Ok(format!("Resumed Ralph loop `{}`", state.name))
        }
        RalphCommand::Continue { name } | RalphCommand::Once { name } => {
            let state = load_ralph_target(project_root, name.as_deref())?;
            let prompt = ralph_loop::build_iteration_prompt(project_root, &state)?;
            Ok(format!(
                "Ralph loop `{}` is ready for iteration {}/{}. In the TUI, `/ralph continue {}` auto-runs it. Iteration prompt:\n\n{}",
                state.name,
                state.iteration.saturating_add(1).min(state.max_iterations),
                state.max_iterations,
                state.name,
                prompt
            ))
        }
        RalphCommand::Steer { name, note } => {
            let state = ralph_loop::append_steering(project_root, &name, note)?;
            Ok(format!("Added steering to Ralph loop `{}`", state.name))
        }
        RalphCommand::Cancel { name } => {
            let state = ralph_loop::cancel_loop(project_root, &name)?;
            Ok(format!("Cancelled Ralph loop `{}`", state.name))
        }
        RalphCommand::Archive { name } => {
            let state = ralph_loop::archive_loop(project_root, &name)?;
            Ok(format!("Archived Ralph loop `{}`", state.name))
        }
        RalphCommand::CleanArchive => {
            let count = ralph_loop::clean_archive(project_root)?;
            Ok(format!(
                "Removed {count} archived Ralph loop file{}",
                if count == 1 { "" } else { "s" }
            ))
        }
        RalphCommand::Record {
            name,
            promise,
            note,
        } => {
            let promise = ralph_promise_from_command(promise);
            let note = if note.trim().is_empty() {
                format!("recorded {promise:?}")
            } else {
                note
            };
            let state = ralph_loop::record_iteration(project_root, &name, promise, note)?;
            Ok(format!(
                "Recorded Ralph loop `{}` iteration {} ({:?})",
                state.name, state.iteration, state.status
            ))
        }
    }
}

async fn compact_session_with_vcc(harness: &Harness) -> Result<(String, Vec<Message>), AppError> {
    let branch = harness.active_branch_entries().await?;
    let compaction = vcc::compact_branch(&branch).ok_or_else(|| {
        AppError::InvalidArguments(
            "Nothing to compact yet; VCC needs at least one earlier entry before the latest user message".into(),
        )
    })?;
    let compacted_entries = compaction.compacted_entries;
    let kept_entries = compaction.kept_entries;
    let messages = harness
        .append_compaction(compaction.summary, compaction.replaces)
        .await?;
    Ok((
        format!(
            "VCC compacted {compacted_entries} session entries and kept {kept_entries} live tail entries"
        ),
        messages,
    ))
}

async fn compact_session_with_llm(
    harness: &Harness,
    stream: &Arc<dyn StreamProvider>,
    model: &Model,
    custom_prompt: Option<&str>,
    cwd: &std::path::Path,
) -> Result<(String, Vec<Message>), AppError> {
    let branch = harness.active_branch_entries().await?;

    // Try loading custom prompt from settings, then from project .oino/prompts/compact.md
    let prompt = match custom_prompt {
        Some(p) => llm_compact::load_custom_prompt(std::path::Path::new(p)).await,
        None => llm_compact::load_project_compact_prompt(cwd).await,
    };

    let compaction = llm_compact::compact_with_llm(
        &branch,
        stream.as_ref(),
        model.clone(),
        prompt.as_deref(),
        AbortSignal::new(),
    )
    .await
    .map_err(AppError::InvalidArguments)?;

    let compacted_entries = compaction.compacted_entries;
    let kept_entries = compaction.kept_entries;
    let messages = harness
        .append_compaction(compaction.summary, compaction.replaces)
        .await?;
    Ok((
        format!(
            "LLM compacted {compacted_entries} session entries and kept {kept_entries} live tail entries"
        ),
        messages,
    ))
}

async fn recall_session_with_vcc(
    harness: &Harness,
    query: Option<String>,
    append_to_context: bool,
) -> Result<(String, Option<Vec<Message>>), AppError> {
    let branch = harness.active_branch_entries().await?;
    let all_entries = harness.all_session_entries().await;
    let options = vcc::VccRecallOptions {
        query,
        ..Default::default()
    };
    let result = vcc::recall(&branch, &all_entries, options);
    let messages = if append_to_context {
        Some(harness.append_branch_summary(result.output.clone()).await?)
    } else {
        None
    };
    Ok((result.output, messages))
}

fn ralph_promise_from_command(promise: RalphRecordPromise) -> ralph_loop::RalphPromise {
    match promise {
        RalphRecordPromise::Continue => ralph_loop::RalphPromise::Continue,
        RalphRecordPromise::Complete => ralph_loop::RalphPromise::Complete,
        RalphRecordPromise::Blocked(reason) => ralph_loop::RalphPromise::Blocked(reason),
        RalphRecordPromise::Decide(question) => ralph_loop::RalphPromise::Decide(question),
        RalphRecordPromise::TaskDone(task_id) => ralph_loop::RalphPromise::TaskDone(task_id),
    }
}

fn compact_status_line(message: &str) -> String {
    message
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Done")
        .chars()
        .take(160)
        .collect()
}

fn ralph_help_text() -> String {
    [
        "Ralph loop commands:",
        "  /ralph start <name> <task>",
        "  /ralph list",
        "  /ralph status [name]",
        "  /ralph resume <name>",
        "  /ralph continue [name]",
        "  /ralph once [name]",
        "  /ralph steer <name> <urgent instruction>",
        "  /ralph pause <name>",
        "  /ralph cancel <name>",
        "  /ralph archive <name>",
        "  /ralph clean",
        "  /ralph record <name> <continue|complete|blocked|decide|done> [note/task-id]",
    ]
    .join("\n")
}

async fn execute_runtime_command(
    command: ParsedCommand,
    harness: &Harness,
    auth: &AuthStorage,
    config: &mut AppConfig,
    session_path: &std::path::Path,
    resource_catalog: &ResourceCatalog,
    extension_runtime_provider_ids: &BTreeSet<String>,
) -> Result<String, AppError> {
    let message = match command {
        ParsedCommand::Help => {
            return Ok("Oino help is available in the TUI with `/help`. Common commands: /sessions, /new, /settings, /model, /thinking, /title. In the composer, use @ to fuzzy-search file paths.".into());
        }
        ParsedCommand::NewSession => {
            return Err(AppError::InvalidArguments(
                "`/new` opens a fresh session in the TUI; start `oino` without `--session` to create a new non-interactive session".into(),
            ));
        }
        ParsedCommand::Sessions => {
            let sessions = session_list_items(None).await?;
            return Ok(format_session_list(&sessions));
        }
        ParsedCommand::Prompts => return Ok(format_prompt_list(resource_catalog)),
        ParsedCommand::Skills => return Ok(format_skill_list(resource_catalog)),
        ParsedCommand::Inspect => {
            let snapshot = harness.inspect_full_prompt().await?;
            return Ok(snapshot.content);
        }
        ParsedCommand::Extensions => {
            return Err(AppError::InvalidArguments(
                "`/extensions` opens the interactive extension manager in the TUI; use `/extensions update` to update installed packages from the shell".into(),
            ));
        }
        ParsedCommand::ExtensionsUpdate => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            let cwd = std::env::current_dir()
                .unwrap_or_else(|_| resource_catalog.paths.project_root.clone());
            return Ok(update_installed_extension_packages(
                &resource_catalog.paths,
                &cwd,
                &settings,
            ));
        }
        ParsedCommand::Compact => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            let user_settings = user_settings::UserSettings::load_default()
                .await
                .unwrap_or_default();
            match user_settings.compact.method {
                oino_types::CompactMethod::Vcc => {
                    if vcc_command_enabled(&resource_catalog.paths, &settings, "compact") {
                        let (message, _messages) = compact_session_with_vcc(harness).await?;
                        harness.save_session_jsonl(session_path).await?;
                        return Ok(message);
                    }
                    return Err(AppError::InvalidArguments(
                        "VCC extension is not enabled; install `builtin:vcc` from `/extensions` before using `/compact`".into(),
                    ));
                }
                oino_types::CompactMethod::Llm => {
                    return Err(AppError::InvalidArguments(
                        "LLM compaction requires the TUI. Use `/compact` inside an interactive session".into(),
                    ));
                }
            }
        }
        ParsedCommand::CompactMethod(method) => match method {
            CompactMethodOverride::Vcc => {
                let settings = load_tool_settings(&resource_catalog.paths).await;
                if vcc_command_enabled(&resource_catalog.paths, &settings, "compact") {
                    let (message, _messages) = compact_session_with_vcc(harness).await?;
                    harness.save_session_jsonl(session_path).await?;
                    return Ok(message);
                }
                return Err(AppError::InvalidArguments(
                        "VCC extension is not enabled; install `builtin:vcc` from `/extensions` before using `/compact`".into(),
                    ));
            }
            CompactMethodOverride::Llm => {
                return Err(AppError::InvalidArguments(
                        "LLM compaction requires the TUI. Use `/compact llm` inside an interactive session".into(),
                    ));
            }
        },
        ParsedCommand::CompactThreshold(pct) => {
            let mut settings = user_settings::UserSettings::load_default()
                .await
                .unwrap_or_default();
            match pct {
                Some(p) => {
                    settings.compact.threshold_pct = Some(p);
                    settings.save_default().await.map_err(|e| {
                        AppError::InvalidArguments(format!("Failed to save settings: {e}"))
                    })?;
                    return Ok(format!("Auto-compact threshold set to {p}%"));
                }
                None => {
                    let current = settings
                        .compact
                        .threshold_pct
                        .map(|p| format!("{p}%"))
                        .unwrap_or_else(|| "disabled".to_string());
                    return Ok(format!("Current auto-compact threshold: {current}"));
                }
            }
        }
        ParsedCommand::CompactAuto(enabled) => {
            let mut settings = user_settings::UserSettings::load_default()
                .await
                .unwrap_or_default();
            settings.compact.auto = enabled;
            settings
                .save_default()
                .await
                .map_err(|e| AppError::InvalidArguments(format!("Failed to save settings: {e}")))?;
            return Ok(format!(
                "Auto-compact {}",
                if enabled { "enabled" } else { "disabled" }
            ));
        }
        ParsedCommand::CompactModel(model) => {
            let mut settings = user_settings::UserSettings::load_default()
                .await
                .unwrap_or_default();
            match model {
                Some(Some(m)) => {
                    settings.compact.model = Some(m.clone());
                    settings.save_default().await.map_err(|e| {
                        AppError::InvalidArguments(format!("Failed to save settings: {e}"))
                    })?;
                    return Ok(format!("LLM compact model set to {m}"));
                }
                Some(None) => {
                    settings.compact.model = None;
                    settings.save_default().await.map_err(|e| {
                        AppError::InvalidArguments(format!("Failed to save settings: {e}"))
                    })?;
                    return Ok("LLM compact model set to inherit".into());
                }
                None => {
                    let current = settings.compact.model.as_deref().unwrap_or("inherit");
                    return Ok(format!("Current LLM compact model: {current}"));
                }
            }
        }
        ParsedCommand::CompactPrompt(path) => {
            let mut settings = user_settings::UserSettings::load_default()
                .await
                .unwrap_or_default();
            match path {
                Some(p) => {
                    settings.compact.prompt = Some(p.clone());
                    settings.save_default().await.map_err(|e| {
                        AppError::InvalidArguments(format!("Failed to save settings: {e}"))
                    })?;
                    return Ok(format!("LLM compact prompt set to {p}"));
                }
                None => {
                    let current = settings.compact.prompt.as_deref().unwrap_or("default");
                    return Ok(format!("Current LLM compact prompt: {current}"));
                }
            }
        }
        ParsedCommand::Recall { query } => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            if vcc_command_enabled(&resource_catalog.paths, &settings, "recall") {
                let (output, _messages) = recall_session_with_vcc(harness, query, false).await?;
                return Ok(output);
            }
            return Err(AppError::InvalidArguments(
                "VCC extension is not enabled; install `builtin:vcc` from `/extensions` before using `/recall`".into(),
            ));
        }
        ParsedCommand::Usage => {
            let report = usage_report_for_current_session(harness, auth, &config.model).await?;
            return Ok(report.format_text());
        }
        ParsedCommand::BtwOpen => {
            return Err(AppError::InvalidArguments(
                "`/btw` opens an interactive side panel in the TUI".into(),
            ));
        }
        ParsedCommand::BtwReset => {
            return Err(AppError::InvalidArguments(
                "`/btw new` opens a fresh interactive BTW panel in the TUI".into(),
            ));
        }
        ParsedCommand::BtwConfigure { model } => {
            let mut settings =
                user_settings::load_from_path(&resource_catalog.paths.global_settings)
                    .await
                    .unwrap_or_default();
            match model {
                None => {
                    return Ok("Usage: /model btw inherit OR /model btw <provider:model>".into())
                }
                Some(None) => settings.btw_model = None,
                Some(Some(model)) => {
                    if Model::from_identifier(&model).is_none() {
                        return Err(AppError::InvalidArguments(format!(
                            "invalid BTW model `{model}`"
                        )));
                    }
                    settings.btw_model = Some(model);
                }
            }
            user_settings::save_to_path(&settings, &resource_catalog.paths.global_settings).await?;
            return Ok(settings.btw_model.as_ref().map_or_else(
                || "BTW model set to inherit".into(),
                |model| format!("BTW model set to `{model}`"),
            ));
        }
        ParsedCommand::SetNotifySummaryModel { model } => {
            let mut settings =
                user_settings::load_from_path(&resource_catalog.paths.global_settings)
                    .await
                    .unwrap_or_default();
            match model {
                None => {
                    return Ok("Usage: /model notify-summary inherit|off OR /model notify-summary <provider:model>".into())
                }
                Some(None) => settings.notify.summarizer.model = None,
                Some(Some(model)) => {
                    if Model::from_identifier(&model).is_none() {
                        return Err(AppError::InvalidArguments(format!(
                            "invalid notify summary model `{model}`"
                        )));
                    }
                    settings.notify.summarizer.model = Some(model);
                }
            }
            user_settings::save_to_path(&settings, &resource_catalog.paths.global_settings).await?;
            return Ok(settings.notify.summarizer.model.as_ref().map_or_else(
                || "Notify summary model set to inherit/default".into(),
                |model| format!("Notify summary model set to `{model}`"),
            ));
        }
        ParsedCommand::AuthStatus { provider } => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            let snapshot = load_extension_snapshot(&resource_catalog.paths, &settings);
            let items = auth_status_items_with_extension_readiness(
                auth,
                provider.as_deref(),
                &config.model,
                &snapshot,
            )
            .await?;
            return Ok(format_auth_status(&items));
        }
        ParsedCommand::AuthQuickstart => {
            return Ok(format_auth_quickstart());
        }
        ParsedCommand::Ralph(command) => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            return execute_ralph_command_if_enabled(command, &resource_catalog.paths, &settings);
        }
        ParsedCommand::ShowAgentModeUsage => {
            return Ok("Usage: /mode plan | /mode work | /mode <profile>".into());
        }
        ParsedCommand::SetAgentMode(mode) => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            if mode_sandbox_command_enabled(&mode, &resource_catalog.paths, &settings) {
                ensure_mode_sandbox_profiles(&resource_catalog.paths)?;
                return Ok(format!("Mode set to {}", mode.label()));
            }
            return Err(AppError::InvalidArguments(format!(
                "Mode sandbox extension is not enabled; install `builtin:mode-sandbox` from `/extensions` before using `/mode {}`",
                mode.value()
            )));
        }
        ParsedCommand::CommandHelp(path) => {
            return format_command_help(&path).ok_or_else(|| {
                AppError::InvalidArguments(format!("no command help for `{path}`"))
            });
        }
        ParsedCommand::SetSessionTitle(title) => {
            harness.set_session_title(title.clone()).await?;
            harness.save_session_jsonl(session_path).await?;
            return Ok(format!("Session title set to {title}"));
        }
        ParsedCommand::ReloadResources => {
            let catalog = load_resource_catalog(&resource_catalog.paths)?;
            harness
                .set_system_prompt(Some(default_system_prompt(
                    &catalog.paths.project_root,
                    &catalog,
                )))
                .await;
            let settings = load_tool_settings(&resource_catalog.paths).await;
            let snapshot = load_extension_snapshot(&resource_catalog.paths, &settings);
            return Ok(format!(
                "Reloaded {} prompts, {} skills, {} packages, {} extensions, and extension contributions",
                catalog.prompts.len(),
                catalog.skills.len(),
                snapshot.packages.len(),
                snapshot.extensions.len()
            ));
        }
        ParsedCommand::Settings(SettingsCommand::OpenAuth) => {
            let settings = load_tool_settings(&resource_catalog.paths).await;
            let snapshot = load_extension_snapshot(&resource_catalog.paths, &settings);
            let items =
                auth_status_items_with_extension_readiness(auth, None, &config.model, &snapshot)
                    .await?;
            return Ok(format_auth_status(&items));
        }
        ParsedCommand::Settings(
            SettingsCommand::Open
            | SettingsCommand::OpenModelSelection
            | SettingsCommand::OpenThinkingLevel
            | SettingsCommand::OpenChatStyle
            | SettingsCommand::OpenTools
            | SettingsCommand::OpenKeymaps
            | SettingsCommand::OpenTheme
            | SettingsCommand::OpenExtensions
            | SettingsCommand::OpenNotify,
        ) => {
            return Err(AppError::InvalidArguments(
                "interactive settings pages cannot be opened in non-interactive mode; provide a setting path such as `/model router:kr/claude-sonnet-4.5` or `/thinking high`".into(),
            ));
        }
        ParsedCommand::Settings(SettingsCommand::SetModel(model)) => {
            let identifier = model.identifier();
            validate_model_identifier_with_extensions(&identifier, extension_runtime_provider_ids)
                .map_err(AppError::InvalidArguments)?;
            harness.set_model(model).await?;
            config.model = identifier.clone();
            format!("Model set to {identifier}")
        }
        ParsedCommand::Settings(SettingsCommand::SetThinkingLevel(level)) => {
            harness.set_thinking_level(level).await?;
            config.thinking_level = level;
            format!("Thinking level set to {}", oino_tui::thinking_label(level))
        }
        ParsedCommand::Settings(SettingsCommand::SetCollapseMode { target, mode }) => {
            match target {
                oino_tui::CollapseTarget::Thinking => config.thinking_collapse_mode = mode,
                oino_tui::CollapseTarget::Tool => config.tool_collapse_mode = mode,
            }
            format!(
                "{} collapse mode set to {}",
                collapse_target_value(target),
                collapse_mode_value(mode)
            )
        }
        ParsedCommand::Settings(SettingsCommand::SetChatStyle(style)) => {
            config.chat_style = style;
            format!("Chat style set to {}", oino_tui::chat_style_label(style))
        }
        ParsedCommand::Settings(SettingsCommand::SetNotifyEnabled { scope, enabled }) => {
            let mut settings = load_tool_settings(&resource_catalog.paths).await;
            set_notify_enabled(&mut settings, scope, enabled);
            save_tool_settings_for_scope(&settings, &resource_catalog.paths, scope).await?;
            return Ok(format!("{} notify enabled set to {enabled}", scope.label()));
        }
        ParsedCommand::Settings(SettingsCommand::SetNotifyField {
            scope,
            field,
            value,
        }) => {
            let mut settings = load_tool_settings(&resource_catalog.paths).await;
            set_notify_field(&mut settings, scope, field, value);
            save_tool_settings_for_scope(&settings, &resource_catalog.paths, scope).await?;
            return Ok(format!(
                "{} notify {} updated",
                scope.label(),
                field.label()
            ));
        }
        ParsedCommand::Settings(SettingsCommand::SetNotifyEvent {
            scope,
            event,
            enabled,
        }) => {
            let mut settings = load_tool_settings(&resource_catalog.paths).await;
            set_notify_event(&mut settings, scope, event, enabled);
            save_tool_settings_for_scope(&settings, &resource_catalog.paths, scope).await?;
            return Ok(format!(
                "{} notify event {} set to {enabled}",
                scope.label(),
                event.label()
            ));
        }
    };
    let mut settings = user_settings::load_from_path(&resource_catalog.paths.global_settings)
        .await
        .unwrap_or_default();
    settings.apply_current(
        config.model.clone(),
        config.thinking_level,
        config.thinking_collapse_mode,
        config.tool_collapse_mode,
        config.chat_style,
    );
    user_settings::save_to_path(&settings, &resource_catalog.paths.global_settings).await?;
    harness.save_session_jsonl(session_path).await?;
    Ok(message)
}

fn session_id_from_path(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown")
        .to_string()
}

async fn session_list_items(
    current_session_id: Option<&str>,
) -> Result<Vec<SessionListItem>, AppError> {
    let root = default_session_root()?;
    session_list_items_from_root(root, current_session_id).await
}

async fn session_list_items_from_root(
    root: PathBuf,
    current_session_id: Option<&str>,
) -> Result<Vec<SessionListItem>, AppError> {
    let repository = SessionRepository::new(root);
    let paths = repository.list().await?;
    let mut rows = Vec::new();
    for path in paths {
        let id = session_id_from_path(&path);
        let modified = tokio::fs::metadata(&path)
            .await
            .and_then(|metadata| metadata.modified())
            .ok();
        let Ok(session) = repository.open(&path).await else {
            continue;
        };
        let Ok(context) = session.build_session_context() else {
            continue;
        };
        let message_count = context.messages.len();
        if message_count == 0 {
            continue;
        }
        let preview = context
            .messages
            .iter()
            .rev()
            .find_map(message_preview)
            .unwrap_or_else(|| "(no preview)".into());
        rows.push((
            modified,
            SessionListItem {
                current: current_session_id == Some(id.as_str()),
                id,
                name: session.get_session_name(),
                cwd: path_to_string(&session.header().cwd),
                message_count,
                preview,
            },
        ));
    }
    rows.sort_by(|(left_modified, left), (right_modified, right)| {
        right_modified
            .cmp(left_modified)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(rows.into_iter().map(|(_, item)| item).collect())
}

fn message_preview(message: &Message) -> Option<String> {
    match message {
        Message::User { content, .. } | Message::Assistant { content, .. } => {
            content_text_preview(content)
        }
        Message::ToolResult {
            tool_name,
            is_error,
            content,
            ..
        } => {
            let prefix = if *is_error { "tool error" } else { "tool" };
            content_text_preview(content).map(|text| format!("{prefix} {tool_name}: {text}"))
        }
        Message::CompactionSummary { summary, .. } | Message::BranchSummary { summary, .. } => {
            Some(summary.clone())
        }
        Message::Custom { name, .. } => Some(format!("custom: {name}")),
    }
}

fn content_text_preview(content: &[ContentBlock]) -> Option<String> {
    let joined = content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } | ContentBlock::Thinking { text, .. } => text.as_str(),
            ContentBlock::ToolCall { name, .. } => name.as_str(),
            ContentBlock::Image { .. } => "image",
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn format_session_list(sessions: &[SessionListItem]) -> String {
    if sessions.is_empty() {
        return "No saved sessions".into();
    }
    sessions
        .iter()
        .map(|session| format!("{}  {} - {}", session.id, session.name, session.preview))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_prompt_list(catalog: &ResourceCatalog) -> String {
    if catalog.prompts.is_empty() {
        return "No prompt templates found".into();
    }
    catalog
        .prompts
        .iter()
        .map(|prompt| {
            let hint = prompt.argument_hint.as_deref().unwrap_or("");
            format!(
                "/prompt:{} {}  [{}]  {}  {}",
                prompt.name,
                hint,
                prompt.scope.label(),
                prompt.description,
                prompt.path.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_skill_list(catalog: &ResourceCatalog) -> String {
    if catalog.skills.is_empty() {
        return "No skills found".into();
    }
    catalog
        .skills
        .iter()
        .map(|skill| {
            format!(
                "/skill:{}  [{}]  {}  {}",
                skill.name,
                skill.scope.label(),
                skill.description,
                skill.path.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Default)]
struct RalphRunController {
    active_loop: Option<String>,
    auto_continue: bool,
    prompt_loop_in_flight: Option<String>,
}

#[derive(Debug)]
enum TuiRuntimeEvent {
    Agent(AgentEvent),
    PromptFinished(Result<Vec<Message>, String>),
    BtwFinished(Result<Vec<Message>, String>),
    SessionTitle(String),
    ModelCatalog(ModelCatalogUpdate),
    UsageProgress(UsageReport),
    AskUserPrompt {
        request: AskUserRequest,
        responder: oneshot::Sender<AskUserOutcome>,
    },
}

async fn persist_current_settings(state: &mut TuiState) {
    let mut settings = UserSettings::load_default().await.unwrap_or_default();
    settings.apply_current(
        state.settings.selected_model().to_string(),
        state.settings.selected_thinking_level,
        state.settings.thinking_collapse_mode,
        state.settings.tool_collapse_mode,
        state.settings.chat_style,
    );
    settings.tools = tool_map_from_state(state, ToolSettingsScope::Global);
    settings.keymap = Some(state.settings.keymap.clone());
    settings.theme = state.settings.global_theme.clone();
    if let Err(err) = settings.save_default().await {
        state.set_error(format!("Settings save failed: {err}"));
        state.status = HELP_STATUS.into();
    }
}

async fn save_tui_session(state: &mut TuiState, harness: &Harness, path: &std::path::Path) {
    if let Err(err) = harness.save_session_jsonl(path).await {
        state.set_error(format!("Session save failed: {err}"));
        state.status = HELP_STATUS.into();
    }
}

async fn refresh_tui_context_status(state: &mut TuiState, harness: &Harness, cwd: &Path) {
    state.set_working_directory(path_to_string(cwd));
    state.set_git_branch(current_git_branch(cwd));
    match harness.inspect_full_prompt().await {
        Ok(snapshot) => state.set_context_tokens(Some(snapshot.token_count)),
        Err(_) => state.set_context_tokens(None),
    }
}

/// Check if auto-compaction should trigger based on settings and current context usage.
/// Returns true if compaction was performed.
async fn try_auto_compact(
    state: &mut TuiState,
    harness: &Harness,
    auth: &AuthStorage,
    provider_config: &OpenRouterConfig,
    extension_snapshot: &ExtensionManagerSnapshot,
    session_path: &std::path::Path,
    cwd: &Path,
) -> bool {
    let user_settings = match user_settings::UserSettings::load_default().await {
        Ok(s) => s,
        Err(_) => return false,
    };

    if !user_settings.compact.auto {
        return false;
    }

    let threshold_pct = match user_settings.compact.threshold_pct {
        Some(p) if p > 0 && p <= 100 => p,
        _ => return false,
    };

    let context_tokens = match state.runtime_status.context_tokens {
        Some(t) => t,
        None => return false,
    };

    let context_length = match state.settings.selected_model_context_length() {
        Some(l) => l,
        None => return false,
    };

    if context_length == 0 {
        return false;
    }

    let usage_pct = (context_tokens as f64 / context_length as f64 * 100.0) as u8;
    if usage_pct < threshold_pct {
        return false;
    }

    // Auto-compact triggered
    let method = user_settings.compact.method;
    match method {
        oino_types::CompactMethod::Vcc => match compact_session_with_vcc(harness).await {
            Ok((message, messages)) => {
                state.set_messages_from_oino(&messages);
                let _ = harness.save_session_jsonl(session_path).await;
                state.clear_error();
                state.status = format!("Auto-compacted (VCC): {message}");
                refresh_tui_context_status(state, harness, cwd).await;
                true
            }
            Err(_) => false,
        },
        oino_types::CompactMethod::Llm => {
            let model_id = user_settings
                .compact
                .model
                .as_deref()
                .filter(|m| *m != "inherit")
                .unwrap_or(&state.settings.selected_model());
            let model = match Model::from_identifier(model_id) {
                Some(m) => m,
                None => return false,
            };
            let compact_stream = build_runtime_provider(
                auth.clone(),
                provider_config.clone(),
                extension_runtime_providers(extension_snapshot),
            );
            match compact_session_with_llm(
                harness,
                &compact_stream,
                &model,
                user_settings.compact.prompt.as_deref(),
                cwd,
            )
            .await
            {
                Ok((message, messages)) => {
                    state.set_messages_from_oino(&messages);
                    let _ = harness.save_session_jsonl(session_path).await;
                    state.clear_error();
                    state.status = format!("Auto-compacted (LLM): {message}");
                    refresh_tui_context_status(state, harness, cwd).await;
                    true
                }
                Err(_) => false,
            }
        }
    }
}

fn spawn_model_catalog_task(
    tx: mpsc::UnboundedSender<TuiRuntimeEvent>,
    auth: AuthStorage,
    openrouter_config: OpenRouterConfig,
    _initial_model: String,
) {
    tokio::spawn(async move {
        if let Some(update) =
            model_catalog::load_cached_update_with_historical_provider_catalog(false).await
        {
            let _ = tx.send(TuiRuntimeEvent::ModelCatalog(update));
        }

        let refresh_configs = model_catalog::all_refresh_configs_with_historical_provider_catalog(
            &openrouter_config,
            false,
        );
        let mut freshness_checks = futures::future::join_all(
            refresh_configs
                .iter()
                .map(|config| model_catalog::cached_is_fresh(&config.provider_id)),
        )
        .await;
        freshness_checks
            .push(model_catalog::cached_is_fresh(model_catalog::ROUTER_PROVIDER_ID).await);
        let fresh = !freshness_checks.is_empty() && freshness_checks.into_iter().all(|fresh| fresh);
        let initial_delay = if fresh {
            model_catalog::MODEL_REFRESH_INTERVAL
        } else {
            Duration::ZERO
        };
        tokio::time::sleep(initial_delay).await;

        loop {
            let _ = tx.send(TuiRuntimeEvent::ModelCatalog(ModelCatalogUpdate {
                models: Vec::new(),
                status: "Refreshing provider model catalogs…".into(),
                refreshing: true,
            }));
            let update = model_catalog::refresh_all_update_with_historical_provider_catalog(
                &auth,
                &openrouter_config,
                false,
            )
            .await;
            let _ = tx.send(TuiRuntimeEvent::ModelCatalog(update));
            if let Ok(config) = load_router_config() {
                let base_url = resolved_router_base_url(&config);
                let update = model_catalog::refresh_openai_proxy_update(
                    model_catalog::ROUTER_PROVIDER_ID,
                    "router",
                    &base_url,
                    Some("OMNIROUTE_API_KEY"),
                )
                .await;
                let _ = tx.send(TuiRuntimeEvent::ModelCatalog(update));
            }
            tokio::time::sleep(model_catalog::MODEL_REFRESH_INTERVAL).await;
        }
    });
}

async fn register_mode_hooks(harness: &Harness, mode: Arc<Mutex<AgentMode>>, paths: ResourcePaths) {
    let guard_mode = Arc::clone(&mode);
    let guard_paths = paths.clone();
    harness
        .hooks()
        .on_before_tool_call(Arc::new(move |call| {
            let mode = guard_mode
                .lock()
                .map_or_else(|_| AgentMode::Work, |guard| guard.clone());
            let profile = load_mode_sandbox_profile(&guard_paths, &mode);
            Box::pin(async move { Ok(mode_before_tool_call_result(&mode, &profile, call)) })
        }))
        .await;

    let context_mode = Arc::clone(&mode);
    let context_paths = paths;
    harness
        .hooks()
        .on_context(Arc::new(move |mut messages| {
            let mode = context_mode
                .lock()
                .map_or_else(|_| AgentMode::Work, |guard| guard.clone());
            let paths = context_paths.clone();
            Box::pin(async move {
                let settings = load_tool_settings(&paths).await;
                if mode_sandbox_command_enabled(&mode, &paths, &settings) {
                    if let Some(message) = mode_sandbox_context_message(&paths, &mode) {
                        messages.insert(0, message);
                    }
                }
                Ok(messages)
            })
        }))
        .await;
}

async fn register_notify_hooks(
    harness: &Harness,
    paths: ResourcePaths,
    stream: Arc<dyn StreamProvider>,
) {
    let client = Arc::new(reqwest::Client::new());
    let last_assistant_message = Arc::new(Mutex::new(None::<String>));
    for hook in [
        NotificationHook::MessageEnd,
        NotificationHook::AgentEnd,
        NotificationHook::ToolExecutionEnd,
    ] {
        let paths = paths.clone();
        let client = Arc::clone(&client);
        let last_assistant_message = Arc::clone(&last_assistant_message);
        let stream = Arc::clone(&stream);
        harness
            .hooks()
            .on_notification(
                hook,
                Arc::new(move |event| {
                    let paths = paths.clone();
                    let client = Arc::clone(&client);
                    let last_assistant_message = Arc::clone(&last_assistant_message);
                    let stream = Arc::clone(&stream);
                    Box::pin(async move {
                        if let AgentEvent::MessageEnd { message } = &event {
                            if let Some(text) = assistant_message_text(message) {
                                if let Ok(mut guard) = last_assistant_message.lock() {
                                    *guard = Some(text);
                                }
                            }
                            return;
                        }
                        let summary_source = last_assistant_message
                            .lock()
                            .ok()
                            .and_then(|guard| guard.clone());
                        send_notify_event_if_enabled(paths, client, event, summary_source, stream)
                            .await;
                    })
                }),
            )
            .await;
    }
}

fn assistant_message_text(message: &Message) -> Option<String> {
    let Message::Assistant { content, .. } = message else {
        return None;
    };
    let text = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

async fn send_notify_event_if_enabled(
    paths: ResourcePaths,
    client: Arc<reqwest::Client>,
    event: AgentEvent,
    summary_source: Option<String>,
    stream: Arc<dyn StreamProvider>,
) {
    let settings = load_tool_settings(&paths).await;
    if !notify_extension_enabled(&paths, &settings) {
        return;
    }
    let Some(config) =
        notify::resolve_notify_config(&settings.global.notify, &settings.project.notify)
    else {
        return;
    };
    let Some(mut message) = notify::notify_message_for_event(&event) else {
        return;
    };
    if !config.events.contains(&message.event) {
        return;
    }
    if message.event == notify::NotifyEvent::AgentEnd {
        if let Some(source) = summary_source.filter(|source| !source.trim().is_empty()) {
            if config.summarizer.enabled {
                message.body = summarize_notify_body(&config, &source, stream).await;
            }
        }
    }
    if let Err(err) = notify::send_ntfy_notification(&client, &config, &message).await {
        eprintln!("Oino notify failed: {err}");
    }
}

async fn summarize_notify_body(
    config: &notify::EffectiveNotifyConfig,
    source: &str,
    stream: Arc<dyn StreamProvider>,
) -> String {
    let fallback = notify::notification_summary_from_text(source, config.summarizer.max_chars);
    let Some(model_id) = config.summarizer.model.as_deref() else {
        return fallback;
    };
    let Some(model) = Model::from_identifier(model_id) else {
        return fallback;
    };
    let prompt = format!(
        "{}\n\nReturn only the notification body. Maximum characters: {}.\n\nRun output:\n{}",
        config.summarizer.prompt, config.summarizer.max_chars, source
    );
    let request = StreamRequest {
        model,
        thinking_level: ThinkingLevel::Off,
        system_prompt: Some("You write concise desktop/mobile notification summaries.".into()),
        messages: vec![Message::user_text(prompt)],
        tools: Vec::new(),
    };
    let Ok(events) = stream.stream(request, AbortSignal::new()).await else {
        return fallback;
    };
    let text = events
        .into_iter()
        .filter_map(|event| match event {
            AssistantStreamEvent::TextDelta { delta } => Some(delta),
            _ => None,
        })
        .collect::<String>();
    if text.trim().is_empty() {
        fallback
    } else {
        notify::notification_summary_from_text(&text, config.summarizer.max_chars)
    }
}

async fn register_tui_stream_hooks(harness: &Harness, tx: mpsc::UnboundedSender<TuiRuntimeEvent>) {
    for hook in [
        NotificationHook::AgentStart,
        NotificationHook::MessageStart,
        NotificationHook::MessageUpdate,
        NotificationHook::MessageEnd,
        NotificationHook::ToolExecutionStart,
        NotificationHook::ToolExecutionEnd,
        NotificationHook::AgentEnd,
        NotificationHook::QueueUpdate,
        NotificationHook::Settled,
    ] {
        let tx = tx.clone();
        harness
            .hooks()
            .on_notification(
                hook,
                Arc::new(move |event| {
                    let tx = tx.clone();
                    Box::pin(async move {
                        let _ = tx.send(TuiRuntimeEvent::Agent(event));
                    })
                }),
            )
            .await;
    }
}

fn materialize_accepted_steer(state: &mut TuiState, message: &Message) {
    state.finish_message(message);
    state.transcript_scroll.jump_bottom();
    state.status = "Steered current response".into();
}

fn apply_tui_runtime_event(
    state: &mut TuiState,
    event: TuiRuntimeEvent,
    prompt_in_flight: &mut bool,
    extension_models: &[ModelOption],
) {
    match event {
        TuiRuntimeEvent::Agent(AgentEvent::AgentStart { .. }) => {
            if state.working {
                state.set_calling_status();
            }
        }
        TuiRuntimeEvent::Agent(AgentEvent::MessageStart { message_id, role }) => {
            state.start_message(message_id, role);
        }
        TuiRuntimeEvent::Agent(AgentEvent::MessageUpdate {
            message_id,
            content,
        }) => {
            state.update_message(message_id, &content);
        }
        TuiRuntimeEvent::Agent(AgentEvent::MessageEnd { message }) => {
            state.finish_message(&message);
        }
        TuiRuntimeEvent::Agent(AgentEvent::ToolExecutionStart { call }) => {
            state.status = format!("Running tool `{}`…", call.name);
        }
        TuiRuntimeEvent::Agent(AgentEvent::ToolExecutionEnd { .. }) => {
            if state.working {
                state.set_calling_status();
            }
        }
        TuiRuntimeEvent::Agent(AgentEvent::QueueUpdate { queue, pending }) => {
            state.status = format!("{queue} queue: {pending} pending");
        }
        TuiRuntimeEvent::Agent(AgentEvent::Settled) => {
            state.status = if state.working {
                "Saving…".into()
            } else {
                HELP_STATUS.into()
            };
        }
        TuiRuntimeEvent::Agent(AgentEvent::AgentEnd { .. }) => {
            state.status = "Saving…".into();
        }
        TuiRuntimeEvent::Agent(_) => {}
        TuiRuntimeEvent::SessionTitle(title) => {
            state.set_session_title(title);
        }
        TuiRuntimeEvent::ModelCatalog(update) => {
            if update.models.is_empty() {
                state.settings.status = update.status;
                state.set_model_catalog_refreshing(update.refreshing);
            } else {
                state.set_model_catalog(
                    merge_extension_models(update.models, extension_models),
                    update.status,
                );
                state.set_model_catalog_refreshing(update.refreshing);
            }
        }
        TuiRuntimeEvent::PromptFinished(result) => {
            *prompt_in_flight = false;
            state.set_working(false);
            match result {
                Ok(messages) => state.set_messages_from_oino(&messages),
                Err(message) => {
                    state.set_error(message);
                    state.status = HELP_STATUS.into();
                }
            }
        }
        TuiRuntimeEvent::UsageProgress(report) => {
            let status = report.status_line();
            state.set_usage_report(report.to_tui_report());
            if !state.working && state.error.is_none() {
                state.status = status;
            }
        }
        TuiRuntimeEvent::AskUserPrompt { .. } | TuiRuntimeEvent::BtwFinished(_) => {}
    }
}

async fn preflight_model_credentials(
    auth: &AuthStorage,
    model_identifier: &str,
    extension_runtime_provider_ids: &BTreeSet<String>,
) -> Result<(), String> {
    let Some(model) = Model::from_identifier(model_identifier) else {
        return Err(format!(
            "Invalid model identifier `{model_identifier}`; expected provider:model-id"
        ));
    };
    if extension_runtime_provider_ids.contains(&model.provider) {
        return Ok(());
    }
    if provider_by_id(&model.provider).is_none() {
        return Err(format!(
            "Provider `{}` is not registered. Install/enable an extension runtime provider for `{}` or run `/router setup` and select a `router:<model>` model.",
            model.provider, model.provider
        ));
    }
    let _ = auth;
    Err(provider_status_for_model_identifier(model_identifier).unwrap_err())
}

fn user_facing_error(err: &HarnessError) -> String {
    let message = err.to_string();
    if message.contains("missing credential") || message.contains("OPENROUTER_API_KEY") {
        "Provider credential is missing, but built-in provider auth/runtime has been removed from Oino core. Run `/router setup` and select a `router:<model>` model, or install/enable an extension runtime provider.".into()
    } else {
        message
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    mouse_capture_enabled: bool,
}

fn paint_url_overlays(
    backend: &mut CrosstermBackend<Stdout>,
    overlays: &[TerminalUrlOverlay],
    restore_cursor: Option<(u16, u16)>,
) -> io::Result<()> {
    for overlay in overlays {
        queue!(
            backend,
            MoveTo(overlay.x, overlay.y),
            SetForegroundColor(CColor::Blue),
            SetAttribute(CAttribute::Underlined),
            Print(osc8_link(&overlay.text, &overlay.url)),
            SetAttribute(CAttribute::NoUnderline),
            ResetColor
        )?;
    }
    if let Some((x, y)) = restore_cursor.filter(|_| !overlays.is_empty()) {
        queue!(backend, MoveTo(x, y))?;
    }
    backend.flush()
}

fn osc8_link(label: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
}

fn terminal_mouse_capture_enabled() -> bool {
    matches!(
        std::env::var("OINO_ENABLE_MOUSE_CAPTURE").ok().as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

impl TerminalGuard {
    fn enter() -> Result<Self, AppError> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        let mouse_capture_enabled = terminal_mouse_capture_enabled();
        execute!(
            stdout,
            EnterAlternateScreen,
            Clear(ClearType::All),
            MoveTo(0, 0),
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        if mouse_capture_enabled {
            execute!(stdout, EnableMouseCapture)?;
        }
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            mouse_capture_enabled,
        })
    }

    fn draw(&mut self, state: &TuiState) -> Result<(), AppError> {
        let size = self.terminal.size()?;
        let overlays = transcript_url_overlays(state, size.width, size.height);
        let cursor = terminal_cursor_position(state, size.width, size.height);
        self.terminal.draw(|frame| render(frame, state))?;
        paint_url_overlays(self.terminal.backend_mut(), &overlays, cursor)?;
        Ok(())
    }

    fn size(&self) -> io::Result<(u16, u16)> {
        self.terminal.size().map(|size| (size.width, size.height))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        if self.mouse_capture_enabled {
            let _ = execute!(self.terminal.backend_mut(), DisableMouseCapture);
        }
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            PopKeyboardEnhancementFlags,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::FauxStream;
    use oino_auth::AuthConfig;
    use oino_session::SessionHeader;
    use oino_types::{AssistantStreamEvent, StopReason};

    #[test]
    fn default_model_is_omniroute_model() {
        assert_eq!(DEFAULT_MODEL, "router:kr/claude-sonnet-4.5");
    }

    #[test]
    fn extension_runtime_provider_is_used_without_builtin_runtime_fallback() {
        let provider: ProviderContribution = serde_json::from_value(serde_json::json!({
            "id": "provider.router",
            "provider_id": "router",
            "display_name": "OmniRoute",
            "privacy": { "can_receive_prompts": true },
            "runtime": {
                "protocol": "open_ai_chat_completions",
                "base_url": "http://localhost:20128/v1",
                "api_key": { "kind": "none" },
                "model_id": "strip_provider_prefix"
            }
        }))
        .unwrap_or_else(|err| panic!("extension provider should parse: {err}"));
        let auth = AuthStorage::new(
            AuthConfig::new(std::env::temp_dir().join("oino-app-extension-runtime-auth.json"))
                .with_process_env(false),
        );
        let router = ProviderRouter::new(auth, vec![provider]);
        let model = Model::from_identifier("router:kr/test-model")
            .unwrap_or_else(|| panic!("model should parse"));
        let result = router.provider_for_model(&model);
        assert!(
            result.is_ok(),
            "extension runtime should be selected without built-in runtime fallback"
        );
    }

    #[test]
    fn model_validation_accepts_extension_runtime_provider_prefixes() {
        let extension_providers = BTreeSet::from(["router".to_string()]);
        let model =
            validate_model_identifier_with_extensions("router:kr/test-model", &extension_providers)
                .unwrap_or_else(|err| panic!("extension model should validate: {err}"));
        assert_eq!(model.provider, "router");

        let err = validate_model_identifier_with_extensions(
            "not-installed:test-model",
            &extension_providers,
        )
        .expect_err("unknown non-extension provider should be rejected");
        assert!(err.contains("extension runtime"));
        assert!(err.contains("/router setup"));
    }

    #[test]
    fn auth_quickstart_is_omniroute_first_after_builtin_auth_removal() {
        let guide = format_auth_quickstart();
        assert!(guide.contains("Recommended path: OmniRoute extension"));
        assert!(guide.contains("/router setup"));
        assert!(guide.contains("Built-in provider OAuth/API-key commands have been removed"));
    }

    #[test]
    fn router_default_config_resolves_known_good_tag() {
        let config = RouterConfig::default();
        assert_eq!(resolved_router_base_url(&config), ROUTER_DEFAULT_BASE_URL);
        assert_eq!(resolved_router_tag(&config), ROUTER_KNOWN_GOOD_TAG);
        assert!(router_managed_run_command(&config, ROUTER_KNOWN_GOOD_TAG)
            .contains("diegosouzapw/omniroute:3.8.7"));
    }

    #[test]
    fn router_tag_validation_rejects_unsafe_values() {
        assert_eq!(validate_router_tag("3.8.7").unwrap(), "3.8.7");
        assert!(validate_router_tag("../latest").is_err());
        assert!(validate_router_tag("").is_err());
    }

    #[test]
    fn router_start_candidates_deduplicate_fallback_order() {
        let config = RouterConfig {
            pinned_tag: Some("3.8.8".into()),
            last_good_tag: Some("3.8.7".into()),
            known_good_tag: "3.8.7".into(),
            ..RouterConfig::default()
        };
        assert_eq!(router_start_candidates(&config), vec!["3.8.8", "3.8.7"]);
    }

    #[test]
    fn router_run_args_include_pinned_image_and_data_dir() {
        let config = RouterConfig::default();
        let args = router_run_args(&config, "3.8.7").join(" ");
        assert!(args.contains("--name oino-router"));
        assert!(args.contains("20128:20128"));
        assert!(args.contains("diegosouzapw/omniroute:3.8.7"));
    }

    #[test]
    fn router_extension_readiness_detail_includes_live_health_and_fallback_state() {
        let config = RouterConfig::default();
        let health = RouterHealth {
            reachable: true,
            status: Some("200 OK".into()),
            model_count: Some(42),
            error: None,
        };
        let detail = format_router_extension_readiness_detail(
            &config,
            "/tmp/oino-router-config.json",
            &health,
        );
        assert!(detail.contains("mode External"));
        assert!(detail.contains("http://localhost:20128/v1"));
        assert!(detail.contains("diegosouzapw/omniroute:3.8.7"));
        assert!(detail.contains("last-good 3.8.7"));
        assert!(detail
            .contains("Live health: reachable at http://localhost:20128/v1/models · 42 models"));
    }

    #[test]
    fn extension_config_helpers_support_dotted_keys_and_safe_provider_ids() {
        let value = serde_json::json!({
            "base_url": "http://localhost:20128/v1",
            "secrets": { "api_key": "secret-token" }
        });
        assert_eq!(
            extension_config_string_from_value(&value, "base_url").as_deref(),
            Some("http://localhost:20128/v1")
        );
        assert_eq!(
            extension_config_string_from_value(&value, "secrets.api_key").as_deref(),
            Some("secret-token")
        );
        assert!(extension_config_string_from_value(&value, "missing.key").is_none());
        assert_eq!(
            extension_config_dir_name("provider.test-1").unwrap(),
            "provider.test-1"
        );
        assert!(extension_config_dir_name("../provider").is_err());
        assert_eq!(
            extension_runtime_base_url_env_candidates("router"),
            vec![
                "OMNIROUTE_BASE_URL".to_string(),
                "ROUTER_BASE_URL".to_string()
            ]
        );
        let runtime: oino_extension_core::ProviderRuntimeContribution =
            serde_json::from_value(serde_json::json!({
                "protocol": "open_ai_chat_completions",
                "base_url": "http://localhost:20128/v1",
                "config": {
                    "base_url_key": "runtime.base_url",
                    "health_url_key": "runtime.health_url",
                    "base_url_env": ["OMNIROUTE_BASE_URL"],
                    "health_url_env": ["OMNIROUTE_HEALTH_URL"]
                }
            }))
            .unwrap_or_else(|err| panic!("runtime config metadata should parse: {err}"));
        assert_eq!(
            runtime.config.base_url_key.as_deref(),
            Some("runtime.base_url")
        );
        assert_eq!(
            runtime.config.health_url_key.as_deref(),
            Some("runtime.health_url")
        );
        assert_eq!(runtime.config.base_url_env, vec!["OMNIROUTE_BASE_URL"]);
        assert_eq!(runtime.config.health_url_env, vec!["OMNIROUTE_HEALTH_URL"]);
    }

    #[test]
    fn removed_provider_runtime_info_documents_openai_split_and_oauth_usage() {
        let openai = provider_by_id("openai")
            .and_then(|provider| auth_readiness::removed_provider_runtime_info(*provider))
            .unwrap_or_else(|| panic!("openai removed-runtime info missing"));
        assert!(openai
            .historical_optional_env
            .contains(&OPENAI_ACCESS_TOKEN_ENV));
        assert!(openai
            .historical_optional_env
            .contains(&OPENAI_REFRESH_TOKEN_ENV));
        assert!(openai.historical_optional_env.contains(&OPENAI_API_KEY_ENV));
        assert_eq!(
            openai.historical_example_model,
            Some("openai-api:gpt-4o-mini")
        );
        assert!(openai.hint.contains("removed from core"));
    }

    #[test]
    fn removed_provider_runtime_info_documents_azure_and_bedrock_config_shapes() {
        let azure = provider_by_id("azure")
            .and_then(|provider| auth_readiness::removed_provider_runtime_info(*provider))
            .unwrap_or_else(|| panic!("azure removed-runtime info missing"));
        assert!(azure
            .historical_required_env
            .contains(&"AZURE_OPENAI_ENDPOINT"));
        assert!(azure
            .historical_required_env
            .contains(&"AZURE_OPENAI_DEPLOYMENT"));
        assert!(azure
            .historical_required_env
            .contains(&"AZURE_OPENAI_API_KEY"));
        assert!(azure
            .historical_optional_env
            .contains(&"AZURE_OPENAI_API_VERSION"));
        assert_eq!(
            azure.historical_example_model,
            Some("azure:<deployment-or-model>")
        );

        let bedrock = provider_by_id("bedrock")
            .and_then(|provider| auth_readiness::removed_provider_runtime_info(*provider))
            .unwrap_or_else(|| panic!("bedrock removed-runtime info missing"));
        assert!(bedrock.historical_required_env.contains(&"AWS_REGION"));
        assert!(bedrock
            .historical_optional_env
            .contains(&"AWS_ACCESS_KEY_ID"));
        assert!(bedrock
            .historical_optional_env
            .contains(&"AWS_BEARER_TOKEN_BEDROCK"));
        assert!(bedrock.hint.contains("removed from core"));
        assert_eq!(bedrock.historical_example_model, Some("bedrock:<model-id>"));
    }

    #[test]
    fn model_identifier_accepts_openrouter_suffix_colon() {
        let model = ensure_model_identifier("openrouter:deepseek/deepseek-v4-flash:free")
            .unwrap_or_else(|err| panic!("model identifier should parse: {err}"));
        assert_eq!(
            model,
            Model::new("openrouter", "deepseek/deepseek-v4-flash:free")
        );
    }

    #[test]
    fn ctrl_left_down_is_required_for_external_mouse_open() {
        assert!(is_external_open_mouse_event(&MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::CONTROL,
        }));
        assert!(!is_external_open_mouse_event(&MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        }));
        assert!(!is_external_open_mouse_event(&MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::CONTROL,
        }));
    }

    #[test]
    fn resolves_external_targets_before_opening() {
        let cwd = Path::new("/tmp/oino-project");

        assert_eq!(
            resolve_external_target("https://example.com", cwd),
            "https://example.com"
        );
        assert_eq!(
            resolve_external_target("file:///tmp/image.png", cwd),
            "file:///tmp/image.png"
        );
        assert_eq!(
            resolve_external_target("assets/image.png", cwd),
            "/tmp/oino-project/assets/image.png"
        );
    }

    #[test]
    fn extension_install_source_accepts_local_paths_and_git_sources() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let cwd = temp.path().join("project");
        let home = temp.path().join("home");
        fs::create_dir_all(cwd.join("examples/extensions"))
            .unwrap_or_else(|err| panic!("create local path failed: {err}"));

        assert_eq!(
            resolve_install_source("examples/extensions", &cwd, &home),
            ExtensionInstallSource::Local(cwd.join("examples/extensions"))
        );
        assert_eq!(
            resolve_install_source("~/packages/example", &cwd, &home),
            ExtensionInstallSource::Local(home.join("packages/example"))
        );
        assert_eq!(
            resolve_install_source("acme/example-extension", &cwd, &home),
            ExtensionInstallSource::Git(GitInstallSource {
                clone_url: "https://github.com/acme/example-extension.git".into(),
                display: "github:acme/example-extension".into(),
                reference: None,
            })
        );
        assert_eq!(
            resolve_install_source("github:acme/example-extension#v1.2.3", &cwd, &home),
            ExtensionInstallSource::Git(GitInstallSource {
                clone_url: "https://github.com/acme/example-extension.git".into(),
                display: "github:acme/example-extension#v1.2.3".into(),
                reference: Some("v1.2.3".into()),
            })
        );
        assert_eq!(
            resolve_install_source(
                "git+https://github.com/acme/example-extension.git#main",
                &cwd,
                &home,
            ),
            ExtensionInstallSource::Git(GitInstallSource {
                clone_url: "https://github.com/acme/example-extension.git".into(),
                display: "https://github.com/acme/example-extension.git#main".into(),
                reference: Some("main".into()),
            })
        );
    }

    #[test]
    fn mode_profile_filters_tools_and_defaults_to_plan_allowlist() {
        let read = ToolCall {
            id: OinoId::nil(),
            name: "read".into(),
            arguments: serde_json::json!({"path":"README.md"}),
        };
        let bash = ToolCall {
            id: OinoId::nil(),
            name: "bash".into(),
            arguments: serde_json::json!({"command":"pwd"}),
        };
        let edit = ToolCall {
            id: OinoId::nil(),
            name: "edit".into(),
            arguments: serde_json::json!({"path":"README.md"}),
        };
        let write = ToolCall {
            id: OinoId::nil(),
            name: "write".into(),
            arguments: serde_json::json!({"path":"README.md"}),
        };
        let plan = default_mode_sandbox_profile(&AgentMode::Plan);
        let work = default_mode_sandbox_profile(&AgentMode::Work);
        assert!(matches!(
            mode_before_tool_call_result(&AgentMode::Plan, &plan, read),
            BeforeToolCallResult::Allow(_)
        ));
        assert!(matches!(
            mode_before_tool_call_result(&AgentMode::Plan, &plan, bash),
            BeforeToolCallResult::Allow(_)
        ));
        match mode_before_tool_call_result(&AgentMode::Plan, &plan, edit) {
            BeforeToolCallResult::Block(reason) => {
                assert!(reason.contains("Plan mode blocked tool `edit`"));
                assert!(reason.contains(".oino/sandbox-mode/plan.json"));
            }
            BeforeToolCallResult::Allow(_) => panic!("plan profile should block edit"),
        }
        match mode_before_tool_call_result(&AgentMode::Plan, &plan, write.clone()) {
            BeforeToolCallResult::Block(reason) => {
                assert!(reason.contains("Plan mode blocked tool `write`"));
                assert!(reason.contains(".oino/sandbox-mode/plan.json"));
            }
            BeforeToolCallResult::Allow(_) => panic!("plan profile should block write"),
        }
        assert!(matches!(
            mode_before_tool_call_result(&AgentMode::Work, &work, write),
            BeforeToolCallResult::Allow(_)
        ));
    }

    #[test]
    fn mode_profiles_are_created_globally_and_project_can_override() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap_or_else(|err| panic!("project dir failed: {err}"));
        let paths = ResourcePaths::from_home_and_cwd(&home, &project)
            .unwrap_or_else(|err| panic!("resource paths failed: {err}"));
        ensure_mode_sandbox_profiles(&paths)
            .unwrap_or_else(|err| panic!("ensure profiles failed: {err}"));
        let read_path = paths.global_dir.join("sandbox-mode/read.json");
        let plan_path = paths.global_dir.join("sandbox-mode/plan.json");
        let work_path = paths.global_dir.join("sandbox-mode/work.json");
        assert!(!read_path.exists());
        assert!(plan_path.is_file());
        assert!(work_path.is_file());
        let plan = load_mode_sandbox_profile(&paths, &AgentMode::Plan);
        assert_eq!(plan.allowed_tools, vec!["read", "bash"]);
        assert!(plan.prompt.contains("PLAN"));

        let project_plan_path = paths.project_dir.join("sandbox-mode/plan.json");
        let project_plan_dir = project_plan_path
            .parent()
            .unwrap_or_else(|| panic!("project plan path should have a parent"));
        fs::create_dir_all(project_plan_dir)
            .unwrap_or_else(|err| panic!("project profile dir failed: {err}"));
        fs::write(
            &project_plan_path,
            serde_json::json!({
                "allowed_tools": ["read", "bash", "edit"],
                "prompt": LEGACY_PLAN_SANDBOX_PROMPT,
            })
            .to_string(),
        )
        .unwrap_or_else(|err| panic!("write legacy profile failed: {err}"));
        ensure_mode_sandbox_profiles(&paths)
            .unwrap_or_else(|err| panic!("legacy profile upgrade failed: {err}"));
        let plan = load_mode_sandbox_profile(&paths, &AgentMode::Plan);
        assert_eq!(plan.allowed_tools, vec!["read", "bash"]);

        fs::write(
            &project_plan_path,
            r#"{"allowed_tools":["read"],"prompt":"custom plan"}"#,
        )
        .unwrap_or_else(|err| panic!("write profile failed: {err}"));
        let plan = load_mode_sandbox_profile(&paths, &AgentMode::Plan);
        assert_eq!(plan.allowed_tools, vec!["read"]);
        assert_eq!(plan.prompt, "custom plan");
        let message = mode_sandbox_context_message(&paths, &AgentMode::Plan)
            .unwrap_or_else(|| panic!("missing context message"));
        assert!(matches!(
            message,
            Message::CompactionSummary { summary, .. } if summary.contains("custom plan")
        ));

        let custom = AgentMode::Custom("review".into());
        let custom_profile = load_mode_sandbox_profile(&paths, &custom);
        assert_eq!(custom_profile.allowed_tools, vec!["read", "bash"]);
        assert!(custom_profile.prompt.contains("Custom profile"));
        let custom_path = mode_sandbox_profile_path(&paths, ToolSettingsScope::Project, &custom);
        assert!(custom_path.ends_with(".oino/sandbox-mode/review.json"));
        let custom_dir = custom_path
            .parent()
            .unwrap_or_else(|| panic!("custom profile path should have a parent"));
        fs::create_dir_all(custom_dir)
            .unwrap_or_else(|err| panic!("custom profile dir failed: {err}"));
        fs::write(
            &custom_path,
            r#"{"allowed_tools":["read"],"prompt":"custom review"}"#,
        )
        .unwrap_or_else(|err| panic!("write custom profile failed: {err}"));
        let custom_profile = load_mode_sandbox_profile(&paths, &custom);
        assert_eq!(custom_profile.allowed_tools, vec!["read"]);
        assert_eq!(custom_profile.prompt, "custom review");
    }

    #[test]
    fn executes_ralph_commands_against_project_state() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let help = execute_ralph_command(RalphCommand::Help, temp.path())
            .unwrap_or_else(|err| panic!("help failed: {err}"));
        assert!(help.contains("/ralph start"));

        let started = execute_ralph_command(
            RalphCommand::Start {
                name: "Demo Loop".into(),
                task: "Build iterative feature".into(),
            },
            temp.path(),
        )
        .unwrap_or_else(|err| panic!("start failed: {err}"));
        assert!(started.contains("demo-loop"));
        assert!(temp.path().join(".oino/ralph/demo-loop.json").is_file());

        let recorded = execute_ralph_command(
            RalphCommand::Record {
                name: "demo-loop".into(),
                promise: RalphRecordPromise::TaskDone("TASK-1".into()),
                note: "scaffolded".into(),
            },
            temp.path(),
        )
        .unwrap_or_else(|err| panic!("record failed: {err}"));
        assert!(recorded.contains("iteration 1"));

        let status = execute_ralph_command(
            RalphCommand::Status {
                name: Some("demo-loop".into()),
            },
            temp.path(),
        )
        .unwrap_or_else(|err| panic!("status failed: {err}"));
        assert!(status.contains("demo-loop: Active iteration 1/60"));

        execute_ralph_command(
            RalphCommand::Archive {
                name: "demo-loop".into(),
            },
            temp.path(),
        )
        .unwrap_or_else(|err| panic!("archive failed: {err}"));
        let cleaned = execute_ralph_command(RalphCommand::CleanArchive, temp.path())
            .unwrap_or_else(|err| panic!("clean failed: {err}"));
        assert!(cleaned.contains("Removed 4 archived Ralph loop files"));
    }

    #[test]
    fn prepares_optional_builtin_extension_install_sources() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let prepared = prepare_install_source("builtin:footer-status", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("builtin package should resolve: {err}"));
        let by_id = prepare_install_source("builtin:oino.footer_status", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("builtin package id should resolve: {err}"));

        assert_eq!(prepared.display, "builtin:footer-status");
        assert!(prepared.path.ends_with("footer-status"));
        assert!(prepared.path.join("oino.package.json").is_file());
        assert_eq!(prepared.path, by_id.path);
        let ralph = prepare_install_source("builtin:ralph-loop", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("ralph builtin package should resolve: {err}"));
        assert!(ralph.path.ends_with("ralph-loop"));
        assert!(ralph.path.join("oino.package.json").is_file());
        let mode = prepare_install_source("builtin:mode-sandbox", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("mode builtin package should resolve: {err}"));
        assert!(mode.path.ends_with("mode-sandbox"));
        assert!(mode.path.join("oino.package.json").is_file());
        let notify = prepare_install_source("builtin:notify", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("notify builtin package should resolve: {err}"));
        assert!(notify.path.ends_with("notify"));
        assert!(notify.path.join("oino.package.json").is_file());
        let craft = prepare_install_source("builtin:craft-skill", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("craft-skill builtin package should resolve: {err}"));
        assert!(craft.path.ends_with("craft-skill"));
        assert!(craft.path.join("oino.package.json").is_file());
        let vcc = prepare_install_source("builtin:vcc", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("vcc builtin package should resolve: {err}"));
        assert!(vcc.path.ends_with("vcc"));
        assert!(vcc.path.join("oino.package.json").is_file());
        let ask_user = prepare_install_source("builtin:ask-user", temp.path(), temp.path())
            .unwrap_or_else(|err| panic!("ask-user builtin package should resolve: {err}"));
        assert!(ask_user.path.ends_with("ask-user"));
        assert!(ask_user.path.join("oino.package.json").is_file());

        let err = prepare_install_source("builtin:missing", temp.path(), temp.path())
            .err()
            .unwrap_or_else(|| panic!("missing builtin should fail"));
        assert!(err.contains("unknown optional built-in extension `missing`"));
        assert!(err.contains("oino.footer_status"));
        assert!(err.contains("oino.ralph_loop"));
        assert!(err.contains("oino.mode_sandbox"));
        assert!(err.contains("oino.notify"));
        assert!(err.contains("oino.craft_skill"));
        assert!(err.contains("oino.vcc"));
        assert!(err.contains("oino.ask_user"));
    }

    #[test]
    fn extensions_update_refreshes_installed_builtin_packages() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let paths = ResourcePaths::from_home_and_cwd(&home, &project)
            .unwrap_or_else(|err| panic!("resource paths failed: {err}"));
        paths
            .ensure_skeleton()
            .unwrap_or_else(|err| panic!("resource skeleton failed: {err}"));
        let settings = ToolSettingsSnapshot::default();
        let mut manager = extension_manager_with_current_policy(&paths, &settings);
        manager.load();
        let service = PackageLifecycleService::new(
            extension_layout_paths(&paths),
            current_extension_version(),
        );
        let package = oino_extension_builtins::optional_builtin_packages()
            .iter()
            .find(|package| package.id == "oino.router")
            .unwrap_or_else(|| panic!("missing OmniRoute optional builtin"));
        service
            .install_local(package.path(), PackageInstallScope::Project, &mut manager)
            .unwrap_or_else(|err| panic!("install failed: {err}"));
        let manifest_path = project
            .join(".oino/extension-packages")
            .join(package.id)
            .join("oino.package.json");
        let mut installed = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(&manifest_path)
                .unwrap_or_else(|err| panic!("read manifest failed: {err}")),
        )
        .unwrap_or_else(|err| panic!("parse manifest failed: {err}"));
        installed["version"] = serde_json::json!("0.0.1");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&installed).unwrap(),
        )
        .unwrap_or_else(|err| panic!("write downgraded manifest failed: {err}"));

        let report = update_installed_extension_packages(&paths, &project, &settings);
        assert!(report.contains("1 updated"), "{report}");
        assert!(report.contains("oino.router"), "{report}");
        let refreshed = serde_json::from_str::<PackageManifest>(
            &fs::read_to_string(&manifest_path)
                .unwrap_or_else(|err| panic!("read refreshed manifest failed: {err}")),
        )
        .unwrap_or_else(|err| panic!("parse refreshed manifest failed: {err}"));
        let source_manifest = serde_json::from_str::<PackageManifest>(
            &fs::read_to_string(package.path().join("oino.package.json"))
                .unwrap_or_else(|err| panic!("read source manifest failed: {err}")),
        )
        .unwrap_or_else(|err| panic!("parse source manifest failed: {err}"));
        assert_eq!(refreshed.version, source_manifest.version);
    }

    #[tokio::test]
    async fn migrates_legacy_router_builtin_install_on_startup() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let paths = ResourcePaths::from_home_and_cwd(&home, &project)
            .unwrap_or_else(|err| panic!("resource paths failed: {err}"));
        paths
            .ensure_skeleton()
            .unwrap_or_else(|err| panic!("resource skeleton failed: {err}"));

        let legacy_dir = home.join(".oino/extension-packages/oino.9router");
        fs::create_dir_all(&legacy_dir).unwrap_or_else(|err| panic!("legacy dir failed: {err}"));
        user_settings::save_to_path(
            &UserSettings {
                extensions: oino_extension_core::ExtensionPolicySettings {
                    packages: BTreeMap::from([(
                        PackageId::new("oino.9router").unwrap(),
                        PolicyToggle::Enabled,
                    )]),
                    ..Default::default()
                },
                ..Default::default()
            },
            &paths.global_settings,
        )
        .await
        .unwrap_or_else(|err| panic!("write settings failed: {err}"));

        migrate_legacy_router_builtin(&paths).await;

        assert!(!legacy_dir.exists());
        assert!(home
            .join(".oino/extension-packages/oino.router/oino.package.json")
            .is_file());
        let settings = user_settings::load_from_path(&paths.global_settings)
            .await
            .unwrap_or_else(|err| panic!("read settings failed: {err}"));
        assert!(!settings
            .extensions
            .packages
            .contains_key(&PackageId::new("oino.9router").unwrap()));
        assert_eq!(
            settings
                .extensions
                .packages
                .get(&PackageId::new("oino.router").unwrap()),
            Some(&PolicyToggle::Enabled)
        );
    }

    #[tokio::test]
    async fn optional_builtin_packages_install_activate_and_toggle() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let paths = ResourcePaths::from_home_and_cwd(&home, &project)
            .unwrap_or_else(|err| panic!("resource paths failed: {err}"));
        paths
            .ensure_skeleton()
            .unwrap_or_else(|err| panic!("resource skeleton failed: {err}"));

        let mut settings = ToolSettingsSnapshot::default();
        let mut manager = extension_manager_with_current_policy(&paths, &settings);
        manager.load();
        let service = PackageLifecycleService::new(
            extension_layout_paths(&paths),
            current_extension_version(),
        );

        for package in oino_extension_builtins::optional_builtin_packages() {
            let report = service
                .install_local(package.path(), PackageInstallScope::Project, &mut manager)
                .unwrap_or_else(|err| panic!("install {} failed: {err}", package.id));
            assert_eq!(report.package_id.as_str(), package.id);
            assert!(project
                .join(".oino/extension-packages")
                .join(package.id)
                .join("oino.package.json")
                .is_file());
            set_extension_enabled(
                &mut settings,
                ExtensionManagementTarget::Package,
                report.package_id.to_string(),
                ToolSettingsScope::Project,
                true,
            );
        }

        let snapshot = load_extension_snapshot(&paths, &settings);
        let packages = snapshot
            .packages
            .iter()
            .map(|package| package.id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        for package in oino_extension_builtins::optional_builtin_packages() {
            assert!(
                packages.contains(package.id),
                "missing package {}",
                package.id
            );
        }

        let ui_surfaces = snapshot
            .registries
            .ui_surfaces
            .active
            .iter()
            .map(|active| active.effective_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(ui_surfaces.contains("footer_status_top"));
        assert!(ui_surfaces.contains("footer_status_bottom"));

        let commands = snapshot
            .registries
            .commands
            .active
            .iter()
            .map(|active| active.effective_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(commands.contains("ralph"));
        assert!(commands.contains("mode_profile"));
        assert!(!commands.contains("mode_read"));
        assert!(!commands.contains("mode_create"));
        assert!(commands.contains("notify"));
        assert!(commands.contains("compact"));
        assert!(commands.contains("recall"));

        let settings_pages = snapshot
            .registries
            .settings_pages
            .active
            .iter()
            .map(|active| active.effective_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(settings_pages.contains("notify"));

        let command_suggestions = extension_command_suggestions(&snapshot)
            .into_iter()
            .map(|command| command.label)
            .collect::<BTreeSet<_>>();
        assert!(command_suggestions.contains("/router"));
        let router_guide = execute_extension_command("/router guide", &snapshot)
            .await
            .unwrap_or_else(|| panic!("OmniRoute extension command should be known"))
            .unwrap_or_else(|err| panic!("OmniRoute extension command failed: {err}"));
        assert!(router_guide.contains("OmniRoute guide"));
        assert!(router_guide.contains("Managed sidecar command"));

        assert!(command_suggestions.contains("/ralph"));
        assert!(command_suggestions.contains("/mode"));
        assert!(!command_suggestions.contains("/mode:read"));
        assert!(!command_suggestions.contains("/mode:plan"));
        assert!(!command_suggestions.contains("/mode:work"));
        assert!(!command_suggestions.contains("/mode:create"));
        assert!(command_suggestions.contains("/settings notify"));
        assert!(command_suggestions.contains("/compact"));
        assert!(command_suggestions.contains("/recall"));

        let hooks = snapshot
            .registries
            .hooks
            .active
            .iter()
            .map(|active| active.effective_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(hooks.contains("notify_agent_end"));
        assert!(hooks.contains("notify_tool_result"));

        let tools = snapshot
            .registries
            .tools
            .active
            .iter()
            .map(|active| active.effective_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        assert!(tools.contains("vcc_recall"));
        assert!(tools.contains("ask_user"));

        let resources = snapshot
            .registries
            .resources
            .active
            .iter()
            .map(|active| active.effective_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        let router_status = extension_readiness_status_items(&snapshot, Some("router"), "router");
        assert!(router_status
            .iter()
            .any(|item| item.auth_kind == "extension custom"
                && item.readiness == "extension-managed"));
        assert!(router_status
            .iter()
            .any(|item| item.auth_kind == "extension provider"
                && item.readiness == "runtime registered"));
        assert!(router_status.iter().all(|item| item.current));
        assert!(resources.contains("ralph_loop_skill"));
        assert!(resources.contains("mode-sandbox"));
        assert!(resources.contains("craft-skill"));
        let (_prompts, skills, diagnostics) = extension_resource_items(&snapshot);
        assert!(
            diagnostics.is_empty(),
            "resource diagnostics: {diagnostics:?}"
        );
        assert!(skills.iter().any(|skill| {
            skill.name == "mode-sandbox" && skill.description.starts_with("Use when")
        }));
        assert!(skills.iter().any(|skill| {
            skill.name == "craft-skill" && skill.description.starts_with("Use when")
        }));

        set_extension_enabled(
            &mut settings,
            ExtensionManagementTarget::Package,
            "oino.router".into(),
            ToolSettingsScope::Project,
            false,
        );
        let router_disabled = load_extension_snapshot(&paths, &settings);
        let inactive_router_status =
            extension_readiness_status_items(&router_disabled, Some("router"), "openrouter");
        assert!(inactive_router_status
            .iter()
            .any(|item| item.provider_id == "router"
                && item.readiness == "inactive"
                && item.detail.contains("Remediation")));

        set_extension_enabled(
            &mut settings,
            ExtensionManagementTarget::Package,
            "oino.craft_skill".into(),
            ToolSettingsScope::Project,
            false,
        );
        let disabled = load_extension_snapshot(&paths, &settings);
        assert!(!disabled
            .registries
            .resources
            .active
            .iter()
            .any(|active| active.effective_id.as_str() == "craft-skill"));
    }

    #[test]
    fn browser_env_candidate_replaces_url_placeholders() {
        let candidate = browser_env_candidate("https://example.com", "firefox --new-tab %s")
            .unwrap_or_else(|| panic!("browser candidate should parse"));

        assert_eq!(
            candidate,
            OpenCommand::new("firefox", ["--new-tab", "https://example.com"])
        );
    }

    #[test]
    fn wsl_interop_candidates_open_urls_with_windows_host() {
        let candidates = wsl_interop_candidates("https://example.com");

        assert_eq!(
            candidates.first(),
            Some(&OpenCommand::new(
                "cmd.exe",
                ["/C", "start", "", "https://example.com"]
            ))
        );
        assert!(candidates.iter().any(|candidate| candidate
            == &OpenCommand::with_target("explorer.exe", "https://example.com")));
        assert!(candidates.iter().any(|candidate| candidate
            == &OpenCommand::new(
                "rundll32.exe",
                ["url.dll,FileProtocolHandler", "https://example.com"]
            )));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_openers_fallback_beyond_xdg_open() {
        let candidates = opener_candidates("https://example.com", Some("firefox --new-tab %s"));

        assert_eq!(
            candidates.first(),
            Some(&OpenCommand::new(
                "firefox",
                ["--new-tab", "https://example.com"]
            ))
        );
        assert!(candidates
            .iter()
            .any(|candidate| candidate
                == &OpenCommand::with_target("xdg-open", "https://example.com")));
        assert!(candidates.iter().any(
            |candidate| candidate == &OpenCommand::new("gio", ["open", "https://example.com"])
        ));
        assert!(candidates.iter().any(|candidate| candidate
            == &OpenCommand::with_target("sensible-browser", "https://example.com")));
    }

    #[test]
    fn cli_parses_session_and_command_input() {
        let session_id = OinoId::nil();
        let cli = CliArgs::parse_from([
            "--session",
            "00000000-0000-0000-0000-000000000000",
            "/settings",
            "model",
            "openrouter:xai/glm-5.1",
        ])
        .unwrap_or_else(|err| panic!("parse failed: {err}"));
        assert_eq!(cli.session, Some(session_id));
        assert_eq!(
            cli.input.as_deref(),
            Some("/settings model openrouter:xai/glm-5.1")
        );

        let cli = CliArgs::parse_from(["settings", "notify", "enabled", "true"])
            .unwrap_or_else(|err| panic!("parse failed: {err}"));
        assert_eq!(cli.input.as_deref(), Some("/settings notify enabled true"));

        let cli = CliArgs::parse_from(["compact", "auto", "--help"])
            .unwrap_or_else(|err| panic!("parse failed: {err}"));
        assert_eq!(cli.input.as_deref(), Some("/compact auto --help"));
    }

    #[test]
    fn tool_settings_inherit_global_values_and_keep_title_tool_off_by_default() {
        let mut settings = ToolSettingsSnapshot::default();
        settings.global.tools.insert("bash".into(), false);

        let items = tool_settings_items(&settings);
        let bash = items
            .iter()
            .find(|item| item.name == "bash")
            .unwrap_or_else(|| panic!("missing bash"));
        assert!(!bash.global_enabled);
        assert!(!bash.project_enabled);
        let title = items
            .iter()
            .find(|item| item.name == oino_tools::SESSION_TITLE_TOOL_NAME)
            .unwrap_or_else(|| panic!("missing title tool"));
        assert!(!title.global_enabled);
        assert!(!title.project_enabled);

        settings.project.tools.insert("bash".into(), true);
        assert!(project_tool_enabled(&settings, "bash"));
    }

    #[tokio::test]
    async fn load_tool_settings_preserves_project_theme_settings() {
        let home = tempfile::tempdir().unwrap_or_else(|err| panic!("home tempdir: {err}"));
        let project = tempfile::tempdir().unwrap_or_else(|err| panic!("project tempdir: {err}"));
        let paths = ResourcePaths::from_home_and_cwd(home.path(), project.path())
            .unwrap_or_else(|err| panic!("resource paths: {err}"));
        std::fs::create_dir_all(&paths.global_dir)
            .unwrap_or_else(|err| panic!("create global dir: {err}"));
        std::fs::create_dir_all(&paths.project_dir)
            .unwrap_or_else(|err| panic!("create project dir: {err}"));

        let mut global = UserSettings::default();
        global.theme.set_active("oino-light");
        user_settings::save_to_path(&global, &paths.global_settings)
            .await
            .unwrap_or_else(|err| panic!("save global settings: {err}"));
        let mut project_settings = UserSettings::default();
        project_settings.theme.set_active("oino-aurora");
        project_settings
            .theme
            .overrides
            .insert("app.bg".into(), "#08111f".into());
        user_settings::save_to_path(&project_settings, &paths.project_settings)
            .await
            .unwrap_or_else(|err| panic!("save project settings: {err}"));

        let snapshot = load_tool_settings(&paths).await;
        assert_eq!(snapshot.global.theme.active.as_deref(), Some("oino-light"));
        assert_eq!(
            snapshot.project.theme.active.as_deref(),
            Some("oino-aurora")
        );
        let catalog = oino_tui::ThemeCatalog::builtins();
        let resolved = oino_tui::resolve_effective_theme(
            &catalog,
            &snapshot.global.theme,
            &snapshot.project.theme,
        );
        assert_eq!(resolved.id, "oino-aurora");
        assert_eq!(
            resolved.selected_scope,
            oino_tui::EffectiveThemeScope::Project
        );
    }

    #[test]
    fn theme_catalog_loads_global_and_project_theme_files() {
        let home = tempfile::tempdir().unwrap_or_else(|err| panic!("home tempdir: {err}"));
        let project = tempfile::tempdir().unwrap_or_else(|err| panic!("project tempdir: {err}"));
        let paths = ResourcePaths::from_home_and_cwd(home.path(), project.path())
            .unwrap_or_else(|err| panic!("resource paths: {err}"));
        fs::create_dir_all(&paths.global_themes_dir)
            .unwrap_or_else(|err| panic!("create global themes: {err}"));
        fs::create_dir_all(&paths.project_themes_dir)
            .unwrap_or_else(|err| panic!("create project themes: {err}"));
        fs::write(
            paths.global_themes_dir.join("team.json"),
            r##"{
              "schema_version": 1,
              "id": "team",
              "display_name": "Global Team",
              "mode": "dark",
              "tokens": { "app.title": "#010203" }
            }"##,
        )
        .unwrap_or_else(|err| panic!("write global theme: {err}"));
        fs::write(
            paths.project_themes_dir.join("team.json"),
            r##"{
              "schema_version": 1,
              "id": "team",
              "display_name": "Project Team",
              "mode": "dark",
              "tokens": { "app.title": "#abcdef" }
            }"##,
        )
        .unwrap_or_else(|err| panic!("write project theme: {err}"));

        let snapshot = load_extension_snapshot(&paths, &ToolSettingsSnapshot::default());
        let catalog = theme_catalog_from_sources(&snapshot, &paths);
        let selected = catalog
            .selected_entry("team")
            .unwrap_or_else(|| panic!("theme file should be catalogued"));
        assert_eq!(selected.source.kind, ThemeSourceKind::File);
        assert_eq!(selected.source.scope, ThemeSourceScope::Project);
        assert_eq!(selected.document.display_name, "Project Team");

        let mut global = oino_tui::ThemeSettings::default();
        global.set_active("team");
        let resolved = oino_tui::resolve_effective_theme(
            &catalog,
            &global,
            &oino_tui::ThemeSettings::default(),
        );
        assert_eq!(resolved.id, "team");
        assert_eq!(resolved.source.scope, ThemeSourceScope::Project);
        assert_eq!(
            resolved.tokens.get("app.title"),
            Some(&ratatui::style::Color::Rgb(0xab, 0xcd, 0xef))
        );
    }

    #[test]
    fn extension_management_toggle_updates_policy_settings() {
        let mut settings = ToolSettingsSnapshot::default();
        set_extension_enabled(
            &mut settings,
            ExtensionManagementTarget::Extension,
            "process.manager".into(),
            ToolSettingsScope::Project,
            false,
        );
        assert_eq!(
            settings.project.extensions.extensions.get(
                &ExtensionId::new("process.manager").unwrap_or_else(|err| panic!("id: {err}"))
            ),
            Some(&PolicyToggle::Disabled)
        );
        set_extension_enabled(
            &mut settings,
            ExtensionManagementTarget::Contribution,
            "ui.processes".into(),
            ToolSettingsScope::Global,
            true,
        );
        assert_eq!(
            settings.global.extensions.contributions.get(
                &ContributionId::new("ui.processes").unwrap_or_else(|err| panic!("id: {err}"))
            ),
            Some(&PolicyToggle::Enabled)
        );

        set_extension_override(
            &mut settings,
            "ui.processes".into(),
            "ui:process.manager:ui.processes:/tmp".into(),
            ToolSettingsScope::Project,
        );
        let contribution_id =
            ContributionId::new("ui.processes").unwrap_or_else(|err| panic!("id: {err}"));
        assert_eq!(
            settings.project.extensions.overrides.get(&contribution_id),
            Some(&RegistryEntryKey::new(
                "ui:process.manager:ui.processes:/tmp"
            ))
        );
        clear_extension_override(
            &mut settings,
            "ui.processes".into(),
            ToolSettingsScope::Project,
        );
        assert!(!settings
            .project
            .extensions
            .overrides
            .contains_key(&contribution_id));
    }

    #[test]
    fn tool_registry_policy_preserves_existing_tool_enablement() {
        let mut settings = ToolSettingsSnapshot::default();
        settings.global.tools.insert("bash".into(), false);
        settings.project.tools.insert("read".into(), false);
        settings
            .project
            .tools
            .insert(oino_tools::SESSION_TITLE_TOOL_NAME.into(), true);

        let policy = tool_registry_policy_from_settings(
            &settings,
            [
                "bash".to_string(),
                "read".to_string(),
                "write".to_string(),
                oino_tools::SESSION_TITLE_TOOL_NAME.to_string(),
            ],
        );

        assert!(policy.disabled_contributions.contains(
            &ContributionId::new("bash").unwrap_or_else(|err| panic!("valid id: {err}"))
        ));
        assert!(policy.disabled_contributions.contains(
            &ContributionId::new("read").unwrap_or_else(|err| panic!("valid id: {err}"))
        ));
        assert!(policy.enabled_contributions.contains(
            &ContributionId::new("write").unwrap_or_else(|err| panic!("valid id: {err}"))
        ));
        assert!(policy.enabled_contributions.contains(
            &ContributionId::new(oino_tools::SESSION_TITLE_TOOL_NAME)
                .unwrap_or_else(|err| panic!("valid id: {err}"))
        ));
    }

    #[test]
    fn extension_resource_description_prefers_frontmatter_description() {
        let id = ContributionId::new("craft-skill")
            .unwrap_or_else(|err| panic!("valid contribution id: {err}"));
        let description = extension_resource_description(
            "---\nname: craft-skill\ndescription: Use when creating Oino skills\n---\n\n# Craft Skill",
            &id,
        );
        assert_eq!(description, "Use when creating Oino skills");

        let fallback = extension_resource_description("# Visible Skill\n\nBody", &id);
        assert_eq!(fallback, "Visible Skill");
    }

    #[tokio::test]
    async fn extension_snapshot_exposes_enabled_project_tools_and_commands() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let extension_dir = project.join(".oino/extensions/acme-visible");
        fs::create_dir_all(&extension_dir)
            .unwrap_or_else(|err| panic!("create extension dir failed: {err}"));
        fs::write(
            extension_dir.join("visible-prompt.md"),
            "# Visible Prompt\n\nPrompt content from extension.",
        )
        .unwrap_or_else(|err| panic!("write prompt failed: {err}"));
        fs::write(
            extension_dir.join("visible-skill.md"),
            "# Visible Skill\n\nSkill content from extension.",
        )
        .unwrap_or_else(|err| panic!("write skill failed: {err}"));
        fs::write(
            extension_dir.join("visible-theme.json"),
            r##"{ "accent": "#abcdef", "panel.bg": "#010203" }"##,
        )
        .unwrap_or_else(|err| panic!("write theme failed: {err}"));
        fs::write(
            extension_dir.join("oino.extension.json"),
            r##"{
              "id": "acme.visible",
              "version": "1.0.0",
              "oino": "^0.1",
              "runtime": { "kind": "wasm", "entry": "plugin.wasm" },
              "permissions": { "tools": ["visible_tool"], "commands": ["visible_command"] },
              "contributes": {
                "tools": [{ "id": "visible_tool", "description": "Visible dogfood tool" }],
                "commands": [{ "id": "visible_command", "description": "Visible dogfood command" }],
                "resources": [
                  { "id": "visible_prompt", "kind": "prompt", "path": "visible-prompt.md" },
                  { "id": "visible_skill", "kind": "skill", "path": "visible-skill.md" }
                ],
                "themes": [
                  { "id": "visible_theme", "path": "visible-theme.json", "tokens": { "accent": "#111111" } }
                ]
              }
            }"##,
        )
        .unwrap_or_else(|err| panic!("write extension failed: {err}"));
        fs::create_dir_all(project.join(".oino"))
            .unwrap_or_else(|err| panic!("create project settings dir failed: {err}"));
        fs::write(
            project.join(".oino/settings.json"),
            r#"{ "extensions": { "extensions": { "acme.visible": "enabled" } } }"#,
        )
        .unwrap_or_else(|err| panic!("write settings failed: {err}"));
        let paths = ResourcePaths::from_home_and_cwd(&home, &project)
            .unwrap_or_else(|err| panic!("paths failed: {err}"));
        let disabled_snapshot = load_extension_snapshot(&paths, &ToolSettingsSnapshot::default());
        assert!(disabled_snapshot.registries.tools.active.is_empty());
        assert!(
            execute_extension_command("/visible_command now", &disabled_snapshot)
                .await
                .is_none()
        );

        let settings = load_tool_settings(&paths).await;
        let snapshot = load_extension_snapshot(&paths, &settings);

        assert!(snapshot
            .registries
            .tools
            .active
            .iter()
            .any(|active| active.effective_id.as_str() == "visible_tool"));
        let tools = extension_tool_map(&snapshot);
        assert!(tools.contains_key("visible_tool"));
        let command = execute_extension_command("/visible_command now", &snapshot)
            .await
            .unwrap_or_else(|| panic!("extension command should be known"))
            .unwrap_or_else(|err| panic!("extension command failed: {err}"));
        assert!(command.contains("visible_command"));
        let (prompts, skills, diagnostics) = extension_resource_items(&snapshot);
        assert!(diagnostics.is_empty());
        assert!(prompts
            .iter()
            .any(|prompt| prompt.name == "visible_prompt"
                && prompt.content.contains("Prompt content")));
        assert!(skills
            .iter()
            .any(|skill| skill.name == "visible_skill" && skill.content.contains("Skill content")));

        let catalog = theme_catalog_from_sources(&snapshot, &paths);
        let theme = catalog
            .selected_entry("visible_theme")
            .unwrap_or_else(|| panic!("missing extension theme"));
        assert_eq!(theme.source.scope, ThemeSourceScope::Project);
        assert_eq!(
            theme.document.tokens.get("accent").map(String::as_str),
            Some("#abcdef")
        );
        assert_eq!(
            theme.document.tokens.get("panel.bg").map(String::as_str),
            Some("#010203")
        );
        let mut project_theme = oino_tui::ThemeSettings::default();
        project_theme.set_active("visible_theme");
        let resolved = oino_tui::resolve_effective_theme(
            &catalog,
            &oino_tui::ThemeSettings::default(),
            &project_theme,
        );
        assert_eq!(resolved.id, "visible-theme");
        assert!(resolved.tokens.contains_key("panel.bg"));
    }

    #[test]
    fn app_config_uses_saved_model_and_thinking_level() {
        let config = AppConfig::from_sources(
            UserSettings {
                model: Some("openrouter:anthropic/claude-3.5-sonnet".into()),
                thinking_level: Some(ThinkingLevel::High),
                thinking_collapse_mode: Some(CollapseMode::Truncate),
                tool_collapse_mode: Some(CollapseMode::Collapse),
                chat_style: Some(oino_tui::ChatStyle::Minimal),
                keymap: None,
                theme: oino_tui::ThemeSettings::default(),
                notify: notify::NotifySettings::default(),
                btw_model: None,
                tools: BTreeMap::new(),
                extensions: oino_extension_core::ExtensionPolicySettings::default(),
                compact: user_settings::CompactSettings::default(),
            },
            None,
            None,
            None,
        );
        assert_eq!(config.model, "openrouter:anthropic/claude-3.5-sonnet");
        assert_eq!(config.thinking_level, ThinkingLevel::High);
        assert_eq!(config.thinking_collapse_mode, CollapseMode::Truncate);
        assert_eq!(config.tool_collapse_mode, CollapseMode::Collapse);
        assert_eq!(config.chat_style, oino_tui::ChatStyle::Minimal);
    }

    #[test]
    fn new_tui_session_is_lazy_until_saved() {
        let temp = match tempfile::tempdir() {
            Ok(temp) => temp,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let (path, session) = new_tui_session(temp.path().to_path_buf(), PathBuf::from("/tmp"));

        assert_eq!(path.parent(), Some(temp.path()));
        assert_eq!(session.get_entries().len(), 0);
        assert!(!path.exists());
    }

    #[test]
    fn export_chat_html_writes_escaped_transcript() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let exports = temp.path().join(".oino/exports");
        let mut state = TuiState::new();
        state.set_session_title("Demo <Chat>");
        state.set_messages_from_oino(&[
            Message::user_text("hello <world>"),
            Message::assistant_text("ok & done", StopReason::EndTurn),
        ]);

        let path =
            export_chat_html(&state, &exports).unwrap_or_else(|err| panic!("export failed: {err}"));
        let html =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("read export failed: {err}"));

        assert_eq!(path.parent(), Some(exports.as_path()));
        assert!(html.contains("Demo &lt;Chat&gt;"));
        assert!(html.contains("hello &lt;world&gt;"));
        assert!(html.contains("ok &amp; done"));
        assert!(!html.contains("hello <world>"));
    }

    #[test]
    fn accepted_steer_materializes_in_tui_transcript() {
        let mut state = TuiState::new();
        state.transcript_scroll.scroll_up(3);
        let message = Message::user_text("steer now");

        materialize_accepted_steer(&mut state, &message);

        assert!(state
            .messages
            .iter()
            .any(|message| message.role == "user" && message.content == "steer now"));
        assert!(state.transcript_scroll.is_at_bottom());
        assert_eq!(state.status, "Steered current response");
    }

    #[tokio::test]
    async fn harness_wiring_works_with_fake_stream() {
        let auth = AuthStorage::new(
            AuthConfig::new(std::env::temp_dir().join("oino-app-auth.json"))
                .with_runtime_override("openrouter", "sk-test")
                .with_process_env(false),
        );
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::TextDelta { delta: "ok".into() },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            },
        ])) as Arc<dyn StreamProvider>;
        let session = SessionManager::new(SessionHeader::new("test", PathBuf::from("/tmp")));
        let temp = match tempfile::tempdir() {
            Ok(temp) => temp,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let resource_paths =
            match ResourcePaths::from_home_and_cwd(temp.path().join("home"), temp.path()) {
                Ok(paths) => paths,
                Err(err) => panic!("resource paths failed: {err}"),
            };
        if let Err(err) = resource_paths.ensure_skeleton() {
            panic!("resource skeleton failed: {err}");
        }
        let resource_catalog = resource_paths.load_catalog();
        let harness = match build_harness(
            "openrouter:test/model".into(),
            ThinkingLevel::Off,
            stream,
            auth,
            session,
            &resource_catalog,
        ) {
            Ok(harness) => harness,
            Err(err) => panic!("harness build failed: {err}"),
        };
        let system_prompt = harness.get_system_prompt().await.unwrap_or_default();
        assert!(system_prompt.contains("You are Oino"));
        assert!(system_prompt.contains("## Karpathy Guidelines"));
        assert!(system_prompt.contains("Simplicity First"));
        assert!(system_prompt.contains("Surgical Changes"));
        assert!(system_prompt.contains("Oino Resource Inclusion Policy"));
        assert!(!system_prompt.contains("Claude"));
        assert!(!system_prompt.contains("Anthropic"));
        assert!(!system_prompt.contains("<available_skills>"));
        let messages = match harness.prompt(Message::user_text("hi")).await {
            Ok(messages) => messages,
            Err(err) => panic!("prompt failed: {err}"),
        };
        assert!(messages
            .iter()
            .any(|message| matches!(message, Message::Assistant { .. })));
    }

    #[test]
    fn cli_resource_references_expand_multiple_resources() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let paths = ResourcePaths::from_home_and_cwd(temp.path().join("home"), temp.path())
            .unwrap_or_else(|err| panic!("resource paths failed: {err}"));
        let catalog = ResourceCatalog {
            paths,
            system_prompt: None,
            project_instructions: None,
            prompts: vec![PromptTemplate {
                name: "review".into(),
                description: "Review".into(),
                argument_hint: None,
                path: temp.path().join("review.md"),
                scope: oino_resource::ResourceScope::Project,
                content: "Review $ARGUMENTS".into(),
            }],
            skills: vec![Skill {
                name: "debug".into(),
                description: "Debug".into(),
                path: temp.path().join("debug/SKILL.md"),
                base_dir: temp.path().join("debug"),
                scope: oino_resource::ResourceScope::Project,
                content: "# Debug Skill".into(),
                disable_model_invocation: false,
            }],
            diagnostics: Vec::new(),
        };

        assert!(!contains_resource_reference("use /P:debug only for search"));
        let prompt = expand_resource_references(
            "fix crash /skill:debug /prompt:review /skill:debug",
            &catalog,
        )
        .unwrap_or_else(|err| panic!("resource expansion failed: {err}"));
        assert!(prompt.contains("Review fix crash"));
        assert!(prompt.contains("# Included Skills"));
        assert!(prompt.contains("## Included Skill: `debug`"));
        assert!(prompt.contains("````markdown\n# Debug Skill\n````"));
        assert!(prompt.contains("# User Request\n\nfix crash"));
        assert!(!prompt.contains("<skill"));
        assert_eq!(prompt.matches("## Included Skill: `debug`").count(), 1);
    }
}
