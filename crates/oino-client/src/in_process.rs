//! [`InProcessBackend`]: the command set of pi `rpc-mode.ts` over an `oino-core`
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
use crate::protocol::{OinoCommand, PiCommand, RpcCommand, RpcResponseData, RpcSessionState};

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

    /// pi `get_state`.
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

    /// Resolves a client path against the session cwd (pi resolves against the process cwd,
    /// which is the session cwd for the `oino` binary).
    fn path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.runtime.services().cwd.join(path)
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
            session_name: session.session_name(),
            auto_compaction_enabled,
            message_count: session.messages().len(),
            pending_message_count: session.pending_message_count(),
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
            RpcCommand::Pi(command) => self.pi(request_id, command).await,
            RpcCommand::Oino(command) => self.oino(command).await,
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per pi RPC command, mirroring rpc-mode.ts handleCommand"
    )]
    async fn pi(
        &self,
        request_id: Option<String>,
        command: PiCommand,
    ) -> Result<RpcResponseData, ClientError> {
        let session = self.session();
        match command {
            PiCommand::Prompt {
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
            PiCommand::Steer { message, images } => {
                let disposition = session
                    .steer(message, images, InputSource::Rpc)
                    .await
                    .map_err(failed)?;
                data(json!({ "disposition": disposition }))
            }
            PiCommand::FollowUp { message, images } => {
                let disposition = session
                    .follow_up(message, images, InputSource::Rpc)
                    .await
                    .map_err(failed)?;
                data(json!({ "disposition": disposition }))
            }
            PiCommand::Abort => {
                session.abort().await;
                Ok(None)
            }
            PiCommand::ClearQueue => {
                let (steering, follow_up) = session.clear_queue();
                data(json!({ "steering": steering, "followUp": follow_up }))
            }
            PiCommand::NewSession { parent_session } => {
                let parent = parent_session.map(|p| self.path(&p));
                let result = self.runtime.new_session(parent).await.map_err(failed)?;
                self.rebind(ReplaceReason::New, result.cancelled);
                data(json!({ "cancelled": result.cancelled }))
            }
            PiCommand::GetState => data(self.state()),
            PiCommand::SetModel { provider, model_id } => {
                let model = session
                    .models()
                    .available()
                    .into_iter()
                    .find(|m| m.provider == provider && m.id == model_id)
                    .ok_or_else(|| {
                        ClientError::Command(format!("Model not found: {provider}/{model_id}"))
                    })?;
                let model = session.set_model(model, true).await.map_err(failed)?;
                data(model)
            }
            PiCommand::CycleModel => {
                let result = session
                    .cycle_model(CycleDirection::Forward, true)
                    .await
                    .map_err(failed)?;
                data(result)
            }
            PiCommand::GetAvailableModels => {
                data(json!({ "models": session.models().available() }))
            }
            PiCommand::SetThinkingLevel { level } => {
                session.set_thinking_level(level, true).await;
                Ok(None)
            }
            PiCommand::CycleThinkingLevel => {
                let level = session.cycle_thinking_level(true).await;
                Ok(Some(level.map_or(
                    JsonValue::Null,
                    |level| json!({ "level": level }),
                )))
            }
            PiCommand::GetAvailableThinkingLevels => {
                data(json!({ "levels": session.available_thinking_levels() }))
            }
            PiCommand::SetSteeringMode { mode } => {
                session.set_steering_mode(mode);
                Ok(None)
            }
            PiCommand::SetFollowUpMode { mode } => {
                session.set_follow_up_mode(mode);
                Ok(None)
            }
            PiCommand::Compact {
                custom_instructions,
            } => data(session.compact(custom_instructions).await.map_err(failed)?),
            PiCommand::SetAutoCompaction { enabled } => {
                session.set_auto_compaction_enabled(enabled);
                Ok(None)
            }
            PiCommand::SetAutoRetry { enabled } => {
                session.set_auto_retry_enabled(enabled);
                Ok(None)
            }
            PiCommand::AbortRetry => {
                session.abort_retry();
                Ok(None)
            }
            PiCommand::Bash {
                command,
                exclude_from_context,
            } => {
                let result = session
                    .execute_bash(command, exclude_from_context.unwrap_or(false), request_id)
                    .await
                    .map_err(failed)?;
                data(result)
            }
            PiCommand::AbortBash => {
                session.abort_bash();
                Ok(None)
            }
            PiCommand::GetSessionStats => data(session.session_stats()),
            PiCommand::ExportHtml { output_path } => {
                let path = session
                    .export_html(output_path.map(|p| self.path(&p)))
                    .await
                    .map_err(failed)?;
                data(json!({ "path": path.display().to_string() }))
            }
            PiCommand::SwitchSession { session_path } => {
                let result = self
                    .runtime
                    .switch_session(self.path(&session_path), None)
                    .await
                    .map_err(failed)?;
                self.rebind(ReplaceReason::Resume, result.cancelled);
                data(json!({ "cancelled": result.cancelled }))
            }
            PiCommand::Fork { entry_id } => {
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
            PiCommand::Clone => {
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
            PiCommand::GetForkMessages => {
                data(json!({ "messages": session.user_messages_for_forking() }))
            }
            PiCommand::GetEntries { since } => {
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
            PiCommand::GetTree => {
                let (tree, leaf) = session.with_manager(|m| (m.tree(), m.leaf_id().cloned()));
                data(json!({ "tree": tree, "leafId": leaf }))
            }
            PiCommand::GetLastAssistantText => {
                data(json!({ "text": session.last_assistant_text() }))
            }
            PiCommand::SetSessionName { name } => {
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
            PiCommand::GetMessages => data(json!({ "messages": session.messages() })),
            PiCommand::GetCommands => data(json!({ "commands": session.commands() })),
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
                }))
            }
            OinoCommand::UiSnapshot => data(json!({ "requests": session.ui().snapshot() })),
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
            | OinoCommand::SetExtensionEnabled { .. }) => self.gui(&session, command).await,
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
            other => Err(ClientError::Protocol(format!(
                "{} is not a GUI command",
                other.kind()
            ))),
        }
    }

    /// `_oino/rewind`: files first (checkpoints at and after the entry), then the
    /// conversation fork before the entry (pi `fork`).
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

    /// The frontend command surface (`_oino/*` completions, actions, settings, auth).
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
            } => {
                session
                    .ui_action(&panel_id, &action_id, value)
                    .map_err(failed)?;
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
            | OinoCommand::UiSnapshot
            | OinoCommand::NavigateTree { .. }
            | OinoCommand::SetLabel { .. }
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
                data(json!({ "values": registry.values(&namespace, scope) }))
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
