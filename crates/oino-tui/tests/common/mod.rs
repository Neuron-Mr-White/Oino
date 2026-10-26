//! Shared helpers: a deterministic host, a scripted backend pump and frame capture.

#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unreadable_literal,
    reason = "test helpers; each test binary uses a subset"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use oino_ai::{ImageContent, JsonObject, JsonValue};
use oino_client::SessionBackend;
use oino_client::fake::FakeBackend;
use oino_core::events::ClientEvent;
use oino_tui::host::{KeybindingOverrides, RenderRequest};
use oino_tui::{App, AppEvent, Effect, TuiHost, TuiOptions};
use oino_view::keys::KeyId;
use oino_view::sessions::{Scope, SessionItem};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

/// Host with fixed paths, in-memory settings and scripted sessions.
#[derive(Default)]
pub struct TestHost {
    pub settings: Mutex<JsonObject>,
    pub keybindings: KeybindingOverrides,
    pub sessions: Vec<SessionItem>,
    pub renders: Mutex<Vec<RenderRequest>>,
    pub render_reply: Option<JsonValue>,
}

impl TuiHost for TestHost {
    fn cwd(&self) -> PathBuf {
        PathBuf::from("/work/project")
    }
    fn home(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/home/u"))
    }
    fn keybindings(&self) -> KeybindingOverrides {
        self.keybindings.clone()
    }
    fn list_sessions(&self, _scope: Scope) -> Result<Vec<SessionItem>, String> {
        Ok(self.sessions.clone())
    }
    fn settings(&self) -> JsonObject {
        self.settings.lock().unwrap().clone()
    }
    fn set_setting(&self, path: &[&str], value: Option<JsonValue>) -> Result<(), String> {
        let mut settings = self.settings.lock().unwrap();
        let key = path.join(".");
        match value {
            Some(v) => settings.insert(key, v),
            None => settings.remove(&key),
        };
        Ok(())
    }
    fn complete_path(&self, prefix: &str) -> Vec<String> {
        ["src/", "src/main.rs", "Cargo.toml"]
            .iter()
            .filter(|p| p.starts_with(prefix))
            .map(|p| (*p).to_owned())
            .collect()
    }
    fn clipboard_image(&self) -> Option<ImageContent> {
        None
    }
    fn git_branch(&self) -> Option<String> {
        Some("main".into())
    }
    fn render(&self, request: &RenderRequest) -> Option<JsonValue> {
        self.renders.lock().unwrap().push(request.clone());
        self.render_reply.clone()
    }
}

pub fn model() -> JsonValue {
    json!({"id": "faux-1", "name": "Faux One", "api": "faux", "provider": "faux",
           "reasoning": true, "contextWindow": 200000, "maxTokens": 8192})
}

/// A fake backend answering the startup commands.
pub fn backend() -> FakeBackend {
    let fake = FakeBackend::new("s-1");
    fake.reply(
        "get_state",
        Ok(Some(json!({
            "model": model(), "thinkingLevel": "medium", "isStreaming": false, "isCompacting": false,
            "steeringMode": "one-at-a-time", "followUpMode": "one-at-a-time",
            "sessionFile": "/home/u/.oino/sessions/--work-project--/s-1.jsonl", "sessionId": "s-1",
            "autoCompactionEnabled": true, "messageCount": 0, "pendingMessageCount": 0
        }))),
    );
    fake.reply(
        "get_entries",
        Ok(Some(json!({"entries": [], "leafId": null}))),
    );
    fake.reply("get_commands", Ok(Some(json!({"commands": [
        {"name": "review", "description": "Review the diff", "source": "prompt", "sourceInfo": {}}
    ]}))));
    fake.reply(
        "get_available_models",
        Ok(Some(json!({"models": [model(), {"id": "faux-2", "name": "Faux Two", "api": "faux", "provider": "other", "contextWindow": 100000}]}))),
    );
    fake.reply(
        "get_available_thinking_levels",
        Ok(Some(json!({"levels": ["off", "low", "medium", "high"]}))),
    );
    fake.reply(
        "get_session_stats",
        Ok(Some(json!({"sessionId": "s-1", "userMessages": 1, "assistantMessages": 1, "toolCalls": 1, "toolResults": 1,
            "totalMessages": 4, "tokens": {"input": 1200, "output": 345, "cacheRead": 0, "cacheWrite": 0, "total": 1545}, "cost": 0.0123}))),
    );
    fake.reply(
        "_oino/get_context_usage",
        Ok(Some(
            json!({"tokens": 1545, "contextWindow": 200000, "percent": 0.8}),
        )),
    );
    fake.reply("_oino/ui_snapshot", Ok(Some(json!({"requests": []}))));
    fake
}

/// The app wired to a fake backend; effects are executed synchronously.
pub struct Harness {
    pub app: App,
    pub fake: FakeBackend,
    pub host: Arc<TestHost>,
    pub effects: Vec<Effect>,
}

impl Harness {
    pub async fn new(host: TestHost) -> Self {
        let host = Arc::new(host);
        let app = App::new(
            host.clone(),
            TuiOptions {
                version: "1.0.0-test".into(),
                ..TuiOptions::default()
            },
        );
        let mut harness = Self {
            app,
            fake: backend(),
            host,
            effects: Vec::new(),
        };
        harness.app.start();
        harness.pump().await;
        harness
    }

    /// Executes queued effects (commands go to the fake backend) until none remain.
    pub async fn pump(&mut self) {
        for _ in 0..20 {
            let effects = self.app.take_effects();
            if effects.is_empty() {
                return;
            }
            for effect in effects {
                match &effect {
                    Effect::Send {
                        tag,
                        command,
                        request_id,
                        ..
                    } => {
                        let result = self
                            .fake
                            .send_with_id(request_id.clone(), command.clone())
                            .await;
                        self.app.handle(AppEvent::Reply(tag.clone(), result));
                    }
                    Effect::Respond(response) => self.fake.ui_respond(response.clone()),
                    Effect::ListSessions(scope) => {
                        let result = self.host.list_sessions(*scope);
                        self.app.handle(AppEvent::Sessions(result));
                    }
                    _ => {}
                }
                self.effects.push(effect);
            }
        }
    }

    pub async fn key(&mut self, key: &str) {
        let id: KeyId = key.parse().unwrap();
        self.app.handle(AppEvent::Key(id));
        self.pump().await;
    }

    pub async fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let id = if c.is_ascii_uppercase() {
                KeyId::new(
                    oino_view::keys::Key::Char(c.to_ascii_lowercase()),
                    oino_view::keys::Modifiers {
                        shift: true,
                        ..oino_view::keys::Modifiers::NONE
                    },
                )
            } else if c == ' ' {
                KeyId::plain(oino_view::keys::Key::Space)
            } else {
                KeyId::plain(oino_view::keys::Key::Char(c))
            };
            self.app.handle(AppEvent::Key(id));
        }
        self.pump().await;
    }

    pub async fn event(&mut self, event: impl Into<ClientEvent>) {
        self.app.handle(AppEvent::Client(Box::new(event.into())));
        self.pump().await;
    }

    pub async fn json_event(&mut self, value: JsonValue) {
        let event: ClientEvent = serde_json::from_value(value).unwrap();
        self.event(event).await;
    }

    /// Commands the backend received, as wire JSON.
    pub fn commands(&self) -> Vec<JsonValue> {
        self.fake
            .commands()
            .iter()
            .map(|c| serde_json::to_value(c).unwrap())
            .collect()
    }

    pub fn command_types(&self) -> Vec<String> {
        self.fake
            .commands()
            .iter()
            .map(|c| c.kind().to_owned())
            .collect()
    }

    /// Renders a frame and returns it as text.
    pub fn frame(&mut self, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| oino_tui::render::draw(f, &mut self.app))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}
