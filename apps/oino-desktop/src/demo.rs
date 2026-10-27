//! `--demo`: a scripted backend that exercises every surface without providers or API keys.
//!
//! Every demo session starts with the ambient unipi chrome (splash header, hint strip, footer
//! frame and stats strip, shortcuts). `OINO_DESKTOP_DEMO_SCENE` picks what the first new
//! session plays (screenshots, perf):
//!
//! | Scene | What |
//! |---|---|
//! | `welcome` (default without a prompt) | empty session with the splash hero |
//! | `run` (default with `OINO_DESKTOP_DEMO_PROMPT`) | plan, subagent, edit diff, approval, bash tail, done |
//! | `approval` | stops at the docked approval card |
//! | `question` | an `ask_user` form |
//! | `plan` | a workflow plan review |
//! | `board` | the kanboard panel |
//! | `info` | the info-screen overlay dashboard |
//! | `stream` | endless streaming markdown (perf) |
//!
//! The frontend `_oino/*` commands are answered too (completions, panel actions, settings
//! hub schema and values, auth providers, tree), so every panel button is interactive.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::{AgentMessage, JsonValue, Model};
use oino_client::fake::FakeBackend;
use oino_client::{
    ClientError, OinoCommand, PiCommand, RpcCommand, RpcResponseData, SessionBackend,
};
use oino_core::events::ClientEvent;
use oino_core::ui::{UiAnswer, UiResponse};
use oino_ui::host::{
    AppSettings, ExtensionInfo, HostError, HostInfo, OpenRequest, OpenedSession, SessionHost,
    ThemePref,
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
        ("oino", "oino-fast", "Oino Fast", 200_000, true),
        ("anthropic", "claude-sonnet-4-6", "Claude Sonnet 4.6", 1_000_000, true),
        ("openai", "gpt-5.2", "GPT-5.2", 400_000, true),
        ("openai-compat", "qwen3-coder", "Qwen3 Coder", 256_000, false),
    ]
    .into_iter()
    .filter_map(|(provider, id, name, window, reasoning)| {
        serde_json::from_value(json!({
            "id": id, "name": name, "api": "demo", "provider": provider, "baseUrl": "",
            "input": ["text", "image"], "cost": {"input": 3, "output": 15, "cacheRead": 0.3, "cacheWrite": 3.75},
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

const LOREM: &str = "Streaming keeps the **tail row** patched while the rest of the timeline stays cached. \
Each frame reduces one coalesced batch, so a burst of tokens costs a single redraw. `ListState` only \
remeasures the rows that changed, and incremental markdown appends to the last block.\n\n";

/// What the first new session plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    Welcome,
    Run,
    Approval,
    Question,
    Plan,
    Board,
    Info,
    Stream,
}

impl Scene {
    fn from_env() -> Self {
        match std::env::var("OINO_DESKTOP_DEMO_SCENE").as_deref() {
            Ok("approval") => Self::Approval,
            Ok("question") => Self::Question,
            Ok("plan") => Self::Plan,
            Ok("board") => Self::Board,
            Ok("info") => Self::Info,
            Ok("stream") => Self::Stream,
            Ok("run") => Self::Run,
            Ok("welcome") => Self::Welcome,
            _ if std::env::var("OINO_DESKTOP_DEMO_PROMPT").is_ok() => Self::Run,
            _ => Self::Welcome,
        }
    }
}

struct Shared {
    fake: FakeBackend,
    waiting: Mutex<HashMap<String, oneshot::Sender<UiAnswer>>>,
    history: Mutex<Vec<AgentMessage>>,
    model: Mutex<Model>,
    abort: AtomicBool,
    busy: AtomicBool,
    seq: AtomicU64,
    board_selected: Mutex<String>,
    info_tab: Mutex<String>,
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
                board_selected: Mutex::new("OIN-12".into()),
                info_tab: Mutex::new("overview".into()),
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

fn splash(cwd: &str) -> JsonValue {
    let body = |wide: bool| {
        json!({"t": "card", "rail": "warning", "effect": "shimmer", "body": {"t": "stack", "direction": "vertical", "gap": 1, "children": [
            {"t": "text", "value": [{"text": "Oino ", "bold": true}, {"text": "1.0.0-alpha.2", "tone": "warning", "code": true}, {"text": " · 23 extensions", "dim": true}]},
            {"t": "text", "value": [{"text": "Good afternoon", "bold": true}, {"text": " — I'm ", "tone": "muted"}, {"text": "Unicrab", "bold": true, "tone": "warning"}, {"text": ".", "tone": "muted"}]},
            {"t": "text", "value": [{"text": "● ", "tone": "success"}, {"text": if wide { "ready in 412ms · 23 modules · 64 tools" } else { "ready · 64 tools" }, "tone": "muted"}]},
            {"t": "text", "value": [{"text": "● ", "tone": "info"}, {"text": "new session in ", "tone": "muted"}, {"text": cwd, "bold": true}, {"text": " on v1-alpha", "dim": true}]},
            {"t": "meter", "value": 1, "max": 1, "tone": "warning", "effect": "drain", "durationMs": 60_000}
        ]}})
    };
    json!({"t": "responsive", "variants": [{"minWidth": 76, "node": body(true)}, {"minWidth": 44, "node": body(false)}], "fallback": {"t": "empty"}})
}

fn board_spec(selected: &str) -> JsonValue {
    let lane = |id: &str, title: &str, items: JsonValue| {
        let n = items.as_array().map_or(0, Vec::len);
        let body = if n == 0 {
            json!({"t": "empty", "message": "empty"})
        } else {
            json!({"t": "list", "items": items, "selectable": "select", "selected": selected})
        };
        json!({"t": "section", "key": format!("col-{id}"), "title": format!("{title} ({n})"), "body": body})
    };
    let item = |id: &str, title: &str, extra: JsonValue| {
        let mut v = json!({"id": id, "label": [{"text": id, "tone": "muted"}, {"text": format!(" {title}")}]});
        if let (Some(obj), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            for (k, val) in e {
                obj.insert(k.clone(), val.clone());
            }
        }
        v
    };
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
        {"t": "text", "value": [{"text": "oino", "bold": true}, {"text": " · 9 tasks · live", "dim": true}]},
        {"t": "split", "key": "lanes", "direction": "horizontal", "children": [
            lane("backlog", "Backlog", json!([item("OIN-20", "Remote attach pairing", json!({}))])),
            lane("todo", "Todo", json!([
                item("OIN-12", "Extension settings hub", json!({"badge": "high", "tone": "warning"})),
                item("OIN-14", "Port watchdog caps", json!({"detail": "waits for OIN-12"})),
                item("OIN-15", "Release matrix", json!({"badge": "low", "tone": "muted"}))])),
            lane("in_progress", "In Progress", json!([
                item("OIN-9", "Docked approval cards", json!({"detail": "claimed by this session", "badge": "urgent", "tone": "error"}))])),
            lane("blocked", "Blocked", json!([
                item("OIN-7", "Notarized macOS build", json!({"detail": "needs a Developer ID", "tone": "error"}))])),
            lane("in_review", "In Review", json!([item("OIN-5", "Frame-time overlay", json!({}))])),
            lane("done", "Done", json!([item("OIN-1", "Design tokens", json!({})), item("OIN-2", "Bundled fonts", json!({}))])),
        ]},
        {"t": "section", "key": "detail", "title": format!("{selected} Extension settings hub"), "body": {"t": "stack", "direction": "vertical", "gap": 0, "children": [
            {"t": "kv", "items": [{"key": "Status", "value": "Todo"}, {"key": "Priority", "value": "high"}, {"key": "After", "value": "OIN-3"}]},
            {"t": "markdown", "value": "Render `get_settings_schema` namespaces natively with scope toggle and reset."},
            {"t": "stack", "direction": "horizontal", "gap": 1, "children": [
                {"t": "button", "label": "→ In Progress", "actionId": "move:in_progress"},
                {"t": "button", "label": "→ Blocked", "actionId": "move:blocked"}]}
        ]}}
    ]})
}

fn info_spec(tab: &str) -> JsonValue {
    let overview = json!({"t": "stack", "direction": "vertical", "gap": 3, "children": [
        {"t": "card", "title": "Session", "rail": "harness", "source": "unipi.info-screen", "body":
            {"t": "kv", "items": [{"key": "Model", "value": "Oino Large"}, {"key": "Workflow", "value": [{"text": "ask", "tone": "c4", "bold": true}]}, {"key": "Tools", "value": "read · edit · bash · grep · run_subagent"}]}},
        {"t": "meter", "label": "Context", "value": 41, "max": 400, "tone": "success"},
        {"t": "sparkline", "label": "tokens/s", "values": [12, 18, 25, 31, 28, 35, 41, 38, 44, 40, 47, 52, 49, 55, 58, 61]},
        {"t": "keyHints", "hints": [{"keys": "r", "label": "refresh"}, {"keys": "esc", "label": "close"}]}
    ]});
    let usage = json!({"t": "stack", "direction": "vertical", "gap": 3, "children": [
        {"t": "barChart", "bars": [{"label": "Oino Large", "value": 41200}, {"label": "Oino Fast", "value": 18800}, {"label": "Sonnet 4.6", "value": 9100}, {"label": "GPT-5.2", "value": 3500}]},
        {"t": "area", "series": [{"label": "input", "values": [3, 5, 9, 4, 7, 12, 15, 9, 11, 14]}, {"label": "output", "values": [1, 2, 3, 2, 4, 5, 3, 6, 4, 5]}]}
    ]});
    let modules = json!({"t": "table", "columns": [{"title": "Extension"}, {"title": "State"}, {"title": "Tools"}], "rows": [
        ["unipi.workflow", [{"text": "active", "tone": "success"}], "—"],
        ["unipi.memory", "12 memories", "3"],
        ["unipi.subagents", [{"text": "idle", "tone": "muted"}], "2"],
        ["unipi.kanboard", [{"text": "live", "tone": "success"}], "4"],
        ["unipi.background-tasks", "1 running", "3"]]});
    json!({"t": "tabs", "active": tab, "onChange": "$tab", "tabs": [
        {"id": "overview", "title": "Overview", "body": overview},
        {"id": "usage", "title": "Usage", "body": usage},
        {"id": "modules", "title": "Modules", "body": modules},
        {"id": "tools", "title": "Tools", "body": {"t": "list", "items": [
            {"id": "read", "label": "read", "detail": "Read a file"}, {"id": "edit", "label": "edit", "detail": "Replace exact text"},
            {"id": "bash", "label": "bash", "detail": "Run a shell command"}, {"id": "run_subagent", "label": "run_subagent", "detail": "Delegate to a subagent", "tone": "c3"}]}}
    ]})
}

fn settings_schema() -> JsonValue {
    json!([
        {"namespace": "notify", "title": "Notify", "extensionId": "unipi.notify", "sections": [
            {"id": "events", "title": "Events", "fields": [
                {"key": "events.agentEnd", "type": "boolean", "label": "Agent finished", "description": "Notify when a run settles while the window is unfocused.", "default": true},
                {"key": "events.ask", "type": "boolean", "label": "Questions", "default": true},
                {"key": "platform", "type": "enum", "label": "Platform", "options": [{"value": "native", "label": "Native"}, {"value": "ntfy", "label": "ntfy"}, {"value": "gotify", "label": "Gotify"}], "default": "native"}]},
            {"id": "ntfy", "title": "ntfy", "fields": [
                {"key": "ntfy.topic", "type": "string", "label": "Topic", "default": ""},
                {"key": "ntfy.token", "type": "secret", "label": "Access token", "scope": "global"},
                {"key": "test", "type": "action", "label": "Send test", "command": "unipi:notify-test"}]}]},
        {"namespace": "workflow", "title": "Workflow", "extensionId": "unipi.workflow", "sections": [
            {"id": "mode", "title": "Permissions", "fields": [
                {"key": "mode", "type": "enum", "label": "Mode here", "options": ["ask", "auto", "full"], "default": "auto"},
                {"key": "judge.timeoutMs", "type": "number", "label": "Judge timeout", "min": 1000, "max": 60000, "default": 15000}]}]},
        {"namespace": "footer", "title": "Footer", "extensionId": "unipi.footer", "sections": [
            {"id": "strip", "title": "Stats strip", "fields": [
                {"key": "enabled", "type": "boolean", "label": "Show the stats strip", "default": true},
                {"key": "icons", "type": "enum", "label": "Icons", "options": ["nerd", "unicode", "text"], "default": "unicode"}]}]},
        {"namespace": "memory", "title": "Memory", "extensionId": "unipi.memory", "sections": [
            {"id": "recall", "title": "Recall", "fields": [{"key": "recall.limit", "type": "number", "label": "Memories per turn", "default": 5}]}]}
    ])
}

fn tree() -> JsonValue {
    let msg = |id: &str, parent: Option<&str>, role: &str, text: &str| {
        let message = if role == "user" {
            json!({"role": "user", "content": text, "timestamp": 0})
        } else {
            assistant(
                &json!([{"type": "text", "text": text}]),
                "stop",
                &usage(0, 0),
            )
        };
        json!({"type": "message", "id": id, "parentId": parent, "timestamp": "2026-10-26T12:00:00.000Z", "message": message})
    };
    json!({"leafId": "a0000004", "tree": [
        {"entry": msg("a0000001", None, "user", "Make the JSON parser accept trailing commas"), "children": [
            {"entry": msg("a0000002", Some("a0000001"), "assistant", "I'll make the parser tolerant of trailing commas."), "children": [
                {"entry": msg("a0000003", Some("a0000002"), "user", "Also objects"), "label": "objects", "children": [
                    {"entry": msg("a0000004", Some("a0000003"), "assistant", "Done — arrays and objects."), "children": []}]},
                {"entry": msg("a0000005", Some("a0000002"), "user", "Only arrays for now"), "children": []}]}]}
    ]})
}

impl Shared {
    fn emit(&self, value: JsonValue) {
        if let Some(e) = event(value) {
            self.fake.emit(e);
        }
    }

    async fn pause(&self, ms: u64) -> bool {
        // `OINO_DESKTOP_DEMO_PACE` slows the script (screenshots of mid-stream states).
        let pace = std::env::var("OINO_DESKTOP_DEMO_PACE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1)
            .max(1);
        tokio::time::sleep(Duration::from_millis(ms * pace)).await;
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

    /// Ambient unipi chrome every demo session shows.
    fn ambient(&self, cwd: &str, splash_on: bool) {
        if splash_on {
            self.ui(
                "setHeader",
                json!({"key": "splash", "spec": splash(cwd)}),
                "unipi.info-screen",
            );
        }
        self.ui(
            "setWidget",
            json!({"widgetKey": "hints", "widgetPlacement": "aboveEditor", "widgetLines": [
            "Press alt+h for the next tip · ↓ in an empty composer opens the work tray",
            "Mention files with @, run skills with $, slash commands with /",
            "Alt+Enter queues a follow-up while Oino is working"]}),
            "unipi.core",
        );
        self.ui("setFrame", json!({"frameKey": "frame",
            "top": [{"id": "branch", "text": "v1-alpha", "icon": "branch", "tone": "muted", "priority": 6, "align": "left"},
                    {"id": "permission", "text": "ask", "tone": "success", "priority": 4, "align": "right"}],
            "bottom": [{"id": "workspace", "text": "json-lite", "icon": "folder", "tone": "muted", "priority": 5, "align": "left"},
                       {"id": "branch", "text": "v1-alpha", "icon": "branch", "tone": "c2", "priority": 4, "align": "left"}]}), "unipi.footer");
        self.footer(4, 18_400, 950, 0.07);
        self.ui("setShortcuts", json!({"shortcuts": [
            {"id": "unipi.workflow.plan", "keys": "alt+p", "description": "Toggle plan mode", "extensionId": "unipi.workflow"},
            {"id": "unipi.core.hint", "keys": "alt+h", "description": "Next tip", "extensionId": "unipi.core"},
            {"id": "unipi.info-screen.open", "keys": "alt+i", "description": "Session dashboard", "extensionId": "unipi.info-screen"}]}), "unipi.input-shortcuts");
    }

    fn footer(&self, ctx: u64, input: u64, output: u64, cost: f64) {
        self.ui("setFooter", json!({"key": "strip", "spec": {"t": "text", "value": format!("ctx {ctx}%")},
            "segments": [
                {"id": "ctx", "text": format!("ctx {ctx}%"), "priority": 7, "command": "compact", "spans": [{"text": "ctx ", "dim": true}, {"text": format!("{ctx}%"), "tone": "success"}]},
                {"id": "tokens", "text": format!("↑{}k ↓{}", input / 1000, output), "priority": 6},
                {"id": "cost", "text": format!("${cost:.2}"), "priority": 5, "spans": [{"text": format!("${cost:.2}"), "tone": "c4"}]},
                {"id": "tps", "text": "52 tok/s", "priority": 3, "spans": [{"text": "52 tok/s", "dim": true}]},
                {"id": "model", "text": "Oino Large", "priority": 4}]}), "unipi.footer");
    }

    async fn stream_text(
        &self,
        prefix: &[JsonValue],
        thinking: &str,
        text: &str,
        chunk: usize,
        ms: u64,
    ) -> bool {
        let mut shown_thinking = String::new();
        for word in thinking.split_inclusive(' ') {
            shown_thinking.push_str(word);
            let mut content = prefix.to_vec();
            content.push(json!({"type": "thinking", "thinking": shown_thinking}));
            let partial = assistant(&json!(content), "stop", &usage(0, 0));
            self.emit(json!({"type": "message_update", "message": partial, "assistantMessageEvent": {"type": "start", "partial": partial}}));
            if !self.pause(18).await {
                return false;
            }
        }
        let mut shown = String::new();
        let chars: Vec<char> = text.chars().collect();
        for c in chars.chunks(chunk) {
            shown.extend(c);
            let mut content = prefix.to_vec();
            if !thinking.is_empty() {
                content.push(json!({"type": "thinking", "thinking": thinking}));
            }
            content.push(json!({"type": "text", "text": shown}));
            let partial = assistant(&json!(content), "stop", &usage(0, 0));
            self.emit(json!({"type": "message_update", "message": partial, "assistantMessageEvent": {"type": "start", "partial": partial}}));
            if !self.pause(ms).await {
                return false;
            }
        }
        true
    }

    fn start_run(&self, prompt: &str) {
        self.abort.store(false, Ordering::SeqCst);
        self.busy.store(true, Ordering::SeqCst);
        self.ui("setHeader", json!({"key": "splash"}), "unipi.info-screen");
        self.emit(json!({"type": "agent_start"}));
        let user = json!({"role": "user", "content": prompt, "timestamp": 0});
        self.emit(json!({"type": "message_start", "message": user}));
        self.emit(json!({"type": "message_end", "message": user}));
    }

    async fn wait_dialog(&self, method: &str, fields: JsonValue, extension: &str) -> UiAnswer {
        let (tx, rx) = oneshot::channel();
        let dialog = format!("demo-ui-{}", self.seq.load(Ordering::SeqCst));
        lock(&self.waiting).insert(dialog, tx);
        self.ui(method, fields, extension);
        rx.await.unwrap_or(UiAnswer::Cancelled)
    }

    #[allow(clippy::too_many_lines, reason = "linear demo script")]
    async fn scenario(self: Arc<Self>, prompt: String, scene: Scene) {
        self.start_run(&prompt);
        if scene == Scene::Stream {
            return self.stream_forever().await;
        }
        self.ui("setStatus", json!({"statusKey": "goal", "statusText": "Ship trailing commas", "tone": "accent", "priority": 6}), "unipi.long-horizon");
        self.ui(
            "setStatus",
            json!({"statusKey": "mem", "statusText": "memory 12", "tone": "muted"}),
            "unipi.memory",
        );
        self.ui(
            "setWorkingMessage",
            json!({"message": "Planning the change…"}),
            "unipi.footer",
        );
        let start = assistant(&json!([]), "stop", &usage(0, 0));
        self.emit(json!({"type": "message_start", "message": start}));
        let thinking = "The user wants trailing commas accepted. The array loop breaks on ']' only; a comma followed by ']' fails in expect_comma_or_end. Patch the peek match and add a lookahead.";
        if !self.stream_text(&[], thinking, PLAN, 6, 14).await {
            return self.finish(true);
        }
        let mut content = vec![
            json!({"type": "thinking", "thinking": thinking}),
            json!({"type": "text", "text": PLAN}),
        ];
        if scene == Scene::Question {
            let end = assistant(&json!(content), "stop", &usage(18_400, 400));
            self.emit(json!({"type": "message_end", "message": end}));
            self.ui(
                "setWorkingMessage",
                json!({"message": "Waiting for your answer…"}),
                "unipi.footer",
            );
            let _ = self.wait_dialog("form", json!({"title": "Questions", "questions": [
                {"id": "scope", "header": "Scope", "question": "Which containers should accept trailing commas?", "options": [
                    {"label": "Arrays and objects", "description": "Matches JSON5 behaviour"},
                    {"label": "Arrays only", "description": "Smallest change"},
                    {"label": "Neither — make it an option", "description": "Off by default"}], "other": true},
                {"id": "tests", "header": "Tests", "question": "Add regression tests?", "multiSelect": true, "options": [
                    {"label": "Unit tests"}, {"label": "Fuzz corpus entry"}]}]}), "unipi.ask-user").await;
            return self.finish(false);
        }
        if scene == Scene::Plan {
            let end = assistant(&json!(content), "stop", &usage(18_400, 400));
            self.emit(json!({"type": "message_end", "message": end}));
            self.ui("setFrame", json!({"frameKey": "frame",
                "top": [{"id": "branch", "text": "v1-alpha", "icon": "branch", "tone": "muted", "priority": 6, "align": "left"},
                        {"id": "plan", "text": "PLAN", "tone": "warning", "priority": 5, "align": "right"}],
                "bottom": [{"id": "workspace", "text": "json-lite", "icon": "folder", "tone": "muted", "priority": 5, "align": "left"}]}), "unipi.footer");
            let _ = self.wait_dialog("custom", json!({"title": "Review plan  .oino/plans/trailing-commas.md", "size": "large", "spec": {"t": "stack", "direction": "vertical", "gap": 1, "children": [
                {"t": "split", "direction": "horizontal", "children": [
                    {"t": "section", "title": "Summary", "body": {"t": "markdown", "value": "Accept a trailing `,` before `]` and `}` in `parse_array` / `parse_object`."}},
                    {"t": "section", "title": "Implementation", "body": {"t": "markdown", "value": "1. Add `lookahead_is(b)` to the lexer\n2. Match `Some(b',') if lookahead` in both loops\n3. Tests: `trailing_comma_array`, `trailing_comma_object`"}}]},
                {"t": "stack", "direction": "horizontal", "gap": 2, "children": [
                    {"t": "button", "label": "1 Approve & implement", "actionId": "approve", "primary": true},
                    {"t": "button", "label": "2 Keep planning…", "actionId": "keep"},
                    {"t": "button", "label": "3 Discard plan", "actionId": "discard", "tone": "error"}]}]}}), "unipi.workflow").await;
            return self.finish(false);
        }
        // Subagent while the edit streams.
        self.emit(json!({"type": "tool_execution_start", "toolCallId": "call_agent", "toolName": "run_subagent",
            "args": {"agent": "explorer", "task": "Map the parser's call sites and existing tests"}}));
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
            self.emit(json!({"type": "message_update", "message": partial, "assistantMessageEvent": {"type": "start", "partial": partial}}));
            if !self.pause(260).await {
                return self.finish(true);
            }
            if i == 2 {
                content = c;
            }
        }
        let end = assistant(&json!(content), "toolUse", &usage(18_400, 950));
        self.emit(json!({"type": "message_end", "message": end}));
        self.emit(json!({"type": "tool_execution_start", "toolCallId": "call_edit", "toolName": "edit", "args": {"path": "src/parser.rs"}}));
        self.pause(350).await;
        self.emit(json!({"type": "tool_execution_end", "toolCallId": "call_edit", "toolName": "edit",
            "result": {"content": [{"type": "text", "text": "Successfully replaced 1 block in src/parser.rs."}], "details": {"patch": PATCH, "firstChangedLine": 44}},
            "isError": false, "durationMs": 12}));
        self.ui(
            "setWorkingMessage",
            json!({"message": "Waiting for approval…"}),
            "unipi.footer",
        );
        let approval = json!({
            "title": "Run shell command?", "toolCallId": "call_bash", "toolName": "bash", "summary": "cargo test parser",
            "detail": {"t": "code", "value": "$ cargo test parser", "language": "sh"},
            "options": [
                {"optionId": "allow", "label": "Allow once", "kind": "allow_once"},
                {"optionId": "always", "label": "Always allow in project", "kind": "allow_always"},
                {"optionId": "deny", "label": "Reject", "kind": "reject_once"},
                {"optionId": "never", "label": "Always reject", "kind": "reject_always"}],
            "feedback": {"placeholder": "Tell the agent what to do instead (optional)"}, "timeout": 120_000
        });
        let auto = std::env::var("OINO_DESKTOP_DEMO_APPROVE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|_| scene == Scene::Run);
        if let Some(ms) = auto {
            let shared = Arc::clone(&self);
            let dialog = format!("demo-ui-{}", self.seq.load(Ordering::SeqCst));
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
        let answer = self
            .wait_dialog("approval", approval, "unipi.workflow")
            .await;
        let allowed = matches!(&answer, UiAnswer::Value(v) if v.get("optionId").and_then(JsonValue::as_str).is_some_and(|o| o == "allow" || o == "always"));
        self.ui(
            "setWorkingMessage",
            json!({"message": "Running tests…"}),
            "unipi.footer",
        );
        self.emit(json!({"type": "tool_execution_start", "toolCallId": "call_bash", "toolName": "bash", "args": {"command": "cargo test parser"}}));
        if allowed {
            let lines = [
                "   \u{1b}[1;32mCompiling\u{1b}[0m json-lite v0.3.1",
                "    \u{1b}[1;32mFinished\u{1b}[0m test profile in 3.21s",
                "     \u{1b}[1;32mRunning\u{1b}[0m unittests src/lib.rs",
                "running 14 tests",
                "test parser::arrays ... \u{1b}[32mok\u{1b}[0m",
                "test parser::objects ... \u{1b}[32mok\u{1b}[0m",
                "test parser::trailing_comma_array ... \u{1b}[32mok\u{1b}[0m",
                "test parser::trailing_comma_object ... \u{1b}[32mok\u{1b}[0m",
                "test result: \u{1b}[32mok\u{1b}[0m. 14 passed; 0 failed",
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
        self.emit(json!({"type": "tool_execution_end", "toolCallId": "call_agent", "toolName": "run_subagent",
            "result": {"content": [{"type": "text", "text": "**Call sites:** `parse_value` → `parse_array`, `parse_object`.\n\n**Tests:** `tests/parser.rs` (12), none cover trailing commas."}]},
            "isError": false, "durationMs": 38_000}));
        let start = assistant(&json!([]), "stop", &usage(0, 0));
        self.emit(json!({"type": "message_start", "message": start}));
        if !self.stream_text(&[], "", DONE, 6, 14).await {
            return self.finish(true);
        }
        let end = assistant(
            &json!([{"type": "text", "text": DONE}]),
            "stop",
            &usage(21_300, 610),
        );
        self.emit(json!({"type": "message_end", "message": end}));
        self.emit(json!({"type": "message_start", "message": {"role": "custom", "customType": "unipi:memory-save", "content": "Saved **json-lite: trailing commas accepted in arrays and objects** to project memory.", "display": true, "details": {"oinoProvenance": {"source": "memory"}}, "timestamp": 0}}));
        self.footer(10, 39_700, 1560, 0.12);
        self.ui(
            "notify",
            json!({"message": "Tests passed — 14/14", "notifyType": "info"}),
            "unipi.notify",
        );
        self.finish(false);
    }

    async fn stream_forever(&self) {
        let start = assistant(&json!([]), "stop", &usage(0, 0));
        self.emit(json!({"type": "message_start", "message": start}));
        // `OINO_DESKTOP_DEMO_STREAM_REPEAT`: reply length in LOREM paragraphs (default 400).
        let repeat = std::env::var("OINO_DESKTOP_DEMO_STREAM_REPEAT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);
        let text = LOREM.repeat(repeat);
        let _ = self.stream_text(&[], "", &text, 4, 8).await;
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

    fn open_board(&self) {
        let selected = lock(&self.board_selected).clone();
        self.ui("openPanel", json!({"panelId": "unipi.kanboard/board", "placement": "sidePane", "title": "Kanboard · oino",
            "spec": board_spec(&selected), "focus": true, "closeOnEscape": true,
            "keys": [{"key": "r", "actionId": "refresh", "label": "Refresh"}, {"key": "a", "actionId": "add", "label": "Add to Backlog"}, {"key": "s", "actionId": "show", "label": "Show in chat"}]}), "unipi.kanboard");
    }

    fn open_tray(&self) {
        self.ui("openPanel", json!({"panelId": "unipi.core/tray", "placement": "editorReplace", "title": "Work tray", "focus": true, "closeOnEscape": true,
            "keys": [{"key": "tab", "actionId": "next", "label": "Next tab"}],
            "spec": {"t": "tabs", "active": "subagents", "onChange": "$tab", "tabs": [
                {"id": "subagents", "title": "Subagents", "body": {"t": "list", "selectable": "open", "items": [{"id": "explorer", "label": "explorer", "detail": "Map the parser's call sites", "badge": "running", "tone": "warning"}]}},
                {"id": "background", "title": "Background", "body": {"t": "empty", "message": "No background tasks"}},
                {"id": "dream", "title": "Dream", "body": {"t": "empty", "message": "Last dream 2h ago"}}]}}), "unipi.core");
    }

    fn open_info(&self) {
        let tab = lock(&self.info_tab).clone();
        self.ui("openPanel", json!({"panelId": "unipi.info-screen/dashboard", "placement": "overlay", "title": "Info", "spec": info_spec(&tab), "focus": true, "closeOnEscape": true,
            "keys": [{"key": "r", "actionId": "refresh", "label": "Refresh"}]}), "unipi.info-screen");
    }

    fn ui_action(&self, panel: &str, action: &str, value: Option<&JsonValue>) {
        match (panel, action) {
            (_, "$close") => {
                self.ui("closePanel", json!({"panelId": panel}), "unipi.core");
            }
            ("unipi.kanboard/board", "select") => {
                if let Some(id) = value.and_then(JsonValue::as_str) {
                    id.clone_into(&mut lock(&self.board_selected));
                }
                let selected = lock(&self.board_selected).clone();
                self.ui(
                    "updatePanel",
                    json!({"panelId": panel, "spec": board_spec(&selected)}),
                    "unipi.kanboard",
                );
            }
            ("unipi.kanboard/board", a) if a.starts_with("move:") => {
                self.ui("notify", json!({"message": format!("Moved {} to {}", lock(&self.board_selected), &a[5..])}), "unipi.kanboard");
            }
            ("unipi.info-screen/dashboard", "$tab") => {
                if let Some(t) = value.and_then(JsonValue::as_str) {
                    t.clone_into(&mut lock(&self.info_tab));
                }
                let tab = lock(&self.info_tab).clone();
                self.ui(
                    "updatePanel",
                    json!({"panelId": panel, "spec": info_spec(&tab)}),
                    "unipi.info-screen",
                );
            }
            _ => {
                self.ui(
                    "notify",
                    json!({"message": format!("{panel}: {action}")}),
                    "unipi.core",
                );
            }
        }
    }

    fn completions(text: &str) -> JsonValue {
        let token = text.rsplit(char::is_whitespace).next().unwrap_or_default();
        let from = text.chars().count() - token.chars().count();
        let groups = match token.chars().next() {
            Some('@') => json!([{"source": "files", "from": from, "items": [
                {"value": "@src/parser.rs", "description": "412 lines"}, {"value": "@src/lexer.rs"}, {"value": "@tests/parser.rs"}, {"value": "@src/", "tag": "dir"}]}]),
            Some('$') => json!([{"source": "skills", "from": from, "items": [
                {"value": "$tdd", "description": "Red-green-refactor loop", "tag": "skill", "tone": "c8"},
                {"value": "$rust-best-practices", "description": "Idiomatic Rust checklist", "tag": "skill", "tone": "c8"}]}]),
            Some('#') => json!([{"source": "unipi.command-enchantment", "from": from, "items": [
                {"value": "#kanboard-todo", "label": "Kanboard TODO summary", "tag": "session", "tone": "c11"}]}]),
            Some('/') => json!([{"source": "commands", "from": 0, "items": [
                {"value": "/compact", "description": "Compact the context"}, {"value": "/unipi:info", "description": "Session dashboard", "tag": "unipi", "tone": "c4"},
                {"value": "/unipi:kanboard", "description": "Open the board", "tag": "unipi", "tone": "c4"}, {"value": "/skill:tdd", "description": "TDD skill", "tag": "skill", "tone": "c8"}]}]),
            _ => json!([]),
        };
        json!({"groups": groups})
    }
}

impl SessionBackend for DemoBackend {
    #[allow(clippy::too_many_lines, reason = "one arm per demo command")]
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        let shared = Arc::clone(&self.shared);
        let id = self.id.clone();
        Box::pin(async move {
            let command = match command {
                RpcCommand::Oino(oino) => {
                    return Ok(match oino {
                        OinoCommand::UiSnapshot => Some(json!({"requests": []})),
                        OinoCommand::GetCompletions { text, .. } => {
                            Some(Shared::completions(&text))
                        }
                        OinoCommand::UiAction {
                            panel_id,
                            action_id,
                            value,
                        } => {
                            shared.ui_action(&panel_id, &action_id, value.as_ref());
                            None
                        }
                        OinoCommand::InvokeShortcut { shortcut_id } => {
                            if shortcut_id == "unipi.info-screen.open" {
                                shared.open_info();
                            } else {
                                shared.ui(
                                    "notify",
                                    json!({"message": format!("Shortcut {shortcut_id}")}),
                                    "unipi.input-shortcuts",
                                );
                            }
                            None
                        }
                        OinoCommand::EditorEvent(e) => {
                            let consumed =
                                e.intent == oino_core::session::EditorIntent::EmptyArrowDown;
                            if consumed {
                                shared.open_tray();
                            }
                            Some(json!({"consumed": consumed}))
                        }
                        OinoCommand::GetSettingsSchema => {
                            Some(json!({"namespaces": settings_schema()}))
                        }
                        OinoCommand::GetExtensionSettings { namespace, .. } => {
                            Some(json!({"values": match namespace.as_str() {
                                "notify" => json!({"events": {"agentEnd": true, "ask": false}, "platform": "ntfy", "ntfy": {"topic": "oino-dev", "token": "••••••"}}),
                                "workflow" => json!({"mode": "ask"}),
                                _ => json!({}),
                            }}))
                        }
                        OinoCommand::AuthProviders => Some(json!({"providers": [
                            {"id": "oino", "name": "Oino Router", "oauth": false, "configured": true, "stored": "api_key", "env": ["OINO_API_KEY"]},
                            {"id": "anthropic", "name": "Anthropic", "oauth": true, "configured": true, "stored": "oauth", "env": ["ANTHROPIC_API_KEY"]},
                            {"id": "openai-codex", "name": "ChatGPT (Codex)", "oauth": true, "configured": false, "env": []},
                            {"id": "github-copilot", "name": "GitHub Copilot", "oauth": true, "configured": false, "env": []}]})),
                        _ => None,
                    });
                }
                RpcCommand::Pi(c) => c,
            };
            Ok(match command {
                PiCommand::Prompt { message, .. } => {
                    if shared.busy.load(Ordering::SeqCst) {
                        shared.emit(
                            json!({"type": "queue_update", "steering": [message], "followUp": []}),
                        );
                    } else if message.starts_with("/unipi:kanboard") {
                        shared.open_board();
                    } else if message.starts_with("/unipi:info") {
                        shared.open_info();
                    } else if message.starts_with('/') {
                        shared.ui(
                            "notify",
                            json!({"message": format!("Ran {message}")}),
                            "unipi.core",
                        );
                    } else if message.starts_with("stream") {
                        // Soak driver: a long streamed reply (`OINO_DESKTOP_SOAK`).
                        tokio::spawn(Arc::clone(&shared).scenario(message, Scene::Stream));
                    } else {
                        tokio::spawn(Arc::clone(&shared).scenario(message, Scene::Run));
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
                PiCommand::GetTree => Some(tree()),
                PiCommand::GetForkMessages => Some(
                    json!({"messages": [{"entryId": "a0000001", "text": "Make the JSON parser accept trailing commas"}]}),
                ),
                PiCommand::GetCommands => Some(json!({"commands": [
                    {"name": "compact", "description": "Compact the context", "source": "builtin", "sourceInfo": null},
                    {"name": "login", "description": "Log in to a provider", "source": "builtin", "sourceInfo": null},
                    {"name": "model", "description": "Select a model", "source": "builtin", "sourceInfo": null},
                    {"name": "unipi:info", "description": "Session dashboard", "source": "extension", "sourceInfo": null},
                    {"name": "unipi:kanboard", "description": "Open the kanboard", "source": "extension", "sourceInfo": null},
                    {"name": "unipi:permission", "description": "Permission mode — ask · auto · full", "source": "extension", "sourceInfo": null},
                    {"name": "unipi:plan", "description": "Plan mode", "source": "extension", "sourceInfo": null},
                    {"name": "unipi:settings", "description": "Open the extension settings hub", "source": "extension", "sourceInfo": null},
                    {"name": "skill:tdd", "description": "Red-green-refactor loop for a feature or bug", "source": "skill", "sourceInfo": null},
                    {"name": "skill:rust-best-practices", "description": "Idiomatic Rust checklist", "source": "skill", "sourceInfo": null}
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
                PiCommand::SetSessionName { name } => {
                    shared.emit(json!({"type": "session_info_changed", "name": name}));
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
    disabled: Mutex<Vec<String>>,
}

fn demo_name(id: &str) -> String {
    match id {
        "oino.router" => "Oino Router".into(),
        "unipi.core" => "Core".into(),
        "unipi.workflow" => "Workflow".into(),
        "unipi.footer" => "Footer".into(),
        "unipi.info-screen" => "Info screen".into(),
        "unipi.kanboard" => "Kanboard".into(),
        "unipi.memory" => "Memory".into(),
        "unipi.mcp" => "MCP".into(),
        other => other.into(),
    }
}

impl DemoHost {
    /// A demo host (preferences kept in memory; `OINO_DESKTOP_THEME=light|dark` for shots).
    #[must_use]
    pub(crate) fn new(agent_dir: std::path::PathBuf, default_cwd: std::path::PathBuf) -> Self {
        let mut settings = AppSettings::default();
        match std::env::var("OINO_DESKTOP_THEME").as_deref() {
            Ok("light") => settings.theme = ThemePref::Light,
            Ok("dark") => settings.theme = ThemePref::Dark,
            _ => {}
        }
        Self {
            agent_dir,
            default_cwd,
            next: AtomicU64::new(1),
            settings: Mutex::new(settings),
            disabled: Mutex::new(Vec::new()),
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
                "d-5",
                "/home/oi/Projects/Personal/unipi",
                "Dream report review",
                30 * 3_600_000,
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
            OpenRequest::New { cwd } => (
                cwd,
                DemoBackend::new(&format!("demo-new-{n}"), Vec::new()),
                None,
            ),
            OpenRequest::Resume { path, cwd } => {
                let id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (cwd, DemoBackend::new(&id, history()), Some(path))
            }
        };
        let scene = Scene::from_env();
        let panes = std::env::var("OINO_DESKTOP_PANES")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1);
        let first = file.is_none() && (n == 1 || (scene == Scene::Stream && n <= panes));
        let prompt = std::env::var("OINO_DESKTOP_DEMO_PROMPT").unwrap_or_else(|_| {
            "Make the JSON parser accept trailing commas and run the tests".into()
        });
        let label = cwd
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let shared = Arc::clone(&backend.shared);
            let fresh = file.is_none();
            tokio::spawn(async move {
                // Let the pane subscribe before the first events.
                tokio::time::sleep(Duration::from_millis(400)).await;
                shared.ambient(&label, fresh);
                if !first {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
                match scene {
                    Scene::Welcome => {}
                    Scene::Board => shared.open_board(),
                    Scene::Info => shared.open_info(),
                    s => shared.scenario(prompt, s).await,
                }
            });
            Ok(OpenedSession {
                backend: Arc::new(backend),
                cwd: cwd.display().to_string(),
                file,
            })
        })
    }

    fn extensions(&self) -> BoxFuture<'static, Vec<ExtensionInfo>> {
        let list = [
            ("oino.anthropic", "provider", None),
            ("oino.openai-compat", "provider", None),
            ("oino.openai-responses", "provider", None),
            ("oino.router", "provider", None),
            ("unipi.core", "extension", None),
            ("unipi.workflow", "extension", None),
            ("unipi.footer", "extension", None),
            ("unipi.info-screen", "extension", None),
            ("unipi.kanboard", "extension", None),
            ("unipi.memory", "extension", None),
            (
                "unipi.mcp",
                "extension",
                Some("mcp.json: no servers configured"),
            ),
        ]
        .into_iter()
        .map(|(id, kind, error)| {
            let mut info =
                ExtensionInfo::basic(id, "1.0.0-alpha.2", kind, true, error.map(str::to_owned));
            info.name = demo_name(id);
            info.description = format!("Bundled {kind}");
            info.permissions = match id {
                "unipi.workflow" => vec!["session control".into(), "ui".into()],
                "unipi.memory" => vec!["fs (agent dir)".into(), "ui".into()],
                "unipi.mcp" => vec!["process".into(), "network".into()],
                "oino.router" => vec!["credentials".into(), "network".into()],
                _ => vec!["ui".into()],
            };
            info.enabled = !lock(&self.disabled).contains(&id.to_owned());
            info
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

    fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<(), HostError> {
        let mut disabled = lock(&self.disabled);
        disabled.retain(|d| d != id);
        if !enabled {
            disabled.push(id.to_owned());
        }
        Ok(())
    }

    fn extensions_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.agent_dir.join("extensions"))
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
        assert!(model.transcript.get("call_agent").is_some());
        assert_eq!(model.ui.statuses().len(), 2);
        assert!(model.ui.footer_segments().count() >= 4);
        assert!(model.usage.output > 0);
    }

    /// Panel actions round-trip: the kanboard selection re-renders the board.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn kanboard_actions_update_the_panel() {
        use oino_client::FrontendApi as _;
        let backend = DemoBackend::new("k", Vec::new());
        let mut events = backend.events();
        backend.shared.open_board();
        backend
            .ui_action("unipi.kanboard/board", "select", Some(json!("OIN-14")))
            .await
            .unwrap();
        let mut model = SessionModel::new("k", "/w");
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(5), events.next())
                .await
                .unwrap()
                .unwrap();
            model.apply(&event);
        }
        let panel = model.ui.panel("unipi.kanboard/board").unwrap();
        let board = oino_ui::model::kanban::recognise(&panel.spec).unwrap();
        assert!(
            board
                .lanes
                .iter()
                .flat_map(|l| &l.cards)
                .any(|c| c.id == "OIN-14" && c.selected)
        );
        let groups = backend.completions("fix @src", 8).await.unwrap();
        assert_eq!(groups[0].source, "files");
    }
}
