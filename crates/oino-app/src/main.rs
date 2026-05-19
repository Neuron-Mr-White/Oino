#![forbid(unsafe_code)]

mod model_catalog;
mod user_settings;

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
use model_catalog::ModelCatalogUpdate;
use oino_agent_loop::{AgentEvent, BoxFuture, LoopError, StreamProvider, Tool};
use oino_auth::{AuthError, AuthStorage, ProviderAuthSpec};
use oino_harness::{AuthResolver, Harness, HarnessConfig, HarnessError, NotificationHook};
use oino_provider_openrouter::{OpenRouterConfig, OpenRouterProvider};
use oino_resource::{PromptTemplate, ResourceCatalog, ResourcePaths, Skill};
use oino_session::{SessionHeader, SessionManager, SessionRepository};
use oino_tui::{
    collapse_mode_value, collapse_target_value, parse_command, render, terminal_cursor_position,
    transcript_click_targets, transcript_url_overlays, transcript_visible_lines, CollapseMode,
    KeymapConfig, MessageView, ParsedCommand, PromptResource, SessionListItem, SettingsCommand,
    SkillResource, TerminalClickTarget, TerminalUrlOverlay, ToolSettingsItem, ToolSettingsScope,
    TuiAction, TuiState, HELP_STATUS,
};
use oino_types::{ContentBlock, Message, Model, OinoId, ThinkingLevel};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Stdout, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::mpsc;
use user_settings::UserSettings;

const DEFAULT_OPENROUTER_MODEL: &str = "openrouter:openai/gpt-4o-mini";
const MISSING_OPENROUTER_API_KEY_MESSAGE: &str =
    "Missing OpenRouter API key. Set OPENROUTER_API_KEY or add ~/.oino/auth.json.";

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
            parsed.input = Some(input_parts.join(" "));
        }
        if parsed.model.is_some() {
            parsed.settings = true;
        }
        Ok(parsed)
    }
}

fn usage() -> &'static str {
    "Usage:\n  oino\n  oino --settings --model openrouter:xai/glm-5.1\n  oino --session <uuid> <message-or-command>\n\nCommands:\n  /new\n  /sessions\n  /settings\n  /prompts\n  /skills\n  /reload\n  /inspect\n  /prompt:<name>\n  /skill:<name>\n  /model [provider:model-id]\n  /thinking [off|minimal|low|medium|high|xhigh]\n  /title <session-title>\n  /settings model <provider:model-id>\n  /settings thinking <off|minimal|low|medium|high|xhigh>\n  /settings collapse <thinking|tool> <full|truncate|collapse>\n  /settings chat-style <chat|agentic|minimal>\n  /settings tools\n  /settings keymaps"
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
                .unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.into()),
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
    let provider = Arc::new(OpenRouterProvider::new(
        auth.clone(),
        provider_config.clone(),
    )?) as Arc<dyn StreamProvider>;
    let harness = build_harness(
        config.model.clone(),
        config.thinking_level,
        provider,
        auth.clone(),
        session,
        &resource_catalog,
    )?;
    let tool_settings = load_tool_settings(&resource_paths).await;
    apply_tool_settings_to_harness(&harness, &tool_settings, &cwd).await;

    if cli.settings || cli.input.is_some() {
        return run_non_interactive(cli, harness, auth, config, session_path, resource_catalog)
            .await;
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
                let spec = if provider == oino_auth::OPENROUTER_PROVIDER_ID {
                    ProviderAuthSpec::openrouter()
                } else {
                    ProviderAuthSpec::new(
                        provider.clone(),
                        provider.clone(),
                        format!("{}_API_KEY", provider.to_uppercase()),
                    )
                };
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

async fn apply_tool_settings_to_harness(
    harness: &Harness,
    settings: &ToolSettingsSnapshot,
    cwd: &Path,
) {
    let mut available = oino_tools::default_tools(harness.env(), cwd.to_path_buf());
    available.insert(
        oino_tools::SESSION_TITLE_TOOL_NAME.into(),
        oino_tools::session_title_tool(harness.session_title_setter()),
    );
    let tools = available
        .into_iter()
        .filter(|(name, _)| project_tool_enabled(settings, name))
        .collect::<BTreeMap<String, Arc<dyn Tool>>>();
    harness.set_tools(tools).await;
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
    apply_tool_settings_to_harness(&harness, &tool_settings, &cwd).await;
    let mut state = TuiState::with_settings(initial_model, initial_thinking_level);
    state.set_session_title(harness.session_title().await);
    state.set_tool_settings(tool_settings_items(&tool_settings));
    apply_resource_catalog_to_state(&mut state, &resource_catalog);
    state.set_file_paths(scan_project_files(&cwd));
    state
        .settings
        .set_collapse_modes(initial_thinking_collapse_mode, initial_tool_collapse_mode);
    state.settings.set_chat_style(initial_chat_style);
    state.set_keymap(initial_keymap);
    if let Ok(messages) = harness.build_context().await {
        state.set_messages_from_oino(&messages);
    }
    if open_settings {
        state.open_settings();
    }
    let mut applied_thinking_level = initial_thinking_level;
    let harness = Arc::new(harness);
    let (tx, mut rx) = mpsc::unbounded_channel();
    register_tui_stream_hooks(&harness, tx.clone()).await;
    spawn_model_catalog_task(tx.clone(), provider_config);
    let mut prompt_in_flight = false;
    loop {
        let mut prompt_finished = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, TuiRuntimeEvent::PromptFinished(_)) {
                prompt_finished = true;
            }
            apply_tui_runtime_event(&mut state, event, &mut prompt_in_flight);
        }
        if prompt_finished {
            start_next_queued_prompt_if_idle(
                &mut state,
                &auth,
                &harness,
                &tx,
                &session_path,
                &mut prompt_in_flight,
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
                if let Err(err) = harness.set_model(parsed_model).await {
                    state.set_error(err.to_string());
                } else if let Err(err) = harness.set_thinking_level(thinking_level).await {
                    state.set_error(err.to_string());
                } else {
                    applied_thinking_level = thinking_level;
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
                apply_tool_settings_to_harness(&harness, &tool_settings, &cwd).await;
                state.set_tool_settings(tool_settings_items(&tool_settings));
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
                reload_tui_resources(&mut state, &harness, &resource_paths, &cwd).await;
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

fn apply_resource_catalog_to_state(state: &mut TuiState, catalog: &ResourceCatalog) {
    let prompts = catalog
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
    let skills = catalog
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
    let diagnostics = catalog
        .diagnostics
        .iter()
        .map(oino_resource::ResourceDiagnostic::message)
        .collect::<Vec<_>>();
    state.set_resources(prompts, skills, diagnostics);
}

async fn reload_tui_resources(
    state: &mut TuiState,
    harness: &Harness,
    resource_paths: &ResourcePaths,
    cwd: &Path,
) {
    match load_resource_catalog(resource_paths) {
        Ok(catalog) => {
            harness
                .set_system_prompt(Some(default_system_prompt(cwd, &catalog)))
                .await;
            apply_resource_catalog_to_state(state, &catalog);
            if let Some(summary) = catalog.diagnostics_summary() {
                state.set_error(format!("Resource warnings: {summary}"));
            }
        }
        Err(err) => state.set_error(format!("Resource reload failed: {err}")),
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

async fn start_next_queued_prompt_if_idle(
    state: &mut TuiState,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    prompt_in_flight: &mut bool,
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
        prompt,
    )
    .await
    {
        let _ = state.pop_next_queued_prompt();
    }
}

async fn start_prompt(
    state: &mut TuiState,
    auth: &AuthStorage,
    harness: &Arc<Harness>,
    tx: &mpsc::UnboundedSender<TuiRuntimeEvent>,
    session_path: &Path,
    prompt_in_flight: &mut bool,
    prompt: String,
) -> bool {
    if *prompt_in_flight {
        state.status =
            "A prompt is already running. Use Enter to steer or Ctrl-O q then q to queue.".into();
        return false;
    }
    if let Err(message) = preflight_openrouter_credentials(auth).await {
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
        let _ = task_tx.send(TuiRuntimeEvent::PromptFinished(result));
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

async fn run_non_interactive(
    cli: CliArgs,
    harness: Harness,
    auth: AuthStorage,
    mut config: AppConfig,
    session_path: PathBuf,
    resource_catalog: ResourceCatalog,
) -> Result<(), AppError> {
    if let Some(model) = cli.model.clone() {
        let command =
            ParsedCommand::Settings(SettingsCommand::SetModel(ensure_model_identifier(&model)?));
        let message = execute_runtime_command(
            command,
            &harness,
            &mut config,
            &session_path,
            &resource_catalog,
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
        let command = parse_command(&input)
            .ok_or_else(|| AppError::InvalidArguments(format!("unknown command `{input}`")))?;
        let message = execute_runtime_command(
            command,
            &harness,
            &mut config,
            &session_path,
            &resource_catalog,
        )
        .await?;
        println!("{message}");
        return Ok(());
    }

    let input = expand_resource_references(&input, &resource_catalog)?;
    preflight_openrouter_credentials(&auth)
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

async fn execute_runtime_command(
    command: ParsedCommand,
    harness: &Harness,
    config: &mut AppConfig,
    session_path: &std::path::Path,
    resource_catalog: &ResourceCatalog,
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
            return Ok(format!(
                "Reloaded {} prompts and {} skills",
                catalog.prompts.len(),
                catalog.skills.len()
            ));
        }
        ParsedCommand::Settings(
            SettingsCommand::Open
            | SettingsCommand::OpenModelSelection
            | SettingsCommand::OpenThinkingLevel
            | SettingsCommand::OpenChatStyle
            | SettingsCommand::OpenTools
            | SettingsCommand::OpenKeymaps,
        ) => {
            return Err(AppError::InvalidArguments(
                "interactive settings pages cannot be opened in non-interactive mode; provide a setting path such as `/model openrouter:xai/glm-5.1` or `/thinking high`".into(),
            ));
        }
        ParsedCommand::Settings(SettingsCommand::SetModel(model)) => {
            let identifier = model.identifier();
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

fn last_assistant_text(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        let Message::Assistant { content, .. } = message else {
            return None;
        };
        let text = content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    })
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

#[derive(Debug)]
enum TuiRuntimeEvent {
    Agent(AgentEvent),
    PromptFinished(Result<Vec<Message>, String>),
    SessionTitle(String),
    ModelCatalog(ModelCatalogUpdate),
}

async fn persist_current_settings(state: &mut TuiState) {
    let mut settings = UserSettings::from_current(
        state.settings.selected_model.clone(),
        state.settings.selected_thinking_level,
        state.settings.thinking_collapse_mode,
        state.settings.tool_collapse_mode,
        state.settings.chat_style,
    )
    .with_tools(tool_map_from_state(state, ToolSettingsScope::Global));
    settings.keymap = Some(state.settings.keymap.clone());
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

fn spawn_model_catalog_task(
    tx: mpsc::UnboundedSender<TuiRuntimeEvent>,
    provider_config: OpenRouterConfig,
) {
    tokio::spawn(async move {
        if let Some(update) = model_catalog::load_cached_update().await {
            let _ = tx.send(TuiRuntimeEvent::ModelCatalog(update));
        }

        let fresh = model_catalog::cached_is_fresh().await;
        let initial_delay = if fresh {
            model_catalog::MODEL_REFRESH_INTERVAL
        } else {
            Duration::ZERO
        };
        tokio::time::sleep(initial_delay).await;

        loop {
            let _ = tx.send(TuiRuntimeEvent::ModelCatalog(ModelCatalogUpdate {
                models: Vec::new(),
                status: "Refreshing OpenRouter models…".into(),
                refreshing: true,
            }));
            let update = model_catalog::refresh_update(&provider_config).await;
            let _ = tx.send(TuiRuntimeEvent::ModelCatalog(update));
            tokio::time::sleep(model_catalog::MODEL_REFRESH_INTERVAL).await;
        }
    });
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
                state.set_model_catalog(update.models, update.status);
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
    }
}

async fn preflight_openrouter_credentials(auth: &AuthStorage) -> Result<(), String> {
    match auth.resolve_openrouter_api_key().await {
        Ok(_) => Ok(()),
        Err(AuthError::MissingCredential { .. }) => Err(MISSING_OPENROUTER_API_KEY_MESSAGE.into()),
        Err(err) => Err(err.to_string()),
    }
}

fn user_facing_error(err: &HarnessError) -> String {
    let message = err.to_string();
    if message.contains("missing credential") || message.contains("OPENROUTER_API_KEY") {
        MISSING_OPENROUTER_API_KEY_MESSAGE.into()
    } else {
        message
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
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

impl TerminalGuard {
    fn enter() -> Result<Self, AppError> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            Clear(ClearType::All),
            MoveTo(0, 0),
            EnableBracketedPaste,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
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
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            DisableMouseCapture,
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
    fn default_model_is_openrouter_model() {
        assert_eq!(DEFAULT_OPENROUTER_MODEL, "openrouter:openai/gpt-4o-mini");
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
                tools: BTreeMap::new(),
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
        assert!(system_prompt.contains("## Available tools"));
        assert!(system_prompt.contains("read"));
        assert!(system_prompt.contains("bash"));
        assert!(system_prompt.contains("edit"));
        assert!(system_prompt.contains("write"));
        assert!(system_prompt.contains("Oino Resource Inclusion Policy"));
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

    #[tokio::test]
    async fn preflight_reports_missing_openrouter_key_as_tui_message() {
        let auth = AuthStorage::new(
            AuthConfig::new(std::env::temp_dir().join("oino-app-preflight-missing-auth.json"))
                .with_process_env(false),
        );
        let result = preflight_openrouter_credentials(&auth).await;
        match result {
            Err(message) => assert_eq!(message, MISSING_OPENROUTER_API_KEY_MESSAGE),
            Ok(()) => panic!("expected missing credential message"),
        }
    }

    #[tokio::test]
    async fn preflight_accepts_runtime_openrouter_key() {
        let auth = AuthStorage::new(
            AuthConfig::new(std::env::temp_dir().join("oino-app-preflight-auth.json"))
                .with_runtime_override("openrouter", "sk-test")
                .with_process_env(false),
        );
        if let Err(message) = preflight_openrouter_credentials(&auth).await {
            panic!("preflight should accept runtime credential: {message}");
        }
    }

    #[tokio::test]
    async fn auth_resolver_returns_none_for_missing_credentials() {
        let auth = AuthStorage::new(
            AuthConfig::new(std::env::temp_dir().join("oino-app-missing-auth.json"))
                .with_process_env(false),
        );
        let resolver = build_auth_resolver(auth);
        let result = resolver("openrouter".into()).await;
        match result {
            Ok(None) => {}
            other => panic!("expected none, got {other:?}"),
        }
    }
}
