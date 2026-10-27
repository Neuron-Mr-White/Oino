//! Browser capability (browser-control.md §8.2): the process-wide [`BrowserHost`] implemented
//! by `oino-browser`, handed to the WASM host and frontends. Wire types live in
//! [`oino_browser_protocol`] (re-exported as [`protocol`]).

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub use oino_browser_protocol as protocol;
use oino_browser_protocol::{
    BrowserCommand, BrowserResult, BrowserScope, BrowserStatus, BrowserViewEvent, FrontendRequest,
    Viewport,
};

use crate::runtime::BoxFut;

/// User approval attached to a resent command (`approval_required` handshake, §10.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    /// Not approved (the host may answer `approval_required`).
    #[default]
    None,
    /// Allow once.
    Once,
    /// Allow for this site in this session.
    Site,
}

/// One command plus its approval state (the WIT `execute` payload).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserRequest {
    /// The command.
    #[serde(flatten)]
    pub command: BrowserCommand,
    /// Approval granted by the user.
    #[serde(default)]
    pub approval: Approval,
}

impl From<BrowserCommand> for BrowserRequest {
    fn from(command: BrowserCommand) -> Self {
        Self {
            command,
            approval: Approval::None,
        }
    }
}

/// An attached viewer: its id and stream of frames / page events.
pub struct ViewStream {
    /// View id (ack / detach with it).
    pub view_id: u64,
    /// Frames and tab events.
    pub events: BoxStream<'static, BrowserViewEvent>,
}

impl std::fmt::Debug for ViewStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewStream")
            .field("view_id", &self.view_id)
            .finish_non_exhaustive()
    }
}

/// Process-wide browser service.
pub trait BrowserHost: Send + Sync {
    /// Runs one command in `scope`. Browser failures are results with `is_error`.
    fn execute(
        &self,
        scope: BrowserScope,
        request: BrowserRequest,
        cancel: CancellationToken,
    ) -> BoxFut<'_, BrowserResult>;

    /// Current status (`scope`: include that session's profile and tabs).
    fn status(&self, scope: Option<&BrowserScope>) -> BrowserStatus;

    /// Tab / reveal / dialog / handoff / status events of a session's viewers.
    fn subscribe(&self, session_id: &str) -> BoxStream<'static, BrowserViewEvent>;

    /// Attaches a screencast viewer to `tab` (owned by `session_id`).
    ///
    /// # Errors
    ///
    /// Unknown tab or the browser is not running.
    fn attach_view(
        &self,
        session_id: Option<&str>,
        tab: &str,
        viewport: Option<Viewport>,
    ) -> Result<ViewStream, String>;

    /// A frontend action (`_oino/browser_*`); returns JSON (tab info, selection text, …).
    fn frontend(
        &self,
        session_id: Option<&str>,
        workspace: &str,
        request: FrontendRequest,
    ) -> BoxFut<'_, Result<serde_json::Value, String>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_wire() {
        let r: BrowserRequest =
            serde_json::from_value(json!({"op": "snapshot", "approval": "once"}))
                .unwrap_or_else(|_| BrowserRequest::from(BrowserCommand::Status));
        assert_eq!(r.approval, Approval::Once);
        assert!(matches!(r.command, BrowserCommand::Snapshot { .. }));
    }
}
