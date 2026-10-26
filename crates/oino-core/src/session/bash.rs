//! `!` command execution (pi `executeBash`, `recordBashResult`, bash-executor.ts).

use oino_ai::{AgentMessage, BashExecutionMessage};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, SessionOpError, lock};
use crate::events::AgentSessionEvent;

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

impl From<oino_tools::BashOutcome> for BashResult {
    fn from(outcome: oino_tools::BashOutcome) -> Self {
        Self {
            output: outcome.output,
            exit_code: outcome.exit_code,
            cancelled: outcome.cancelled,
            truncated: outcome.truncated,
            full_output_path: outcome.full_output_path,
        }
    }
}

impl AgentSession {
    /// pi `executeBash`: `user_bash` handlers first (`{result}` short-circuits), then the
    /// configured shell with `shellCommandPrefix`; output streams as `bash_execution_update`
    /// and the result is recorded as a `bashExecution` message (`!!` excludes it from context).
    ///
    /// # Errors
    ///
    /// A failing `user_bash` handler or a command that could not run.
    pub async fn execute_bash(
        &self,
        command: String,
        exclude_from_context: bool,
        id: Option<String>,
    ) -> Result<BashResult, SessionOpError> {
        let runner = self.runner();
        if runner.has_handlers("user_bash") {
            let payload = json!({"type": "user_bash", "command": command, "excludeFromContext": exclude_from_context, "cwd": self.cwd().display().to_string()});
            match runner.emit_user_bash(payload).await {
                Ok(Some(result)) => {
                    let result: BashResult = serde_json::from_value(result)
                        .map_err(|e| SessionOpError::Failed(e.to_string()))?;
                    self.record_bash_result(&command, &result, exclude_from_context);
                    return Ok(result);
                }
                Ok(None) => {}
                Err(error) => return Err(SessionOpError::Failed(error)),
            }
        }
        let settings = lock(&self.inner.settings).snapshot();
        let resolved = match settings.shell_command_prefix() {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}\n{command}"),
            _ => command.clone(),
        };
        let options = oino_tools::ShellRunOptions {
            shell_path: settings.shell_path().map(|p| p.display().to_string()),
            agent_dir: self.agent_dir().display().to_string(),
            process_env: None,
        };
        let cancel = CancellationToken::new();
        let slot = {
            let mut cancels = lock(&self.inner.cancels);
            cancels.next_bash += 1;
            let slot = cancels.next_bash;
            cancels.bash.push((slot, cancel.clone()));
            slot
        };
        let session = self.clone();
        let mut on_chunk = move |delta: &str| {
            session.emit(AgentSessionEvent::BashExecutionUpdate {
                id: id.clone(),
                delta: delta.to_owned(),
            });
        };
        let env = oino_tools::LocalEnv::new();
        let cwd = self.cwd().display().to_string();
        let outcome =
            oino_tools::run_shell_command(&env, &resolved, &cwd, &options, &mut on_chunk, &cancel)
                .await;
        lock(&self.inner.cancels).bash.retain(|(id, _)| *id != slot);
        let result: BashResult = outcome.map_err(|e| SessionOpError::Failed(e.0))?.into();
        self.record_bash_result(&command, &result, exclude_from_context);
        Ok(result)
    }

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
