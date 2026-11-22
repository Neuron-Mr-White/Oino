//! The agent's view of the ACP client: capabilities and typed agent → client calls.
//!
//! Outgoing payloads are built in their ACP wire shape (`serde_json::json!`) and decoded into
//! the schema types before sending, so a shape mistake fails loudly instead of reaching the
//! client.

use agent_client_protocol::schema::v1::{
    ClientCapabilities, CreateElicitationRequest, CreateElicitationResponse, CreateTerminalRequest,
    CreateTerminalResponse, KillTerminalRequest, ReadTextFileRequest, ReadTextFileResponse,
    ReleaseTerminalRequest, RequestPermissionRequest, RequestPermissionResponse,
    SessionNotification, TerminalOutputRequest, TerminalOutputResponse, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest,
};
use agent_client_protocol::{Client, ConnectionTo, JsonRpcRequest, UntypedMessage};
use oino_ai::JsonValue;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::sync::{Arc, OnceLock};

use crate::error::AcpError;

/// What the client advertised in `initialize` (only the parts Oino uses).
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per ACP client capability"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientCaps {
    /// `fs.readTextFile`.
    pub read_text_file: bool,
    /// `fs.writeTextFile`.
    pub write_text_file: bool,
    /// `terminal`.
    pub terminal: bool,
    /// `elicitation.form`.
    pub elicitation_form: bool,
}

impl From<&ClientCapabilities> for ClientCaps {
    fn from(caps: &ClientCapabilities) -> Self {
        Self {
            read_text_file: caps.fs.read_text_file,
            write_text_file: caps.fs.write_text_file,
            terminal: caps.terminal,
            elicitation_form: caps.elicitation.as_ref().is_some_and(|e| e.form.is_some()),
        }
    }
}

/// Decodes a wire-shaped value into an ACP schema type.
pub(crate) fn typed<T: DeserializeOwned>(value: JsonValue) -> Result<T, AcpError> {
    serde_json::from_value(value).map_err(AcpError::from)
}

/// Connection to the client scoped to one ACP session.
#[derive(Clone, Debug)]
pub struct ClientLink {
    cx: ConnectionTo<Client>,
    session_id: Arc<OnceLock<String>>,
    caps: ClientCaps,
}

impl ClientLink {
    /// A link whose session id is bound later with [`ClientLink::bind`] (a new session's id is
    /// known only once its runtime exists, after the tools were built).
    #[must_use]
    pub fn new(cx: ConnectionTo<Client>, caps: ClientCaps) -> Self {
        Self {
            cx,
            session_id: Arc::new(OnceLock::new()),
            caps,
        }
    }

    /// Binds the ACP session id (first call wins).
    pub fn bind(&self, session_id: &str) {
        let _ = self.session_id.set(session_id.to_owned());
    }

    /// The ACP session id (empty before [`ClientLink::bind`]).
    #[must_use]
    pub fn session_id(&self) -> &str {
        self.session_id.get().map_or("", String::as_str)
    }

    /// Client capabilities.
    #[must_use]
    pub const fn caps(&self) -> ClientCaps {
        self.caps
    }

    /// Sends `session/update` with `update` (wire shape, e.g. `{"sessionUpdate": …}`).
    pub(crate) fn update(&self, update: JsonValue) -> Result<(), AcpError> {
        let mut notification = json!({ "sessionId": self.session_id() });
        notification["update"] = update;
        let notification: SessionNotification = typed(notification)?;
        self.cx
            .send_notification(notification)
            .map_err(|_| AcpError::Closed)
    }

    /// Sends an Oino extension notification (`_oino/…`) with `sessionId` added.
    pub(crate) fn ext_notification(&self, method: &str, mut params: JsonValue) {
        if let Some(object) = params.as_object_mut() {
            object.insert("sessionId".to_owned(), json!(self.session_id()));
        }
        match UntypedMessage::new(method, params) {
            Ok(message) => {
                if self.cx.send_notification(message).is_err() {
                    tracing::debug!("ACP connection closed; dropped {method}");
                }
            }
            Err(error) => tracing::warn!("cannot encode {method}: {error}"),
        }
    }

    async fn call<Req>(&self, method: &'static str, request: Req) -> Result<Req::Response, AcpError>
    where
        Req: JsonRpcRequest,
    {
        self.cx
            .send_request(request)
            .block_task()
            .await
            .map_err(|error| AcpError::Client {
                method,
                message: error.message,
            })
    }

    /// `fs/read_text_file`.
    pub(crate) async fn read_text_file(&self, path: &str) -> Result<String, AcpError> {
        let request: ReadTextFileRequest =
            typed(json!({ "sessionId": self.session_id(), "path": path }))?;
        let response: ReadTextFileResponse = self.call("fs/read_text_file", request).await?;
        Ok(response.content)
    }

    /// `fs/write_text_file`.
    pub(crate) async fn write_text_file(&self, path: &str, content: &str) -> Result<(), AcpError> {
        let request: WriteTextFileRequest =
            typed(json!({ "sessionId": self.session_id(), "path": path, "content": content }))?;
        self.call("fs/write_text_file", request).await.map(|_| ())
    }

    /// `terminal/create`; returns the terminal id.
    pub(crate) async fn create_terminal(&self, spec: JsonValue) -> Result<String, AcpError> {
        let mut spec = spec;
        if let Some(object) = spec.as_object_mut() {
            object.insert("sessionId".to_owned(), json!(self.session_id()));
        }
        let request: CreateTerminalRequest = typed(spec)?;
        let response: CreateTerminalResponse = self.call("terminal/create", request).await?;
        Ok(response.terminal_id.0.to_string())
    }

    fn terminal_params(&self, terminal_id: &str) -> JsonValue {
        json!({ "sessionId": self.session_id(), "terminalId": terminal_id })
    }

    /// `terminal/output`.
    pub(crate) async fn terminal_output(
        &self,
        terminal_id: &str,
    ) -> Result<TerminalOutputResponse, AcpError> {
        let request: TerminalOutputRequest = typed(self.terminal_params(terminal_id))?;
        self.call("terminal/output", request).await
    }

    /// `terminal/wait_for_exit`.
    pub(crate) async fn wait_for_terminal_exit(
        &self,
        terminal_id: &str,
    ) -> Result<WaitForTerminalExitResponse, AcpError> {
        let request: WaitForTerminalExitRequest = typed(self.terminal_params(terminal_id))?;
        self.call("terminal/wait_for_exit", request).await
    }

    /// `terminal/kill`.
    pub(crate) async fn kill_terminal(&self, terminal_id: &str) -> Result<(), AcpError> {
        let request: KillTerminalRequest = typed(self.terminal_params(terminal_id))?;
        self.call("terminal/kill", request).await.map(|_| ())
    }

    /// `terminal/release`.
    pub(crate) async fn release_terminal(&self, terminal_id: &str) -> Result<(), AcpError> {
        let request: ReleaseTerminalRequest = typed(self.terminal_params(terminal_id))?;
        self.call("terminal/release", request).await.map(|_| ())
    }

    /// `session/request_permission`; returns the selected option id (`None` = cancelled).
    pub(crate) async fn request_permission(
        &self,
        tool_call: JsonValue,
        options: JsonValue,
    ) -> Result<Option<String>, AcpError> {
        let request: RequestPermissionRequest = typed(json!({
            "sessionId": self.session_id(),
            "toolCall": tool_call,
            "options": options,
        }))?;
        let response: RequestPermissionResponse =
            self.call("session/request_permission", request).await?;
        let outcome = serde_json::to_value(&response.outcome)?;
        Ok(outcome
            .get("optionId")
            .and_then(JsonValue::as_str)
            .map(str::to_owned))
    }

    /// `elicitation/create` in form mode; returns the accepted content (`None` = declined or
    /// cancelled).
    pub(crate) async fn elicit_form(
        &self,
        message: &str,
        schema: JsonValue,
        tool_call_id: Option<&str>,
    ) -> Result<Option<JsonValue>, AcpError> {
        let mut request = json!({
            "mode": "form",
            "sessionId": self.session_id(),
            "requestedSchema": schema,
            "message": message,
        });
        if let Some(id) = tool_call_id {
            request["toolCallId"] = json!(id);
        }
        let request: CreateElicitationRequest = typed(request)?;
        let response: CreateElicitationResponse = self.call("elicitation/create", request).await?;
        let response = serde_json::to_value(&response)?;
        if response.get("action").and_then(JsonValue::as_str) != Some("accept") {
            return Ok(None);
        }
        Ok(Some(response.get("content").cloned().unwrap_or(json!({}))))
    }
}
