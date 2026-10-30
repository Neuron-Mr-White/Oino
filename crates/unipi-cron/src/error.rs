//! Typed errors for the cron scheduler ([`CronError`]).

use std::path::PathBuf;

/// Everything the scheduler can fail with.
#[derive(Debug, thiserror::Error)]
pub enum CronError {
    /// The cron directory could not be resolved (no home directory, bad `$OINO_AGENT_DIR`).
    #[error("no cron directory: {0}")]
    NoDir(String),
    /// A file could not be read or written.
    #[error("{1}: {0}")]
    Io(#[source] std::io::Error, PathBuf),
    /// JSON in a cron file is malformed.
    #[error("{1}: {0}")]
    Json(#[source] serde_json::Error, PathBuf),
    /// Another scheduler process owns the tick loop already.
    #[error("another scheduler already owns cron ({0})")]
    AlreadyOwned(String),
    /// The daemon is not running (ops that need it fail with this).
    #[error(
        "the cron scheduler is not running — start it with `oino cron serve` or `oino cron install`"
    )]
    NotRunning,
    /// A control request failed (message meant to be shown).
    #[error("{0}")]
    Control(String),
}

impl CronError {
    /// An I/O error with the path it happened on.
    pub fn io(error: std::io::Error, path: impl Into<PathBuf>) -> Self {
        Self::Io(error, path.into())
    }

    /// A JSON error with the file it happened in.
    pub fn json(error: serde_json::Error, path: impl Into<PathBuf>) -> Self {
        Self::Json(error, path.into())
    }

    /// A control-op failure with a user-facing message.
    pub fn control(message: impl Into<String>) -> Self {
        Self::Control(message.into())
    }
}

/// `Result<T, CronError>`.
pub type Result<T> = std::result::Result<T, CronError>;
