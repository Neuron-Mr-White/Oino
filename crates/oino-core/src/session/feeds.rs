//! Frontend-facing session state the GUI needs beyond pi (gui-audit area E): the runtime
//! handle background work is spawned on, the `_oino/topics` subscription, attached frontends,
//! running `bash` calls with their detach registry, and file checkpoints.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};
use oino_tools::BashDetachRegistry;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::checkpoints::{Checkpoint, CheckpointStore, RewindResult};
use super::{AgentSession, SessionOpError, lock};
use crate::events::OinoEvent;
use crate::extensions::api::ExtensionMode;
use crate::extensions::host::HostError;
use crate::topics::{self, TopicValue};

/// A running foreground `bash` call (`control.running-bash`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningBash {
    /// Tool-call id.
    pub tool_call_id: String,
    /// Command line.
    pub command: String,
    /// Start (epoch ms).
    pub started_at: i64,
    /// Shell process id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// A process taken over from a detached `bash` call (core side of
/// `oino_extension_api::DetachedBash`).
pub struct DetachedBash {
    /// The command line.
    pub command: String,
    /// Process id, when known.
    pub pid: Option<u32>,
    /// Output the tool call returned before the detach marker.
    pub output_so_far: String,
    /// Temp file with the complete output so far.
    pub full_output_path: Option<String>,
    /// Further output chunks.
    pub output: BoxStream<'static, Vec<u8>>,
    /// Exit code once the process ended (`None` for a signal).
    pub exit: BoxFuture<'static, Option<i32>>,
    /// Cancel to kill the process tree.
    pub kill: CancellationToken,
}

impl std::fmt::Debug for DetachedBash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetachedBash")
            .field("command", &self.command)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

/// One extension as frontends see it (`_oino/extensions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionInfo {
    /// Manifest id.
    pub id: String,
    /// Provider component or extension.
    pub kind: crate::extensions::api::ExtensionKind,
    /// Not listed in `disabledExtensions`.
    pub enabled: bool,
    /// Loaded in this session (disabled ones are not).
    pub loaded: bool,
    /// Package path (loaded extensions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Frontend-facing state of one session.
pub(crate) struct FrontendState {
    /// Runtime the session was created on; background work from calls made off the runtime
    /// (GPUI executors) is spawned here (GA-40).
    runtime: Option<tokio::runtime::Handle>,
    topics: Mutex<Vec<String>>,
    frontends: Mutex<Vec<String>>,
    running_bash: Mutex<IndexMap<String, RunningBash>>,
    bash_detach: OnceLock<BashDetachRegistry>,
    pub(crate) checkpoints: Mutex<CheckpointStore>,
}

impl FrontendState {
    pub(crate) fn new() -> Self {
        Self {
            runtime: tokio::runtime::Handle::try_current().ok(),
            topics: Mutex::new(Vec::new()),
            frontends: Mutex::new(Vec::new()),
            running_bash: Mutex::new(IndexMap::new()),
            bash_detach: OnceLock::new(),
            checkpoints: Mutex::new(CheckpointStore::default()),
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

/// Tools whose file changes are checkpointed.
const CHECKPOINTED_TOOLS: [&str; 2] = ["write", "edit"];
/// Shell tools listed by `running-bash`.
const SHELL_TOOLS: [&str; 2] = ["bash", "powershell"];

impl AgentSession {
    /// Spawns `task` on the current tokio runtime, or on the runtime the session was created
    /// on when called from another executor. Dropped (with a warning) when neither exists.
    pub(crate) fn spawn_task(&self, task: impl Future<Output = ()> + Send + 'static) {
        let handle = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.inner.frontend.runtime.clone());
        if let Some(handle) = handle {
            handle.spawn(task);
        } else {
            tracing::warn!("no tokio runtime for a background session task");
        }
    }

    /// The runtime the session was created on.
    #[must_use]
    pub fn runtime_handle(&self) -> Option<tokio::runtime::Handle> {
        self.inner.frontend.runtime.clone()
    }

    // ── topics ──

    /// Adds and removes `_oino/topics` patterns (exact, `prefix*`, `*`) and returns the
    /// current sticky values matching the resulting subscription. Subscribing to
    /// [`topics::GIT_TOPIC`] refreshes it first.
    pub async fn subscribe_topics(
        &self,
        subscribe: &[String],
        unsubscribe: &[String],
    ) -> Vec<TopicValue> {
        let patterns = {
            let mut patterns = lock(&self.inner.frontend.topics);
            patterns.retain(|p| !unsubscribe.contains(p));
            for pattern in subscribe {
                if !patterns.contains(pattern) {
                    patterns.push(pattern.clone());
                }
            }
            patterns.clone()
        };
        if topics::any_matches(&patterns, topics::GIT_TOPIC) {
            self.refresh_git_topic().await;
        }
        self.inner
            .agents
            .router()
            .map(|router| router.topics().snapshot(&patterns))
            .unwrap_or_default()
    }

    /// The current `_oino/topics` patterns.
    #[must_use]
    pub fn topic_patterns(&self) -> Vec<String> {
        lock(&self.inner.frontend.topics).clone()
    }

    /// Last sticky value of `topic` (`_oino/bus_get`).
    #[must_use]
    pub fn bus_get(&self, topic: &str) -> Option<TopicValue> {
        self.inner
            .agents
            .router()
            .and_then(|router| router.topics().get(topic))
    }

    /// Publishes `value` to this session's frontends when it matches the subscription.
    pub(crate) fn offer_topic(&self, value: &TopicValue) {
        if topics::any_matches(&lock(&self.inner.frontend.topics), &value.topic) {
            self.inner.hub.publish(OinoEvent::from(value.clone()));
        }
    }

    /// Recomputes [`topics::GIT_TOPIC`] for the session cwd.
    pub async fn refresh_git_topic(&self) {
        let Some(router) = self.inner.agents.router() else {
            return;
        };
        let cwd = self.inner.cwd.clone();
        let info = tokio::task::spawn_blocking(move || topics::git_info(&cwd))
            .await
            .unwrap_or(JsonValue::Null);
        router.publish_topic(topics::CORE_PUBLISHER, topics::GIT_TOPIC, info, true);
    }

    /// Refreshes the git topic in the background when a frontend follows it (after runs).
    pub(crate) fn refresh_git_topic_if_followed(&self) {
        if self.inner.agents.link.is_none()
            && topics::any_matches(&lock(&self.inner.frontend.topics), topics::GIT_TOPIC)
        {
            let session = self.clone();
            self.spawn_task(async move { session.refresh_git_topic().await });
        }
    }

    // ── frontends ──

    /// Records a frontend attached through `_oino/hello` (`client.name`).
    pub fn register_frontend(&self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut frontends = lock(&self.inner.frontend.frontends);
        if !frontends.iter().any(|f| f == name) {
            frontends.push(name.to_owned());
        }
    }

    /// Attached frontends for `host-info` (`tui`, `desktop`, `mobile`, `rpc`, `acp`, plus
    /// `_oino/hello` client names).
    #[must_use]
    pub fn frontends(&self) -> Vec<String> {
        let base = match self.inner.mode {
            ExtensionMode::Tui => Some("tui"),
            ExtensionMode::Desktop => Some("desktop"),
            ExtensionMode::Mobile => Some("mobile"),
            ExtensionMode::Rpc => Some("rpc"),
            ExtensionMode::Acp => Some("acp"),
            ExtensionMode::Print | ExtensionMode::Json => None,
        };
        let mut out: Vec<String> = base.into_iter().map(str::to_owned).collect();
        for name in lock(&self.inner.frontend.frontends).iter() {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    /// Carries the frontend subscriptions over to the session that replaced `old`.
    pub(crate) fn inherit_frontend_state(&self, old: &Self) {
        lock(&self.inner.frontend.topics).clone_from(&lock(&old.inner.frontend.topics));
        lock(&self.inner.frontend.frontends).clone_from(&lock(&old.inner.frontend.frontends));
        self.follow_agents(old.follows_agents());
    }

    // ── extensions ──

    /// Loaded extensions plus the ids listed in `disabledExtensions` (`_oino/extensions`).
    #[must_use]
    pub fn extension_infos(&self) -> Vec<ExtensionInfo> {
        let disabled = self.with_settings(|s| s.settings().disabled_extensions());
        let mut out: Vec<ExtensionInfo> = self
            .inner
            .agents
            .all_extensions
            .extensions()
            .iter()
            .map(|extension| ExtensionInfo {
                id: extension.instance.id().to_owned(),
                kind: extension.instance.kind(),
                enabled: !disabled.iter().any(|d| d == extension.instance.id()),
                loaded: true,
                path: Some(extension.instance.path()),
            })
            .collect();
        for id in disabled {
            if !out.iter().any(|e| e.id == id) {
                out.push(ExtensionInfo {
                    id,
                    kind: crate::extensions::api::ExtensionKind::Extension,
                    enabled: false,
                    loaded: false,
                    path: None,
                });
            }
        }
        out
    }

    /// Adds `id` to or removes it from `disabledExtensions` (global settings). Takes effect
    /// for sessions created afterwards (`_oino/set_extension_enabled`).
    ///
    /// # Errors
    ///
    /// The settings file cannot be written.
    pub fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<(), SessionOpError> {
        self.with_settings(|settings| {
            let mut disabled = settings.settings().disabled_extensions();
            disabled.retain(|d| d != id);
            if !enabled {
                disabled.push(id.to_owned());
            }
            settings.set_disabled_extensions(&disabled)
        })
        .map_err(|e| SessionOpError::Failed(e.to_string()))
    }

    // ── bash ──

    /// Binds the detach registry of the session's built-in shell tools (once).
    pub fn bind_bash_detach(&self, registry: BashDetachRegistry) {
        let _ = self.inner.frontend.bash_detach.set(registry);
    }

    /// Tracks tool executions for `running-bash` and snapshots files for checkpoints.
    pub(crate) fn note_tool_start(&self, tool_call_id: &str, tool_name: &str, args: &JsonObject) {
        if SHELL_TOOLS.contains(&tool_name) {
            let command = args
                .get("command")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned();
            lock(&self.inner.frontend.running_bash).insert(
                tool_call_id.to_owned(),
                RunningBash {
                    tool_call_id: tool_call_id.to_owned(),
                    command,
                    started_at: now_ms(),
                    pid: None,
                },
            );
        }
    }

    /// Forgets a finished tool execution.
    pub(crate) fn note_tool_end(&self, tool_call_id: &str) {
        lock(&self.inner.frontend.running_bash).shift_remove(tool_call_id);
    }

    /// Foreground `bash` calls still running (`control.running-bash`), oldest first.
    pub async fn running_bash(&self) -> Vec<RunningBash> {
        let registry = self.inner.frontend.bash_detach.get().cloned();
        let mut list: Vec<RunningBash> = lock(&self.inner.frontend.running_bash)
            .values()
            .cloned()
            .collect();
        if let Some(registry) = registry {
            list.retain(|call| registry.is_running(&call.tool_call_id));
            for call in &mut list {
                call.pid = registry.pid(&call.tool_call_id).await;
            }
        }
        list
    }

    /// Detaches the running `bash` call `tool_call_id` into background task `task_id`: the
    /// call returns at once with the output so far plus a detach marker, the process keeps
    /// running and is handed over.
    ///
    /// # Errors
    ///
    /// [`HostError::Failed`] when no such call runs (or it finished meanwhile).
    pub async fn detach_bash(
        &self,
        tool_call_id: &str,
        task_id: &str,
        reason: &str,
    ) -> Result<DetachedBash, HostError> {
        let not_running = || HostError::Failed(format!("No running bash call {tool_call_id}"));
        let registry = self
            .inner
            .frontend
            .bash_detach
            .get()
            .cloned()
            .ok_or_else(not_running)?;
        let command = lock(&self.inner.frontend.running_bash)
            .get(tool_call_id)
            .map(|call| call.command.clone())
            .unwrap_or_default();
        let detached = registry
            .detach(tool_call_id, task_id, reason)
            .await
            .ok_or_else(not_running)?;
        let mut process = detached.process;
        let pid = process.control.pid();
        let kill = CancellationToken::new();
        let killed = kill.clone();
        let mut control = process.control;
        let exit = Box::pin(async move {
            let status = tokio::select! {
                status = control.wait() => status,
                () = killed.cancelled() => {
                    control.kill_tree();
                    control.wait().await
                }
            };
            status.ok().and_then(|s| s.code)
        });
        let output = futures::stream::poll_fn(move |cx| process.output.poll_recv(cx)).boxed();
        Ok(DetachedBash {
            command,
            pid,
            output_so_far: detached.output_so_far,
            full_output_path: detached.full_output_path,
            output,
            exit,
            kill,
        })
    }

    // ── checkpoints ──

    fn checkpoint_dir(&self) -> Option<PathBuf> {
        let persisted = self.with_manager(|m| m.is_persisted());
        persisted.then(|| {
            self.inner
                .agent_dir
                .join("checkpoints")
                .join(self.session_id())
        })
    }

    fn checkpoints_store(&self) -> std::sync::MutexGuard<'_, CheckpointStore> {
        let dir = self.checkpoint_dir();
        let mut store = lock(&self.inner.frontend.checkpoints);
        store.bind(dir);
        store
    }

    /// Entry id of the latest user message on the current branch.
    fn current_user_entry(&self) -> Option<String> {
        self.with_manager(|m| {
            m.branch(None)
                .iter()
                .rev()
                .find(|entry| matches!(entry.message(), Some(oino_ai::AgentMessage::User(_))))
                .map(|entry| entry.id.as_str().to_owned())
        })
    }

    /// Snapshots the file a built-in `write` / `edit` call is about to change.
    pub(crate) fn checkpoint_tool_call(&self, tool_name: &str, args: &JsonObject) {
        if !CHECKPOINTED_TOOLS.contains(&tool_name) {
            return;
        }
        let Some(path) = args.get("path").and_then(JsonValue::as_str) else {
            return;
        };
        let Some(entry) = self.current_user_entry() else {
            return;
        };
        let cwd = self.inner.cwd.display().to_string();
        let Ok(resolved) = oino_tools::path_utils::resolve_to_cwd(path, &cwd) else {
            return;
        };
        self.checkpoints_store()
            .record(&entry, std::path::Path::new(&resolved));
    }

    /// File checkpoints of this session, oldest first (`_oino/checkpoints`).
    #[must_use]
    pub fn checkpoints(&self) -> Vec<Checkpoint> {
        self.checkpoints_store().list()
    }

    /// Restores the files changed at or after the user message `entry_id` (file part of
    /// `_oino/rewind`) and drops those checkpoints.
    ///
    /// # Errors
    ///
    /// [`SessionOpError::Failed`] while streaming or without a checkpoint for the entry.
    pub fn rewind_files(&self, entry_id: &str) -> Result<RewindResult, SessionOpError> {
        if self.is_streaming() {
            return Err(SessionOpError::Failed(
                "Cannot rewind files while the agent is running".to_owned(),
            ));
        }
        self.checkpoints_store().rewind(entry_id).ok_or_else(|| {
            SessionOpError::Failed(format!("No file checkpoint for entry {entry_id}"))
        })
    }

    /// Whether a checkpoint exists for the user message `entry_id`.
    #[must_use]
    pub fn has_checkpoint(&self, entry_id: &str) -> bool {
        self.checkpoints_store().contains(entry_id)
    }

    /// Copies the checkpoints of `source` whose user message is on this session's branch
    /// (a fork keeps the file history of the conversation it kept).
    pub(crate) fn inherit_checkpoints(&self, source: &Self) {
        let kept: std::collections::HashSet<String> = self.with_manager(|m| {
            m.branch(None)
                .iter()
                .map(|entry| entry.id.as_str().to_owned())
                .collect()
        });
        let source_store = source.checkpoints_store();
        let mut target = self.checkpoints_store();
        source_store.copy_matching(|id| kept.contains(id), &mut target);
    }
}
