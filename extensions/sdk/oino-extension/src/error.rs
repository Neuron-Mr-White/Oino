//! SDK error type; crosses the WIT boundary as its display text.

use crate::bindings::oino::extension::types::HostError;

/// SDK error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A host import failed.
    #[error("{0}")]
    Host(String),
    /// JSON (de)serialisation.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Anything else.
    #[error("{0}")]
    Message(String),
}

impl Error {
    /// A message error.
    pub fn msg(m: impl Into<String>) -> Self {
        Self::Message(m.into())
    }
}

/// SDK result.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<HostError> for Error {
    fn from(e: HostError) -> Self {
        Self::Host(match e {
            HostError::Denied(p) => format!("permission denied: {p}"),
            HostError::StaleContext => "stale context: the session was replaced".into(),
            HostError::CommandContextRequired => {
                "This operation is only available in command and shortcut contexts".into()
            }
            HostError::InvalidArgument(m) | HostError::Io(m) | HostError::Failed(m) => m,
            HostError::NotFound(m) => format!("not found: {m}"),
            HostError::Timeout => "timed out".into(),
            HostError::Cancelled => "cancelled".into(),
        })
    }
}

impl From<String> for Error {
    fn from(m: String) -> Self {
        Self::Message(m)
    }
}

impl From<&str> for Error {
    fn from(m: &str) -> Self {
        Self::Message(m.to_owned())
    }
}

/// Parses host JSON.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_str(text)?)
}
