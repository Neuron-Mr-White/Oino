//! [`RpcClient`]: typed client for `oino --mode rpc` (pi `rpc-client.ts`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::StreamExt;
use futures::stream::BoxStream;
use oino_ai::{AgentMessage, ImageContent, JsonValue, Model, ThinkingLevel};
use oino_client::{
    ClientError, PiCommand, RpcCommand, RpcResponse, RpcResponseData, RpcSessionState,
};
use oino_core::compaction::CompactionResult;
use oino_core::session::{
    BashResult, CycleModelResult, ForkMessage, PromptDisposition, SessionStats, SlashCommandInfo,
    StreamingBehavior,
};
use oino_core::ui::UiResponse;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::{broadcast, oneshot};

use crate::jsonl::{LineReader, serialize_line};

/// Event buffer per subscriber.
const EVENT_BUFFER: usize = 4096;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A child process speaking RPC on its stdio (`oino --mode rpc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildCommand {
    /// Executable.
    pub program: PathBuf,
    /// Arguments (include `--mode rpc`).
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
    /// Extra environment.
    pub env: Vec<(String, String)>,
}

/// Where the server is.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// Spawn a child and talk over its stdin/stdout (stderr is inherited).
    StdioChild(ChildCommand),
}

type Writer = Box<dyn AsyncWrite + Send + Unpin>;

struct Inner {
    /// `None` once closed (dropping the pipe is what signals EOF to a child).
    writer: tokio::sync::Mutex<Option<Writer>>,
    pending: Mutex<HashMap<String, oneshot::Sender<RpcResponse>>>,
    next_id: AtomicU64,
    events: Mutex<broadcast::Receiver<JsonValue>>,
    child: Mutex<Option<Child>>,
}

/// Typed RPC client. Cheap to clone.
#[derive(Clone)]
pub struct RpcClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for RpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcClient").finish_non_exhaustive()
    }
}

/// `fork` data.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ForkOutcome {
    /// Text of the forked-from user message.
    #[serde(default)]
    pub text: Option<String>,
    /// An extension cancelled.
    pub cancelled: bool,
}

/// `clear_queue` data.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearedQueues {
    /// Steering texts.
    pub steering: Vec<String>,
    /// Follow-up texts.
    pub follow_up: Vec<String>,
}

/// `get_entries` / `get_tree` data.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntriesOutcome {
    /// Entries (`get_entries`) or tree nodes (`get_tree`), as JSON.
    #[serde(alias = "tree")]
    pub entries: Vec<JsonValue>,
    /// Current leaf.
    pub leaf_id: Option<String>,
}

#[derive(Deserialize)]
struct Disposition {
    disposition: PromptDisposition,
}

#[derive(Deserialize)]
struct Cancelled {
    cancelled: bool,
}

impl RpcClient {
    /// Connects over `transport`.
    ///
    /// # Errors
    ///
    /// The child could not be spawned.
    pub fn connect(transport: Transport) -> Result<Self, ClientError> {
        match transport {
            Transport::StdioChild(command) => {
                let mut builder = tokio::process::Command::new(&command.program);
                builder
                    .args(&command.args)
                    .envs(command.env.iter().map(|(k, v)| (k, v)))
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .kill_on_drop(true);
                if let Some(cwd) = &command.cwd {
                    builder.current_dir(cwd);
                }
                let mut child = builder
                    .spawn()
                    .map_err(|e| ClientError::Transport(e.to_string()))?;
                let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                    return Err(ClientError::Transport("child stdio unavailable".to_owned()));
                };
                let client = Self::from_io(stdout, stdin);
                *lock(&client.inner.child) = Some(child);
                Ok(client)
            }
        }
    }

    /// A client over an already connected byte stream pair.
    pub fn from_io<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (sender, receiver) = broadcast::channel(EVENT_BUFFER);
        let inner = Arc::new(Inner {
            writer: tokio::sync::Mutex::new(Some(Box::new(writer))),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events: Mutex::new(receiver),
            child: Mutex::new(None),
        });
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut lines = LineReader::new(reader);
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
                    continue;
                };
                if let Some(response) = RpcResponse::from_value(&value) {
                    let waiter = response
                        .id
                        .as_ref()
                        .and_then(JsonValue::as_str)
                        .and_then(|id| weak.upgrade().and_then(|i| lock(&i.pending).remove(id)));
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(response);
                        continue;
                    }
                }
                let _ = sender.send(value);
            }
            if let Some(inner) = weak.upgrade() {
                lock(&inner.pending).clear();
            }
        });
        Self { inner }
    }

    /// Raw wire events (everything that is not a reply to this client), from now on.
    pub fn events(&self) -> BoxStream<'static, JsonValue> {
        let receiver = lock(&self.inner.events).resubscribe();
        futures::stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => return Some((event, receiver)),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "rpc client lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }

    async fn write(&self, value: &JsonValue) -> Result<(), ClientError> {
        let line = serialize_line(value);
        let mut guard = self.inner.writer.lock().await;
        let writer = guard.as_mut().ok_or(ClientError::Closed)?;
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))
    }

    /// Sends any command and waits for its reply.
    ///
    /// # Errors
    ///
    /// [`ClientError::Command`] with the server's error text, [`ClientError::Closed`] when
    /// the server went away.
    pub async fn request(&self, command: RpcCommand) -> Result<RpcResponseData, ClientError> {
        let id = format!("req_{}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        lock(&self.inner.pending).insert(id.clone(), tx);
        let request = oino_client::RpcRequest {
            id: Some(JsonValue::String(id.clone())),
            command,
        };
        if let Err(error) = self.write(&request.to_value()).await {
            lock(&self.inner.pending).remove(&id);
            return Err(error);
        }
        let response = rx.await.map_err(|_| ClientError::Closed)?;
        response.result.map_err(ClientError::Command)
    }

    async fn typed<T: DeserializeOwned>(&self, command: PiCommand) -> Result<T, ClientError> {
        let data = self.request(command.into()).await?;
        serde_json::from_value(data.unwrap_or(JsonValue::Null))
            .map_err(|e| ClientError::Protocol(e.to_string()))
    }

    async fn unit(&self, command: PiCommand) -> Result<(), ClientError> {
        self.request(command.into()).await.map(|_| ())
    }

    /// Answers a dialog.
    ///
    /// # Errors
    ///
    /// The write failed.
    pub async fn ui_respond(&self, response: &UiResponse) -> Result<(), ClientError> {
        let value =
            serde_json::to_value(response).map_err(|e| ClientError::Protocol(e.to_string()))?;
        self.write(&value).await
    }

    /// Closes stdin (orderly server shutdown) and waits for the child to exit.
    ///
    /// # Errors
    ///
    /// Waiting for the child failed.
    pub async fn close(&self) -> Result<Option<i32>, ClientError> {
        if let Some(mut writer) = self.inner.writer.lock().await.take() {
            let _ = writer.shutdown().await;
        }
        let child = lock(&self.inner.child).take();
        match child {
            Some(mut child) => child
                .wait()
                .await
                .map(|status| status.code())
                .map_err(|e| ClientError::Transport(e.to_string())),
            None => Ok(None),
        }
    }

    /// `prompt`.
    ///
    /// # Errors
    ///
    /// pi's prompt errors.
    pub async fn prompt(
        &self,
        message: impl Into<String>,
        images: Vec<ImageContent>,
        streaming_behavior: Option<StreamingBehavior>,
    ) -> Result<PromptDisposition, ClientError> {
        let reply: Disposition = self
            .typed(PiCommand::Prompt {
                message: message.into(),
                images,
                streaming_behavior,
            })
            .await?;
        Ok(reply.disposition)
    }

    /// `steer`.
    ///
    /// # Errors
    ///
    /// Extension commands cannot be queued.
    pub async fn steer(
        &self,
        message: impl Into<String>,
        images: Vec<ImageContent>,
    ) -> Result<PromptDisposition, ClientError> {
        let reply: Disposition = self
            .typed(PiCommand::Steer {
                message: message.into(),
                images,
            })
            .await?;
        Ok(reply.disposition)
    }

    /// `follow_up`.
    ///
    /// # Errors
    ///
    /// Extension commands cannot be queued.
    pub async fn follow_up(
        &self,
        message: impl Into<String>,
        images: Vec<ImageContent>,
    ) -> Result<PromptDisposition, ClientError> {
        let reply: Disposition = self
            .typed(PiCommand::FollowUp {
                message: message.into(),
                images,
            })
            .await?;
        Ok(reply.disposition)
    }

    /// `abort`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn abort(&self) -> Result<(), ClientError> {
        self.unit(PiCommand::Abort).await
    }

    /// `clear_queue`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn clear_queue(&self) -> Result<ClearedQueues, ClientError> {
        self.typed(PiCommand::ClearQueue).await
    }

    /// `new_session`; `true` when an extension cancelled.
    ///
    /// # Errors
    ///
    /// Session file failures.
    pub async fn new_session(&self, parent_session: Option<String>) -> Result<bool, ClientError> {
        let reply: Cancelled = self.typed(PiCommand::NewSession { parent_session }).await?;
        Ok(reply.cancelled)
    }

    /// `get_state`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_state(&self) -> Result<RpcSessionState, ClientError> {
        self.typed(PiCommand::GetState).await
    }

    /// `set_model`.
    ///
    /// # Errors
    ///
    /// `Model not found: …`, missing credentials.
    pub async fn set_model(
        &self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Model, ClientError> {
        self.typed(PiCommand::SetModel {
            provider: provider.into(),
            model_id: model_id.into(),
        })
        .await
    }

    /// `cycle_model`.
    ///
    /// # Errors
    ///
    /// Persisting the selection failed.
    pub async fn cycle_model(&self) -> Result<Option<CycleModelResult>, ClientError> {
        self.typed(PiCommand::CycleModel).await
    }

    /// `get_available_models`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_available_models(&self) -> Result<Vec<Model>, ClientError> {
        #[derive(Deserialize)]
        struct Models {
            models: Vec<Model>,
        }
        let reply: Models = self.typed(PiCommand::GetAvailableModels).await?;
        Ok(reply.models)
    }

    /// `set_thinking_level`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn set_thinking_level(&self, level: ThinkingLevel) -> Result<(), ClientError> {
        self.unit(PiCommand::SetThinkingLevel { level }).await
    }

    /// `cycle_thinking_level`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn cycle_thinking_level(&self) -> Result<Option<ThinkingLevel>, ClientError> {
        #[derive(Deserialize)]
        struct Level {
            level: ThinkingLevel,
        }
        let reply: Option<Level> = self.typed(PiCommand::CycleThinkingLevel).await?;
        Ok(reply.map(|l| l.level))
    }

    /// `get_available_thinking_levels`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_available_thinking_levels(&self) -> Result<Vec<ThinkingLevel>, ClientError> {
        #[derive(Deserialize)]
        struct Levels {
            levels: Vec<ThinkingLevel>,
        }
        let reply: Levels = self.typed(PiCommand::GetAvailableThinkingLevels).await?;
        Ok(reply.levels)
    }

    /// `set_steering_mode`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn set_steering_mode(&self, mode: oino_agent::QueueMode) -> Result<(), ClientError> {
        self.unit(PiCommand::SetSteeringMode { mode }).await
    }

    /// `set_follow_up_mode`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn set_follow_up_mode(&self, mode: oino_agent::QueueMode) -> Result<(), ClientError> {
        self.unit(PiCommand::SetFollowUpMode { mode }).await
    }

    /// `compact`.
    ///
    /// # Errors
    ///
    /// pi's compaction errors.
    pub async fn compact(
        &self,
        custom_instructions: Option<String>,
    ) -> Result<CompactionResult, ClientError> {
        self.typed(PiCommand::Compact {
            custom_instructions,
        })
        .await
    }

    /// `set_auto_compaction`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn set_auto_compaction(&self, enabled: bool) -> Result<(), ClientError> {
        self.unit(PiCommand::SetAutoCompaction { enabled }).await
    }

    /// `set_auto_retry`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn set_auto_retry(&self, enabled: bool) -> Result<(), ClientError> {
        self.unit(PiCommand::SetAutoRetry { enabled }).await
    }

    /// `abort_retry`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn abort_retry(&self) -> Result<(), ClientError> {
        self.unit(PiCommand::AbortRetry).await
    }

    /// `bash`.
    ///
    /// # Errors
    ///
    /// The command could not run.
    pub async fn bash(
        &self,
        command: impl Into<String>,
        exclude_from_context: bool,
    ) -> Result<BashResult, ClientError> {
        self.typed(PiCommand::Bash {
            command: command.into(),
            exclude_from_context: exclude_from_context.then_some(true),
        })
        .await
    }

    /// `abort_bash`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn abort_bash(&self) -> Result<(), ClientError> {
        self.unit(PiCommand::AbortBash).await
    }

    /// `get_session_stats`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_session_stats(&self) -> Result<SessionStats, ClientError> {
        self.typed(PiCommand::GetSessionStats).await
    }

    /// `export_html`; returns the written path.
    ///
    /// # Errors
    ///
    /// Write failures.
    pub async fn export_html(&self, output_path: Option<String>) -> Result<String, ClientError> {
        #[derive(Deserialize)]
        struct Path {
            path: String,
        }
        let reply: Path = self.typed(PiCommand::ExportHtml { output_path }).await?;
        Ok(reply.path)
    }

    /// `switch_session`; `true` when an extension cancelled.
    ///
    /// # Errors
    ///
    /// The file cannot be opened.
    pub async fn switch_session(
        &self,
        session_path: impl Into<String>,
    ) -> Result<bool, ClientError> {
        let reply: Cancelled = self
            .typed(PiCommand::SwitchSession {
                session_path: session_path.into(),
            })
            .await?;
        Ok(reply.cancelled)
    }

    /// `fork`.
    ///
    /// # Errors
    ///
    /// `Invalid entry ID for forking`.
    pub async fn fork(&self, entry_id: impl Into<String>) -> Result<ForkOutcome, ClientError> {
        self.typed(PiCommand::Fork {
            entry_id: entry_id.into(),
        })
        .await
    }

    /// `clone`; `true` when an extension cancelled.
    ///
    /// # Errors
    ///
    /// `Cannot clone session: no current entry selected`.
    pub async fn clone_session(&self) -> Result<bool, ClientError> {
        let reply: Cancelled = self.typed(PiCommand::Clone).await?;
        Ok(reply.cancelled)
    }

    /// `get_fork_messages`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_fork_messages(&self) -> Result<Vec<ForkMessage>, ClientError> {
        #[derive(Deserialize)]
        struct Messages {
            messages: Vec<ForkMessage>,
        }
        let reply: Messages = self.typed(PiCommand::GetForkMessages).await?;
        Ok(reply.messages)
    }

    /// `get_entries`.
    ///
    /// # Errors
    ///
    /// `Entry not found: …`.
    pub async fn get_entries(&self, since: Option<String>) -> Result<EntriesOutcome, ClientError> {
        self.typed(PiCommand::GetEntries { since }).await
    }

    /// `get_tree` (nodes as JSON in [`EntriesOutcome::entries`]).
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_tree(&self) -> Result<EntriesOutcome, ClientError> {
        self.typed(PiCommand::GetTree).await
    }

    /// `get_last_assistant_text`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_last_assistant_text(&self) -> Result<Option<String>, ClientError> {
        #[derive(Deserialize)]
        struct Text {
            text: Option<String>,
        }
        let reply: Text = self.typed(PiCommand::GetLastAssistantText).await?;
        Ok(reply.text)
    }

    /// `set_session_name`.
    ///
    /// # Errors
    ///
    /// `Session name cannot be empty`.
    pub async fn set_session_name(&self, name: impl Into<String>) -> Result<(), ClientError> {
        self.unit(PiCommand::SetSessionName { name: name.into() })
            .await
    }

    /// `get_messages`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_messages(&self) -> Result<Vec<AgentMessage>, ClientError> {
        #[derive(Deserialize)]
        struct Messages {
            messages: Vec<AgentMessage>,
        }
        let reply: Messages = self.typed(PiCommand::GetMessages).await?;
        Ok(reply.messages)
    }

    /// `get_commands`.
    ///
    /// # Errors
    ///
    /// Transport failures.
    pub async fn get_commands(&self) -> Result<Vec<SlashCommandInfo>, ClientError> {
        #[derive(Deserialize)]
        struct Commands {
            commands: Vec<SlashCommandInfo>,
        }
        let reply: Commands = self.typed(PiCommand::GetCommands).await?;
        Ok(reply.commands)
    }
}
