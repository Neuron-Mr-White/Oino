//! [`RpcServer`]: pi `rpc-mode.ts` over any [`SessionBackend`].
//!
//! Each input line is handled as soon as it arrives (pi does not serialise commands, so
//! `abort` reaches a running `compact`); a command whose work completes without waiting
//! answers before the next line is read, like pi's synchronous handlers.
//!
//! One writer task owns the output and the backend's event stream. Before it writes a
//! reply it drains every event already queued, so events a command caused precede its
//! reply (pi writes events synchronously while the command runs). A `prompt` reply is
//! written before that run's `agent_start` (pi `preflightResult` fires before the agent
//! starts).

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Duration;

use futures::stream::{BoxStream, FuturesUnordered};
use futures::{Future, FutureExt, StreamExt};
use oino_ai::JsonValue;
use oino_client::{
    ClientError, OinoCommand, PiCommand, RpcCommand, RpcRequest, RpcResponse, SessionBackend,
};
use oino_core::events::{AgentSessionEvent, ClientEvent};
use oino_core::ui::{UiAnswer, UiResponse};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::jsonl::{LineReader, serialize_line};
use crate::wire::{WireEvent, rpc_event};

/// Reply queue depth.
const REPLY_QUEUE: usize = 1024;
/// How long queued events may still be written after shutdown.
const FORWARD_GRACE: Duration = Duration::from_millis(500);
/// Longest time an `agent_start` waits for its prompt's reply.
const PREFLIGHT_HOLD: Duration = Duration::from_secs(1);

/// Failure of the server loop.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RpcServerError {
    /// Reading commands failed.
    #[error("reading commands: {0}")]
    Read(#[source] std::io::Error),
}

/// A reply for the writer.
struct Reply {
    response: RpcResponse,
    /// A `prompt` reply (releases a held `agent_start`).
    prompt: bool,
}

struct Shared {
    backend: Arc<dyn SessionBackend>,
    replies: mpsc::Sender<Reply>,
    /// `_oino/hello` was received.
    oino: Arc<AtomicBool>,
    /// Prompts whose reply is not written yet.
    preflight: Arc<AtomicUsize>,
}

impl Shared {
    async fn reply(&self, response: RpcResponse, prompt: bool) {
        let _ = self.replies.send(Reply { response, prompt }).await;
    }

    async fn handle_line(self: Arc<Self>, line: String) {
        let value: JsonValue = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                let message = format!("Failed to parse command: {error}");
                self.reply(
                    RpcResponse::error(None, Some("parse".to_owned()), message),
                    false,
                )
                .await;
                return;
            }
        };
        if value.get("type").and_then(JsonValue::as_str) == Some("extension_ui_response") {
            if let Ok(response) = serde_json::from_value::<UiResponse>(value) {
                self.backend.ui_respond(response);
            }
            return;
        }
        let request = match RpcRequest::from_value(value) {
            Ok(request) => request,
            Err(rejected) => {
                let response = RpcResponse::error(rejected.id, rejected.command, rejected.error);
                self.reply(response, false).await;
                return;
            }
        };
        let kind = request.command.kind();
        let prompt = matches!(request.command, RpcCommand::Pi(PiCommand::Prompt { .. }));
        let hello = matches!(request.command, RpcCommand::Oino(OinoCommand::Hello { .. }));
        if prompt {
            self.preflight.fetch_add(1, Ordering::AcqRel);
        }
        let request_id = request.id.as_ref().map(id_text);
        let result = self.backend.send_with_id(request_id, request.command).await;
        if hello && result.is_ok() {
            self.oino.store(true, Ordering::Release);
        }
        let response = match result {
            Ok(data) => RpcResponse::success(request.id, kind, data),
            Err(ClientError::Command(message)) => {
                RpcResponse::error(request.id, Some(kind.to_owned()), message)
            }
            Err(error) => RpcResponse::error(request.id, Some(kind.to_owned()), error.to_string()),
        };
        self.reply(response, prompt).await;
    }
}

/// The text of a JSON id as pi passes it to `executeBash` (`bash_execution_update.id`).
fn id_text(id: &JsonValue) -> String {
    match id {
        JsonValue::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Owns the output: events and replies in pi order.
struct Writer<W> {
    output: W,
    events: BoxStream<'static, ClientEvent>,
    backend: Arc<dyn SessionBackend>,
    oino: Arc<AtomicBool>,
    preflight: Arc<AtomicUsize>,
    /// `agent_start` waiting for its prompt's reply.
    held: Option<ClientEvent>,
    broken: bool,
}

fn is_agent_start(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::Session(AgentSessionEvent::AgentStart))
}

impl<W: AsyncWrite + Unpin> Writer<W> {
    async fn line(&mut self, value: &impl serde::Serialize) {
        if self.broken {
            return;
        }
        let line = serialize_line(value);
        if self.output.write_all(line.as_bytes()).await.is_err()
            || self.output.flush().await.is_err()
        {
            self.broken = true;
        }
    }

    async fn event(&mut self, event: &ClientEvent) {
        match rpc_event(event, self.oino.load(Ordering::Acquire)) {
            WireEvent::Line(value) => self.line(&value).await,
            WireEvent::Drop => {}
            WireEvent::CancelDialog(id) => self.backend.ui_respond(UiResponse {
                id,
                answer: UiAnswer::Cancelled,
            }),
        }
    }

    /// Writes queued events; for a prompt reply stops at (and holds) `agent_start`.
    async fn drain(&mut self, prompt: bool) {
        while self.held.is_none() {
            let Some(Some(event)) = self.events.next().now_or_never() else {
                return;
            };
            if prompt && is_agent_start(&event) {
                self.held = Some(event);
                return;
            }
            self.event(&event).await;
        }
    }

    async fn reply(&mut self, reply: Reply) {
        self.drain(reply.prompt).await;
        self.line(&reply.response).await;
        if reply.prompt {
            self.preflight.fetch_sub(1, Ordering::AcqRel);
            self.release().await;
        }
    }

    async fn release(&mut self) {
        if let Some(event) = self.held.take() {
            self.event(&event).await;
        }
    }

    async fn run(mut self, mut replies: mpsc::Receiver<Reply>) {
        let mut events_open = true;
        loop {
            let holding = self.held.is_some();
            tokio::select! {
                reply = replies.recv() => match reply {
                    Some(reply) => self.reply(reply).await,
                    None => break,
                },
                event = self.events.next(), if events_open && !holding => match event {
                    Some(event) if is_agent_start(&event)
                        && self.preflight.load(Ordering::Acquire) > 0 =>
                    {
                        self.held = Some(event);
                    }
                    Some(event) => self.event(&event).await,
                    None => events_open = false,
                },
                () = tokio::time::sleep(PREFLIGHT_HOLD), if holding => self.release().await,
            }
        }
        self.release().await;
        if events_open {
            let rest = async {
                while let Some(event) = self.events.next().await {
                    self.event(&event).await;
                }
            };
            let _ = tokio::time::timeout(FORWARD_GRACE, rest).await;
        }
        let _ = self.output.flush().await;
    }
}

/// pi RPC mode over a backend.
pub struct RpcServer {
    backend: Arc<dyn SessionBackend>,
}

impl std::fmt::Debug for RpcServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcServer").finish_non_exhaustive()
    }
}

type Pending = Pin<Box<dyn Future<Output = ()> + Send>>;

impl RpcServer {
    /// A server for `backend`.
    #[must_use]
    pub fn new(backend: Arc<dyn SessionBackend>) -> Self {
        Self { backend }
    }

    /// Serves commands from `input` and writes replies/events to `output` until `input`
    /// ends or an extension requested shutdown and the session is idle, then shuts the
    /// backend down (pi: stdin end ⇒ orderly shutdown, exit 0).
    ///
    /// # Errors
    ///
    /// Reading `input` failed (the backend is still shut down).
    pub async fn run<R, W>(self, input: R, output: W) -> Result<(), RpcServerError>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (replies, replies_rx) = mpsc::channel::<Reply>(REPLY_QUEUE);
        let oino = Arc::new(AtomicBool::new(false));
        let preflight = Arc::new(AtomicUsize::new(0));
        let writer = Writer {
            output,
            events: self.backend.events(),
            backend: Arc::clone(&self.backend),
            oino: Arc::clone(&oino),
            preflight: Arc::clone(&preflight),
            held: None,
            broken: false,
        };
        let writer = tokio::spawn(writer.run(replies_rx));
        let shared = Arc::new(Shared {
            backend: Arc::clone(&self.backend),
            replies,
            oino,
            preflight,
        });
        let mut reader = LineReader::new(input);
        let mut pending: FuturesUnordered<Pending> = FuturesUnordered::new();
        let mut shutdown = self.backend.shutdown_requested();
        let result = loop {
            tokio::select! {
                () = &mut shutdown => break Ok(()),
                line = reader.next_line() => match line {
                    Ok(Some(line)) => {
                        let mut handler: Pending = Box::pin(Arc::clone(&shared).handle_line(line));
                        if futures::poll!(handler.as_mut()) == Poll::Pending {
                            pending.push(handler);
                        }
                    }
                    Ok(None) => break Ok(()),
                    Err(error) => break Err(RpcServerError::Read(error)),
                },
                Some(()) = pending.next(), if !pending.is_empty() => {}
            }
        };
        drop(pending);
        self.backend.shutdown().await;
        drop(shared);
        let _ = writer.await;
        result
    }
}
