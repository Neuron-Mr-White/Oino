//! Chats: one `oino --mode rpc` child per chat, translated to/from the
//! app-bridge line protocol (docs/m5/PROTOCOL.md §3) that the casing's chat
//! screens speak over `chat_open` streams.
//!
//! Bridge → app lines we emit: `hello`, `state`, `session`, `entry`,
//! `msg_start`, `delta`, `msg_end`, `tool_start`, `tool_update`, `tool_end`,
//! `input`, `ack`, `queue`, `notify`, `error`, `sessions`, `files`.
//! App → bridge requests we handle: `prompt`, `abort`, `set_model`,
//! `set_thinking`, `compact`, `resync`, `sessions`, `files`, `history`,
//! `session_new`, `session_resume`, `session_rename`. Everything else answers
//! an error the app shows as unsupported.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;

use crate::rpc::RpcChild;
use crate::{Bridge, error, internal, read_frame, write_control};
use unipi_proto::chat::{ChatCloseData, ChatNewData};
use unipi_proto::{AppErrorCode, AppErrorDto, Control, MuxKind};
use unipi_proto::chat::ChatClosed;

/// One live chat.
pub struct Chat {
    pub pid: u32,
    pub rpc: Arc<RpcChild>,
    /// Bridge lines out to every open chat_open stream.
    lines: broadcast::Sender<String>,
    /// Display fields for chat_list.
    info: Mutex<ChatInfo>,
    /// Cached `hello` (rebuilt on connect and resync).
    hello: Mutex<Value>,
    /// True once the oino child has exited.
    gone: Mutex<bool>,
}

#[derive(Clone, Default)]
struct ChatInfo {
    session_name: Option<String>,
    cwd: String,
    running: bool,
}

#[derive(Default)]
pub struct Chats {
    chats: Mutex<HashMap<u32, Arc<Chat>>>,
}

impl Chats {
    /// `chat_list`: one fake herdr session ("oino") with one workspace/tab/pane
    /// per live chat child.
    pub fn list(&self) -> Value {
        let chats = self.chats.lock().unwrap();
        let mut panes = Vec::new();
        for (i, chat) in chats.values().enumerate() {
            let info = chat.info.lock().unwrap().clone();
            let label = info
                .session_name
                .clone()
                .unwrap_or_else(|| info.cwd.rsplit('/').next().unwrap_or("chat").to_owned());
            let status = if info.running { "working" } else { "idle" };
            panes.push(json!({
                "id": format!("p{}", chat.pid),
                "focused": i == 0,
                "cwd": info.cwd,
                "title": label,
                "agent": {"name": "pi", "status": status, "session": {"kind": "file", "value": ""}},
                "chat": {
                    "pid": chat.pid,
                    "session_name": info.session_name,
                    "cwd": info.cwd,
                    "running": info.running,
                },
            }));
        }
        let ws_status = if panes.iter().any(|p| p["chat"]["running"].as_bool().unwrap_or(false)) {
            "working"
        } else {
            "idle"
        };
        json!({
            "herdr": {
                "available": true,
                "sessions": [{
                    "name": "oino",
                    "running": true,
                    "default": true,
                    "workspaces": [{
                        "id": "oino",
                        "label": "Oino",
                        "number": 1,
                        "focused": true,
                        "agent_status": ws_status,
                        "tab_count": panes.len(),
                        "pane_count": panes.len(),
                        "tabs": [{
                            "id": "oino",
                            "label": "Oino",
                            "number": 1,
                            "focused": true,
                            "agent_status": ws_status,
                            "panes": panes,
                        }],
                    }],
                }],
            },
            "protected": [],
        })
    }

    /// `chat_new`: spawn a child, wait until RPC answers `get_state`.
    pub async fn new_chat(
        &self,
        bin: &Path,
        agent_dir: &Path,
        model: Option<&str>,
        _session: String,
        cwd: &str,
    ) -> std::io::Result<ChatNewData> {
        let dir = Path::new(cwd);
        if !dir.is_dir() {
            return Err(std::io::Error::other(format!("{cwd} is not a folder")));
        }
        let rpc = RpcChild::spawn(bin, agent_dir, model, dir)?;
        let pid = rpc.pid;
        // Readiness: get_state must answer.
        let state = tokio::time::timeout(std::time::Duration::from_secs(20), rpc.request(json!({"type": "get_state"})))
            .await
            .map_err(|_| std::io::Error::other("oino did not start in 20s"))?
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let (lines_tx, _) = broadcast::channel(256);
        let chat = Arc::new(Chat {
            pid,
            rpc: Arc::clone(&rpc),
            lines: lines_tx,
            info: Mutex::new(ChatInfo { session_name: None, cwd: cwd.to_owned(), running: false }),
            hello: Mutex::new(Value::Null),
            gone: Mutex::new(false),
        });
        *chat.hello.lock().unwrap() = build_hello(&state, None, None, None, None, cwd);
        if let Some(name) = state["sessionName"].as_str() {
            chat.info.lock().unwrap().session_name = Some(name.to_owned());
        }
        self.chats.lock().unwrap().insert(pid, Arc::clone(&chat));
        spawn_event_pump(Arc::clone(&rpc), Arc::clone(&chat));
        Ok(ChatNewData {
            pid,
            mux: MuxKind::Herdr,
            session: "oino".into(),
            workspace_id: Some("oino".into()),
            tab_id: Some("oino".into()),
            pane_id: format!("p{pid}"),
            cwd: Some(cwd.to_owned()),
        })
    }

    /// `chat_open`: `ok`, then relay bridge lines out / phone requests in.
    pub async fn open<R: AsyncReadExt + Unpin, W: AsyncWriteExt + Unpin + Send + 'static>(
        &self,
        pid: u32,
        mut read: R,
        write: Arc<tokio::sync::Mutex<W>>,
    ) -> Result<(), AppErrorDto> {
        let chat = self
            .chats
            .lock()
            .unwrap()
            .get(&pid)
            .cloned()
            .ok_or_else(|| error(AppErrorCode::E_NOT_FOUND, "That chat is gone. Pull to refresh."))?;
        {
            let mut guard = write.lock().await;
            write_control(&mut *guard, &Control::Ok { data: None })
                .await
                .map_err(internal)?;
        }
        // Fresh hello on every (re)connect.
        let hello = refresh_hello(&chat).await;
        {
            let mut guard = write.lock().await;
            send_line(&mut *guard, &hello_line(&chat, &hello))
                .await
                .map_err(internal)?;
        }

        let mut rx = chat.lines.subscribe();
        let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);
        // Out: bridge lines.
        let writer_out = Arc::clone(&write);
        let out = tokio::spawn(async move {
            loop {
                tokio::select! {
                    line = rx.recv() => {
                        match line {
                            Ok(line) => {
                                let mut guard = writer_out.lock().await;
                                if send_line(&mut *guard, &line).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = done_rx.recv() => break,
                }
            }
        });
        // In: phone → pi lines.
        loop {
            let frame = tokio::select! {
                frame = read_frame(&mut read) => frame,
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                    if *chat.gone.lock().unwrap() {
                        break;
                    }
                    continue;
                }
            };
            let Ok(Some((kind, bytes))) = frame else { break };
            if kind != 0 {
                continue;
            }
            let Ok(Control::Chat { line }) = serde_json::from_slice::<Control>(&bytes) else {
                continue;
            };
            if let Some(reply) = handle_phone_line(&chat, &line).await {
                let mut guard = write.lock().await;
                let _ = send_line(&mut *guard, &reply).await;
            }
        }
        let _ = done_tx.send(()).await;
        out.abort();
        Ok(())
    }

    /// `chat_close` (Tauri `chat_stop`).
    pub async fn close(&self, pid: u32) -> std::io::Result<ChatCloseData> {
        let chat = self.chats.lock().unwrap().remove(&pid);
        match chat {
            Some(chat) => {
                chat.rpc.stop().await;
                Ok(ChatCloseData { stopped: true, closed: ChatClosed::Pane })
            }
            None => Ok(ChatCloseData { stopped: false, closed: ChatClosed::None }),
        }
    }
}

async fn send_line<W: AsyncWriteExt + Unpin>(write: &mut W, line: &str) -> std::io::Result<()> {
    write_control(write, &Control::Chat { line: line.to_owned() }).await
}

fn hello_line(chat: &Chat, hello: &Value) -> String {
    let mut hello = hello.clone();
    if let Some(obj) = hello.as_object_mut() {
        obj.insert("pid".into(), json!(chat.pid));
    }
    serde_json::to_string(&hello).unwrap_or_else(|_| "{}".into())
}

/// Builds a `hello` bridge message from `get_state` (+optional snapshots).
fn build_hello(
    state: &Value,
    entries: Option<Value>,
    commands: Option<Value>,
    models: Option<Value>,
    stats: Option<Value>,
    cwd: &str,
) -> Value {
    let session = json!({
        "file": state["sessionFile"],
        "id": state["sessionId"],
        "name": state["sessionName"],
        "cwd": cwd,
    });
    let mut run = json!({
        "running": state["isStreaming"].as_bool().unwrap_or(false),
        "model": model_info(&state["model"]),
        "thinking": state["thinkingLevel"],
    });
    if let Some(stats) = &stats {
        run["context"] = stats["context"].clone();
        run["cost"] = stats["cost"].clone();
    }
    json!({
        "t": "hello",
        "v": 1,
        "session": session,
        "state": run,
        "entries": entries.unwrap_or(json!([])),
        "commands": commands.unwrap_or(json!([])),
        "models": models.unwrap_or(json!([])),
        "dialogs": [],
        "queue": [],
        "work": [],
    })
}

/// Rebuilds the cached hello from live RPC snapshots.
async fn refresh_hello(chat: &Arc<Chat>) -> Value {
    let state = chat.rpc.request(json!({"type": "get_state"})).await.unwrap_or(json!({}));
    let entries = chat
        .rpc
        .request(json!({"type": "get_entries"}))
        .await
        .map(|v| v["entries"].clone())
        .unwrap_or(json!([]));
    let commands = chat
        .rpc
        .request(json!({"type": "get_commands"}))
        .await
        .map(|v| v["commands"].clone())
        .unwrap_or(json!([]));
    let models = chat
        .rpc
        .request(json!({"type": "get_available_models"}))
        .await
        .map(|v| {
            Value::Array(
                v["models"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| model_info(&m))
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or(json!([]));
    let stats = chat
        .rpc
        .request(json!({"type": "get_session_stats"}))
        .await
        .ok()
        .map(|v| map_stats(&v));
    let cwd = chat.info.lock().unwrap().cwd.clone();
    let hello = build_hello(&state, Some(entries), Some(commands), Some(models), stats, &cwd);
    if let Some(name) = state["sessionName"].as_str() {
        chat.info.lock().unwrap().session_name = Some(name.to_owned());
    }
    chat.info.lock().unwrap().running = state["isStreaming"].as_bool().unwrap_or(false);
    *chat.hello.lock().unwrap() = hello.clone();
    hello
}

fn map_stats(stats: &Value) -> Value {
    json!({
        "context": stats.get("context").cloned().unwrap_or(Value::Null),
        "tokensIn": stats.get("inputTokens").cloned().unwrap_or(json!(0)),
        "tokensOut": stats.get("outputTokens").cloned().unwrap_or(json!(0)),
        "cacheHit": 0,
        "cost": stats.get("totalCost").cloned().unwrap_or(json!(0)),
        "tps": 0,
    })
}

fn model_info(model: &Value) -> Value {
    if model.is_null() {
        return Value::Null;
    }
    json!({
        "provider": model["provider"],
        "id": model["id"],
        "name": model["name"],
        "reasoning": model["reasoning"].as_bool().unwrap_or(false),
    })
}

impl Chat {
    fn broadcast(&self, line: &Value) {
        let _ = self.lines.send(serde_json::to_string(line).unwrap_or_default());
    }

    fn set_running(&self, running: bool) {
        self.info.lock().unwrap().running = running;
    }
}

/// Pump oino RPC events → bridge lines for one chat.
fn spawn_event_pump(rpc: Arc<RpcChild>, chat: Arc<Chat>) {
    tokio::spawn(async move {
        let mut events = rpc.events();
        let mut stream_id: Option<String> = None;
        let mut entry_seq: u64 = 0;
        let mut next_entry_id = || {
            entry_seq += 1;
            format!("l{entry_seq:06x}")
        };
        loop {
            let Ok(event) = events.recv().await else { break };
            let Some(kind) = event["type"].as_str() else { continue };
            match kind {
                "agent_start" => {
                    chat.set_running(true);
                    chat.broadcast(&json!({"t": "state", "running": true}));
                }
                "agent_settled" => {
                    chat.set_running(false);
                    chat.broadcast(&json!({"t": "state", "running": false}));
                }
                "message_start" => {
                    let role = event["message"]["role"].as_str().unwrap_or("");
                    if role == "assistant" {
                        let id = next_entry_id();
                        stream_id = Some(id.clone());
                        chat.broadcast(&json!({"t": "msg_start", "id": id}));
                    }
                }
                "message_update" => {
                    let Some(id) = stream_id.clone() else { continue };
                    let inner = &event["assistantMessageEvent"];
                    let Some(what) = inner["type"].as_str() else { continue };
                    let (bridge_kind, index, text) = match what {
                        "text_delta" => (
                            "text",
                            inner["contentIndex"].as_u64().unwrap_or(0),
                            inner["delta"].as_str().unwrap_or(""),
                        ),
                        "thinking_delta" => (
                            "thinking",
                            inner["contentIndex"].as_u64().unwrap_or(0),
                            inner["delta"].as_str().unwrap_or(""),
                        ),
                        _ => continue,
                    };
                    chat.broadcast(&json!({"t": "delta", "id": id, "kind": bridge_kind, "index": index, "text": text}));
                }
                "message_end" => {
                    let message = event["message"].clone();
                    let role = message["role"].as_str().unwrap_or("").to_owned();
                    if role == "assistant" && stream_id.is_some() {
                        chat.broadcast(&json!({"t": "msg_end", "id": stream_id.clone().unwrap()}));
                        stream_id = None;
                    }
                    if matches!(role.as_str(), "user" | "assistant" | "toolResult" | "bashExecution" | "custom") {
                        let entry = json!({
                            "type": "message",
                            "id": next_entry_id(),
                            "timestamp": message.get("timestamp").cloned().unwrap_or(json!(0)),
                            "message": message,
                        });
                        chat.broadcast(&json!({"t": "entry", "entry": entry}));
                    }
                }
                "tool_execution_start" => {
                    chat.broadcast(&json!({
                        "t": "tool_start",
                        "callId": event["toolCallId"],
                        "name": event["toolName"],
                        "args": event["args"],
                    }));
                }
                "tool_execution_update" => {
                    chat.broadcast(&json!({
                        "t": "tool_update",
                        "callId": event["toolCallId"],
                        "text": tool_tail(&event["partialResult"]),
                    }));
                }
                "tool_execution_end" => {
                    chat.broadcast(&json!({
                        "t": "tool_end",
                        "callId": event["toolCallId"],
                        "isError": event["isError"].as_bool().unwrap_or(false),
                    }));
                }
                "queue_update" => {
                    let mut items = Vec::new();
                    for (i, text) in event["steering"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                        items.push(json!({"id": format!("tui-s{i}"), "text": text, "mode": "steer", "source": "tui", "editable": false}));
                    }
                    for (i, text) in event["followUp"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                        items.push(json!({"id": format!("tui-f{i}"), "text": text, "mode": "followUp", "source": "tui", "editable": false}));
                    }
                    chat.broadcast(&json!({"t": "queue", "items": items}));
                }
                "session_info_changed" => {
                    let name = event["name"].clone();
                    chat.info.lock().unwrap().session_name = name.as_str().map(str::to_owned);
                    chat.broadcast(&json!({
                        "t": "session",
                        "id": Value::Null,
                        "name": name,
                        "cwd": chat.info.lock().unwrap().cwd,
                        "reason": "rename",
                    }));
                }
                "thinking_level_changed" => {
                    chat.broadcast(&json!({"t": "state", "thinking": event["level"]}));
                }
                "entry_appended" => {
                    chat.broadcast(&json!({"t": "entry", "entry": event["entry"]}));
                }
                "compaction_end" => {
                    chat.broadcast(&json!({"t": "entry", "entry": {
                        "type": "compaction",
                        "id": next_entry_id(),
                        "summary": event["result"].get("summary").cloned().unwrap_or(Value::Null),
                    }}));
                }
                "extension_ui_request" => {
                    // `notify` is fire-and-forget and renderable; dialogs are
                    // auto-cancelled by the RPC server for pi clients.
                    if event["method"].as_str() == Some("notify") {
                        chat.broadcast(&json!({"t": "notify", "text": event["title"].as_str().unwrap_or(""), "level": "info"}));
                    }
                }
                "extension_error" => {
                    chat.broadcast(&json!({
                        "t": "notify",
                        "text": event["message"].as_str().unwrap_or("extension error"),
                        "level": "error",
                    }));
                }
                _ => {}
            }
        }
        *chat.gone.lock().unwrap() = true;
        chat.set_running(false);
        chat.broadcast(&json!({"t": "error", "message": "The oino session ended"}));
    });
}

/// Last ~2 KB of a tool result's text.
fn tool_tail(result: &Value) -> String {
    let text = match &result["content"] {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    text.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect()
}

/// One phone → pi line. Returns the direct reply line (ack/error/reply), if any.
async fn handle_phone_line(chat: &Arc<Chat>, line: &str) -> Option<String> {
    let Ok(msg) = serde_json::from_str::<Value>(line) else {
        return Some(json!({"t": "error", "message": "unreadable line"}).to_string());
    };
    let t = msg["t"].as_str().unwrap_or("").to_owned();
    let r#ref = msg["ref"].as_str().map(str::to_owned);
    let ack = |r: &Option<String>| json!({"t": "ack", "ref": r}).to_string();
    let error_reply = |message: String, r: &Option<String>| json!({"t": "error", "message": message, "ref": r}).to_string();
    match t.as_str() {
        "prompt" => {
            let text = msg["text"].as_str().unwrap_or("").to_owned();
            let mode = msg["mode"].as_str().unwrap_or("auto").to_owned();
            let running = chat.info.lock().unwrap().running;
            let (command_type, bridge_mode) = match mode.as_str() {
                "steer" => ("steer", "steer"),
                "followUp" => ("follow_up", "followUp"),
                "now" => ("prompt", "prompt"),
                "after" => ("follow_up", "followUp"),
                _ => {
                    if running {
                        ("steer", "steer")
                    } else {
                        ("prompt", "prompt")
                    }
                }
            };
            let request = json!({"type": command_type, "message": text});
            match chat.rpc.request(request).await {
                Ok(_) => {
                    if let Some(r) = &r#ref {
                        chat.broadcast(&json!({"t": "ack", "ref": r}));
                    }
                    chat.broadcast(&json!({
                        "t": "input",
                        "text": msg["text"],
                        "source": "rpc",
                        "mode": bridge_mode,
                        "ref": r#ref,
                    }));
                    None
                }
                Err(e) => Some(error_reply(e.to_string(), &r#ref)),
            }
        }
        "abort" => match chat.rpc.request(json!({"type": "abort"})).await {
            Ok(_) => None,
            Err(e) => Some(error_reply(e.to_string(), &r#ref)),
        },
        "set_model" => {
            let provider = msg["provider"].as_str().unwrap_or_default().to_owned();
            let model = msg["model"].as_str().unwrap_or_default().to_owned();
            match chat.rpc.request(json!({"type": "set_model", "provider": provider, "modelId": model})).await {
                Ok(_) => Some(ack(&r#ref)),
                Err(e) => Some(error_reply(e.to_string(), &r#ref)),
            }
        }
        "set_thinking" => {
            let level = msg["level"].clone();
            match chat.rpc.request(json!({"type": "set_thinking_level", "level": level})).await {
                Ok(_) => Some(ack(&r#ref)),
                Err(e) => Some(error_reply(e.to_string(), &r#ref)),
            }
        }
        "compact" => {
            let request = json!({"type": "compact", "customInstructions": msg["instructions"]});
            match chat.rpc.request(request).await {
                Ok(_) => Some(ack(&r#ref)),
                Err(e) => Some(error_reply(e.to_string(), &r#ref)),
            }
        }
        "resync" | "session_new" | "session_resume" => {
            let request = match t.as_str() {
                "session_resume" => json!({"type": "switch_session", "sessionPath": msg["path"]}),
                "session_new" => json!({"type": "new_session"}),
                _ => json!({"type": "get_state"}),
            };
            match chat.rpc.request(request).await {
                Ok(_) => {
                    let hello = refresh_hello(chat).await;
                    Some(hello_line(chat, &hello))
                }
                Err(e) => {
                    let mut reply = json!({"t": "error", "message": e.0, "ref": r#ref});
                    if e.0.contains("busy") || e.0.to_lowercase().contains("running") {
                        reply["code"] = json!("busy");
                    }
                    Some(reply.to_string())
                }
            }
        }
        "session_rename" => {
            let name = msg["name"].as_str().unwrap_or_default().to_owned();
            match chat.rpc.request(json!({"type": "set_session_name", "name": name})).await {
                Ok(_) => Some(ack(&r#ref)),
                Err(e) => Some(error_reply(e.to_string(), &r#ref)),
            }
        }
        "sessions" => {
            let items = scan_sessions();
            Some(json!({"t": "sessions", "items": items, "more": false, "ref": r#ref}).to_string())
        }
        "files" => {
            // No `@` finder in the spike: an empty answer (the app copes).
            Some(json!({"t": "files", "query": msg["query"], "items": [], "ref": r#ref}).to_string())
        }
        "history" => {
            // Full history ships in hello; nothing older to page in.
            Some(json!({"t": "history", "before": msg["before"], "entries": [], "more": false, "ref": r#ref}).to_string())
        }
        other => Some(error_reply(format!("{other} is not in the oino casing spike yet"), &r#ref)),
    }
}

/// Sessions to resume, from the isolated agent dir (pi layout:
/// `<agent>/sessions/--<cwd>--/<ts>_<id>.jsonl`).
fn scan_sessions() -> Vec<Value> {
    let home = std::env::var("HOME").unwrap_or_default();
    let agent = std::env::var("OINO_CASING_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(format!("{home}/.oino-casing")))
        .join("agent");
    let sessions = agent.join("sessions");
    let mut out: Vec<Value> = Vec::new();
    let Ok(dirs) = std::fs::read_dir(&sessions) else { return out };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for file in files.flatten() {
            if file.path().extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let path = file.path();
            let mut name = None;
            let mut first_message = String::new();
            let mut count = 0u64;
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    count += 1;
                    let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
                    if name.is_none() && (value["type"] == "session_meta" || value["type"] == "sessionMeta") {
                        name = value["name"].as_str().map(str::to_owned);
                    }
                    if first_message.is_empty()
                        && value["type"] == "message"
                        && value["message"]["role"] == "user"
                    {
                        let content = &value["message"]["content"];
                        first_message = content.as_str().unwrap_or("").to_owned();
                        if first_message.is_empty() && let Some(blocks) = content.as_array() {
                            first_message = blocks
                                .iter()
                                .filter_map(|b| b["text"].as_str())
                                .collect::<Vec<_>>()
                                .join(" ");
                        }
                    }
                }
            }
            let cwd = dir
                .file_name()
                .to_string_lossy()
                .trim_start_matches("--")
                .trim_end_matches("--")
                .replace("--", "/");
            let modified = file
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            out.push(json!({
                "path": path.display().to_string(),
                "id": path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                "name": name,
                "cwd": format!("/{cwd}"),
                "firstMessage": first_message.chars().take(120).collect::<String>(),
                "modified": modified,
                "messageCount": count,
                "current": false,
            }));
        }
    }
    out.sort_by(|a, b| b["modified"].as_u64().unwrap_or(0).cmp(&a["modified"].as_u64().unwrap_or(0)));
    out.truncate(50);
    out
}

impl Bridge {
    #[allow(dead_code)]
    pub fn chats(&self) -> &Chats {
        &self.chats
    }
}
