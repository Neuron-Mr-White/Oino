//! `--demo`: a scripted backend that exercises every surface without providers or API keys
//! (streaming markdown + thinking, tool previews, edit diffs, bash tails, an approval dialog,
//! status / frame / working message, a side-pane panel, toasts). Used for UI development,
//! screenshots and smoke tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::{AgentMessage, JsonValue, Model};
use oino_client::fake::FakeBackend;
use oino_client::{ClientError, PiCommand, RpcCommand, RpcResponseData, SessionBackend};
use oino_core::events::ClientEvent;
use oino_core::ui::{UiAnswer, UiResponse};
use oino_ui::host::{
    AppSettings, ExtensionInfo, HostError, HostInfo, OpenRequest, OpenedSession, SessionHost,
};
use oino_ui::model::sessions::SessionSummary;
use serde_json::json;
use tokio::sync::oneshot;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn event(value: JsonValue) -> Option<ClientEvent> {
    match serde_json::from_value(value) {
        Ok(event) => Some(event),
        Err(error) => {
            tracing::warn!(%error, "demo event");
            None
        }
    }
}

fn models() -> Vec<Model> {
    [
        ("oino", "oino-large", "Oino Large", 400_000, true),
        ("anthropic", "claude-sonnet-4-6", "Claude Sonnet 4.6", 200_000, true),
        ("openai", "gpt-5.2", "GPT-5.2", 400_000, true),
        ("openai-compat", "qwen3-coder", "Qwen3 Coder", 256_000, false),
    ]
    .into_iter()
    .filter_map(|(provider, id, name, window, reasoning)| {
        serde_json::from_value(json!({
            "id": id, "name": name, "api": "demo", "provider": provider, "baseUrl": "",
            "input": ["text"], "cost": {"input": 3, "output": 15, "cacheRead": 0.3, "cacheWrite": 3.75},
            "reasoning": reasoning, "contextWindow": window, "maxTokens": 32_000
        }))
        .ok()
    })
    .collect()
}

fn usage(input: u64, output: u64) -> JsonValue {
    #[allow(clippy::cast_precision_loss, reason = "demo numbers")]
    let cost = (input as f64 * 3.0 + output as f64 * 15.0) / 1_000_000.0;
    json!({"input": input, "output": output, "cacheRead": 0, "cacheWrite": 0, "totalTokens": input + output,
           "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": cost}})
}

fn assistant(content: &JsonValue, stop: &str, usage_value: &JsonValue) -> JsonValue {
    json!({"role": "assistant", "content": content, "api": "demo", "provider": "oino", "model": "oino-large",
           "usage": usage_value, "stopReason": stop, "timestamp": 0})
}

const PLAN: &str = "I'll make the parser tolerant of trailing commas, then run the tests.\n\n\
1. **Locate** the token loop in `src/parser.rs`\n\
2. **Patch** the `,` handling so a closing `]` after a comma is accepted\n\
3. **Verify** with `cargo test parser`\n";

const DONE: &str = "Done — trailing commas now parse in arrays and objects.\n\n\
```rust\nlet v: Value = parse(\"[1, 2, 3,]\")?;\nassert_eq!(v.len(), 3);\n```\n\n\
| Check | Result |\n|---|---|\n| `cargo test parser` | 14 passed |\n| clippy | clean |\n\n\
The workflow approval was recorded for this project.";

const PATCH: &str = "--- a/src/parser.rs\n+++ b/src/parser.rs\n@@ -41,9 +41,12 @@ fn parse_array(&mut self) -> Result<Value> {\n     let mut items = Vec::new();\n     loop {\n         self.skip_ws();\n-        if self.peek() == Some(b']') {\n-            break;\n-        }\n+        match self.peek() {\n+            Some(b']') => break,\n+            Some(b',') if self.lookahead_is(b']') => {\n+                self.bump();\n+                break;\n+            }\n+            _ => {}\n+        }\n         items.push(self.parse_value()?);\n         self.expect_comma_or_end(b']')?;\n     }\n";

struct Shared {
    fake: FakeBackend,
    waiting: Mutex<HashMap<String, oneshot::Sender<UiAnswer>>>,
    history: Mutex<Vec<AgentMessage>>,
    model: Mutex<Model>,
    abort: AtomicBool,
    busy: AtomicBool,
    seq: AtomicU64,
}

/// A scripted session.
#[derive(Clone)]
pub(crate) struct DemoBackend {
    shared: Arc<Shared>,
    id: String,
}

impl DemoBackend {
    /// A demo session `id` (optionally with history).
    #[must_use]
    pub(crate) fn new(id: &str, history: Vec<AgentMessage>) -> Self {
        let model = models().into_iter().next();
        Self {
            shared: Arc::new(Shared {
                fake: FakeBackend::new(id),
                waiting: Mutex::new(HashMap::new()),
                history: Mutex::new(history),
                model: Mutex::new(model.unwrap_or_else(unreachable_model)),
                abort: AtomicBool::new(false),
                busy: AtomicBool::new(false),
                seq: AtomicU64::new(0),
            }),
            id: id.to_owned(),
        }
    }
}

#[allow(
    clippy::expect_used,
    reason = "the static demo model list always parses"
)]
fn unreachable_model() -> Model {
    serde_json::from_value(json!({"id":"demo","name":"Demo","api":"demo","provider":"demo","baseUrl":"","input":["text"],
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"reasoning":false,"contextWindow":1,"maxTokens":1}))
        .expect("static model")
}

impl Shared {
    fn emit(&self, value: JsonValue) {
        if let Some(e) = event(value) {
            self.fake.emit(e);
        }
    }

    async fn pause(&self, ms: u64) -> bool {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        !self.abort.load(Ordering::SeqCst)
    }

    fn ui(&self, method: &str, mut fields: JsonValue, extension: &str) -> String {
        let id = format!("demo-ui-{}", self.seq.fetch_add(1, Ordering::SeqCst));
        fields["type"] = json!("extension_ui_request");
        fields["id"] = json!(id);
        fields["method"] = json!(method);
        fields["extensionId"] = json!(extension);
        self.emit(fields);
        id
    }

    async fn stream_text(&self, content_prefix: &[JsonValue], thinking: &str, text: &str) -> bool {
        let mut shown_thinking = String::new();
        for word in thinking.split_inclusive(' ') {
            shown_thinking.push_str(word);
            let mut content = content_prefix.to_vec();
            content.push(json!({"type": "thinking", "thinking": shown_thinking}));
            let partial = assistant(&json!(content), "stop", &usage(0, 0));
            self.emit(json!({"type": "message_update", "message": partial,
                "assistantMessageEvent": {"type": "start", "partial": partial}}));
            if !self.pause(18).await {
                return false;
            }
        }
        let mut shown = String::new();
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(6) {
            shown.extend(chunk);
            let mut content = content_prefix.to_vec();
            if !thinking.is_empty() {
                content.push(json!({"type": "thinking", "thinking": thinking}));
            }
            content.push(json!({"type": "text", "text": shown}));
            let partial = assistant(&json!(content), "stop", &usage(0, 0));
            self.emit(json!({"type": "message_update", "message": partial,
                "assistantMessageEvent": {"type": "start", "partial": partial}}));
            if !self.pause(14).await {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_lines, reason = "linear demo script")]
    async fn scenario(self: Arc<Self>, prompt: String) {
        self.abort.store(false, Ordering::SeqCst);
        self.busy.store(true, Ordering::SeqCst);
        self.emit(json!({"type": "agent_start"}));
        let user = json!({"role": "user", "content": prompt, "timestamp": 0});
        self.emit(json!({"type": "message_start", "message": user}));
        self.emit(json!({"type": "message_end", "message": user}));
        self.ui("setStatus", json!({"statusKey": "workflow", "statusText": "workflow · build", "tone": "c4", "priority": 5}), "unipi.workflow");
        self.ui(
            "setStatus",
            json!({"statusKey": "mem", "statusText": "memory 12", "tone": "muted"}),
            "unipi.memory",
        );
        self.ui("setFrame", json!({"frameKey": "footer",
            "bottom": [{"text": "⎇ v1-alpha", "tone": "c2", "priority": 3, "align": "left"},
                       {"text": "plan: 3 steps", "tone": "muted", "priority": 1, "align": "left"},
                       {"text": "auto-compact on", "tone": "muted", "priority": 2, "align": "right"}]}), "unipi.footer");
        self.ui(
            "setWorkingMessage",
            json!({"message": "Planning the change…"}),
            "unipi.footer",
        );

        let start = assistant(&json!([]), "stop", &usage(0, 0));
        self.emit(json!({"type": "message_start", "message": start}));
        let thinking = "The user wants trailing commas accepted. The array loop breaks on ']' only; a comma followed by ']' fails in expect_comma_or_end. Patch the peek match and add a lookahead.";
        if !self.stream_text(&[], thinking, PLAN).await {
            return self.finish(true);
        }
        let mut content = vec![
            json!({"type": "thinking", "thinking": thinking}),
            json!({"type": "text", "text": PLAN}),
        ];
        // Streaming tool-call arguments (preview shows path and line counts).
        for (i, args) in [
            json!({"path": "src/parser.rs"}),
            json!({"path": "src/parser.rs", "edits": [{"oldText": "if self.peek() == Some(b']') {\n    break;\n}", "newText": ""}]}),
            json!({"path": "src/parser.rs", "edits": [{"oldText": "if self.peek() == Some(b']') {\n    break;\n}", "newText": "match self.peek() {\n    Some(b']') => break,\n    Some(b',') if self.lookahead_is(b']') => {\n        self.bump();\n        break;\n    }\n    _ => {}\n}"}]}),
        ]
        .into_iter()
        .enumerate()
        {
            let mut c = content.clone();
            c.push(json!({"type": "toolCall", "id": "call_edit", "name": "edit", "arguments": args}));
            if i == 2 {
                c.push(json!({"type": "toolCall", "id": "call_bash", "name": "bash", "arguments": {"command": "cargo test parser"}}));
            }
            let partial = assistant(&json!(c), "toolUse", &usage(0, 0));
            self.emit(json!({"type": "message_update", "message": partial,
                "assistantMessageEvent": {"type": "start", "partial": partial}}));
            if !self.pause(260).await {
                return self.finish(true);
            }
            if i == 2 {
                content = c;
            }
        }
        let end = assistant(&json!(content), "toolUse", &usage(18_400, 950));
        self.emit(json!({"type": "message_end", "message": end}));

        // Edit.
        self.emit(
            json!({"type": "tool_execution_start", "toolCallId": "call_edit", "toolName": "edit",
            "args": {"path": "src/parser.rs"}}),
        );
        self.pause(350).await;
        self.emit(json!({"type": "tool_execution_end", "toolCallId": "call_edit", "toolName": "edit",
            "result": {"content": [{"type": "text", "text": "Successfully replaced 1 block in src/parser.rs."}],
                       "details": {"patch": PATCH, "firstChangedLine": 44}},
            "isError": false, "durationMs": 12}));

        // Approval (unipi workflow guard) before bash.
        let (tx, rx) = oneshot::channel();
        let dialog = format!("demo-ui-{}", self.seq.load(Ordering::SeqCst));
        lock(&self.waiting).insert(dialog.clone(), tx);
        self.ui("approval", json!({
            "title": "Run shell command?", "toolCallId": "call_bash", "toolName": "bash",
            "summary": "cargo test parser",
            "detail": {"t": "code", "value": "$ cargo test parser", "language": "sh"},
            "options": [
                {"optionId": "allow", "label": "Allow once", "kind": "allow_once"},
                {"optionId": "always", "label": "Always allow in project", "kind": "allow_always"},
                {"optionId": "deny", "label": "Deny", "kind": "reject_once"}
            ],
            "feedback": {"placeholder": "Tell the agent what to do instead (optional)"}
        }), "unipi.workflow");
        self.ui(
            "setWorkingMessage",
            json!({"message": "Waiting for approval…"}),
            "unipi.footer",
        );
        if let Some(ms) = std::env::var("OINO_DESKTOP_DEMO_APPROVE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            let shared = Arc::clone(&self);
            let dialog = dialog.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let waiting = lock(&shared.waiting).remove(&dialog);
                shared.emit(
                    json!({"type": "extension_ui_dismiss", "id": dialog, "reason": "answered"}),
                );
                if let Some(tx) = waiting {
                    let _ = tx.send(UiAnswer::Value(json!({"optionId": "allow"})));
                }
            });
        }
        let answer = rx.await.unwrap_or(UiAnswer::Cancelled);
        let allowed = matches!(&answer, UiAnswer::Value(v) if v.get("optionId").and_then(JsonValue::as_str) != Some("deny"));
        self.ui(
            "setWorkingMessage",
            json!({"message": "Running tests…"}),
            "unipi.footer",
        );
        self.emit(
            json!({"type": "tool_execution_start", "toolCallId": "call_bash", "toolName": "bash",
            "args": {"command": "cargo test parser"}}),
        );
        if allowed {
            let lines = [
                "   Compiling json-lite v0.3.1",
                "    Finished test profile in 3.21s",
                "     Running unittests src/lib.rs",
                "running 14 tests",
                "test parser::arrays ... ok",
                "test parser::objects ... ok",
                "test parser::trailing_comma_array ... ok",
                "test parser::trailing_comma_object ... ok",
                "test result: ok. 14 passed; 0 failed",
            ];
            let mut out = String::new();
            for line in lines {
                out.push_str(line);
                out.push('\n');
                self.emit(json!({"type": "tool_execution_update", "toolCallId": "call_bash", "toolName": "bash",
                    "args": {"command": "cargo test parser"}, "partialResult": {"content": [{"type": "text", "text": out}]}}));
                if !self.pause(220).await {
                    return self.finish(true);
                }
            }
            self.emit(json!({"type": "tool_execution_end", "toolCallId": "call_bash", "toolName": "bash",
                "result": {"content": [{"type": "text", "text": out}]}, "isError": false, "durationMs": 3420}));
        } else {
            self.emit(json!({"type": "tool_execution_end", "toolCallId": "call_bash", "toolName": "bash",
                "result": {"content": [{"type": "text", "text": "Blocked by unipi.workflow: denied by user"}]}, "isError": true}));
        }

        let start = assistant(&json!([]), "stop", &usage(0, 0));
        self.emit(json!({"type": "message_start", "message": start}));
        if !self.stream_text(&[], "", DONE).await {
            return self.finish(true);
        }
        let end = assistant(
            &json!([{"type": "text", "text": DONE}]),
            "stop",
            &usage(21_300, 610),
        );
        self.emit(json!({"type": "message_end", "message": end}));
        self.ui("openPanel", json!({
            "panelId": "unipi.info-screen/dashboard", "placement": "sidePane", "title": "Info", "focus": true,
            "spec": {"t": "stack", "direction": "vertical", "gap": 12, "children": [
                {"t": "card", "title": "Session", "rail": "harness", "source": "unipi.info-screen", "body":
                    {"t": "kv", "items": [{"key": "Model", "value": "Oino Large"}, {"key": "Workflow", "value": [{"text": "build", "tone": "c4", "bold": true}]}, {"key": "Tools", "value": "read · edit · bash · grep"}]}},
                {"t": "meter", "label": "Context", "value": 40, "max": 400, "tone": "success"},
                {"t": "sparkline", "label": "tokens/s", "values": [12, 18, 25, 31, 28, 35, 41, 38, 44, 40, 47, 52]},
                {"t": "table", "columns": [{"title": "Extension"}, {"title": "State"}], "rows": [
                    ["unipi.workflow", [{"text": "active", "tone": "success"}]],
                    ["unipi.memory", "12 memories"],
                    ["unipi.subagents", [{"text": "idle", "tone": "muted"}]]]},
                {"t": "keyHints", "hints": [{"keys": "r", "label": "refresh"}, {"keys": "esc", "label": "close"}]}
            ]}
        }), "unipi.info-screen");
        self.ui(
            "notify",
            json!({"message": "Tests passed — 14/14", "notifyType": "info"}),
            "unipi.notify",
        );
        self.finish(false);
    }

    fn finish(&self, aborted: bool) {
        self.ui(
            "setWorkingMessage",
            json!({"visible": false}),
            "unipi.footer",
        );
        self.emit(json!({"type": "agent_end", "messages": [], "willRetry": false}));
        self.emit(json!({"type": "agent_settled", "aborted": aborted}));
        self.busy.store(false, Ordering::SeqCst);
    }
}

impl SessionBackend for DemoBackend {
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        let shared = Arc::clone(&self.shared);
        let id = self.id.clone();
        Box::pin(async move {
            let RpcCommand::Pi(command) = command else {
                return Ok(match command {
                    RpcCommand::Oino(oino_client::OinoCommand::UiSnapshot) => {
                        Some(json!({"requests": []}))
                    }
                    _ => None,
                });
            };
            Ok(match command {
                PiCommand::Prompt { message, .. } => {
                    if shared.busy.load(Ordering::SeqCst) {
                        shared.emit(
                            json!({"type": "queue_update", "steering": [message], "followUp": []}),
                        );
                    } else {
                        tokio::spawn(Arc::clone(&shared).scenario(message));
                    }
                    None
                }
                PiCommand::Abort => {
                    shared.abort.store(true, Ordering::SeqCst);
                    None
                }
                PiCommand::ClearQueue => {
                    shared.emit(json!({"type": "queue_update", "steering": [], "followUp": []}));
                    None
                }
                PiCommand::GetState => {
                    let model = lock(&shared.model).clone();
                    Some(
                        json!({"model": model, "thinkingLevel": "medium", "isStreaming": false, "isCompacting": false,
                        "steeringMode": "all", "followUpMode": "all", "sessionId": id, "autoCompactionEnabled": true,
                        "messageCount": lock(&shared.history).len(), "pendingMessageCount": 0}),
                    )
                }
                PiCommand::GetMessages => Some(json!({"messages": *lock(&shared.history)})),
                PiCommand::GetAvailableModels => Some(json!({"models": models()})),
                PiCommand::GetCommands => Some(json!({"commands": [
                    {"name": "compact", "description": "Compact the context", "source": "builtin", "sourceInfo": null},
                    {"name": "login", "description": "Log in to a provider", "source": "builtin", "sourceInfo": null},
                    {"name": "model", "description": "Select a model", "source": "builtin", "sourceInfo": null},
                    {"name": "unipi:settings", "description": "Open the extension settings hub", "source": "extension", "sourceInfo": null},
                    {"name": "unipi:workflow", "description": "Switch workflow (plan, build, review)", "source": "extension", "sourceInfo": null},
                    {"name": "skill:rust-best-practices", "description": "Rust guidelines", "source": "skill", "sourceInfo": null}
                ]})),
                PiCommand::SetModel { provider, model_id } => {
                    if let Some(model) = models()
                        .into_iter()
                        .find(|m| m.provider == provider && m.id == model_id)
                    {
                        *lock(&shared.model) = model.clone();
                        shared.emit(
                            json!({"type": "model_changed", "model": model, "source": "set"}),
                        );
                    }
                    None
                }
                PiCommand::SetThinkingLevel { level } => {
                    shared.emit(json!({"type": "thinking_level_changed", "level": level}));
                    None
                }
                _ => None,
            })
        })
    }

    fn events(&self) -> BoxStream<'static, ClientEvent> {
        self.shared.fake.events()
    }

    fn ui_respond(&self, response: UiResponse) {
        let waiting = lock(&self.shared.waiting).remove(&response.id);
        self.shared
            .emit(json!({"type": "extension_ui_dismiss", "id": response.id, "reason": "answered"}));
        if let Some(tx) = waiting {
            let _ = tx.send(response.answer);
        }
    }

    fn session_id(&self) -> String {
        self.id.clone()
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// Host for `--demo`.
pub(crate) struct DemoHost {
    agent_dir: std::path::PathBuf,
    default_cwd: std::path::PathBuf,
    next: AtomicU64,
    settings: Mutex<AppSettings>,
}

impl DemoHost {
    /// A demo host (preferences kept in memory).
    #[must_use]
    pub(crate) fn new(agent_dir: std::path::PathBuf, default_cwd: std::path::PathBuf) -> Self {
        Self {
            agent_dir,
            default_cwd,
            next: AtomicU64::new(1),
            settings: Mutex::new(AppSettings::default()),
        }
    }
}

fn history() -> Vec<AgentMessage> {
    [
        json!({"role": "user", "content": "Summarise the open TODOs in the kanboard", "timestamp": 0}),
        assistant(&json!([{"type": "text", "text": "There are **3** open cards:\n\n- Port `watchdog` caps\n- GPUI settings hub\n- Release matrix"}]), "stop", &usage(5_000, 200)),
    ]
    .into_iter()
    .filter_map(|v| serde_json::from_value(v).ok())
    .collect()
}

impl SessionHost for DemoHost {
    fn info(&self) -> HostInfo {
        HostInfo {
            agent_dir: self.agent_dir.clone(),
            default_cwd: self.default_cwd.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            backend: "demo".to_owned(),
        }
    }

    fn list_sessions(&self) -> BoxFuture<'static, Result<Vec<SessionSummary>, HostError>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or_default();
        let cwd = self.default_cwd.display().to_string();
        let list = vec![
            ("d-1", cwd.as_str(), "Kanboard TODO summary", 4 * 60_000),
            ("d-2", cwd.as_str(), "Fix flaky retry test", 3 * 3_600_000),
            (
                "d-3",
                "/home/oi/Projects/Personal/unipi",
                "Port watchdog caps",
                26 * 3_600_000,
            ),
            (
                "d-4",
                "/home/oi/Projects/site",
                "Landing page copy",
                5 * 86_400_000,
            ),
        ]
        .into_iter()
        .map(|(id, cwd, title, ago)| SessionSummary {
            id: id.to_owned(),
            path: Some(format!("/demo/{id}.jsonl").into()),
            cwd: cwd.to_owned(),
            title: title.to_owned(),
            modified_ms: now - ago,
            message_count: 2,
        })
        .collect();
        Box::pin(async move { Ok(list) })
    }

    fn open(&self, request: OpenRequest) -> BoxFuture<'static, Result<OpenedSession, HostError>> {
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let (cwd, backend, file) = match request {
            OpenRequest::New { cwd } => {
                let backend = DemoBackend::new(&format!("demo-new-{n}"), Vec::new());
                (cwd, backend, None)
            }
            OpenRequest::Resume { path, cwd } => {
                let id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (cwd, DemoBackend::new(&id, history()), Some(path))
            }
        };
        // `OINO_DESKTOP_DEMO_PROMPT`: the first new session plays the script on its own
        // (screenshots and smoke runs without input automation).
        let autoplay = std::env::var("OINO_DESKTOP_DEMO_PROMPT")
            .ok()
            .filter(|_| n == 1 && file.is_none());
        Box::pin(async move {
            if let Some(prompt) = autoplay {
                let shared = Arc::clone(&backend.shared);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    shared.scenario(prompt).await;
                });
            }
            Ok(OpenedSession {
                backend: Arc::new(backend),
                cwd: cwd.display().to_string(),
                file,
            })
        })
    }

    fn extensions(&self) -> BoxFuture<'static, Vec<ExtensionInfo>> {
        let list = [
            ("oino.anthropic", "provider"),
            ("oino.openai-compat", "provider"),
            ("oino.openai-responses", "provider"),
            ("oino.router", "provider"),
            ("unipi.core", "extension"),
            ("unipi.workflow", "extension"),
            ("unipi.footer", "extension"),
            ("unipi.info-screen", "extension"),
        ]
        .into_iter()
        .map(|(id, kind)| ExtensionInfo {
            id: id.to_owned(),
            version: "1.0.0".to_owned(),
            kind: kind.to_owned(),
            bundled: true,
            error: None,
        })
        .collect();
        Box::pin(async move { list })
    }

    fn app_settings(&self) -> AppSettings {
        lock(&self.settings).clone()
    }

    fn save_app_settings(&self, settings: &AppSettings) {
        *lock(&self.settings) = settings.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;
    use oino_ui::model::session::SessionModel;

    /// The whole demo script reduces cleanly through the view models.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scenario_drives_the_view_models() {
        let backend = DemoBackend::new("t", Vec::new());
        let mut events = backend.events();
        backend
            .send(
                PiCommand::Prompt {
                    message: "go".into(),
                    images: Vec::new(),
                    streaming_behavior: None,
                }
                .into(),
            )
            .await
            .unwrap();
        let mut model = SessionModel::new("t", "/w");
        let mut answered = false;
        loop {
            let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_secs(20), events.next()).await
            else {
                panic!("demo stalled");
            };
            model.apply(&event);
            if let Some(dialog) = model.ui.dialog().cloned()
                && !answered
            {
                answered = true;
                backend.ui_respond(dialog.approve("allow", None));
            }
            if matches!(
                event,
                ClientEvent::Session(oino_core::events::AgentSessionEvent::AgentSettled { .. })
            ) {
                break;
            }
        }
        assert!(answered);
        assert!(!model.is_busy());
        assert!(model.transcript.get("call_edit").is_some());
        assert!(model.transcript.get("call_bash").is_some());
        assert!(model.ui.panel("unipi.info-screen/dashboard").is_some());
        assert_eq!(model.ui.statuses().len(), 2);
        assert!(model.usage.output > 0);
    }
}
