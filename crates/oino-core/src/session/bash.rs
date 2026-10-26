//! `!` command execution (pi `executeBash`, `recordBashResult`, bash-executor.ts).

use oino_ai::{AgentMessage, BashExecutionMessage};
use serde::{Deserialize, Serialize};

use super::{AgentSession, lock};

/// pi `BashResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashResult {
    /// Combined output (possibly truncated).
    pub output: String,
    /// Exit code (`None` when killed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Cancelled by the user.
    pub cancelled: bool,
    /// Output was truncated.
    pub truncated: bool,
    /// Full output file when truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
}

impl AgentSession {
    /// pi `recordBashResult`: records a `bashExecution` message (deferred while streaming).
    pub fn record_bash_result(
        &self,
        command: &str,
        result: &BashResult,
        exclude_from_context: bool,
    ) {
        let message = BashExecutionMessage {
            command: command.to_owned(),
            output: result.output.clone(),
            exit_code: result.exit_code,
            cancelled: result.cancelled,
            truncated: result.truncated,
            full_output_path: result.full_output_path.clone(),
            timestamp: oino_agent::now_ms(),
            exclude_from_context: exclude_from_context.then_some(true),
        };
        if self.is_streaming() {
            lock(&self.inner.book).pending_bash.push(message);
            return;
        }
        if let Err(error) =
            lock(&self.inner.manager).append_message(AgentMessage::BashExecution(message))
        {
            tracing::error!("persisting bash execution failed: {error}");
        }
        self.refresh_context();
    }

    /// pi `_flushPendingBashMessages`.
    pub(crate) fn flush_pending_bash_messages(&self) {
        let pending = std::mem::take(&mut lock(&self.inner.book).pending_bash);
        if pending.is_empty() {
            return;
        }
        {
            let mut manager = lock(&self.inner.manager);
            for message in pending {
                if let Err(error) = manager.append_message(AgentMessage::BashExecution(message)) {
                    tracing::error!("persisting bash execution failed: {error}");
                }
            }
        }
        self.refresh_context();
    }
}
