//! [`FakeBackend`]: scripted events and replies, recorded commands. Lets frontends
//! (tui-client, gpui) be built and tested without a runtime.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_core::events::ClientEvent;
use oino_core::ui::UiResponse;
use tokio::sync::broadcast;

use crate::backend::{ClientError, SessionBackend};
use crate::protocol::{RpcCommand, RpcResponseData};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Script {
    replies: HashMap<String, Result<RpcResponseData, String>>,
    commands: Vec<RpcCommand>,
    ui_responses: Vec<UiResponse>,
}

/// A scripted backend. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct FakeBackend {
    session_id: String,
    events: broadcast::Sender<ClientEvent>,
    script: Arc<Mutex<Script>>,
}

impl std::fmt::Debug for FakeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeBackend")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl FakeBackend {
    /// A backend for session `session_id`; unscripted commands succeed without data.
    #[must_use]
    pub fn new(session_id: impl Into<String>) -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            session_id: session_id.into(),
            events,
            script: Arc::new(Mutex::new(Script::default())),
        }
    }

    /// Replies to every command of type `kind` with `result`.
    pub fn reply(&self, kind: &str, result: Result<RpcResponseData, String>) {
        lock(&self.script).replies.insert(kind.to_owned(), result);
    }

    /// Publishes `event` to current subscribers.
    pub fn emit(&self, event: impl Into<ClientEvent>) {
        let _ = self.events.send(event.into());
    }

    /// Every command received, in order.
    #[must_use]
    pub fn commands(&self) -> Vec<RpcCommand> {
        lock(&self.script).commands.clone()
    }

    /// Every dialog answer received, in order.
    #[must_use]
    pub fn ui_responses(&self) -> Vec<UiResponse> {
        lock(&self.script).ui_responses.clone()
    }
}

impl SessionBackend for FakeBackend {
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        let kind = command.kind();
        let reply = {
            let mut script = lock(&self.script);
            script.commands.push(command);
            script.replies.get(kind).cloned()
        };
        Box::pin(async move {
            match reply {
                None => Ok(None),
                Some(result) => result.map_err(ClientError::Command),
            }
        })
    }

    fn events(&self) -> BoxStream<'static, ClientEvent> {
        let receiver = self.events.subscribe();
        futures::stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => return Some((event, receiver)),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }

    fn ui_respond(&self, response: UiResponse) {
        lock(&self.script).ui_responses.push(response);
    }

    fn session_id(&self) -> String {
        self.session_id.clone()
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PiCommand;
    use oino_core::events::AgentSessionEvent;
    use serde_json::json;

    #[tokio::test]
    async fn records_commands_and_replays_events() {
        let fake = FakeBackend::new("s1");
        fake.reply("get_state", Ok(Some(json!({"sessionId": "s1"}))));
        fake.reply("set_model", Err("Model not found: a/b".into()));
        let mut events = fake.events();
        fake.emit(AgentSessionEvent::AgentStart);
        assert_eq!(
            fake.send(PiCommand::GetState.into()).await.unwrap(),
            Some(json!({"sessionId": "s1"}))
        );
        assert_eq!(
            fake.send(
                PiCommand::SetModel {
                    provider: "a".into(),
                    model_id: "b".into()
                }
                .into()
            )
            .await,
            Err(ClientError::Command("Model not found: a/b".into()))
        );
        assert_eq!(fake.send(PiCommand::Abort.into()).await.unwrap(), None);
        assert_eq!(fake.commands().len(), 3);
        assert_eq!(
            events.next().await,
            Some(ClientEvent::Session(AgentSessionEvent::AgentStart))
        );
    }
}
