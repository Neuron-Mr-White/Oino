//! Thin CDP client (browser-control.md §4.5): one reader task, request ids with a `oneshot`
//! per call, a broadcast of events keyed by flattened `sessionId`, deadlines on every call.

mod transport;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};

pub(crate) use transport::Transport;

/// Default deadline of one CDP call.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// CDP failures.
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum CdpError {
    /// The browser answered with a protocol error.
    #[error("{method}: {message}")]
    Protocol {
        /// Method.
        method: String,
        /// Message.
        message: String,
    },
    /// The connection is gone (browser exited or crashed).
    #[error("browser connection closed")]
    Closed,
    /// No reply in time.
    #[error("{0}: no reply in time")]
    Timeout(String),
}

/// One CDP event.
#[derive(Debug)]
pub(crate) struct CdpEvent {
    /// Flattened session (`None`: browser target).
    pub(crate) session: Option<String>,
    /// `Domain.event`.
    pub(crate) method: String,
    /// Params.
    pub(crate) params: Value,
}

type Pending = Mutex<HashMap<u64, (String, oneshot::Sender<Result<Value, CdpError>>)>>;

/// A connection to one browser process.
pub(crate) struct Cdp {
    out: mpsc::UnboundedSender<String>,
    pending: Arc<Pending>,
    next: AtomicU64,
    events: broadcast::Sender<Arc<CdpEvent>>,
    closed: Arc<AtomicBool>,
}

impl std::fmt::Debug for Cdp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cdp")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Cdp {
    /// Starts the reader/writer over `transport`.
    pub(crate) fn start(transport: Transport) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<String>();
        transport.spawn(out_rx, in_tx);
        let (events, _) = broadcast::channel(4096);
        let cdp = Arc::new(Self {
            out: out_tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next: AtomicU64::new(1),
            events,
            closed: Arc::new(AtomicBool::new(false)),
        });
        let pending = Arc::clone(&cdp.pending);
        let closed = Arc::clone(&cdp.closed);
        let events = cdp.events.clone();
        tokio::spawn(async move {
            while let Some(text) = in_rx.recv().await {
                dispatch(&text, &pending, &events);
            }
            closed.store(true, Ordering::SeqCst);
            for (_, (_, tx)) in lock(&pending).drain() {
                let _ = tx.send(Err(CdpError::Closed));
            }
            let _ = events.send(Arc::new(CdpEvent {
                session: None,
                method: "Oino.disconnected".into(),
                params: Value::Null,
            }));
        });
        cdp
    }

    /// The connection ended.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Event subscription.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
        self.events.subscribe()
    }

    /// Calls `method` on the browser (`session = None`) or a flattened session.
    pub(crate) async fn call(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, CdpError> {
        self.call_timeout(session, method, params, CALL_TIMEOUT)
            .await
    }

    /// [`Self::call`] with an explicit deadline.
    pub(crate) async fn call_timeout(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        if self.is_closed() {
            return Err(CdpError::Closed);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = Value::String(s.to_owned());
        }
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, (method.to_owned(), tx));
        if self.out.send(msg.to_string()).is_err() {
            lock(&self.pending).remove(&id);
            return Err(CdpError::Closed);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(CdpError::Closed),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(CdpError::Timeout(method.to_owned()))
            }
        }
    }

    /// Sends without waiting for the reply (acks).
    pub(crate) fn fire(&self, session: Option<&str>, method: &str, params: &Value) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = Value::String(s.to_owned());
        }
        let _ = self.out.send(msg.to_string());
    }
}

fn dispatch(text: &str, pending: &Pending, events: &broadcast::Sender<Arc<CdpEvent>>) {
    let Ok(mut msg) = serde_json::from_str::<Value>(text) else {
        tracing::debug!(len = text.len(), "cdp: undecodable message");
        return;
    };
    if let Some(id) = msg.get("id").and_then(Value::as_u64) {
        let Some((method, tx)) = lock(pending).remove(&id) else {
            return;
        };
        let reply = match msg.get_mut("error") {
            Some(err) => Err(CdpError::Protocol {
                method,
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_owned(),
            }),
            None => Ok(msg.get_mut("result").map_or(Value::Null, Value::take)),
        };
        let _ = tx.send(reply);
        return;
    }
    let Some(method) = msg.get("method").and_then(Value::as_str).map(str::to_owned) else {
        return;
    };
    let session = msg
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let params = msg.get_mut("params").map_or(Value::Null, Value::take);
    let _ = events.send(Arc::new(CdpEvent {
        session,
        method,
        params,
    }));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replies_and_events_are_routed() {
        let (transport, mut browser_in, browser_out) = Transport::memory();
        let cdp = Cdp::start(transport);
        let mut events = cdp.subscribe();
        let call = {
            let cdp = Arc::clone(&cdp);
            tokio::spawn(async move { cdp.call(Some("S1"), "Page.enable", json!({})).await })
        };
        let sent: Value = serde_json::from_str(&browser_in.recv().await.unwrap()).unwrap();
        assert_eq!(sent["method"], "Page.enable");
        assert_eq!(sent["sessionId"], "S1");
        browser_out
            .send(
                json!({"method":"Page.loadEventFired","sessionId":"S1","params":{"timestamp":1}})
                    .to_string(),
            )
            .unwrap();
        browser_out
            .send(json!({"id": sent["id"], "result": {"ok": true}}).to_string())
            .unwrap();
        assert_eq!(call.await.unwrap().unwrap(), json!({"ok": true}));
        let e = events.recv().await.unwrap();
        assert_eq!(e.method, "Page.loadEventFired");
        assert_eq!(e.session.as_deref(), Some("S1"));
        drop(browser_out);
        let e = events.recv().await.unwrap();
        assert_eq!(e.method, "Oino.disconnected");
        assert!(matches!(
            cdp.call(None, "Browser.getVersion", json!({})).await,
            Err(CdpError::Closed)
        ));
    }
}
