//! [`InProcessBackend`]: the full command set over an `oino-core`
//! [`AgentSessionRuntime`], without serialisation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::JsonValue;
use oino_core::events::{ClientEvent, OinoEvent, ReplaceReason};
use oino_core::runtime::{AgentSessionRuntime, ForkPosition};
use oino_core::session::{AgentSession, CycleDirection, InputSource, PromptOptions, RewindResult};
use oino_core::ui::UiResponse;
use oino_session::EntryId;
use serde::Serialize;
use serde_json::json;

use crate::backend::{ClientError, SessionBackend};
use crate::follow::Followers;
use crate::protocol::{AgentCommand, OinoCommand, RpcCommand, RpcResponseData, RpcSessionState};

/// Name and version reported by `_oino/hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    /// Server name (`oino`).
    pub name: String,
    /// Server version.
    pub version: String,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: "oino".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// A [`SessionBackend`] over a live runtime in this process.
///
/// Runtime-agnostic (GA-40): commands awaited from a non-tokio executor (GPUI) run on the
/// tokio runtime the backend was created on.
pub struct InProcessBackend {
    shared: Arc<Shared>,
    handle: Option<tokio::runtime::Handle>,
}

struct Shared {
    runtime: Arc<AgentSessionRuntime>,
    server: ServerInfo,
    closed: AtomicBool,
    followers: Followers,
}

impl std::fmt::Debug for InProcessBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessBackend")
            .field("server", &self.shared.server)
            .finish_non_exhaustive()
    }
}

fn data(value: impl Serialize) -> Result<RpcResponseData, ClientError> {
    serde_json::to_value(value)
        .map(Some)
        .map_err(|e| ClientError::Protocol(e.to_string()))
}

fn failed(error: impl std::fmt::Display) -> ClientError {
    ClientError::Command(error.to_string())
}

/// Default `_oino/entries_page` size.
const PAGE_LIMIT: usize = 120;
/// Raw-bytes bound of one page (the host bounds big blocks again for its frame).
const PAGE_MAX_BYTES: u64 = 24 << 20;

/// `_oino/entries_page`: a page of the current branch (or entries by id), serialized only
/// for the page — never the whole history (see `oino_session::manager::page_start`).
fn entries_page(
    manager: &oino_session::manager::SessionManager,
    before: Option<String>,
    limit: Option<usize>,
    ids: Option<Vec<String>>,
) -> Result<JsonValue, ClientError> {
    let leaf = manager.leaf_id().cloned();
    if let Some(ids) = ids {
        let entries: Vec<JsonValue> = ids
            .iter()
            .filter_map(|id| manager.entry(&EntryId::new(id.as_str())))
            .filter_map(|entry| serde_json::to_value(entry).ok())
            .collect();
        return Ok(json!({ "entries": entries, "leafId": leaf }));
    }
    let branch = manager.branch(None);
    let end = match before.as_deref() {
        None => branch.len(),
        Some(before) => branch
            .iter()
            .position(|entry| entry.id.as_str() == before)
            .ok_or_else(|| ClientError::Command(format!("Entry not found: {before}")))?,
    };
    let limit = limit.unwrap_or(PAGE_LIMIT).max(1);
    // Serialize only the candidates (at most `limit` before `end`).
    let first = end.saturating_sub(limit);
    let candidates: Vec<(JsonValue, u64)> = branch[first..end]
        .iter()
        .map(|entry| {
            let value = serde_json::to_value(entry).unwrap_or(JsonValue::Null);
            let size = serde_json::to_vec(&value).map_or(0, |bytes| bytes.len() as u64);
            (value, size)
        })
        .collect();
    let start = oino_session::manager::page_start(
        end,
        limit,
        PAGE_MAX_BYTES,
        |at| candidates[at - first].1,
        |at| {
            let value = &candidates[at - first].0;
            value["type"] == "message" && value["message"]["role"] == "user"
        },
    );
    let cursor = (start < end).then(|| branch[start].id.as_str().to_owned());
    let entries: Vec<JsonValue> = candidates
        .into_iter()
        .skip(start - first)
        .map(|(value, _)| value)
        .collect();
    Ok(json!({
        "entries": entries,
        "leafId": leaf,
        "total": branch.len(),
        "hasMore": start > 0,
        "cursor": cursor,
    }))
}

fn find_agent(
    session: &AgentSession,
    agent_id: &str,
) -> Result<Arc<oino_core::agents::ChildAgent>, ClientError> {
    session
        .find_agent(agent_id)
        .ok_or_else(|| ClientError::Command(format!("Unknown agent: {agent_id}")))
}

impl InProcessBackend {
    /// Wraps `runtime`.
    #[must_use]
    pub fn new(runtime: Arc<AgentSessionRuntime>) -> Self {
        Self::with_server_info(runtime, ServerInfo::default())
    }

    /// Wraps `runtime`, reporting `server` from `_oino/hello`. Commands awaited outside a
    /// tokio runtime run on the runtime current here (or the session's own runtime).
    #[must_use]
    pub fn with_server_info(runtime: Arc<AgentSessionRuntime>, server: ServerInfo) -> Self {
        let handle = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| runtime.session().runtime_handle());
        Self {
            shared: Arc::new(Shared {
                runtime,
                server,
                closed: AtomicBool::new(false),
                followers: Followers::default(),
            }),
            handle,
        }
    }

    /// The runtime.
    #[must_use]
    pub fn runtime(&self) -> &Arc<AgentSessionRuntime> {
        &self.shared.runtime
    }

    /// Returns the state.
    #[must_use]
    pub fn state(&self) -> RpcSessionState {
        self.shared.state()
    }

    /// Runs `task` here when already on a tokio runtime, else on the backend's runtime.
    fn on_runtime<T: Send + 'static>(
        &self,
        task: impl Future<Output = T> + Send + 'static,
        closed: T,
    ) -> BoxFuture<'static, T> {
        match (tokio::runtime::Handle::try_current(), &self.handle) {
            (Err(_), Some(handle)) => {
                let join = handle.spawn(task);
                Box::pin(async move { join.await.unwrap_or(closed) })
            }
            _ => Box::pin(task),
        }
    }
}

impl Shared {
    fn session(&self) -> AgentSession {
        self.runtime.session()
    }

    /// Resolves a client path against the session cwd (the process cwd is not used,
    /// which is the session cwd for the `oino` binary).
    fn path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.runtime.session().work_cwd().join(path)
        }
    }

    /// Points every event stream at the session that replaced the previous one.
    fn rebind(&self, reason: ReplaceReason, cancelled: bool) {
        if cancelled {
            return;
        }
        let session = self.session();
        let announce = ClientEvent::Oino(OinoEvent::SessionReplaced {
            session_file: session.session_file().map(|p| p.display().to_string()),
            session_id: session.session_id(),
            reason,
        });
        self.followers
            .rebind(|| self.runtime.session().subscribe(), &announce);
    }

    fn state(&self) -> RpcSessionState {
        let session = self.session();
        let auto_compaction_enabled = session.with_settings(|s| s.settings().compaction_enabled());
        RpcSessionState {
            model: session.model(),
            thinking_level: session.thinking_level(),
            is_streaming: session.is_streaming(),
            is_compacting: session.is_compacting(),
            steering_mode: session.steering_mode(),
            follow_up_mode: session.follow_up_mode(),
            session_file: session.session_file().map(|p| p.display().to_string()),
            session_id: session.session_id(),
            cwd: Some(session.work_cwd().display().to_string()),
            session_name: session.session_name(),
            auto_compaction_enabled,
            message_count: session.message_count(),
            pending_message_count: session.pending_message_count(),
            waiting_on: session.waiting_on(),
            background_on: session.background_on(),
        }
    }

    async fn dispatch(
        &self,
        request_id: Option<String>,
        command: RpcCommand,
    ) -> Result<RpcResponseData, ClientError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ClientError::Closed);
        }
        match command {
            RpcCommand::Agent(command) => self.agent(request_id, command).await,
            RpcCommand::Oino(command) => self.oino(command).await,
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per agent command, mirroring handleCommand"
    )]
    async fn agent(
        &self,
        request_id: Option<String>,
        command: AgentCommand,
    ) -> Result<RpcResponseData, ClientError> {
        let session = self.session();
        match command {
            AgentCommand::Prompt {
                message,
                images,
                streaming_behavior,
            } => {
                let disposition = session
                    .prompt(
                        message,
                        PromptOptions {
                            images,
                            streaming_behavior,
                            source: InputSource::Rpc,
                            ..PromptOptions::default()
                        },
                    )
                    .await
                    .map_err(failed)?;
                data(json!({ "disposition": disposition }))
            }
            AgentCommand::Steer { message, images } => {
                let disposition = session
                    .steer(message, images, InputSource::Rpc)
                    .await
                    .map_err(failed)?;
                data(json!({ "disposition": disposition }))
            }
            AgentCommand::FollowUp { message, images } => {
                let disposition = session
                    .follow_up(message, images, InputSource::Rpc)
                    .await
                    .map_err(failed)?;
                data(json!({ "disposition": disposition }))
            }
            AgentCommand::Abort => {
                session.abort().await;
                Ok(None)
            }
            AgentCommand::ClearQueue => {
                let (steering, follow_up) = session.clear_queue();
                data(json!({ "steering": steering, "followUp": follow_up }))
            }
            AgentCommand::NewSession { parent_session } => {
                let parent = parent_session.map(|p| self.path(&p));
                let result = self.runtime.new_session(parent).await.map_err(failed)?;
                self.rebind(ReplaceReason::New, result.cancelled);
                data(json!({ "cancelled": result.cancelled }))
            }
            AgentCommand::GetState => data(self.state()),
            AgentCommand::SetModel {
                provider,
                model_id,
                persist,
            } => {
                let model = session
                    .models()
                    .available()
                    .into_iter()
                    .find(|m| m.provider == provider && m.id == model_id)
                    .ok_or_else(|| {
                        ClientError::Command(format!("Model not found: {provider}/{model_id}"))
                    })?;
                // E-19: `persist` names what the caller wants. Absent = save the PC's
                // default (the TUI/CLI never send the flag); the app's session stream
                // rewrites an in-chat pick to `false` so only that chat switches.
                let model = session
                    .set_model(model, persist.unwrap_or(true))
                    .await
                    .map_err(failed)?;
                data(model)
            }
            AgentCommand::CycleModel { models } => {
                // E-19: a cycle is a per-session probe, not a choice — the PC's default
                // model stays (use the picker's `set_model` to change it for real).
                let result = session
                    .cycle_model(
                        CycleDirection::Forward,
                        false,
                        models.as_deref().unwrap_or(&[]),
                    )
                    .await
                    .map_err(failed)?;
                data(result)
            }
            AgentCommand::GetAvailableModels => {
                // F-02: every entry names its derived kind (`chat` vs `image`/`speech`/…)
                // so the UI can filter and mark rows. Per-model readiness (`ready`/
                // `reason`) rides on the app path (the hub's cache), not here.
                let models = session
                    .models()
                    .available()
                    .into_iter()
                    .map(|model| {
                        let mut entry =
                            serde_json::to_value(&model).unwrap_or_else(|_| json!(null));
                        entry["kind"] = json!(oino_models::kind::model_kind(&model));
                        entry
                    })
                    .collect::<Vec<_>>();
                data(json!({ "models": models }))
            }
            AgentCommand::SetThinkingLevel { level } => {
                session.set_thinking_level(level, true).await;
                Ok(None)
            }
            AgentCommand::CycleThinkingLevel => {
                let level = session.cycle_thinking_level(true).await;
                Ok(Some(level.map_or(
                    JsonValue::Null,
                    |level| json!({ "level": level }),
                )))
            }
            AgentCommand::GetAvailableThinkingLevels => {
                data(json!({ "levels": session.available_thinking_levels() }))
            }
            AgentCommand::SetSteeringMode { mode } => {
                session.set_steering_mode(mode);
                Ok(None)
            }
            AgentCommand::SetFollowUpMode { mode } => {
                session.set_follow_up_mode(mode);
                Ok(None)
            }
            AgentCommand::Compact {
                custom_instructions,
            } => data(session.compact(custom_instructions).await.map_err(failed)?),
            AgentCommand::SetAutoCompaction { enabled } => {
                session.set_auto_compaction_enabled(enabled);
                Ok(None)
            }
            AgentCommand::SetAutoRetry { enabled } => {
                session.set_auto_retry_enabled(enabled);
                Ok(None)
            }
            AgentCommand::AbortRetry => {
                session.abort_retry();
                Ok(None)
            }
            AgentCommand::Bash {
                command,
                exclude_from_context,
            } => {
                let result = session
                    .execute_bash(command, exclude_from_context.unwrap_or(false), request_id)
                    .await
                    .map_err(failed)?;
                data(result)
            }
            AgentCommand::AbortBash => {
                session.abort_bash();
                Ok(None)
            }
            AgentCommand::GetSessionStats => data(session.session_stats()),
            AgentCommand::ExportHtml { output_path } => {
                let path = session
                    .export_html(output_path.map(|p| self.path(&p)))
                    .await
                    .map_err(failed)?;
                data(json!({ "path": path.display().to_string() }))
            }
            AgentCommand::SwitchSession { session_path } => {
                let result = self
                    .runtime
                    .switch_session(self.path(&session_path), None)
                    .await
                    .map_err(failed)?;
                self.rebind(ReplaceReason::Resume, result.cancelled);
                data(json!({ "cancelled": result.cancelled }))
            }
            AgentCommand::Fork { entry_id } => {
                let result = self
                    .runtime
                    .fork(&EntryId::new(entry_id), ForkPosition::Before)
                    .await
                    .map_err(failed)?;
                self.rebind(ReplaceReason::Fork, result.cancelled);
                let mut value = json!({ "cancelled": result.cancelled });
                if let Some(text) = result.selected_text {
                    value = json!({ "text": text, "cancelled": result.cancelled });
                }
                Ok(Some(value))
            }
            AgentCommand::Clone => {
                let leaf = session.with_manager(|m| m.leaf_id().cloned());
                if leaf.is_none() {
                    return Err(ClientError::Command(
                        "Cannot clone session: no current entry selected".to_owned(),
                    ));
                }
                let result = self.runtime.clone_session().await.map_err(failed)?;
                self.rebind(ReplaceReason::Clone, result.cancelled);
                data(json!({ "cancelled": result.cancelled }))
            }
            AgentCommand::GetForkMessages => {
                data(json!({ "messages": session.user_messages_for_forking() }))
            }
            AgentCommand::GetEntries { since } => {
                let (entries, leaf) =
                    session.with_manager(|m| (m.entries().to_vec(), m.leaf_id().cloned()));
                let entries = match since {
                    None => entries,
                    Some(since) => {
                        let index = entries
                            .iter()
                            .position(|e| e.id.as_str() == since)
                            .ok_or_else(|| {
                                ClientError::Command(format!("Entry not found: {since}"))
                            })?;
                        entries[index + 1..].to_vec()
                    }
                };
                data(json!({ "entries": entries, "leafId": leaf }))
            }
            AgentCommand::GetTree => {
                let (tree, leaf) = session.with_manager(|m| (m.tree(), m.leaf_id().cloned()));
                data(json!({ "tree": tree, "leafId": leaf }))
            }
            AgentCommand::GetLastAssistantText => {
                data(json!({ "text": session.last_assistant_text() }))
            }
            AgentCommand::SetSessionName { name } => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(ClientError::Command(
                        "Session name cannot be empty".to_owned(),
                    ));
                }
                session
                    .set_session_name(Some(name.to_owned()))
                    .await
                    .map_err(failed)?;
                Ok(None)
            }
            AgentCommand::GetMessages => data(json!({ "messages": session.messages() })),
            AgentCommand::GetCommands => data(json!({ "commands": session.commands() })),
        }
    }

    async fn oino(&self, command: OinoCommand) -> Result<RpcResponseData, ClientError> {
        let session = self.session();
        match command {
            OinoCommand::Hello { client, .. } => {
                session.register_frontend(&client.name);
                data(json!({
                    "server": {"name": self.server.name, "version": self.server.version},
                    "protocol": 1,
                    "sessionId": session.session_id(),
                    "safeMode": self
                        .runtime
                        .services()
                        .safe_mode
                        .as_ref()
                        .is_some_and(|plan| plan.disable_extensions),
                }))
            }
            OinoCommand::UiSnapshot => {
                let snapshot = session.ui_snapshot();
                let mut value = serde_json::to_value(snapshot).unwrap_or(JsonValue::Null);
                if let Some(object) = value.as_object_mut() {
                    // Transitional: v1 frontends still replay `requests` until the
                    // TUI renderer (WP-U4) and `oino-view` (WP-U5) read the registry.
                    object.insert(
                        "requests".to_owned(),
                        serde_json::to_value(session.ui().snapshot()).unwrap_or(JsonValue::Null),
                    );
                }
                data(value)
            }
            OinoCommand::UiVisible {
                entry_id,
                visible,
                frontend,
            } => {
                let frontend = frontend
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("rpc");
                session.ui_visible(frontend, &entry_id, visible);
                Ok(None)
            }
            OinoCommand::UiResync { .. } => data(session.ui_resync()),
            OinoCommand::NavigateTree { target_id, options } => {
                let result = session
                    .navigate_tree(&EntryId::new(target_id), options)
                    .await
                    .map_err(failed)?;
                data(result)
            }
            OinoCommand::SetLabel { entry_id, label } => {
                session
                    .set_label(&EntryId::new(entry_id), label)
                    .await
                    .map_err(failed)?;
                Ok(None)
            }
            OinoCommand::ImportJsonl { path } => {
                let result = self
                    .runtime
                    .import_jsonl(self.path(&path), None)
                    .await
                    .map_err(failed)?;
                self.rebind(ReplaceReason::Import, result.cancelled);
                data(json!({ "cancelled": result.cancelled }))
            }
            OinoCommand::ExportJsonl { output_path } => {
                let path = session
                    .export_jsonl(output_path.map(|p| self.path(&p)))
                    .await
                    .map_err(failed)?;
                data(json!({ "path": path.display().to_string() }))
            }
            OinoCommand::GetContextUsage => data(session.context_usage()),
            OinoCommand::EntriesPage { before, limit, ids } => {
                let page = session.with_manager(|m| entries_page(m, before, limit, ids))?;
                data(page)
            }
            command @ (OinoCommand::Topics { .. }
            | OinoCommand::BusGet { .. }
            | OinoCommand::Agents { .. }
            | OinoCommand::AgentMessages { .. }
            | OinoCommand::AgentAbort { .. }
            | OinoCommand::QueueEdit(_)
            | OinoCommand::Checkpoints
            | OinoCommand::Rewind { .. }
            | OinoCommand::RunningBash
            | OinoCommand::Extensions
            | OinoCommand::SetExtensionEnabled { .. }
            | OinoCommand::SetCwd { .. }) => self.gui(&session, command).await,
            command => self.frontend(&session, command).await,
        }
    }

    /// GUI feeds and controls (gui-audit area E): topics, agents, queue edits, checkpoints,
    /// running bash calls, extensions.
    async fn gui(
        &self,
        session: &AgentSession,
        command: OinoCommand,
    ) -> Result<RpcResponseData, ClientError> {
        match command {
            OinoCommand::Topics {
                subscribe,
                unsubscribe,
            } => {
                let topics = session.subscribe_topics(&subscribe, &unsubscribe).await;
                data(json!({ "topics": topics, "patterns": session.topic_patterns() }))
            }
            OinoCommand::BusGet { topic } => Ok(Some(session.bus_get(&topic).map_or_else(
                || json!({ "data": null }),
                |v| json!({ "data": v.data, "publisher": v.publisher }),
            ))),
            OinoCommand::Agents { follow } => {
                if let Some(follow) = follow {
                    session.follow_agents(follow);
                }
                data(json!({ "agents": session.agent_tree(), "follow": session.follows_agents() }))
            }
            OinoCommand::AgentMessages { agent_id } => {
                let agent = find_agent(session, &agent_id)?;
                data(json!({ "messages": agent.messages() }))
            }
            OinoCommand::AgentAbort { agent_id } => {
                find_agent(session, &agent_id)?.abort();
                Ok(None)
            }
            OinoCommand::QueueEdit(edit) => {
                session.edit_queue(edit).await.map_err(failed)?;
                Ok(None)
            }
            OinoCommand::Checkpoints => data(json!({ "checkpoints": session.checkpoints() })),
            OinoCommand::Rewind {
                entry_id,
                files,
                conversation,
            } => self.rewind(session, &entry_id, files, conversation).await,
            OinoCommand::RunningBash => data(json!({ "calls": session.running_bash().await })),
            OinoCommand::Extensions => data(json!({ "extensions": session.extension_infos() })),
            OinoCommand::SetExtensionEnabled { id, enabled } => {
                session
                    .set_extension_enabled(&id, enabled)
                    .map_err(failed)?;
                Ok(None)
            }
            OinoCommand::SetCwd { cwd } => {
                session
                    .change_cwd(PathBuf::from(cwd))
                    .await
                    .map_err(failed)?;
                data(json!({ "cwd": session.work_cwd().display().to_string() }))
            }
            other => Err(ClientError::Protocol(format!(
                "{} is not a GUI command",
                other.kind()
            ))),
        }
    }

    /// `_oino/rewind`: files first (checkpoints at and after the entry), then the
    /// conversation fork before the entry.
    async fn rewind(
        &self,
        session: &AgentSession,
        entry_id: &str,
        files: bool,
        conversation: bool,
    ) -> Result<RpcResponseData, ClientError> {
        if session.is_streaming() {
            return Err(ClientError::Command(
                "Cannot rewind while the agent is running".to_owned(),
            ));
        }
        let mut result = if files && (session.has_checkpoint(entry_id) || !conversation) {
            session.rewind_files(entry_id).map_err(failed)?
        } else {
            RewindResult::default()
        };
        if conversation {
            let forked = self
                .runtime
                .fork(&EntryId::new(entry_id), ForkPosition::Before)
                .await
                .map_err(failed)?;
            self.rebind(ReplaceReason::Fork, forked.cancelled);
            result.cancelled = forked.cancelled;
            result.text = forked.selected_text;
        }
        data(result)
    }

    /// `_oino/render` → core render request (`ui-protocol §6.4`).
    fn render_request(
        command: &OinoCommand,
    ) -> Result<oino_core::session::RenderRequest, ClientError> {
        let OinoCommand::Render {
            target,
            key,
            entry_id,
            turn_entry_id,
            turn_index,
            running,
            density,
            expanded,
            width,
            data: payload,
        } = command
        else {
            return Err(ClientError::Protocol(format!(
                "{} is not a render command",
                command.kind()
            )));
        };
        let target = oino_core::session::RenderTarget::parse(target)
            .ok_or_else(|| ClientError::Protocol(format!("render target `{target}`")))?;
        Ok(oino_core::session::RenderRequest {
            target,
            key: key.clone(),
            entry_id: entry_id.clone(),
            turn_entry_id: turn_entry_id.clone(),
            turn_index: *turn_index,
            running: *running,
            density: density.as_deref().map_or_else(Default::default, |d| {
                oino_core::session::RenderDensity::parse(d).unwrap_or_default()
            }),
            expanded: *expanded,
            width: width.as_deref().map_or_else(Default::default, |w| {
                oino_core::session::RenderWidth::parse(w).unwrap_or_default()
            }),
            data: payload.clone(),
        })
    }

    /// The frontend command surface (`_oino/*` completions, actions, settings, auth).
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per frontend command, mirroring the RPC surface"
    )]
    async fn frontend(
        &self,
        session: &AgentSession,
        command: OinoCommand,
    ) -> Result<RpcResponseData, ClientError> {
        match command {
            OinoCommand::GetCompletions { text, cursor } => {
                data(json!({ "groups": session.completions(&text, cursor).await }))
            }
            OinoCommand::UiAction {
                panel_id,
                action_id,
                value,
                values,
                path,
                source,
            } => {
                let action = oino_core::ui::UiAction {
                    panel_id,
                    action_id,
                    value,
                    values: values.and_then(|values| values.as_object().cloned()),
                    path: path.and_then(|path| serde_json::from_value(path).ok()),
                    source: source.as_deref().and_then(|name| match name {
                        "tui" => Some(oino_core::ui::ActionSource::Tui),
                        "app" => Some(oino_core::ui::ActionSource::App),
                        "rpc" => Some(oino_core::ui::ActionSource::Rpc),
                        _ => None,
                    }),
                };
                session.ui_action_with(&action).map_err(failed)?;
                Ok(None)
            }
            OinoCommand::EditorEvent(event) => {
                let consumed = session.editor_event(event).await;
                data(json!({ "consumed": consumed }))
            }
            OinoCommand::InvokeShortcut { shortcut_id } => {
                session.invoke_shortcut(&shortcut_id).map_err(failed)?;
                Ok(None)
            }
            OinoCommand::GetShortcuts => {
                let shortcuts: Vec<JsonValue> = session
                    .runner()
                    .shortcuts()
                    .into_iter()
                    .map(|(extension, spec)| {
                        json!({
                            "id": spec.id,
                            "defaultKeys": spec.default_keys,
                            "description": spec.description,
                            "extensionId": extension,
                        })
                    })
                    .collect();
                data(json!({ "shortcuts": shortcuts }))
            }
            command @ (OinoCommand::GetSettingsSchema
            | OinoCommand::GetExtensionSettings { .. }
            | OinoCommand::SetExtensionSetting { .. }
            | OinoCommand::UnsetExtensionSetting { .. }
            | OinoCommand::GetSettings { .. }
            | OinoCommand::SetSetting { .. }) => self.settings_command(session, command),
            OinoCommand::ServiceCall {
                target,
                name,
                payload,
            } => data(
                session
                    .service_call(target.as_deref(), &name, payload)
                    .await
                    .map_err(failed)?,
            ),
            OinoCommand::AuthProviders => data(json!({ "providers": session.auth_providers() })),
            OinoCommand::Login { provider } => {
                session.login(&provider).await.map_err(failed)?;
                Ok(None)
            }
            OinoCommand::Logout { provider } => {
                session.logout(&provider).map_err(failed)?;
                Ok(None)
            }
            OinoCommand::SetApiKey { provider, key } => {
                session.set_api_key(&provider, &key).await.map_err(failed)?;
                Ok(None)
            }
            render @ OinoCommand::Render { .. } => {
                let node = session
                    .render_spec(&Self::render_request(&render)?)
                    .map_err(failed)?;
                data(json!({ "node": node }))
            }
            OinoCommand::Hello { .. }
            | OinoCommand::Topics { .. }
            | OinoCommand::BusGet { .. }
            | OinoCommand::Agents { .. }
            | OinoCommand::AgentMessages { .. }
            | OinoCommand::AgentAbort { .. }
            | OinoCommand::QueueEdit(_)
            | OinoCommand::Checkpoints
            | OinoCommand::Rewind { .. }
            | OinoCommand::RunningBash
            | OinoCommand::Extensions
            | OinoCommand::SetExtensionEnabled { .. }
            | OinoCommand::SetCwd { .. }
            | OinoCommand::UiSnapshot
            | OinoCommand::UiVisible { .. }
            | OinoCommand::UiResync { .. }
            | OinoCommand::NavigateTree { .. }
            | OinoCommand::SetLabel { .. }
            | OinoCommand::EntriesPage { .. }
            | OinoCommand::ImportJsonl { .. }
            | OinoCommand::ExportJsonl { .. }
            | OinoCommand::GetContextUsage => Err(ClientError::Protocol(format!(
                "{} is not a frontend command",
                command.kind()
            ))),
        }
    }

    /// Extension settings registry and core settings commands.
    fn settings_command(
        &self,
        session: &AgentSession,
        command: OinoCommand,
    ) -> Result<RpcResponseData, ClientError> {
        let registry = self.runtime.extension_settings();
        match command {
            OinoCommand::GetSettingsSchema => data(json!({ "namespaces": registry.schema() })),
            OinoCommand::GetExtensionSettings { namespace, scope } => {
                if registry.namespace(&namespace).is_none() {
                    return Err(ClientError::Command(format!(
                        "Unknown settings namespace: {namespace}"
                    )));
                }
                // `diagnostics` lists stored values `resolve` rejected (ui-protocol §7.1).
                let resolved = registry.resolved(&namespace, scope);
                data(json!({
                    "values": resolved.values,
                    "diagnostics": resolved.diagnostics,
                }))
            }
            OinoCommand::SetExtensionSetting {
                namespace,
                key,
                value,
                scope,
            } => {
                let value = value.filter(|v| !v.is_null());
                registry
                    .set(&namespace, &key, value, scope)
                    .map_err(failed)?;
                Ok(None)
            }
            OinoCommand::UnsetExtensionSetting {
                namespace,
                key,
                scope,
            } => {
                registry
                    .set(&namespace, &key, None, scope)
                    .map_err(failed)?;
                Ok(None)
            }
            OinoCommand::GetSettings { key } => {
                let merged = registry.merged_settings();
                match key {
                    None => data(json!({ "settings": merged })),
                    Some(key) => {
                        let path: Vec<&str> = key.split('.').collect();
                        let value = oino_settings::json::get_path(&merged, &path).cloned();
                        data(json!({ "value": value }))
                    }
                }
            }
            OinoCommand::SetSetting { key, value, scope } => {
                let path: Vec<&str> = key.split('.').filter(|p| !p.is_empty()).collect();
                if path.is_empty() {
                    return Err(ClientError::Command("Setting key cannot be empty".into()));
                }
                let value = value.filter(|v| !v.is_null());
                session
                    .with_settings(|settings| settings.set(scope, &path, value))
                    .map_err(failed)?;
                Ok(None)
            }
            other => Err(ClientError::Protocol(format!(
                "{} is not a settings command",
                other.kind()
            ))),
        }
    }
}

impl SessionBackend for InProcessBackend {
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        self.send_with_id(None, command)
    }

    fn send_with_id(
        &self,
        request_id: Option<String>,
        command: RpcCommand,
    ) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        let shared = Arc::clone(&self.shared);
        self.on_runtime(
            async move { shared.dispatch(request_id, command).await },
            Err(ClientError::Closed),
        )
    }

    /// Events of the current session; streams follow session replacements made through
    /// this backend and end after [`SessionBackend::shutdown`].
    fn events(&self) -> BoxStream<'static, ClientEvent> {
        self.shared
            .followers
            .stream(self.shared.session().subscribe())
            .boxed()
    }

    fn ui_respond(&self, response: UiResponse) {
        self.shared.session().ui().respond(response);
    }

    fn session_id(&self) -> String {
        self.shared.session().session_id()
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        let shared = Arc::clone(&self.shared);
        self.on_runtime(
            async move {
                if !shared.closed.swap(true, Ordering::AcqRel) {
                    shared.runtime.dispose().await;
                    shared.followers.close();
                }
            },
            (),
        )
    }

    /// Resolves once an extension called `ctx.shutdown()` and the session is idle.
    fn shutdown_requested(&self) -> BoxFuture<'static, ()> {
        let runtime = Arc::clone(&self.shared.runtime);
        self.on_runtime(
            async move {
                let mut requested = runtime.shutdown_requested();
                if requested.wait_for(|on| *on).await.is_err() {
                    return std::future::pending().await;
                }
                runtime.session().wait_for_idle().await;
            },
            (),
        )
    }
}

#[cfg(test)]
mod entries_page_tests {
    use super::*;
    use oino_session::manager::SessionManager;

    fn manager(turns: usize) -> (tempfile::TempDir, SessionManager) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut text = String::from(
            "{\"type\":\"session\",\"version\":3,\"id\":\"s1\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"cwd\":\"/tmp\"}\n",
        );
        let mut parent = serde_json::Value::Null;
        for n in 0..turns {
            for (id, role) in [(format!("u{n}"), "user"), (format!("a{n}"), "assistant")] {
                let content = if role == "user" {
                    json!([{"type": "text", "text": format!("q{n}")}])
                } else {
                    json!([{"type": "text", "text": format!("a{n}")}])
                };
                let mut message = json!({"role": role, "content": content, "timestamp": 1});
                if role == "assistant" {
                    message["api"] = json!("x");
                    message["provider"] = json!("p");
                    message["model"] = json!("m");
                    message["usage"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}});
                    message["stopReason"] = json!("stop");
                }
                let line = json!({"type": "message", "id": id, "parentId": parent,
                    "timestamp": "2026-01-01T00:00:00.000Z", "message": message});
                text.push_str(&line.to_string());
                text.push('\n');
                parent = json!(id);
            }
        }
        std::fs::write(&path, text).unwrap();
        let manager = SessionManager::open(&path, dir.path(), None).unwrap();
        (dir, manager)
    }

    fn ids(page: &JsonValue) -> Vec<&str> {
        page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn pages_the_branch_from_the_tail_on_turn_boundaries() {
        let (_dir, manager) = manager(10);
        assert_eq!(manager.entries().len(), 20, "fixture parsed");
        let page = entries_page(&manager, None, Some(5), None).unwrap();
        // 5 newest = a7 u8 a8 u9 a9 → aligned at the user message u8.
        assert_eq!(ids(&page), ["u8", "a8", "u9", "a9"]);
        assert_eq!(page["total"], 20);
        assert_eq!(page["hasMore"], true);
        assert_eq!(page["cursor"], "u8");
        assert_eq!(page["leafId"], "a9");
        let older = entries_page(&manager, Some("u8".into()), Some(100), None).unwrap();
        assert_eq!(ids(&older).len(), 16);
        assert_eq!(older["hasMore"], false);
        let by_id =
            entries_page(&manager, None, None, Some(vec!["a3".into(), "nope".into()])).unwrap();
        assert_eq!(ids(&by_id), ["a3"]);
        assert!(entries_page(&manager, Some("nope".into()), None, None).is_err());
    }
}
