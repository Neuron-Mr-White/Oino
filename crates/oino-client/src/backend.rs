//! The [`SessionBackend`] contract every frontend talks to (architecture.md §4.2).

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_core::events::ClientEvent;
use oino_core::ui::UiResponse;

use crate::protocol::{RpcCommand, RpcResponseData};

/// Failure of a backend command.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// The command failed; the text is pi's error message (RPC `error`).
    #[error("{0}")]
    Command(String),
    /// The backend is gone (session disposed, connection closed).
    #[error("backend closed")]
    Closed,
    /// Transport failure (spawn, I/O).
    #[error("transport: {0}")]
    Transport(String),
    /// The peer sent something that is not a valid record.
    #[error("protocol: {0}")]
    Protocol(String),
}

/// One live session as a frontend sees it: commands in, one ordered event stream out.
pub trait SessionBackend: Send + Sync {
    /// Any pi or `_oino/*` command; returns the response `data`.
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>>;

    /// [`SessionBackend::send`] with the RPC request id; `bash` streams
    /// `bash_execution_update{id}` with it (pi passes the command id to `executeBash`).
    fn send_with_id(
        &self,
        request_id: Option<String>,
        command: RpcCommand,
    ) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        let _ = request_id;
        self.send(command)
    }

    /// Ordered event stream for this session (a new subscription each call).
    fn events(&self) -> BoxStream<'static, ClientEvent>;

    /// Answers a dialog (`extension_ui_response`); idempotent, first answer wins.
    fn ui_respond(&self, response: UiResponse);

    /// Current session id.
    fn session_id(&self) -> String;

    /// Orderly shutdown (`session_shutdown{reason:"quit"}`); later commands fail with
    /// [`ClientError::Closed`].
    fn shutdown(&self) -> BoxFuture<'_, ()>;

    /// Resolves once an extension asked the frontend to exit (`ctx.shutdown()`, event
    /// `shutdown_requested`) and the session is idle; frontends then shut down and exit.
    /// Never resolves by default.
    fn shutdown_requested(&self) -> BoxFuture<'static, ()> {
        Box::pin(futures::future::pending())
    }
}
