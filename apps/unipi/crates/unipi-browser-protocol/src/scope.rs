//! [`BrowserScope`] (browser-control.md §6.1).

use serde::{Deserialize, Serialize};

/// The caller of a command: `(profile, workspaceKey, sessionId)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserScope {
    /// Root conversation session (nested in-process agents resolve to their root).
    pub session_id: String,
    /// Workspace identity or canonical cwd.
    pub workspace: String,
    /// Explicit profile; `None` = the session's profile (setting `browser.profile`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// `browserScope: "own"` agent: isolated context, tabs closed with the agent.
    #[serde(default)]
    pub own: bool,
}

impl BrowserScope {
    /// Scope of a root session in `workspace`.
    #[must_use]
    pub fn session(session_id: impl Into<String>, workspace: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            workspace: workspace.into(),
            profile: None,
            own: false,
        }
    }
}
