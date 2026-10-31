//! A small Chrome DevTools Protocol client over one websocket (the
//! browser-level endpoint), with flattened target sessions
//! (`Target.attachToTarget{flatten:true}` → commands carry `sessionId`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// One CDP event.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// A connected CDP client. Cheap to clone.
#[derive(Clone)]
pub struct Cdp {
    tx: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next: Arc<AtomicU64>,
    events: broadcast::Sender<Arc<CdpEvent>>,
    closed: Arc<AtomicBool>,
    closed_rx: tokio::sync::watch::Receiver<bool>,
}

/// How long one command may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

impl Cdp {
    /// Connects to `ws://127.0.0.1:<port>/devtools/browser/<id>`.
    pub async fn connect(ws_url: &str) -> anyhow::Result<Cdp> {
        let mut config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
        // Screencast frames arrive base64'd in one text message.
        config.max_message_size = Some(64 << 20);
        config.max_frame_size = Some(64 << 20);
        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(ws_url, Some(config), true).await?;
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(512);
        let closed = Arc::new(AtomicBool::new(false));
        let (closed_tx, closed_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let reader_pending = pending.clone();
        let reader_events = events.clone();
        let reader_closed = closed.clone();
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let text = match message {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    let sender = reader_pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                    if let Some(sender) = sender {
                        let result = match value.get("error") {
                            Some(error) => Err(error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("CDP error")
                                .to_string()),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = sender.send(result);
                    }
                } else if let Some(method) = value.get("method").and_then(Value::as_str) {
                    let _ = reader_events.send(Arc::new(CdpEvent {
                        method: method.to_string(),
                        params: value.get("params").cloned().unwrap_or(Value::Null),
                        session_id: value
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    }));
                }
            }
            reader_closed.store(true, Ordering::SeqCst);
            for (_, sender) in reader_pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
            {
                let _ = sender.send(Err("The browser connection closed".into()));
            }
            let _ = closed_tx.send(true);
        });

        Ok(Cdp {
            tx,
            pending,
            next: Arc::new(AtomicU64::new(1)),
            events,
            closed,
            closed_rx,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Resolves once the websocket is gone.
    pub async fn closed(&self) {
        let mut rx = self.closed_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
        self.events.subscribe()
    }

    fn post(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
        if self.is_closed() {
            return Err("The browser connection closed".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_string());
        }
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        self.tx
            .send(Message::Text(message.to_string().into()))
            .map_err(|_| "The browser connection closed".to_string())?;
        Ok(rx)
    }

    /// Sends a command and waits for its result.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        let rx = self.post(method, params, session)?;
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("The browser connection closed".into()),
            Err(_) => Err(format!("{method} timed out")),
        }
    }

    /// Sends a command without waiting (input that must not block the stream).
    pub fn fire(&self, method: &str, params: Value, session: Option<&str>) {
        let _ = self.post(method, params, session);
    }
}
