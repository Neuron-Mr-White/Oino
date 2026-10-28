//! The ACP agent: request dispatch for `initialize`, `session/*` and `_oino/*`
//! (modes §6.1, §6.2).
//!
//! Handlers run inside the SDK's dispatch loop, so every request that waits on the runtime or
//! on the client is answered from a spawned task; the loop only reads state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use agent_client_protocol::schema::v1::{
    ClientNotification, ClientRequest, ForkSessionRequest, InitializeRequest, ListSessionsRequest,
    LoadSessionRequest, McpServer, NewSessionRequest, PromptRequest, ResumeSessionRequest,
    SetSessionConfigOptionRequest,
};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, Responder};
use oino_ai::JsonValue;
use oino_client::{PiCommand, RpcRequest, SessionBackend};
use oino_core::runtime::SessionSource;
use oino_core::session::StreamingBehavior;
use oino_session::SessionManager;
use oino_tools::env::{ExecutionEnv, LocalEnv};
use serde_json::json;

use crate::client::{ClientCaps, ClientLink};
use crate::commands::{BuiltinCommand, session_report};
use crate::config::{config_options, set_config_option, set_mode};
use crate::convert::{clip, prompt_input};
use crate::env::AcpEnv;
use crate::error::AcpError;
use crate::factory::{SessionFactory, SessionRequest};
use crate::session::{AcpSession, Shared, replay};

/// Page size of `session/list`.
const LIST_PAGE: usize = 50;

/// Oino's ACP extension methods advertised in `agentCapabilities._meta.oino.extensions`.
pub const OINO_EXTENSIONS: [&str; 2] = ["_oino/extension_ui", "_oino/rpc"];

/// Bus topic carrying the client's `session/new` / `session/load` `mcpServers`
/// (`{servers: [...]}`; consumed by `unipi.mcp`, unipi-integrations R3).
pub const MCP_SERVERS_TOPIC: &str = "oino:acp:mcp-servers";

/// Publisher id of the ACP frontend on the bus.
pub const ACP_PUBLISHER: &str = "oino.acp";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Identity reported in `initialize` (`agentInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentInfo {
    /// Programmatic name.
    pub name: String,
    /// Display name.
    pub title: String,
    /// Version.
    pub version: String,
}

impl Default for AgentInfo {
    fn default() -> Self {
        Self {
            name: "oino".to_owned(),
            title: "Oino".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

#[derive(Default)]
struct AgentState {
    caps: ClientCaps,
    sessions: HashMap<String, Arc<AcpSession>>,
}

/// The ACP agent over a [`SessionFactory`].
pub struct AcpAgent {
    factory: Arc<dyn SessionFactory>,
    info: AgentInfo,
    default_cwd: PathBuf,
    state: Mutex<AgentState>,
}

impl std::fmt::Debug for AcpAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAgent")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

/// Result of a request handler: the wire-shaped response.
type Reply = Result<JsonValue, AcpError>;

fn respond(responder: Responder<JsonValue>, reply: Reply) {
    let result = match reply {
        Ok(value) => responder.respond(value),
        Err(error) => responder.respond_with_error(error.into()),
    };
    if let Err(error) = result {
        tracing::debug!("response dropped: {error}");
    }
}

impl AcpAgent {
    /// An agent creating sessions with `factory`; `default_cwd` scopes `session/list` without
    /// `cwd`.
    #[must_use]
    pub fn new(
        factory: Arc<dyn SessionFactory>,
        info: AgentInfo,
        default_cwd: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            factory,
            info,
            default_cwd,
            state: Mutex::new(AgentState::default()),
        })
    }

    /// Serves one ACP connection over `transport` until the client disconnects, then disposes
    /// every session.
    ///
    /// # Errors
    ///
    /// Transport and protocol failures reported by the SDK.
    pub async fn serve(
        self: Arc<Self>,
        transport: impl ConnectTo<Agent> + 'static,
    ) -> Result<(), agent_client_protocol::Error> {
        let requests = Arc::clone(&self);
        let notifications = Arc::clone(&self);
        let result = Agent
            .builder()
            .name(self.info.name.clone())
            .on_receive_request(
                async move |request: ClientRequest,
                            responder: Responder<JsonValue>,
                            cx: ConnectionTo<Client>| {
                    Arc::clone(&requests).dispatch(request, responder, cx);
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async move |notification: ClientNotification, _cx: ConnectionTo<Client>| {
                    notifications.notify(&notification);
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .connect_to(transport)
            .await;
        self.close_all().await;
        result
    }

    async fn close_all(&self) {
        let sessions: Vec<_> = lock(&self.state).sessions.drain().map(|(_, s)| s).collect();
        for session in sessions {
            session.close().await;
        }
    }

    fn session(&self, id: &str) -> Result<Arc<AcpSession>, AcpError> {
        lock(&self.state)
            .sessions
            .get(id)
            .cloned()
            .ok_or_else(|| AcpError::SessionNotFound(id.to_owned()))
    }

    fn caps(&self) -> ClientCaps {
        lock(&self.state).caps
    }

    fn notify(&self, notification: &ClientNotification) {
        if let ClientNotification::CancelNotification(cancel) = notification
            && let Ok(session) = self.session(&cancel.session_id.0)
        {
            session.shared.request_cancel();
            tokio::spawn(async move {
                if let Err(error) = session.backend.send(PiCommand::Abort.into()).await {
                    tracing::debug!("abort failed: {error}");
                }
            });
        }
    }

    /// Routes one request; long work runs in a task that answers `responder`.
    fn dispatch(
        self: Arc<Self>,
        request: ClientRequest,
        responder: Responder<JsonValue>,
        cx: ConnectionTo<Client>,
    ) {
        if let ClientRequest::InitializeRequest(initialize) = &request {
            respond(responder, Ok(self.initialize(initialize)));
            return;
        }
        tokio::spawn(async move {
            let reply = match request {
                ClientRequest::AuthenticateRequest(_) | ClientRequest::LogoutRequest(_) => {
                    Ok(json!({}))
                }
                ClientRequest::NewSessionRequest(request) => {
                    return self.new_session(request, responder, cx).await;
                }
                ClientRequest::LoadSessionRequest(request) => {
                    return self.load_session(request, responder, cx).await;
                }
                ClientRequest::ResumeSessionRequest(request) => {
                    return self.resume_session(request, responder, cx).await;
                }
                ClientRequest::ForkSessionRequest(request) => {
                    return self.fork_session(request, responder, cx).await;
                }
                ClientRequest::ListSessionsRequest(request) => self.list_sessions(request).await,
                ClientRequest::CloseSessionRequest(request) => {
                    self.close_session(&request.session_id.0).await
                }
                ClientRequest::DeleteSessionRequest(request) => {
                    self.delete_session(&request.session_id.0).await
                }
                ClientRequest::PromptRequest(request) => self.prompt(request).await,
                ClientRequest::SetSessionConfigOptionRequest(request) => {
                    self.set_config_option(request).await
                }
                ClientRequest::SetSessionModeRequest(request) => {
                    match self.session(&request.session_id.0) {
                        Ok(session) => set_mode(&session.session(), &request.mode_id.0)
                            .await
                            .map(|()| json!({})),
                        Err(error) => Err(error),
                    }
                }
                ClientRequest::ExtMethodRequest(request) => {
                    let params: JsonValue =
                        serde_json::from_str(request.params.get()).unwrap_or(JsonValue::Null);
                    self.ext_method(&request.method, params).await
                }
                _ => Err(AcpError::InvalidParams("Method not supported".to_owned())),
            };
            respond(responder, reply);
        });
    }

    fn initialize(&self, request: &InitializeRequest) -> JsonValue {
        lock(&self.state).caps = ClientCaps::from(&request.client_capabilities);
        json!({
            "protocolVersion": 1,
            "agentInfo": {
                "name": self.info.name,
                "title": self.info.title,
                "version": self.info.version,
            },
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {"image": true, "audio": false, "embeddedContext": true},
                "mcpCapabilities": {"http": false, "sse": false},
                "sessionCapabilities": {
                    "list": {}, "resume": {}, "close": {}, "delete": {}, "fork": {},
                },
                "_meta": {"oino": {"extensions": OINO_EXTENSIONS}},
            },
            "authMethods": [],
        })
    }

    /// Creates the runtime behind an ACP session and registers it.
    async fn open(
        &self,
        cwd: &Path,
        source: SessionSource,
        cx: ConnectionTo<Client>,
    ) -> Result<Arc<AcpSession>, AcpError> {
        let caps = self.caps();
        let link = ClientLink::new(cx, caps);
        let shared = Shared::new(link.clone(), cwd.to_path_buf());
        let env: Arc<dyn ExecutionEnv> =
            if caps.read_text_file || caps.write_text_file || caps.terminal {
                Arc::new(AcpEnv::new(link, Arc::clone(&shared) as _))
            } else {
                Arc::new(LocalEnv::new())
            };
        let runtime = self
            .factory
            .create(SessionRequest {
                cwd: cwd.to_path_buf(),
                source,
                env,
            })
            .await?;
        let session = Arc::new(AcpSession::start(runtime, shared));
        let previous = lock(&self.state)
            .sessions
            .insert(session.id.clone(), Arc::clone(&session));
        if let Some(previous) = previous {
            previous.close().await;
        }
        Ok(session)
    }

    /// Publishes the client's `mcpServers` on [`MCP_SERVERS_TOPIC`] so the bundled MCP extension
    /// connects them, and warns when no MCP extension is loaded to consume them.
    fn announce_mcp_servers(session: &AcpSession, servers: &[McpServer]) {
        let services = session.backend.runtime().services();
        services.host.publish_topic(
            ACP_PUBLISHER,
            MCP_SERVERS_TOPIC,
            json!({"servers": servers}),
            true,
        );
        if servers.is_empty() {
            return;
        }
        let has_mcp = services
            .extensions
            .extensions()
            .iter()
            .any(|extension| extension.instance.id() == "unipi.mcp");
        if !has_mcp {
            let _ = session.shared.link().update(json!({
                "sessionUpdate": "notice",
                "severity": "warning",
                "title": "MCP servers ignored: no MCP extension enabled",
            }));
        }
    }

    async fn new_session(
        &self,
        request: NewSessionRequest,
        responder: Responder<JsonValue>,
        cx: ConnectionTo<Client>,
    ) {
        let session = match self.open(&request.cwd, SessionSource::New, cx).await {
            Ok(session) => session,
            Err(error) => return respond(responder, Err(error)),
        };
        respond(
            responder,
            Ok(json!({
                "sessionId": session.id,
                "configOptions": config_options(&session.session()),
            })),
        );
        session.announce();
        Self::announce_mcp_servers(&session, &request.mcp_servers);
    }

    /// The session file of `id` under `cwd`.
    async fn find_session_file(&self, cwd: &Path, id: &str) -> Result<PathBuf, AcpError> {
        let root = self.factory.sessions_root(cwd).await?;
        let (cwd, id) = (cwd.to_path_buf(), id.to_owned());
        tokio::task::spawn_blocking(move || {
            let local = SessionManager::find_by_id(&cwd, &id, &root).ok().flatten();
            local
                .or_else(|| {
                    SessionManager::list_all(&root, None, None)
                        .ok()?
                        .into_iter()
                        .find(|s| s.id == id)
                        .map(|s| s.path)
                })
                .ok_or(AcpError::SessionNotFound(id))
        })
        .await
        .map_err(|e| AcpError::Factory(e.to_string()))?
    }

    async fn reopen(
        &self,
        cwd: &Path,
        id: &str,
        cx: ConnectionTo<Client>,
    ) -> Result<Arc<AcpSession>, AcpError> {
        if let Ok(session) = self.session(id) {
            return Ok(session);
        }
        let path = self.find_session_file(cwd, id).await?;
        self.open(cwd, SessionSource::Open(path), cx).await
    }

    async fn load_session(
        &self,
        request: LoadSessionRequest,
        responder: Responder<JsonValue>,
        cx: ConnectionTo<Client>,
    ) {
        let session = match self.reopen(&request.cwd, &request.session_id.0, cx).await {
            Ok(session) => session,
            Err(error) => return respond(responder, Err(error)),
        };
        replay(&session.session(), &session.cwd, session.shared.link());
        respond(
            responder,
            Ok(json!({ "configOptions": config_options(&session.session()) })),
        );
        session.announce();
        Self::announce_mcp_servers(&session, &request.mcp_servers);
    }

    async fn resume_session(
        &self,
        request: ResumeSessionRequest,
        responder: Responder<JsonValue>,
        cx: ConnectionTo<Client>,
    ) {
        let session = match self.reopen(&request.cwd, &request.session_id.0, cx).await {
            Ok(session) => session,
            Err(error) => return respond(responder, Err(error)),
        };
        respond(
            responder,
            Ok(json!({ "configOptions": config_options(&session.session()) })),
        );
        session.announce();
    }

    /// `session/fork` (unstable): a new session file copying the source's current branch.
    async fn fork_session(
        &self,
        request: ForkSessionRequest,
        responder: Responder<JsonValue>,
        cx: ConnectionTo<Client>,
    ) {
        let source = match self.session(&request.session_id.0) {
            Ok(session) => session.session().session_file(),
            Err(_) => self
                .find_session_file(&request.cwd, &request.session_id.0)
                .await
                .ok(),
        };
        let Some(path) = source else {
            return respond(
                responder,
                Err(AcpError::SessionNotFound(request.session_id.0.to_string())),
            );
        };
        let session = match self.open(&request.cwd, SessionSource::Fork(path), cx).await {
            Ok(session) => session,
            Err(error) => return respond(responder, Err(error)),
        };
        respond(
            responder,
            Ok(json!({
                "sessionId": session.id,
                "configOptions": config_options(&session.session()),
            })),
        );
        session.announce();
    }

    async fn list_sessions(&self, request: ListSessionsRequest) -> Reply {
        let cwd = request.cwd.clone();
        let root = self
            .factory
            .sessions_root(cwd.as_deref().unwrap_or(&self.default_cwd))
            .await?;
        let mut sessions = tokio::task::spawn_blocking(move || match &cwd {
            Some(cwd) => SessionManager::list(cwd, &root, None, None),
            None => SessionManager::list_all(&root, None, None),
        })
        .await
        .map_err(|e| AcpError::Factory(e.to_string()))?
        .map_err(|e| AcpError::Factory(e.to_string()))?;
        sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
        let offset: usize = request
            .cursor
            .as_deref()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let page: Vec<JsonValue> = sessions
            .iter()
            .skip(offset)
            .take(LIST_PAGE)
            .map(|info| {
                let title = info
                    .name
                    .clone()
                    .unwrap_or_else(|| clip(&info.first_message, 80));
                json!({
                    "sessionId": info.id,
                    "cwd": info.cwd,
                    "title": title,
                    "updatedAt": oino_session::time::format_iso(
                        oino_session::time::system_time_ms(info.modified)
                    ),
                })
            })
            .collect();
        let mut response = json!({ "sessions": page });
        if offset + LIST_PAGE < sessions.len() {
            response["nextCursor"] = json!((offset + LIST_PAGE).to_string());
        }
        Ok(response)
    }

    async fn close_session(&self, id: &str) -> Reply {
        let session = lock(&self.state).sessions.remove(id);
        match session {
            Some(session) => {
                session.close().await;
                Ok(json!({}))
            }
            None => Err(AcpError::SessionNotFound(id.to_owned())),
        }
    }

    /// `session/delete`: disposes the session and moves its file to `<agent_dir>/trash/`.
    async fn delete_session(&self, id: &str) -> Reply {
        let live = lock(&self.state).sessions.remove(id);
        let (cwd, file) = match &live {
            Some(session) => (session.cwd.clone(), session.session().session_file()),
            None => (self.default_cwd.clone(), None),
        };
        if let Some(session) = live {
            session.close().await;
        }
        let file = match file {
            Some(file) => file,
            None => self.find_session_file(&cwd, id).await?,
        };
        let trash = self.factory.agent_dir(&cwd).await?.join("trash");
        tokio::fs::create_dir_all(&trash)
            .await
            .map_err(|e| AcpError::Factory(e.to_string()))?;
        let name = file.file_name().map_or_else(
            || format!("{id}.jsonl"),
            |n| n.to_string_lossy().into_owned(),
        );
        tokio::fs::rename(&file, trash.join(name))
            .await
            .map_err(|e| AcpError::Factory(e.to_string()))?;
        Ok(json!({}))
    }

    async fn prompt(&self, request: PromptRequest) -> Reply {
        let session = self.session(&request.session_id.0)?;
        let input = prompt_input(&request.prompt);
        if input.images.is_empty()
            && let Some(command) = BuiltinCommand::parse(&input.text)
        {
            return run_builtin(&session, command).await;
        }
        let streaming = session.session().is_streaming();
        let ticket = session.shared.register_prompt();
        let sent = session
            .backend
            .send(
                PiCommand::Prompt {
                    message: input.text,
                    images: input.images,
                    streaming_behavior: streaming.then_some(StreamingBehavior::FollowUp),
                }
                .into(),
            )
            .await;
        match sent {
            Ok(data) => match data
                .as_ref()
                .and_then(|d| d.get("disposition"))
                .and_then(JsonValue::as_str)
            {
                Some("handled") => session.shared.resolve_prompt(&ticket, "end_turn"),
                Some("queued") => session.shared.prompt_queued(&ticket),
                _ => {}
            },
            Err(error) => {
                session.shared.resolve_prompt(&ticket, "end_turn");
                return Err(error.into());
            }
        }
        Ok(json!({ "stopReason": ticket.settled().await }))
    }

    async fn set_config_option(&self, request: SetSessionConfigOptionRequest) -> Reply {
        let session = self.session(&request.session_id.0)?;
        let value = serde_json::to_value(&request.value)?;
        let value = value.get("value").cloned().unwrap_or(value);
        let agent_session = session.session();
        set_config_option(&agent_session, &request.config_id.0, &value).await?;
        Ok(json!({ "configOptions": config_options(&agent_session) }))
    }

    /// `_oino/rpc {sessionId, command}` and `_oino/<command> {sessionId, …}`.
    async fn ext_method(&self, method: &str, params: JsonValue) -> Reply {
        // The SDK routes `_name` methods with the leading underscore removed.
        let method = if method.starts_with('_') {
            method.to_owned()
        } else {
            format!("_{method}")
        };
        let method = method.as_str();
        let session_id = params
            .get("sessionId")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| AcpError::InvalidParams("sessionId is required".to_owned()))?;
        let session = self.session(session_id)?;
        let command = if method == "_oino/rpc" {
            params.get("command").cloned().unwrap_or(JsonValue::Null)
        } else {
            let mut command = params.clone();
            if let Some(object) = command.as_object_mut() {
                object.shift_remove("sessionId");
                object.insert("type".to_owned(), json!(method));
            }
            command
        };
        let request =
            RpcRequest::from_value(command).map_err(|e| AcpError::InvalidParams(e.error))?;
        let data = session.backend.send(request.command).await?;
        Ok(data.unwrap_or(JsonValue::Null))
    }
}

/// Runs a built-in slash command and reports it as agent text.
async fn run_builtin(session: &AcpSession, command: BuiltinCommand) -> Reply {
    let agent_session = session.session();
    let text = match command {
        BuiltinCommand::Compact(instructions) => {
            let result = agent_session
                .compact(instructions)
                .await
                .map_err(|e| AcpError::Command(e.to_string()))?;
            format!(
                "Compacted the conversation ({} tokens before).",
                result.tokens_before
            )
        }
        BuiltinCommand::Name(name) => {
            agent_session
                .set_session_name(Some(name.clone()))
                .await
                .map_err(|e| AcpError::Command(e.to_string()))?;
            format!("Session name set to {name}.")
        }
        BuiltinCommand::Export(path) => {
            let path = agent_session
                .export_html(path.map(|p| crate::tools::resolve_path(&session.cwd, &p)))
                .await
                .map_err(|e| AcpError::Command(e.to_string()))?;
            format!("Exported to {}", path.display())
        }
        BuiltinCommand::Session => session_report(&agent_session),
    };
    session.shared.link().update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": text},
    }))?;
    Ok(json!({ "stopReason": "end_turn" }))
}
