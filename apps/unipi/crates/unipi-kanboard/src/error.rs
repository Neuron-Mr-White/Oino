//! Errors. Rule violations name the rule that was broken so the message can be
//! shown to a human or an agent verbatim.

use std::fmt;

/// Crate result.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors of the board. `Usage` exits 2, everything else 1; JSON errors carry the `kind`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A storage/transition/format rule was violated (exit code 1).
    #[error("{0}")]
    Rule(String),
    /// Bad invocation that clap cannot catch (exit code 2).
    #[error("{0}")]
    Usage(String),
    /// Something expected is missing (exit code 1).
    #[error("{0}")]
    NotFound(String),
    /// A YAML value could not be parsed.
    #[error("{0}")]
    Yaml(String),
    /// A JSON value could not be (de)serialised.
    #[error("{0}")]
    Json(String),
    /// A filesystem or process failure.
    #[error("{0}")]
    Io(String),
}

impl Error {
    /// A rule violation (exit 1, kind `rule`).
    pub fn rule(message: impl Into<String>) -> Self {
        Error::Rule(message.into())
    }

    /// A usage error (exit 2, kind `usage`).
    pub fn usage(message: impl Into<String>) -> Self {
        Error::Usage(message.into())
    }

    /// Something that does not exist (exit 1, kind `not_found`).
    pub fn not_found(message: impl Into<String>) -> Self {
        Error::NotFound(message.into())
    }

    /// Process exit code for this error.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            _ => 1,
        }
    }

    /// Whether this is a rule violation.
    #[must_use]
    pub fn is_rule(&self) -> bool {
        matches!(self, Error::Rule(_) | Error::NotFound(_))
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Json(err.to_string())
    }
}

impl From<serde_yaml_ng::Error> for Error {
    fn from(err: serde_yaml_ng::Error) -> Self {
        Error::Yaml(err.to_string())
    }
}

/// A validation problem with a 1-based line number inside a task file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// File relative to the kanboard home.
    pub file: String,
    /// 1-based line number (0 = whole file).
    pub line: usize,
    /// What is wrong.
    pub message: String,
    /// `validate --fix` can repair it.
    pub fixable: bool,
}

impl Problem {
    /// A problem at `file:line`.
    pub fn new(file: impl fmt::Display, line: usize, message: impl Into<String>) -> Self {
        Problem {
            file: file.to_string(),
            line,
            message: message.into(),
            fixable: false,
        }
    }

    /// Marks the problem as fixable (or not).
    #[must_use]
    pub fn fixable(mut self, fixable: bool) -> Self {
        self.fixable = fixable;
        self
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}
