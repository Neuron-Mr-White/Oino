//! Settings scopes and errors (pi `SettingsScope`, `SettingsError`).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The settings layer a read or write targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsScope {
    /// `<agent>/settings.json`.
    Global,
    /// `<cwd>/.oino/settings.json` (only when the project is trusted).
    Project,
}

impl fmt::Display for SettingsScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Global => "global",
            Self::Project => "project",
        })
    }
}

/// What went wrong with a settings file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsErrorKind {
    /// The file could not be read.
    #[error("{0}")]
    Read(String),
    /// The file is not valid JSON.
    #[error("{0}")]
    Parse(String),
    /// The file is valid JSON but not an object.
    #[error("settings must be a JSON object")]
    NotAnObject,
    /// The lock file could not be created or locked.
    #[error("failed to lock settings: {0}")]
    Lock(String),
    /// The file could not be written.
    #[error("{0}")]
    Write(String),
    /// A project write was attempted while the project is untrusted (pi message).
    #[error("Project is not trusted; refusing to write project settings")]
    ProjectUntrusted,
}

/// A settings diagnostic (pi `SettingsError{scope, path?, error}`); collected by the manager and
/// returned by [`crate::SettingsManager::drain_errors`], or returned directly by writes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct SettingsError {
    /// Layer the error belongs to.
    pub scope: SettingsScope,
    /// File path when file-backed.
    pub path: Option<PathBuf>,
    /// Cause.
    pub kind: SettingsErrorKind,
}

impl SettingsError {
    /// The human-readable message (the error's own message, without the path prefix).
    #[must_use]
    pub fn message(&self) -> String {
        self.kind.to_string()
    }
}

impl fmt::Display for SettingsError {
    /// pi `collectSettingsDiagnostics` wording.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "Invalid settings file {}: {}", path.display(), self.kind),
            None => write!(f, "Invalid {} settings: {}", self.scope, self.kind),
        }
    }
}

/// A setting value that a validating getter rejects (pi throws `Error` with these messages).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InvalidSettingError {
    /// pi's exact message, e.g. `Invalid httpIdleTimeoutMs setting: abc`.
    pub message: String,
}

impl InvalidSettingError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
