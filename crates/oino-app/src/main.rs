#![forbid(unsafe_code)]

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use oino_agent_loop::{BoxFuture, LoopError, StreamProvider};
use oino_auth::{AuthConfig, AuthError, AuthStorage, ProviderAuthSpec};
use oino_harness::{AuthResolver, Harness, HarnessConfig, HarnessError};
use oino_provider_openrouter::{OpenRouterConfig, OpenRouterProvider};
use oino_session::{SessionHeader, SessionManager};
use oino_tui::{render, TuiAction, TuiState};
use oino_types::{Message, Model};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io::{self, Stdout}, path::PathBuf, sync::Arc, time::Duration};
use thiserror::Error;

const DEFAULT_OPENROUTER_MODEL: &str = "openai/gpt-4o-mini";

#[derive(Debug, Error)]
enum AppError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Provider(#[from] oino_provider_openrouter::OpenRouterError),
    #[error(transparent)]
    Harness(#[from] HarnessError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppConfig {
    model: String,
    referer: Option<String>,
    title: Option<String>,
}

impl AppConfig {
    fn from_env() -> Self {
        Self {
            model: std::env::var("OINO_MODEL").unwrap_or_else(|_| DEFAULT_OPENROUTER_MODEL.into()),
            referer: non_empty_env("OINO_OPENROUTER_REFERER"),
            title: non_empty_env("OINO_OPENROUTER_TITLE"),
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        Ok(_) | Err(_) => None,
    }
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let config = AppConfig::from_env();
    let auth = AuthStorage::default_file()?;
    let provider_config = OpenRouterConfig {
        referer: config.referer.clone(),
        title: config.title.clone(),
        ..OpenRouterConfig::default()
    };
    let provider = Arc::new(OpenRouterProvider::new(auth.clone(), provider_config)?) as Arc<dyn StreamProvider>;
    let harness = build_harness(config.model, provider, auth)?;
    run_tui(harness).await
}

fn build_auth_resolver(auth: AuthStorage) -> AuthResolver {
    Arc::new(move |provider: String| {
        let auth = auth.clone();
        let fut: BoxFuture<'static, oino_agent_loop::LoopResult<Option<String>>> = Box::pin(async move {
            let spec = if provider == oino_auth::OPENROUTER_PROVIDER_ID {
                ProviderAuthSpec::openrouter()
            } else {
                ProviderAuthSpec::new(provider.clone(), provider.clone(), format!("{}_API_KEY", provider.to_uppercase()))
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
    model_name: String,
    provider: Arc<dyn StreamProvider>,
    auth: AuthStorage,
) -> Result<Harness, AppError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let session = SessionManager::new(SessionHeader::new("oino", cwd));
    let mut config = HarnessConfig::new(Model::new("openrouter", model_name), provider, session);
    config.auth_resolver = Some(build_auth_resolver(auth));
    Ok(Harness::new(config))
}

async fn run_tui(harness: Harness) -> Result<(), AppError> {
    let mut terminal = TerminalGuard::enter()?;
    let mut state = TuiState::new();
    loop {
        terminal.draw(&state)?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match state.handle_key(key) {
            TuiAction::None => {}
            TuiAction::Quit => break,
            TuiAction::SubmitPrompt(prompt) => {
                state.set_working(true);
                terminal.draw(&state)?;
                match harness.prompt(Message::user_text(prompt)).await {
                    Ok(messages) => {
                        state.set_messages_from_oino(&messages);
                        state.set_working(false);
                    }
                    Err(err) => {
                        state.set_error(user_facing_error(&err));
                        state.status = "Enter send • Esc/Ctrl-C quit".into();
                    }
                }
            }
        }
    }
    Ok(())
}

fn user_facing_error(err: &HarnessError) -> String {
    let message = err.to_string();
    if message.contains("missing credential") || message.contains("OPENROUTER_API_KEY") {
        "Missing OpenRouter API key. Set OPENROUTER_API_KEY or add ~/.oino/auth.json.".into()
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
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, state: &TuiState) -> Result<(), AppError> {
        self.terminal.draw(|frame| render(frame, state))?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::FauxStream;
    use oino_types::{AssistantStreamEvent, StopReason};

    #[test]
    fn default_model_is_openrouter_model() {
        assert_eq!(DEFAULT_OPENROUTER_MODEL, "openai/gpt-4o-mini");
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
        let harness = match build_harness("test/model".into(), stream, auth) {
            Ok(harness) => harness,
            Err(err) => panic!("harness build failed: {err}"),
        };
        let messages = match harness.prompt(Message::user_text("hi")).await {
            Ok(messages) => messages,
            Err(err) => panic!("prompt failed: {err}"),
        };
        assert!(messages.iter().any(|message| matches!(message, Message::Assistant { .. })));
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
