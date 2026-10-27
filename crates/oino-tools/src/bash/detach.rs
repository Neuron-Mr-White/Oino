//! Detaching running `bash` calls into background tasks (Oino addition, runtime §8 C11).
//!
//! Every running shell call registers under its tool-call id. Core's background-task service
//! calls [`BashDetachRegistry::detach`] with the task id it allocated; the call then returns at
//! once with the output so far plus `\n\n[Detached to background task {taskId}: {reason}]`
//! (`isError: false`) and the still-running process is handed to the caller.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{mpsc, oneshot};

use crate::env::SpawnedProcess;

/// A request sent to a running call.
#[derive(Debug)]
pub(crate) struct DetachRequest {
    pub(crate) task_id: String,
    pub(crate) reason: String,
    pub(crate) reply: oneshot::Sender<DetachedProcess>,
}

/// What a running call is asked for.
#[derive(Debug)]
pub(crate) enum CallMessage {
    /// Hand the process over.
    Detach(DetachRequest),
    /// Report the process id.
    Pid(oneshot::Sender<Option<u32>>),
}

/// A process taken over from a detached `bash` call.
#[derive(Debug)]
pub struct DetachedProcess {
    /// The running process; further output arrives on `process.output`.
    pub process: SpawnedProcess,
    /// The text the tool call returned before the detach marker.
    pub output_so_far: String,
    /// Temp file holding the complete output so far (when output was truncated).
    pub full_output_path: Option<String>,
}

/// Running shell calls by tool-call id. Cheap to clone; clones share the registry.
#[derive(Clone, Debug, Default)]
pub struct BashDetachRegistry {
    calls: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<CallMessage>>>>,
}

/// Removes the registration when the call ends.
pub(crate) struct Registration {
    registry: BashDetachRegistry,
    id: String,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.registry.lock().remove(&self.id);
    }
}

impl BashDetachRegistry {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, mpsc::UnboundedSender<CallMessage>>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn register(
        &self,
        tool_call_id: &str,
    ) -> (Registration, mpsc::UnboundedReceiver<CallMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.lock().insert(tool_call_id.to_owned(), tx);
        (
            Registration {
                registry: self.clone(),
                id: tool_call_id.to_owned(),
            },
            rx,
        )
    }

    /// Whether a shell call with this id is running.
    #[must_use]
    pub fn is_running(&self, tool_call_id: &str) -> bool {
        self.lock().contains_key(tool_call_id)
    }

    /// Detaches the running call `tool_call_id` into background task `task_id`. `None` when no
    /// such call runs or it finished before the request was seen.
    pub async fn detach(
        &self,
        tool_call_id: &str,
        task_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Option<DetachedProcess> {
        let sender = self.lock().get(tool_call_id).cloned()?;
        let (reply, response) = oneshot::channel();
        sender
            .send(CallMessage::Detach(DetachRequest {
                task_id: task_id.into(),
                reason: reason.into(),
                reply,
            }))
            .ok()?;
        response.await.ok()
    }

    /// Process id of the running call `tool_call_id` (`None` when it is not running or the
    /// process id is unknown).
    pub async fn pid(&self, tool_call_id: &str) -> Option<u32> {
        let sender = self.lock().get(tool_call_id).cloned()?;
        let (reply, response) = oneshot::channel();
        sender.send(CallMessage::Pid(reply)).ok()?;
        response.await.ok().flatten()
    }

    /// Tool-call ids of the running calls.
    #[must_use]
    pub fn running(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }
}
