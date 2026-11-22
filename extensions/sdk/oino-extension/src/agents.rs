//! Nested agent sessions (subagents, btw, sidekick; runtime §12). Permission `agents.spawn`.

use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::agents as wa;
use crate::error::parse;

/// A nested agent; closed when dropped.
pub struct Agent(wa::Agent);

impl Agent {
    /// Spawns a nested session from an `AgentSpec` JSON.
    pub async fn spawn(spec: &Value) -> Result<Self> {
        Ok(Self(wa::Agent::spawn(spec.to_string()).await?))
    }
    /// Agent id.
    #[must_use]
    pub fn id(&self) -> String {
        self.0.id()
    }
    /// Child session file.
    #[must_use]
    pub fn session_file(&self) -> Option<String> {
        self.0.session_file()
    }
    /// Prompts → disposition (`started`/`queued`).
    pub async fn prompt(&self, text: &str) -> Result<String> {
        Ok(self.0.prompt(text.to_owned(), None).await?)
    }
    /// Steering message.
    pub async fn steer(&self, text: &str) -> Result<()> {
        Ok(self.0.steer(text.to_owned()).await?)
    }
    /// Follow-up message.
    pub async fn follow_up(&self, text: &str) -> Result<()> {
        Ok(self.0.follow_up(text.to_owned()).await?)
    }
    /// Aborts the current run.
    pub fn abort(&self) {
        self.0.abort();
    }
    /// Sets thinking level → clamped.
    pub fn set_thinking(&self, level: &str) -> Result<String> {
        Ok(self.0.set_thinking(level)?)
    }
    /// Last assistant text.
    #[must_use]
    pub fn last_assistant_text(&self) -> Option<String> {
        self.0.last_assistant_text()
    }
    /// Waits until idle.
    pub async fn wait_idle(&self) -> Result<()> {
        Ok(self.0.wait_idle().await?)
    }
    /// Usage (`SessionStats` subset).
    pub fn usage(&self) -> Result<Value> {
        parse(&self.0.usage())
    }
    /// Child `ClientEvent` JSON stream.
    pub fn events(&self) -> impl Stream<Item = Value> + use<> {
        self.0
            .events()
            .into_stream()
            .map(|t: String| serde_json::from_str(&t).unwrap_or(Value::Null))
    }
    /// Closes after `grace_ms`.
    pub async fn close(self, grace_ms: u32) {
        self.0.close(grace_ms).await;
    }
}

/// Children of the current session.
pub fn list() -> Result<Vec<Value>> {
    parse(&wa::list())
}
