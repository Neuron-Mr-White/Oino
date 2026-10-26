//! Shared vocabulary of `types.wit`: ids, modes, call contexts, errors.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Extension id (`^[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtensionId(String);

/// The id does not match the manifest id grammar.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid extension id `{0}`: expected lowercase dot-separated segments of [a-z0-9-]")]
pub struct InvalidExtensionId(pub String);

impl ExtensionId {
    /// Validates and wraps `id`.
    ///
    /// # Errors
    ///
    /// `id` violates the grammar.
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidExtensionId> {
        let id = id.into();
        if is_valid_id(&id) {
            Ok(Self(id))
        } else {
            Err(InvalidExtensionId(id))
        }
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|segment| {
            let mut chars = segment.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

impl TryFrom<String> for ExtensionId {
    type Error = InvalidExtensionId;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExtensionId> for String {
    fn from(value: ExtensionId) -> Self {
        value.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ExtensionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// `log-level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Trace.
    Trace,
    /// Debug.
    Debug,
    /// Info.
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

/// `mode`: how the hosting process runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Interactive terminal.
    Tui,
    /// RPC.
    Rpc,
    /// JSON event stream.
    Json,
    /// Print mode.
    #[default]
    Print,
    /// ACP adapter.
    Acp,
    /// Desktop app.
    Desktop,
    /// Mobile app.
    Mobile,
}

/// `call-kind`: what a guest call serves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    /// `init`.
    Init,
    /// Event handler.
    #[default]
    Event,
    /// Tool execution.
    Tool,
    /// Slash command.
    Command,
    /// Shortcut.
    Shortcut,
    /// Service request.
    Service,
    /// Arbiter nudge.
    Nudge,
    /// Renderer.
    Render,
    /// Virtual model route.
    Route,
    /// Provider stream.
    Provider,
}

impl CallKind {
    /// Command-context-only operations are allowed (pi `ExtensionCommandContext`).
    #[must_use]
    pub fn is_command_context(self) -> bool {
        matches!(self, Self::Command | Self::Shortcut)
    }
}

/// `call-context`: carried by every guest call; host imports made while serving the call act
/// on its session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallContext {
    /// Session the call belongs to.
    pub session_id: String,
    /// Run mode.
    pub mode: Mode,
    /// Dialog-capable UI attached.
    pub has_ui: bool,
    /// Session cwd.
    pub cwd: PathBuf,
    /// Nested (child) session.
    pub is_child: bool,
    /// Child role for nested sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_role: Option<String>,
    /// Call kind.
    pub kind: CallKind,
    /// Set while serving a tool call (enables `control.execute-tool`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl CallContext {
    /// The same context for another call kind (clears the tool call id unless `Tool`).
    #[must_use]
    pub fn with_kind(&self, kind: CallKind) -> Self {
        Self {
            kind,
            tool_call_id: if kind == CallKind::Tool {
                self.tool_call_id.clone()
            } else {
                None
            },
            ..self.clone()
        }
    }
}

/// `host-error`: failure of a host import.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// Permission missing (names the permission).
    #[error("permission denied: {0}")]
    Denied(String),
    /// Session replaced since this call started (pi `assertActive`).
    #[error(
        "This extension ctx is stale after session replacement or reload. Do not use a captured pi or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload()."
    )]
    StaleContext,
    /// Session-replacing operations need a command or shortcut context.
    #[error("This operation is only available in command and shortcut contexts")]
    CommandContextRequired,
    /// Invalid argument.
    #[error("{0}")]
    InvalidArgument(String),
    /// Not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// Timed out.
    #[error("timed out")]
    Timeout,
    /// Cancelled.
    #[error("cancelled")]
    Cancelled,
    /// I/O failure.
    #[error("{0}")]
    Io(String),
    /// Anything else.
    #[error("{0}")]
    Failed(String),
}

impl HostError {
    /// The host does not implement `capability` (yet).
    #[must_use]
    pub fn unsupported(capability: &str) -> Self {
        Self::Failed(format!("{capability} is not supported by this host"))
    }

    /// Wraps an I/O error.
    #[must_use]
    pub fn io(err: &std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

/// Failure of a guest call (`ExtensionError{extension, call, message, trap}`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ExtensionError {
    /// Extension id (or path when the id is unknown).
    pub extension: String,
    /// Guest call (`init`, `on-event:tool_call`, `run-tool:bash`, …).
    pub call: String,
    /// Error text (pi `error.message`).
    pub message: String,
    /// The guest trapped (instance restarts lazily).
    pub trap: bool,
}

impl ExtensionError {
    /// A non-trap error.
    #[must_use]
    pub fn new(
        extension: impl Into<String>,
        call: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            extension: extension.into(),
            call: call.into(),
            message: message.into(),
            trap: false,
        }
    }

    /// A trap.
    #[must_use]
    pub fn trap(
        extension: impl Into<String>,
        call: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            trap: true,
            ..Self::new(extension, call, message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_grammar() {
        for ok in [
            "a",
            "unipi.background-tasks",
            "oino.openai-compat",
            "x1.y-2",
        ] {
            assert!(ExtensionId::new(ok).is_ok(), "{ok}");
        }
        for bad in ["", "A", "-a", "a..b", "a.", ".a", "a_b", "a/b"] {
            assert!(ExtensionId::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn with_kind_keeps_tool_id_only_for_tools() {
        let cx = CallContext {
            tool_call_id: Some("t1".into()),
            kind: CallKind::Tool,
            ..CallContext::default()
        };
        assert_eq!(cx.with_kind(CallKind::Event).tool_call_id, None);
        assert_eq!(
            cx.with_kind(CallKind::Tool).tool_call_id.as_deref(),
            Some("t1")
        );
        assert!(CallKind::Shortcut.is_command_context());
    }
}
