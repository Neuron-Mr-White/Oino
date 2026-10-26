//! Compaction errors (pi throws `Error` with these texts).

use thiserror::Error;

/// Failure of a compaction summary.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CompactionError {
    /// The provider returned an error or hit the token cap (pi `getSummarizationFailure`
    /// text, e.g. `Summarization failed: …`).
    #[error("{0}")]
    Summarization(String),
    /// The model called a tool instead of answering (label, e.g. `Summarization`).
    #[error("{0} attempted to call a tool")]
    ToolCall(&'static str),
    /// The preparation's first kept entry has no id.
    #[error("First kept entry has no UUID - session may need migration")]
    MissingFirstKept,
}
