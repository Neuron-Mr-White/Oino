#![forbid(unsafe_code)]

mod model_catalog;
mod user_settings;

use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use model_catalog::ModelCatalogUpdate;
use oino_agent_loop::{AgentEvent, BoxFuture, LoopError, StreamProvider};
use oino_auth::{AuthError, AuthStorage, ProviderAuthSpec};
use oino_harness::{AuthResolver, Harness, HarnessConfig, HarnessError, NotificationHook};
use oino_provider_openrouter::{OpenRouterConfig, OpenRouterProvider};
use oino_session::{SessionManager, SessionRepository};
use oino_tui::{
    collapse_mode_value, collapse_target_value, parse_command, render, transcript_visible_lines,
    CollapseMode, ParsedCommand, SettingsCommand, TuiAction, TuiState, HELP_STATUS,
};
use oino_types::{ContentBlock, Message, Model, OinoId, ThinkingLevel};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, Stdout},
    path::PathBuf,
    sync::Arc,
    time::Duration,
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
    "Usage:\n  oino\n  oino --settings --model openrouter:xai/glm-5.1\n  oino --session <uuid> <message-or-command>\n\nCommands:\n  /settings\n  /model [provider:model-id]\n  /thinking [off|minimal|low|medium|high|xhigh]\n  /settings model <provider:model-id>\n  /settings thinking <off|minimal|low|medium|high|xhigh>\n  /settings collapse <thinking|tool> <full|truncate|collapse>\n  /settings chat-style <chat|agentic|minimal>"
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
    provider_config: OpenRouterConfig,
    session_path: PathBuf,
    open_settings: bool,
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
    let repository = SessionRepository::new(default_session_root()?);
    if let Some(session_id) = session_id {
        let path = default_session_root()?.join(format!("{session_id}.jsonl"));
        let session = repository.open(&path).await?;
        return Ok((path, session));
    }
    Ok(repository.create("oino", cwd).await?)
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
    )?;

    if cli.settings || cli.input.is_some() {
        return run_non_interactive(cli, harness, auth, config, session_path).await;
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
            provider_config,
            session_path,
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
) -> Result<Harness, AppError> {
    let cwd = session.header().cwd.clone();
    let model = Model::from_identifier(&model_identifier)
        .ok_or_else(|| AppError::InvalidModelIdentifier(model_identifier.clone()))?;
    let mut config = HarnessConfig::new(model, provider, session);
    config.tools = oino_tools::default_tools(Arc::clone(&config.env), cwd.clone());
    config.system_prompt = Some(default_system_prompt(&cwd));
    config.thinking_level = thinking_level;
    config.auth_resolver = Some(build_auth_resolver(auth));
    Ok(Harness::new(config))
}

fn default_system_prompt(cwd: &std::path::Path) -> String {
    format!(
        "You are an expert coding assistant operating inside Oino, a terminal coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.\n\nAvailable tools:\n- read: Read file contents\n- bash: Execute bash commands (ls, rg, find, etc.)\n- edit: Make precise file edits with exact text replacement\n- write: Create or overwrite files\n\nGuidelines:\n- Use bash for file operations like ls, rg, find.\n- Use read to examine files instead of cat or sed.\n- Use edit for precise changes; oldText must match exactly and uniquely.\n- Use write only for new files or complete rewrites.\n- Be concise in your responses.\n- Show file paths clearly when working with files.\n\nCurrent working directory: {}",
        cwd.display()
    )
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
        provider_config,
        session_path,
        open_settings,
    } = launch;
    let mut terminal = TerminalGuard::enter()?;
    let mut state = TuiState::with_settings(initial_model, initial_thinking_level);
    state
        .settings
        .set_collapse_modes(initial_thinking_collapse_mode, initial_tool_collapse_mode);
    state.settings.set_chat_style(initial_chat_style);
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
        while let Ok(event) = rx.try_recv() {
            apply_tui_runtime_event(&mut state, event, &mut prompt_in_flight);
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
            Event::Paste(text) => state.handle_paste(&text),
            _ => continue,
        };
        match action {
            TuiAction::None => {}
            TuiAction::Quit => break,
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
            TuiAction::SetCollapseMode(_, _) | TuiAction::SetChatStyle(_) => {
                persist_current_settings(&mut state).await;
                save_tui_session(&mut state, &harness, &session_path).await;
            }
            TuiAction::SubmitPrompt(prompt) => {
                if prompt_in_flight {
                    state.set_error("A prompt is already running.");
                    state.status = "● Generating… input paused".into();
                    continue;
                }
                if let Err(message) = preflight_openrouter_credentials(&auth).await {
                    state.set_error(message);
                    state.status = HELP_STATUS.into();
                    continue;
                }
                state.set_working(true);
                prompt_in_flight = true;
                let prompt_message = Message::user_text(prompt);
                let task_harness = Arc::clone(&harness);
                let task_tx = tx.clone();
                let task_session_path = session_path.clone();
                tokio::spawn(async move {
                    let result = match task_harness.prompt(prompt_message).await {
                        Ok(messages) => {
                            match task_harness.save_session_jsonl(&task_session_path).await {
                                Ok(()) => Ok(messages),
                                Err(err) => Err(err.to_string()),
                            }
                        }
                        Err(err) => Err(user_facing_error(&err)),
                    };
                    let _ = task_tx.send(TuiRuntimeEvent::PromptFinished(result));
                });
            }
        }
    }
    Ok(())
}

async fn run_non_interactive(
    cli: CliArgs,
    harness: Harness,
    auth: AuthStorage,
    mut config: AppConfig,
    session_path: PathBuf,
) -> Result<(), AppError> {
    if let Some(model) = cli.model.clone() {
        let command =
            ParsedCommand::Settings(SettingsCommand::SetModel(ensure_model_identifier(&model)?));
        let message =
            execute_runtime_command(command, &harness, &mut config, &session_path).await?;
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
                provider_config: OpenRouterConfig {
                    referer: config.referer,
                    title: config.title,
                    ..OpenRouterConfig::default()
                },
                session_path,
                open_settings: true,
            },
        )
        .await?;
        return Ok(());
    }

    let Some(input) = cli.input else {
        return Ok(());
    };

    if input.trim_start().starts_with('/') {
        let command = parse_command(&input)
            .ok_or_else(|| AppError::InvalidArguments(format!("unknown command `{input}`")))?;
        let message =
            execute_runtime_command(command, &harness, &mut config, &session_path).await?;
        println!("{message}");
        return Ok(());
    }

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

async fn execute_runtime_command(
    command: ParsedCommand,
    harness: &Harness,
    config: &mut AppConfig,
    session_path: &std::path::Path,
) -> Result<String, AppError> {
    let message = match command {
        ParsedCommand::Settings(
            SettingsCommand::Open
            | SettingsCommand::OpenModelSelection
            | SettingsCommand::OpenThinkingLevel
            | SettingsCommand::OpenChatStyle,
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
    UserSettings::from_current(
        config.model.clone(),
        config.thinking_level,
        config.thinking_collapse_mode,
        config.tool_collapse_mode,
        config.chat_style,
    )
    .save_default()
    .await?;
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

#[derive(Debug)]
enum TuiRuntimeEvent {
    Agent(AgentEvent),
    PromptFinished(Result<Vec<Message>, String>),
    ModelCatalog(ModelCatalogUpdate),
}

async fn persist_current_settings(state: &mut TuiState) {
    let settings = UserSettings::from_current(
        state.settings.selected_model.clone(),
        state.settings.selected_thinking_level,
        state.settings.thinking_collapse_mode,
        state.settings.tool_collapse_mode,
        state.settings.chat_style,
    );
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
        NotificationHook::MessageStart,
        NotificationHook::MessageUpdate,
        NotificationHook::MessageEnd,
        NotificationHook::AgentEnd,
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

fn apply_tui_runtime_event(
    state: &mut TuiState,
    event: TuiRuntimeEvent,
    prompt_in_flight: &mut bool,
) {
    match event {
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
        TuiRuntimeEvent::Agent(AgentEvent::Settled) => {
            state.status = HELP_STATUS.into();
        }
        TuiRuntimeEvent::Agent(AgentEvent::AgentEnd { .. }) => {
            state.status = "Saving…".into();
        }
        TuiRuntimeEvent::Agent(_) => {}
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

impl TerminalGuard {
    fn enter() -> Result<Self, AppError> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, state: &TuiState) -> Result<(), AppError> {
        self.terminal.draw(|frame| render(frame, state))?;
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
    fn app_config_uses_saved_model_and_thinking_level() {
        let config = AppConfig::from_sources(
            UserSettings {
                model: Some("openrouter:anthropic/claude-3.5-sonnet".into()),
                thinking_level: Some(ThinkingLevel::High),
                thinking_collapse_mode: Some(CollapseMode::Truncate),
                tool_collapse_mode: Some(CollapseMode::Collapse),
                chat_style: Some(oino_tui::ChatStyle::Minimal),
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
        let harness = match build_harness(
            "openrouter:test/model".into(),
            ThinkingLevel::Off,
            stream,
            auth,
            session,
        ) {
            Ok(harness) => harness,
            Err(err) => panic!("harness build failed: {err}"),
        };
        let system_prompt = harness.get_system_prompt().await.unwrap_or_default();
        assert!(system_prompt.contains("Available tools:"));
        assert!(system_prompt.contains("read"));
        assert!(system_prompt.contains("bash"));
        assert!(system_prompt.contains("edit"));
        assert!(system_prompt.contains("write"));
        let messages = match harness.prompt(Message::user_text("hi")).await {
            Ok(messages) => messages,
            Err(err) => panic!("prompt failed: {err}"),
        };
        assert!(messages
            .iter()
            .any(|message| matches!(message, Message::Assistant { .. })));
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
