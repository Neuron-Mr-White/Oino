//! The gated DevTools endpoint (UNI-296): what agents connect to instead of
//! the browser's raw `--remote-debugging-port`.
//!
//! A tiny proxy on `127.0.0.1:<own port>` per running browser:
//!
//! - HTTP (`/json/version`, `/json/list`, `/json/new` …) is passed to the
//!   browser with the browser's port rewritten to ours in the reply, so a
//!   client that discovers its websocket URL there stays on the gate;
//! - a websocket (`/devtools/browser/…`, `/devtools/page/…`) is piped to the
//!   same path on the browser, and every client → browser command goes
//!   through [`AgentGate::check`]: while the user has taken over, input and
//!   navigation are answered with a CDP error instead of being forwarded,
//!   and while it isn't they mark the agent as driving.
//! - UNI-293: once past the take-over check, every agent's input is turned
//!   into a human-like sequence by [`rewrite`] before it reaches the
//!   browser — a real mouse path instead of a teleporting cursor,
//!   per-character typing instead of `insertText`, eased wheel notches
//!   instead of one jump — and `Browser.close` never kills the whole
//!   browser, only the targets this connection itself created
//!   ([`Target.closeTarget`]).
//!
//! The connection counts as one agent ([`AgentGate::client`]) while open.
//! `unipi-host browser start` prints this endpoint as the one to use.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

use super::agent::{AgentGate, TAKEN_OVER_CODE};
use super::humanize::{self, Humanize, KeyStep};

/// The CDP error an agent gets for a refused command (same id/session).
pub fn refusal(command: &Value, message: &str) -> Option<Value> {
    let id = command.get("id")?.clone();
    let mut reply = json!({ "id": id, "error": { "code": TAKEN_OVER_CODE, "message": message } });
    if let Some(session) = command.get("sessionId") {
        reply["sessionId"] = session.clone();
    }
    Some(reply)
}

/// Checks one client → browser text message: `None` = forward it,
/// `Some(reply)` = answer the client with `reply` instead.
pub fn filter(gate: &AgentGate, text: &str) -> Option<String> {
    let command: Value = serde_json::from_str(text).ok()?;
    let method = command.get("method")?.as_str()?;
    match gate.check(method) {
        Ok(()) => None,
        Err(message) => {
            tracing::info!(
                op = "browser_gate",
                method,
                "refused: the user has taken over"
            );
            refusal(&command, message).map(|r| r.to_string())
        }
    }
}

/// `/json*` replies name the browser's own port: point them at the gate.
pub fn rewrite_ports(body: &str, upstream: u16, ours: u16) -> String {
    body.replace(
        &format!("127.0.0.1:{upstream}"),
        &format!("127.0.0.1:{ours}"),
    )
    .replace(
        &format!("localhost:{upstream}"),
        &format!("127.0.0.1:{ours}"),
    )
}

// ------------------------------------------------------------- humanizing

/// One command to send to the browser before the final step, with the
/// delay (ms) to wait before sending it.
#[derive(Debug, Clone, PartialEq)]
pub struct Timed {
    pub command: Value,
    pub wait_ms: f64,
}

/// What ends a rewritten sequence.
#[derive(Debug, Clone, PartialEq)]
pub enum Finish {
    /// Forward `command` (keeping the client's id) after waiting `wait_ms`.
    Forward { command: Value, wait_ms: f64 },
    /// Answer the client directly with `value`; nothing more goes to the browser.
    Reply(Value),
}

/// What the relay does with one client → browser command.
#[derive(Debug, Clone, PartialEq)]
pub enum Rewrite {
    /// Forward the original message, unchanged, byte for byte.
    Forward,
    /// Send `synthetic` in order (each a fire-and-wait-for-reply command,
    /// its reply dropped, not forwarded to the client), then `finish`.
    Replace {
        synthetic: Vec<Timed>,
        finish: Finish,
    },
}

fn with_session(mut value: Value, session: Option<&Value>) -> Value {
    if let Some(session) = session {
        value["sessionId"] = session.clone();
    }
    value
}

/// Decides what one agent → browser command becomes (UNI-293's "Build"
/// list): pure given the command JSON, the humanizer (`None` = `off`, pass
/// everything through), and this connection's own targets (for scoping
/// `Browser.close`). No I/O, so it's unit-tested directly; [`Relay::run`]
/// executes the decision over the real websockets.
pub fn rewrite(
    command: &Value,
    human: Option<&mut humanize::Humanizer>,
    own_targets: &[String],
) -> Rewrite {
    let Some(method) = command.get("method").and_then(Value::as_str) else {
        return Rewrite::Forward;
    };
    let id = command.get("id").cloned().unwrap_or(Value::Null);
    let session = command.get("sessionId");
    let params = command.get("params").cloned().unwrap_or_else(|| json!({}));

    // Scoped close: regardless of the humanize setting, an agent's
    // `Browser.close` never kills the whole browser.
    if method == "Browser.close" {
        let synthetic = own_targets
            .iter()
            .map(|target| Timed { command: json!({ "method": "Target.closeTarget", "params": { "targetId": target } }), wait_ms: 0.0 })
            .collect();
        return Rewrite::Replace {
            synthetic,
            finish: Finish::Reply(json!({ "id": id, "result": {} })),
        };
    }

    let Some(human) = human else {
        return Rewrite::Forward;
    };

    match method {
        "Input.dispatchMouseEvent" => {
            let kind = params.get("type").and_then(Value::as_str).unwrap_or("");
            let dx = params.get("deltaX").and_then(Value::as_f64).unwrap_or(0.0);
            let dy = params.get("deltaY").and_then(Value::as_f64).unwrap_or(0.0);
            if kind == "mouseWheel" && (dx != 0.0 || dy != 0.0) {
                let (steps_x, steps_y) = (human.wheel(dx), human.wheel(dy));
                let n = steps_x.len().max(steps_y.len());
                let synthetic = (0..n)
                    .map(|i| {
                        let (ddx, wx) = steps_x
                            .get(i)
                            .map(|s| (s.delta, s.wait_ms))
                            .unwrap_or((0.0, 0.0));
                        let (ddy, wy) = steps_y
                            .get(i)
                            .map(|s| (s.delta, s.wait_ms))
                            .unwrap_or((0.0, 0.0));
                        let mut p = params.clone();
                        p["deltaX"] = json!(ddx);
                        p["deltaY"] = json!(ddy);
                        Timed {
                            command: with_session(
                                json!({ "method": method, "params": p }),
                                session,
                            ),
                            wait_ms: wx.max(wy),
                        }
                    })
                    .collect();
                return Rewrite::Replace {
                    synthetic,
                    finish: Finish::Reply(json!({ "id": id, "result": {} })),
                };
            }
            let (Some(x), Some(y)) = (
                params.get("x").and_then(Value::as_f64),
                params.get("y").and_then(Value::as_f64),
            ) else {
                return Rewrite::Forward;
            };
            match kind {
                "mouseMoved" => {
                    let synthetic = move_steps(human, x, y, session);
                    Rewrite::Replace {
                        synthetic,
                        finish: Finish::Forward {
                            command: command.clone(),
                            wait_ms: 0.0,
                        },
                    }
                }
                "mousePressed" => {
                    let moved = (human.pointer().0 - x).hypot(human.pointer().1 - y) > 0.5;
                    let synthetic = if moved {
                        move_steps(human, x, y, session)
                    } else {
                        Vec::new()
                    };
                    let wait = human.rand_range(human.cfg.click_aim_delay_button);
                    Rewrite::Replace {
                        synthetic,
                        finish: Finish::Forward {
                            command: command.clone(),
                            wait_ms: wait,
                        },
                    }
                }
                "mouseReleased" => {
                    human.set_pointer((x, y));
                    let wait = human.rand_range(human.cfg.click_hold_button);
                    Rewrite::Replace {
                        synthetic: Vec::new(),
                        finish: Finish::Forward {
                            command: command.clone(),
                            wait_ms: wait,
                        },
                    }
                }
                _ => Rewrite::Forward,
            }
        }
        "Input.insertText" => {
            let text = params
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let plan = human.typing(&text);
            let synthetic = plan
                .into_iter()
                .map(|step| match step {
                    KeyStep::Key { params, wait_ms } => Timed {
                        command: with_session(
                            json!({ "method": "Input.dispatchKeyEvent", "params": params }),
                            session,
                        ),
                        wait_ms,
                    },
                    KeyStep::Text { text, wait_ms } => Timed {
                        command: with_session(
                            json!({ "method": "Input.insertText", "params": { "text": text } }),
                            session,
                        ),
                        wait_ms,
                    },
                })
                .collect();
            Rewrite::Replace {
                synthetic,
                finish: Finish::Reply(json!({ "id": id, "result": {} })),
            }
        }
        _ => Rewrite::Forward,
    }
}

fn move_steps(
    human: &mut humanize::Humanizer,
    x: f64,
    y: f64,
    session: Option<&Value>,
) -> Vec<Timed> {
    human
        .move_to((x, y))
        .into_iter()
        .map(|step| {
            let params = json!({ "type": "mouseMoved", "x": step.x, "y": step.y, "button": "none", "clickCount": 0 });
            Timed { command: with_session(json!({ "method": "Input.dispatchMouseEvent", "params": params }), session), wait_ms: step.wait_ms }
        })
        .collect()
}

// --------------------------------------------------------------- the relay

/// Synthetic command ids the relay invents for the browser (one step of a
/// path, one key event, one wheel notch, one scoped `Target.closeTarget`
/// …): never collide with a real client id (CDP ids are small incrementing
/// integers in every client we've seen), and their replies are dropped —
/// never forwarded to the client.
const SYNTHETIC_ID_BASE: u64 = 1_000_000_000;

/// How long a synthetic command may take before the relay gives up waiting
/// on it and moves on (a slow/odd reply must never wedge a client's command).
const SYNTHETIC_TIMEOUT: Duration = Duration::from_secs(5);

type Waiters = Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>;

/// One agent connection's relay state. Commands from this client are
/// processed strictly in order (the next one is only read once the
/// current one — including any synthetic sequence — is done); other
/// clients run on their own `Relay` and aren't blocked by this one.
struct Relay {
    write_tx: tokio::sync::mpsc::UnboundedSender<Message>,
    reply_tx: tokio::sync::mpsc::UnboundedSender<Message>,
    waiters: Waiters,
    /// Ids of this connection's own in-flight `Target.createTarget` calls.
    create_ids: Arc<Mutex<HashSet<u64>>>,
    /// Targets this connection created — all `Browser.close` may close.
    own_targets: Arc<Mutex<HashSet<String>>>,
    gate: Arc<AgentGate>,
    human: Option<humanize::Humanizer>,
    next_synthetic: u64,
}

impl Relay {
    fn next_id(&mut self) -> u64 {
        let id = SYNTHETIC_ID_BASE + self.next_synthetic;
        self.next_synthetic += 1;
        id
    }

    async fn pace(wait_ms: f64) {
        if wait_ms > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(wait_ms / 1000.0)).await;
        }
    }

    /// Sends a synthetic command (own id) and waits for (and drops) its reply.
    async fn send_synthetic(&mut self, mut command: Value, wait_ms: f64) {
        Self::pace(wait_ms).await;
        let id = self.next_id();
        command["id"] = json!(id);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        if self
            .write_tx
            .send(Message::Text(command.to_string().into()))
            .is_err()
        {
            return;
        }
        self.gate.note_input();
        let _ = tokio::time::timeout(SYNTHETIC_TIMEOUT, rx).await;
    }

    /// Forwards the client's own command (its id, so its reply reaches it
    /// normally through the plain passthrough); remembers `Target.createTarget`
    /// calls so their resulting target is recorded as this connection's.
    fn forward(&mut self, command: &Value) {
        if command.get("method").and_then(Value::as_str) == Some("Target.createTarget") {
            if let Some(id) = command.get("id").and_then(Value::as_u64) {
                self.create_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id);
            }
        }
        let _ = self
            .write_tx
            .send(Message::Text(command.to_string().into()));
    }

    fn reply(&self, value: Value) {
        let _ = self.reply_tx.send(Message::Text(value.to_string().into()));
    }

    /// Runs one decision: the synthetic sequence, then the finishing step.
    async fn apply(&mut self, decision: Rewrite, original: &Value) {
        match decision {
            Rewrite::Forward => self.forward(original),
            Rewrite::Replace { synthetic, finish } => {
                for step in synthetic {
                    self.send_synthetic(step.command, step.wait_ms).await;
                }
                match finish {
                    Finish::Forward { command, wait_ms } => {
                        Self::pace(wait_ms).await;
                        self.forward(&command);
                    }
                    Finish::Reply(value) => self.reply(value),
                }
            }
        }
    }
}

/// A running gate: its port. The task ends when `closed` resolves.
pub async fn serve(
    gate: Arc<AgentGate>,
    upstream: u16,
    humanize: Humanize,
    closed: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let ours = listener.local_addr()?.port();
    tokio::spawn(async move {
        tokio::pin!(closed);
        loop {
            tokio::select! {
                _ = &mut closed => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let gate = gate.clone();
                    tokio::spawn(async move {
                        if let Err(error) = connection(gate, stream, upstream, ours, humanize).await {
                            tracing::debug!(op = "browser_gate", %error, "connection ended");
                        }
                    });
                }
            }
        }
    });
    Ok(ours)
}

/// The request head (without consuming it) once complete.
async fn peek_head(stream: &TcpStream) -> anyhow::Result<String> {
    let mut buf = vec![0u8; 16 * 1024];
    for _ in 0..200 {
        let n = stream.peek(&mut buf).await?;
        if n == 0 {
            anyhow::bail!("closed before a request");
        }
        if let Some(end) = buf[..n].windows(4).position(|w| w == b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&buf[..end + 4]).into_owned());
        }
        if n == buf.len() {
            anyhow::bail!("request head too large");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    anyhow::bail!("request head timed out")
}

fn is_upgrade(head: &str) -> bool {
    head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("upgrade:") && l.contains("websocket")
    })
}

async fn connection(
    gate: Arc<AgentGate>,
    stream: TcpStream,
    upstream: u16,
    ours: u16,
    humanize: Humanize,
) -> anyhow::Result<()> {
    let head = peek_head(&stream).await?;
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (
        first.next().unwrap_or("GET").to_string(),
        first.next().unwrap_or("/").to_string(),
    );
    if !path.starts_with('/') {
        anyhow::bail!("bad request path");
    }
    if is_upgrade(&head) {
        websocket(gate, stream, upstream, &path, humanize).await
    } else {
        http(stream, &head, &method, &path, upstream, ours).await
    }
}

async fn http(
    mut stream: TcpStream,
    head: &str,
    method: &str,
    path: &str,
    upstream: u16,
    ours: u16,
) -> anyhow::Result<()> {
    // Consume the head we peeked (DevTools requests carry no body).
    let mut sink = vec![0u8; head.len()];
    stream.read_exact(&mut sink).await?;
    let mut up = TcpStream::connect(("127.0.0.1", upstream)).await?;
    up.write_all(
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\nConnection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .await?;
    let mut reply = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), read_response(&mut up, &mut reply)).await;
    let split = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(reply.len());
    let (up_head, body) = reply.split_at(split);
    let status = String::from_utf8_lossy(up_head)
        .lines()
        .next()
        .unwrap_or("HTTP/1.1 502 Bad Gateway")
        .to_string();
    let body = rewrite_ports(&String::from_utf8_lossy(body), upstream, ours);
    let content_type = String::from_utf8_lossy(up_head)
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
        .map(|l| l.to_string())
        .unwrap_or_else(|| "Content-Type: application/json; charset=UTF-8".into());
    let out = format!(
        "{status}\r\n{content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(out.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Reads one HTTP response (Content-Length or until close).
async fn read_response(up: &mut TcpStream, out: &mut Vec<u8>) -> std::io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        let n = up.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        out.extend_from_slice(&buf[..n]);
        if let Some(end) = out.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&out[..end]).to_ascii_lowercase();
            if let Some(len) = head.lines().find_map(|l| {
                l.strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse::<usize>().ok())
            }) {
                if out.len() >= end + 4 + len {
                    out.truncate(end + 4 + len);
                    return Ok(());
                }
            }
        }
    }
}

async fn websocket(
    gate: Arc<AgentGate>,
    stream: TcpStream,
    upstream: u16,
    path: &str,
    humanize_setting: Humanize,
) -> anyhow::Result<()> {
    let mut config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    config.max_message_size = Some(256 << 20);
    config.max_frame_size = Some(256 << 20);
    let client = tokio_tungstenite::accept_async_with_config(stream, Some(config)).await?;
    let (browser, _) = tokio_tungstenite::connect_async_with_config(
        format!("ws://127.0.0.1:{upstream}{path}"),
        Some(config),
        true,
    )
    .await?;
    let _agent = gate.client();
    tracing::info!(op = "browser_gate", path, "agent connected");
    let (mut client_tx, mut client_rx) = client.split();
    let (mut browser_tx, mut browser_rx) = browser.split();

    // The browser-bound writer: a channel so the relay's synthetic commands
    // and the client's forwarded ones share one ordered pipe to the browser.
    let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let writer = async move {
        while let Some(message) = write_rx.recv().await {
            if browser_tx.send(message).await.is_err() {
                break;
            }
        }
        let _ = browser_tx.close().await;
    };

    // Everything bound for the client (events, forwarded replies, refusals,
    // "finish" replies for a rewritten sequence), in the order queued.
    let (reply_tx, mut reply_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let waiters: Waiters = Arc::default();
    let create_ids: Arc<Mutex<HashSet<u64>>> = Arc::default();
    let own_targets: Arc<Mutex<HashSet<String>>> = Arc::default();

    // The browser → relay reader: matches a synthetic command's reply
    // (dropped, not forwarded), learns this connection's created targets,
    // forwards everything else untouched.
    let reader_waiters = waiters.clone();
    let reader_create_ids = create_ids.clone();
    let reader_own_targets = own_targets.clone();
    let reader_reply_tx = reply_tx.clone();
    let reader = async move {
        while let Some(Ok(message)) = browser_rx.next().await {
            let Message::Text(text) = &message else {
                let _ = reader_reply_tx.send(message);
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                let _ = reader_reply_tx.send(message);
                continue;
            };
            if let Some(id) = value.get("id").and_then(Value::as_u64) {
                let waiter = reader_waiters
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                if let Some(waiter) = waiter {
                    let _ = waiter.send(value);
                    continue;
                }
                if reader_create_ids
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id)
                {
                    if let Some(target) = value
                        .get("result")
                        .and_then(|r| r.get("targetId"))
                        .and_then(Value::as_str)
                    {
                        reader_own_targets
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(target.to_string());
                    }
                }
            }
            let _ = reader_reply_tx.send(message);
        }
    };

    let down = async move {
        while let Some(reply) = reply_rx.recv().await {
            if client_tx.send(reply).await.is_err() {
                break;
            }
        }
        let _ = client_tx.close().await;
    };

    let human = humanize_setting.config().map(humanize::Humanizer::new);
    let mut relay = Relay {
        write_tx,
        reply_tx: reply_tx.clone(),
        waiters,
        create_ids,
        own_targets: own_targets.clone(),
        gate: gate.clone(),
        human,
        next_synthetic: 0,
    };

    let up = async move {
        while let Some(Ok(message)) = client_rx.next().await {
            let Message::Text(text) = &message else {
                if matches!(message, Message::Close(_)) {
                    break;
                }
                continue;
            };
            let Ok(command) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            let Some(method) = command
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            if let Err(message) = relay.gate.check(&method) {
                tracing::info!(
                    op = "browser_gate",
                    method = method.as_str(),
                    "refused: the user has taken over"
                );
                if let Some(reply) = refusal(&command, message) {
                    relay.reply(reply);
                }
                continue;
            }
            let targets: Vec<String> = relay
                .own_targets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned()
                .collect();
            let decision = rewrite(&command, relay.human.as_mut(), &targets);
            if matches!(
                decision,
                Rewrite::Replace {
                    finish: Finish::Reply(_),
                    ..
                }
            ) && method == "Browser.close"
            {
                relay
                    .own_targets
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
            }
            relay.apply(decision, &command).await;
        }
    };
    tokio::select! {
        _ = writer => {}
        _ = reader => {}
        _ = down => {}
        _ = up => {}
    }
    tracing::info!(op = "browser_gate", path, "agent disconnected");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::agent::TAKEN_OVER_MESSAGE;
    use rand::{rngs::StdRng, SeedableRng};
    use unipi_proto::BrowserAgent;

    fn humanizer() -> humanize::Humanizer {
        humanize::Humanizer::with_rng(
            humanize::HumanConfig::default_preset(),
            StdRng::seed_from_u64(7),
        )
    }

    #[test]
    fn refusal_keeps_id_and_session() {
        let cmd = json!({ "id": 7, "method": "Input.dispatchMouseEvent", "sessionId": "S1", "params": {} });
        let r = refusal(&cmd, TAKEN_OVER_MESSAGE).unwrap();
        assert_eq!(
            r,
            json!({ "id": 7, "sessionId": "S1", "error": { "code": -32000, "message": TAKEN_OVER_MESSAGE } })
        );
        assert!(refusal(&json!({ "method": "x" }), "m").is_none());
    }

    #[test]
    fn filter_passes_until_taken_over() {
        let gate = AgentGate::new();
        let click =
            r#"{"id":1,"method":"Input.dispatchMouseEvent","params":{"type":"mousePressed"}}"#;
        assert!(filter(&gate, click).is_none());
        assert!(gate.snapshot(0).driving);
        gate.set_takeover(true);
        let refused: Value = serde_json::from_str(&filter(&gate, click).unwrap()).unwrap();
        assert_eq!(refused["id"], 1);
        assert_eq!(refused["error"]["message"], TAKEN_OVER_MESSAGE);
        assert!(filter(&gate, r#"{"id":2,"method":"Runtime.evaluate","params":{}}"#).is_none());
        assert!(filter(&gate, "not json").is_none());
    }

    #[test]
    fn rewrite_without_a_humanizer_only_scopes_close() {
        let click = json!({ "id": 1, "method": "Input.dispatchMouseEvent", "params": { "type": "mouseMoved", "x": 10.0, "y": 10.0 } });
        assert_eq!(rewrite(&click, None, &[]), Rewrite::Forward);
        let text = json!({ "id": 2, "method": "Input.insertText", "params": { "text": "hi" } });
        assert_eq!(rewrite(&text, None, &[]), Rewrite::Forward);
        let close = json!({ "id": 3, "method": "Browser.close", "params": {} });
        let Rewrite::Replace { synthetic, finish } =
            rewrite(&close, None, &["T1".into(), "T2".into()])
        else {
            panic!()
        };
        assert_eq!(synthetic.len(), 2, "one Target.closeTarget per own target");
        assert_eq!(finish, Finish::Reply(json!({ "id": 3, "result": {} })));
    }

    #[test]
    fn mouse_move_becomes_a_path_then_the_original() {
        let mut human = humanizer();
        let cmd = json!({ "id": 5, "method": "Input.dispatchMouseEvent", "sessionId": "S1", "params": { "type": "mouseMoved", "x": 300.0, "y": 300.0 } });
        let Rewrite::Replace { synthetic, finish } = rewrite(&cmd, Some(&mut human), &[]) else {
            panic!()
        };
        assert!(synthetic.len() > 5, "{} steps", synthetic.len());
        for step in &synthetic {
            assert_eq!(step.command["method"], "Input.dispatchMouseEvent");
            assert_eq!(step.command["params"]["type"], "mouseMoved");
            assert_eq!(step.command["sessionId"], "S1");
            assert!(
                step.command.get("id").is_none(),
                "the executor assigns synthetic ids"
            );
        }
        let last = synthetic.last().unwrap();
        assert_eq!(
            (
                last.command["params"]["x"].as_f64(),
                last.command["params"]["y"].as_f64()
            ),
            (Some(300.0), Some(300.0))
        );
        assert_eq!(
            finish,
            Finish::Forward {
                command: cmd,
                wait_ms: 0.0
            }
        );
    }

    #[test]
    fn mouse_press_waits_and_keeps_its_own_id() {
        let mut human = humanizer();
        human.move_to((300.0, 300.0)); // position the tracked pointer where we'll press
        let cmd = json!({ "id": 9, "method": "Input.dispatchMouseEvent", "params": { "type": "mousePressed", "x": 300.0, "y": 300.0, "button": "left" } });
        let Rewrite::Replace { synthetic, finish } = rewrite(&cmd, Some(&mut human), &[]) else {
            panic!()
        };
        assert!(synthetic.is_empty(), "already at the point: no extra path");
        let Finish::Forward { command, wait_ms } = finish else {
            panic!()
        };
        assert_eq!(command, cmd, "kept exactly, including its id");
        assert!(wait_ms > 0.0);
    }

    #[test]
    fn wheel_becomes_steps_and_the_id_is_answered_directly() {
        let mut human = humanizer();
        let cmd = json!({ "id": 11, "method": "Input.dispatchMouseEvent", "params": { "type": "mouseWheel", "x": 0, "y": 0, "deltaX": 0.0, "deltaY": 500.0 } });
        let Rewrite::Replace { synthetic, finish } = rewrite(&cmd, Some(&mut human), &[]) else {
            panic!()
        };
        assert!(synthetic.len() > 5);
        let sum: f64 = synthetic
            .iter()
            .map(|s| s.command["params"]["deltaY"].as_f64().unwrap())
            .sum();
        assert_eq!(sum, 500.0, "the total scroll is exact");
        assert_eq!(finish, Finish::Reply(json!({ "id": 11, "result": {} })));
    }

    #[test]
    fn insert_text_becomes_key_events_answered_directly() {
        let mut human = humanizer();
        let cmd = json!({ "id": 13, "method": "Input.insertText", "params": { "text": "hi" } });
        let Rewrite::Replace { synthetic, finish } = rewrite(&cmd, Some(&mut human), &[]) else {
            panic!()
        };
        assert_eq!(synthetic.len(), 4, "keyDown/keyUp for 'h' and 'i'");
        assert!(synthetic
            .iter()
            .all(|s| s.command["method"] == "Input.dispatchKeyEvent"));
        assert_eq!(finish, Finish::Reply(json!({ "id": 13, "result": {} })));

        // Non-ASCII falls back to insertText for that character.
        let cmd = json!({ "id": 14, "method": "Input.insertText", "params": { "text": "é" } });
        let Rewrite::Replace { synthetic, finish } = rewrite(&cmd, Some(&mut human), &[]) else {
            panic!()
        };
        assert_eq!(synthetic.len(), 1);
        assert_eq!(synthetic[0].command["method"], "Input.insertText");
        assert_eq!(synthetic[0].command["params"]["text"], "é");
        assert_eq!(finish, Finish::Reply(json!({ "id": 14, "result": {} })));
    }

    /// A fake browser: `/json/version` names its own port; its websocket
    /// answers every command with `{id, result:{method}}` and records it.
    async fn fake_browser() -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let head = peek_head(&stream).await.unwrap();
                    if is_upgrade(&head) {
                        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                        while let Some(Ok(Message::Text(text))) = ws.next().await {
                            let v: Value = serde_json::from_str(text.as_str()).unwrap();
                            let method = v["method"].as_str().unwrap_or("").to_string();
                            log.lock().unwrap().push(method.clone());
                            let reply = json!({ "id": v["id"], "result": { "method": method } });
                            ws.send(Message::Text(reply.to_string().into()))
                                .await
                                .unwrap();
                        }
                    } else {
                        let mut sink = vec![0u8; head.len()];
                        stream.read_exact(&mut sink).await.unwrap();
                        let body = format!(
                            r#"{{"Browser":"Fake/1","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/browser/abc"}}"#
                        );
                        let out = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                        stream.write_all(out.as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (port, seen)
    }

    async fn call(
        ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        id: u64,
        method: &str,
    ) -> Value {
        ws.send(Message::Text(
            json!({ "id": id, "method": method, "params": {} })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        loop {
            let Some(Ok(Message::Text(text))) =
                tokio::time::timeout(Duration::from_secs(5), ws.next())
                    .await
                    .unwrap()
            else {
                panic!("closed")
            };
            let v: Value = serde_json::from_str(text.as_str()).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    }

    #[tokio::test]
    async fn a_fake_agent_through_the_gate_is_tracked_and_paused_by_a_takeover() {
        let (upstream, seen) = fake_browser().await;
        let gate = AgentGate::new();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let port = serve(gate.clone(), upstream, Humanize::Off, async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();

        // Discovery over HTTP stays on the gate.
        let mut http = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        http.write_all(b"GET /json/version HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut body = String::new();
        http.read_to_string(&mut body).await.unwrap();
        assert!(body.starts_with("HTTP/1.1 200"), "{body}");
        let ws_url = format!("ws://127.0.0.1:{port}/devtools/browser/abc");
        assert!(body.contains(&ws_url), "{body}");
        assert!(!body.contains(&format!(":{upstream}/")), "{body}");

        // An agent connects: connected, not driving yet.
        let (mut agent, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
        for _ in 0..50 {
            if gate.snapshot(0).connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            gate.snapshot(0),
            BrowserAgent {
                connected: true,
                driving: false,
                taken_over: false
            }
        );
        // A read passes and doesn't drive.
        assert_eq!(
            call(&mut agent, 1, "Runtime.evaluate").await["result"]["method"],
            "Runtime.evaluate"
        );
        assert!(!gate.snapshot(0).driving);
        // Input passes and marks it driving.
        assert_eq!(
            call(&mut agent, 2, "Input.dispatchMouseEvent").await["result"]["method"],
            "Input.dispatchMouseEvent"
        );
        assert!(gate.snapshot(0).driving);

        // The user takes over: input and navigation are refused with the message, reads still pass.
        gate.set_takeover(true);
        let refused = call(&mut agent, 3, "Input.insertText").await;
        assert_eq!(
            refused["error"]["message"],
            crate::browser::agent::TAKEN_OVER_MESSAGE
        );
        assert_eq!(refused["error"]["code"], TAKEN_OVER_CODE);
        assert_eq!(
            call(&mut agent, 4, "Page.navigate").await["error"]["message"],
            crate::browser::agent::TAKEN_OVER_MESSAGE
        );
        assert_eq!(
            call(&mut agent, 5, "DOM.getDocument").await["result"]["method"],
            "DOM.getDocument"
        );
        assert_eq!(
            gate.snapshot(0),
            BrowserAgent {
                connected: true,
                driving: false,
                taken_over: true
            }
        );

        // Give back: input flows again.
        gate.set_takeover(false);
        assert_eq!(
            call(&mut agent, 6, "Input.dispatchKeyEvent").await["result"]["method"],
            "Input.dispatchKeyEvent"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "Runtime.evaluate",
                "Input.dispatchMouseEvent",
                "DOM.getDocument",
                "Input.dispatchKeyEvent"
            ],
            "refused commands never reached the browser"
        );

        // Disconnect: no longer connected.
        agent.close(None).await.unwrap();
        drop(agent);
        for _ in 0..50 {
            if !gate.snapshot(0).connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!gate.snapshot(0).connected);
        let _ = stop_tx.send(());
    }

    #[tokio::test]
    async fn browser_close_from_an_agent_only_closes_its_own_targets() {
        let (upstream, seen) = fake_browser().await;
        let gate = AgentGate::new();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let port = serve(gate.clone(), upstream, Humanize::Default, async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
        let (mut agent, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/devtools/browser/abc"))
                .await
                .unwrap();

        // The fake browser just echoes {id, result:{method}} — it has no
        // targetId, so this checks only that Browser.close never reaches it.
        agent
            .send(Message::Text(
                json!({ "id": 1, "method": "Browser.close", "params": {} })
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), agent.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Message::Text(text) = reply else { panic!() };
        let v: Value = serde_json::from_str(text.as_str()).unwrap();
        assert_eq!(v, json!({ "id": 1, "result": {} }));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !seen.lock().unwrap().contains(&"Browser.close".to_string()),
            "never forwarded to the browser"
        );
        let _ = stop_tx.send(());
    }

    #[test]
    fn ports_are_rewritten() {
        let body = r#"{"webSocketDebuggerUrl": "ws://127.0.0.1:41234/devtools/browser/x", "devtoolsFrontendUrl": "/devtools/inspector.html?ws=localhost:41234/devtools/page/y"}"#;
        let out = rewrite_ports(body, 41234, 5555);
        assert!(out.contains("ws://127.0.0.1:5555/devtools/browser/x"));
        assert!(out.contains("ws=127.0.0.1:5555/devtools/page/y"));
        assert!(!out.contains("41234"));
    }
}
