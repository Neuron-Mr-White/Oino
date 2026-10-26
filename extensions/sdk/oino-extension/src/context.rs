//! Call contexts handed to handlers.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::bindings::oino::extension::types::{CallContext, CallKind, Mode};

/// Context of the current guest call (`call-context`). Host imports made while a handler runs
/// act on this call's session automatically.
#[derive(Clone, Debug)]
pub struct Ctx {
    raw: CallContext,
}

impl Ctx {
    pub(crate) fn new(raw: CallContext) -> Self {
        Self { raw }
    }

    /// Session id.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.raw.session_id
    }

    /// Run mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.raw.mode
    }

    /// A dialog-capable UI is attached (pi `ctx.hasUI`).
    #[must_use]
    pub fn has_ui(&self) -> bool {
        self.raw.has_ui
    }

    /// Session cwd (host path).
    #[must_use]
    pub fn cwd(&self) -> PathBuf {
        PathBuf::from(&self.raw.cwd)
    }

    /// Nested (child) agent session.
    #[must_use]
    pub fn is_child(&self) -> bool {
        self.raw.is_child
    }

    /// Child role for nested sessions.
    #[must_use]
    pub fn child_role(&self) -> Option<&str> {
        self.raw.child_role.as_deref()
    }

    /// What the call serves.
    #[must_use]
    pub fn kind(&self) -> CallKind {
        self.raw.kind
    }

    /// Raw WIT record.
    #[must_use]
    pub fn raw(&self) -> &CallContext {
        &self.raw
    }
}

/// Cooperative cancellation flag set when the host cancels the call.
#[derive(Clone, Debug, Default)]
pub struct Cancel(pub(crate) Rc<Cell<bool>>);

impl Cancel {
    /// The call was cancelled (pi `signal.aborted`).
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

/// Context of a tool call.
#[derive(Clone, Debug)]
pub struct ToolCtx {
    /// Base context.
    pub cx: Ctx,
    /// Tool call id.
    pub tool_call_id: String,
    /// Cancellation.
    pub cancel: Cancel,
}

impl ToolCtx {
    /// Session id.
    #[must_use]
    pub fn session_id(&self) -> &str {
        self.cx.session_id()
    }

    /// Reports partial output (pi `onUpdate`).
    pub fn progress(&self, partial: &crate::tool::ToolResult) {
        if let Ok(json) = serde_json::to_string(partial) {
            crate::bindings::oino::extension::control::tool_progress(&self.tool_call_id, &json);
        }
    }

    /// The call was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Context of a slash command or shortcut (enables command-context-only `control` calls).
#[derive(Clone, Debug)]
pub struct CommandCtx {
    /// Base context.
    pub cx: Ctx,
    /// Cancellation.
    pub cancel: Cancel,
}

impl CommandCtx {
    /// Session id.
    #[must_use]
    pub fn session_id(&self) -> &str {
        self.cx.session_id()
    }

    /// A dialog-capable UI is attached.
    #[must_use]
    pub fn has_ui(&self) -> bool {
        self.cx.has_ui()
    }
}
