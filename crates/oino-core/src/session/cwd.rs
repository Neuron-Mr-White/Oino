//! Moving a session's working directory (`AgentSession::change_cwd`).
//!
//! Append-only and prompt-cache safe: the header cwd, the system prompt (built from
//! [`super::Inner::cwd`], which never changes) and every earlier entry stay byte-identical.
//! The move is one `custom_message` entry of type [`CWD_CHANGE_CUSTOM_TYPE`] whose content
//! is a user-role reminder the model reads on its next turn, and whose `details` carry the
//! new and old folders so a reopened session keeps working in the new one.

use std::path::{Path, PathBuf};

use oino_ai::{CustomMessage, UserContentField};
use oino_session::CWD_CHANGE_CUSTOM_TYPE;
use serde_json::json;

use super::{AgentSession, PoisonError, SessionOpError};

/// The reminder appended to the conversation when the working directory moves from
/// `previous` to `cwd` (sent as a user-role message, so the system prompt stays as it was).
#[must_use]
pub fn cwd_change_reminder(previous: &Path, cwd: &Path) -> String {
    format!(
        "<system-reminder>\nThe workspace changed: from now on the working directory is {} \
         (was {}). Tools, shell commands and relative paths run in the new folder; paths \
         relative to the old folder no longer apply. The working directory named earlier in \
         this conversation is out of date.\n</system-reminder>",
        cwd.display(),
        previous.display()
    )
}

impl AgentSession {
    /// Moves the session's working directory to `cwd` from the next turn on.
    ///
    /// Appends one `oino.cwd_change` custom message (persisted, in context) and points the
    /// tools, `!bash`, completions and extension calls at `cwd`. Nothing earlier is
    /// rewritten: the header cwd, the system prompt and the earlier entries keep their
    /// bytes, so the provider's prompt cache still matches. Policy (which moves are
    /// allowed) belongs to the caller; the folder must exist.
    ///
    /// # Errors
    ///
    /// A run is in progress (move between turns), `cwd` is not an absolute existing
    /// directory, or it is already the working directory.
    pub async fn change_cwd(&self, cwd: PathBuf) -> Result<(), SessionOpError> {
        if self.is_streaming() || self.is_compacting() {
            return Err(SessionOpError::Failed(
                "The agent is working — move the chat once it has finished".to_owned(),
            ));
        }
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(SessionOpError::Failed(format!(
                "{} is not a folder",
                cwd.display()
            )));
        }
        let previous = self.work_cwd();
        if previous == cwd {
            return Err(SessionOpError::Failed(format!(
                "The chat already works in {}",
                cwd.display()
            )));
        }
        let message = CustomMessage {
            custom_type: CWD_CHANGE_CUSTOM_TYPE.to_owned(),
            content: UserContentField::Text(cwd_change_reminder(&previous, &cwd)),
            display: true,
            details: Some(json!({
                "cwd": cwd.display().to_string(),
                "previousCwd": previous.display().to_string(),
            })),
            timestamp: oino_agent::now_ms(),
        };
        // The new folder applies before the entry is published, so a frontend that reads
        // the state on `message_end` already sees it.
        *self
            .inner
            .work_cwd
            .write()
            .unwrap_or_else(PoisonError::into_inner) = cwd.clone();
        {
            let mut runner = self
                .inner
                .runner
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let mut cx = runner.call_context().clone();
            cx.cwd = cwd;
            runner.set_call_context(cx);
        }
        self.append_custom_message(message);
        // Durable before the caller reports success (the reopen must find the move).
        self.boundary_sync("change cwd");
        self.refresh_git_topic().await;
        Ok(())
    }
}
