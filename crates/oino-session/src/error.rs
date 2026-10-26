//! Error type of the session crate.

use std::path::PathBuf;

use crate::entry::EntryId;

/// Errors raised by parsing, migration and [`crate::SessionManager`] operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// File-system failure.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// A line could not be (de)serialised.
    #[error("invalid JSON in {path} line {line}: {source}")]
    Json {
        /// Affected path (empty for in-memory data).
        path: PathBuf,
        /// 1-based line number (0 when not line based).
        line: usize,
        /// Underlying error.
        source: serde_json::Error,
    },
    /// Session id failed pi's validation (message text is pi's, verbatim).
    #[error(
        "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character"
    )]
    InvalidSessionId(String),
    /// Entry id not present.
    #[error("Entry {0} not found")]
    UnknownEntry(EntryId),
    /// Session id prefix matched several files.
    #[error("session id is ambiguous: {} matches", .0.len())]
    AmbiguousId(Vec<PathBuf>),
    /// File written by a newer session format.
    #[error("unsupported session version {0}")]
    UnsupportedVersion(u32),
    /// The file has content but no valid header line.
    #[error("Session file is not a valid Oino session: {}", .0.display())]
    MissingHeader(PathBuf),
    /// The header has no cwd and no override was supplied.
    #[error("session header has no cwd: {}", .0.display())]
    MissingCwd(PathBuf),
    /// The append is not allowed (reason given).
    #[error("invalid append: {0}")]
    InvalidAppend(&'static str),
    /// Required file does not exist.
    #[error("not found: {}", .0.display())]
    NotFound(PathBuf),
    /// Listing was cancelled through the cancel flag.
    #[error("operation cancelled")]
    Cancelled,
}

#[cfg(feature = "manager")]
impl SessionError {
    /// Wraps an I/O error with its path.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
