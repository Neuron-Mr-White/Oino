//! [`RpcBackend`]: a [`SessionBackend`] over an RPC connection, so remote frontends see the
//! same commands and events as in-process ones.
//!
//! The wire drops the cumulative `message_update.message`/`partial`; the decoder rebuilds
//! them from `message_start` and the deltas with [`PartialAssembler`] (the host's own
//! assembler), so [`ClientEvent`]s are complete again.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::stream::PartialAssembler;
use oino_ai::{AgentMessage, JsonValue, ProviderEvent, Usage};
use oino_client::protocol::ClientInfo;
use oino_client::{ClientError, OinoCommand, RpcCommand, RpcResponseData, SessionBackend};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};
use oino_core::ui::UiResponse;

use crate::client::{RpcClient, Transport};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Turns wire records back into [`ClientEvent`]s (one decoder per subscription).
#[derive(Debug, Default)]
pub struct EventDecoder {
    assembler: Option<PartialAssembler>,
}

impl EventDecoder {
    /// Decodes one wire record; `None` for records that are not client events.
    pub fn decode(&mut self, value: JsonValue) -> Option<ClientEvent> {
        if value.get("type").and_then(JsonValue::as_str) == Some("message_update") {
            let event: ProviderEvent =
                serde_json::from_value(value.get("assistantMessageEvent")?.clone()).ok()?;
            let assembler = self.assembler.as_mut()?;
            let assistant_message_event = assembler.apply(event);
            let mut partial = assembler.partial().clone();
            if let Some(usage) = value
                .get("usage")
                .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok())
            {
                partial.usage = usage;
            }
            return Some(ClientEvent::Session(AgentSessionEvent::MessageUpdate {
                message: AgentMessage::Assistant(partial),
                assistant_message_event,
            }));
        }
        let event: ClientEvent = serde_json::from_value(value).ok()?;
        match &event {
            ClientEvent::Session(AgentSessionEvent::MessageStart {
                message: AgentMessage::Assistant(message),
            }) => self.assembler = Some(PartialAssembler::new(message.clone())),
            ClientEvent::Session(AgentSessionEvent::MessageEnd { .. }) => self.assembler = None,
            _ => {}
        }
        Some(event)
    }
}

/// A session reached over RPC (`oino --mode rpc` child or `oino serve`).
#[derive(Clone)]
pub struct RpcBackend {
    client: RpcClient,
    session_id: Arc<Mutex<String>>,
}

impl std::fmt::Debug for RpcBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcBackend")
            .field("session_id", &*lock(&self.session_id))
            .finish_non_exhaustive()
    }
}

impl RpcBackend {
    /// Connects and opts into Oino events with `_oino/hello`.
    ///
    /// # Errors
    ///
    /// Spawn or handshake failures.
    pub async fn connect(
        transport: Transport,
        client: ClientInfo,
        ui: Option<JsonValue>,
    ) -> Result<Self, ClientError> {
        Self::over(RpcClient::connect(transport)?, client, ui).await
    }

    /// Wraps a connected client (sends `_oino/hello`).
    ///
    /// # Errors
    ///
    /// Handshake failures.
    pub async fn over(
        rpc: RpcClient,
        client: ClientInfo,
        ui: Option<JsonValue>,
    ) -> Result<Self, ClientError> {
        let hello = rpc
            .request(RpcCommand::Oino(OinoCommand::Hello {
                client,
                ui,
                protocol: 1,
            }))
            .await?;
        let session_id = hello
            .as_ref()
            .and_then(|h| h.get("sessionId"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| ClientError::Protocol("_oino/hello without sessionId".to_owned()))?
            .to_owned();
        let session_id = Arc::new(Mutex::new(session_id));
        let tracked = Arc::downgrade(&session_id);
        let mut events = rpc.events();
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if let Ok(ClientEvent::Oino(OinoEvent::SessionReplaced { session_id, .. })) =
                    serde_json::from_value::<ClientEvent>(event)
                {
                    let Some(tracked) = tracked.upgrade() else {
                        return;
                    };
                    *lock(&tracked) = session_id;
                }
            }
        });
        Ok(Self {
            client: rpc,
            session_id,
        })
    }

    /// The typed client.
    #[must_use]
    pub const fn client(&self) -> &RpcClient {
        &self.client
    }
}

impl SessionBackend for RpcBackend {
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        Box::pin(self.client.request(command))
    }

    fn events(&self) -> BoxStream<'static, ClientEvent> {
        let mut decoder = EventDecoder::default();
        self.client
            .events()
            .filter_map(move |value| futures::future::ready(decoder.decode(value)))
            .boxed()
    }

    fn ui_respond(&self, response: UiResponse) {
        let client = self.client.clone();
        tokio::spawn(async move {
            if let Err(error) = client.ui_respond(&response).await {
                tracing::warn!("extension_ui_response failed: {error}");
            }
        });
    }

    fn session_id(&self) -> String {
        lock(&self.session_id).clone()
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let _ = self.client.close().await;
        })
    }
}
