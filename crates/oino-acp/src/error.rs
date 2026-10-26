//! Adapter errors and their JSON-RPC forms.

use agent_client_protocol::Error as RpcError;
use oino_client::ClientError;

/// A failure inside the adapter.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AcpError {
    /// Unknown ACP session id.
    #[error("Session not found: {0}")]
    SessionNotFound(String),
    /// The request is malformed for Oino (bad config value, unknown option, …).
    #[error("{0}")]
    InvalidParams(String),
    /// Creating or opening a session failed.
    #[error("{0}")]
    Factory(String),
    /// A session command failed (pi error text).
    #[error("{0}")]
    Command(String),
    /// The client rejected or failed a request the agent sent.
    #[error("client request {method} failed: {message}")]
    Client {
        /// ACP method.
        method: &'static str,
        /// Client error text.
        message: String,
    },
    /// The connection is gone.
    #[error("connection closed")]
    Closed,
    /// JSON shape mismatch between Oino and the ACP schema.
    #[error("protocol: {0}")]
    Protocol(String),
}

impl From<ClientError> for AcpError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Command(message) => Self::Command(message),
            ClientError::Closed => Self::Closed,
            other => Self::Protocol(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for AcpError {
    fn from(error: serde_json::Error) -> Self {
        Self::Protocol(error.to_string())
    }
}

impl From<AcpError> for RpcError {
    fn from(error: AcpError) -> Self {
        let code = match &error {
            AcpError::SessionNotFound(_) => RpcError::resource_not_found(None).code,
            AcpError::InvalidParams(_) => RpcError::invalid_params().code,
            _ => RpcError::internal_error().code,
        };
        RpcError::new(code.into(), error.to_string())
    }
}
