//! Extension UI bridge (ui-protocol.md §2–§4).
//!
//! Extensions raise dialogs (`select`, `confirm`, `input`, `editor`, Oino dialogs) and
//! fire-and-forget updates (`notify`, `setStatus`, `setWidget`, …). The bridge publishes them
//! as `extension_ui_request` client events, resolves dialogs with the first valid
//! `extension_ui_response` (or the pi default on timeout / cancel / no UI), broadcasts
//! `extension_ui_dismiss`, and retains fire-and-forget state for late-joining frontends.
//! Wire types will move to `oino-protocol::ui`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// `extension_ui_request`: `{type, id, method, …fields, extensionId?, sessionId?}`.
#[derive(Clone, Debug, PartialEq)]
pub struct UiRequest {
    /// Request id (UUID v4).
    pub id: String,
    /// Method name (`select`, `setStatus`, …).
    pub method: String,
    /// Method fields, pi names.
    pub params: JsonObject,
    /// Owning extension (Oino).
    pub extension_id: Option<String>,
    /// Session (Oino).
    pub session_id: Option<String>,
}

impl Serialize for UiRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", "extension_ui_request")?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("method", &self.method)?;
        for (key, value) in &self.params {
            map.serialize_entry(key, value)?;
        }
        if let Some(extension) = &self.extension_id {
            map.serialize_entry("extensionId", extension)?;
        }
        if let Some(session) = &self.session_id {
            map.serialize_entry("sessionId", session)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for UiRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut object = JsonObject::deserialize(deserializer)?;
        if object.get("type").and_then(JsonValue::as_str) != Some("extension_ui_request") {
            return Err(serde::de::Error::custom("not an extension_ui_request"));
        }
        object.shift_remove("type");
        let take_string = |object: &mut JsonObject, key: &str| {
            object
                .shift_remove(key)
                .and_then(|v| v.as_str().map(str::to_owned))
        };
        let id =
            take_string(&mut object, "id").ok_or_else(|| serde::de::Error::missing_field("id"))?;
        let method = take_string(&mut object, "method")
            .ok_or_else(|| serde::de::Error::missing_field("method"))?;
        let extension_id = take_string(&mut object, "extensionId");
        let session_id = take_string(&mut object, "sessionId");
        Ok(Self {
            id,
            method,
            params: object,
            extension_id,
            session_id,
        })
    }
}

/// Answer of a dialog (pi response forms).
#[derive(Clone, Debug, PartialEq)]
pub enum UiAnswer {
    /// `{value}`.
    Value(JsonValue),
    /// `{confirmed}`.
    Confirmed(bool),
    /// `{cancelled: true}` (also the result of timeouts and missing UI).
    Cancelled,
}

/// `extension_ui_response` from a frontend.
#[derive(Clone, Debug, PartialEq)]
pub struct UiResponse {
    /// Request id.
    pub id: String,
    /// Answer.
    pub answer: UiAnswer,
}

impl Serialize for UiResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", "extension_ui_response")?;
        map.serialize_entry("id", &self.id)?;
        match &self.answer {
            UiAnswer::Value(value) => map.serialize_entry("value", value)?,
            UiAnswer::Confirmed(confirmed) => map.serialize_entry("confirmed", confirmed)?,
            UiAnswer::Cancelled => map.serialize_entry("cancelled", &true)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for UiResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let object = JsonObject::deserialize(deserializer)?;
        let id = object
            .get("id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| serde::de::Error::missing_field("id"))?
            .to_owned();
        let answer = if object.get("cancelled") == Some(&JsonValue::Bool(true)) {
            UiAnswer::Cancelled
        } else if let Some(confirmed) = object.get("confirmed").and_then(JsonValue::as_bool) {
            UiAnswer::Confirmed(confirmed)
        } else if let Some(value) = object.get("value") {
            UiAnswer::Value(value.clone())
        } else {
            UiAnswer::Cancelled
        };
        Ok(Self { id, answer })
    }
}

/// Why a dialog closed (`extension_ui_dismiss.reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DismissReason {
    /// A frontend answered.
    Answered,
    /// The timeout expired.
    Timeout,
    /// The caller cancelled.
    Cancelled,
}

/// Output of the bridge: requests to publish and dismissals.
#[derive(Clone, Debug, PartialEq)]
pub enum UiOutput {
    /// Publish `extension_ui_request`.
    Request(UiRequest),
    /// Publish `extension_ui_dismiss`.
    Dismiss {
        /// Request id.
        id: String,
        /// Why.
        reason: DismissReason,
    },
}

/// Where the bridge publishes; implemented by the session's event hub.
pub trait UiSink: Send + Sync {
    /// Publishes one output in order.
    fn publish(&self, output: UiOutput);
}

/// Dialog methods (everything else is fire-and-forget).
const DIALOG_METHODS: [&str; 10] = [
    "select",
    "confirm",
    "input",
    "editor",
    "multiSelect",
    "form",
    "approval",
    "custom",
    "clipboardReadImage",
    "editExternally",
];

/// Whether `method` waits for an answer.
#[must_use]
pub fn is_dialog(method: &str) -> bool {
    DIALOG_METHODS.contains(&method)
}

/// pi `UIPromptKind` of a dialog method (Oino-only dialogs map to `custom`).
#[must_use]
pub fn prompt_kind(method: &str) -> &'static str {
    match method {
        "select" => "select",
        "confirm" => "confirm",
        "input" => "input",
        "editor" => "editor",
        _ => "custom",
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct BridgeState {
    pending: HashMap<String, (oneshot::Sender<UiAnswer>, UiRequest)>,
    /// Retained fire-and-forget state keyed by `<method>:<extension>/<key>`.
    retained: IndexMap<String, UiRequest>,
}

/// Session-side UI bridge. Cheap to clone.
#[derive(Clone)]
pub struct UiBridge {
    has_ui: bool,
    session_id: String,
    sink: Arc<dyn UiSink>,
    state: Arc<Mutex<BridgeState>>,
}

impl std::fmt::Debug for UiBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiBridge")
            .field("has_ui", &self.has_ui)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl UiBridge {
    /// A bridge publishing to `sink`; without UI every dialog resolves as cancelled.
    #[must_use]
    pub fn new(has_ui: bool, session_id: impl Into<String>, sink: Arc<dyn UiSink>) -> Self {
        Self {
            has_ui,
            session_id: session_id.into(),
            sink,
            state: Arc::new(Mutex::new(BridgeState::default())),
        }
    }

    /// Whether dialog-capable UI is attached (pi `hasUI`).
    #[must_use]
    pub const fn has_ui(&self) -> bool {
        self.has_ui
    }

    fn request(&self, extension: Option<&str>, method: &str, params: JsonObject) -> UiRequest {
        UiRequest {
            id: uuid::Uuid::new_v4().to_string(),
            method: method.to_owned(),
            params,
            extension_id: extension.map(str::to_owned),
            session_id: Some(self.session_id.clone()),
        }
    }

    /// Raises a dialog and waits for the first answer, the timeout (`params.timeout` ms) or
    /// `cancel`. Returns [`UiAnswer::Cancelled`] without UI.
    pub async fn dialog(
        &self,
        extension: Option<&str>,
        method: &str,
        params: JsonObject,
        cancel: Option<CancellationToken>,
    ) -> UiAnswer {
        if !self.has_ui {
            return UiAnswer::Cancelled;
        }
        let timeout = params
            .get("timeout")
            .and_then(JsonValue::as_u64)
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis);
        let request = self.request(extension, method, params);
        let id = request.id.clone();
        let (tx, rx) = oneshot::channel();
        lock(&self.state)
            .pending
            .insert(id.clone(), (tx, request.clone()));
        self.sink.publish(UiOutput::Request(request));
        let cancelled = async {
            match &cancel {
                Some(token) => token.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let expired = async {
            match timeout {
                Some(duration) => tokio::time::sleep(duration).await,
                None => std::future::pending().await,
            }
        };
        let (answer, reason) = tokio::select! {
            answer = rx => (answer.unwrap_or(UiAnswer::Cancelled), None),
            () = expired => (UiAnswer::Cancelled, Some(DismissReason::Timeout)),
            () = cancelled => (UiAnswer::Cancelled, Some(DismissReason::Cancelled)),
        };
        if let Some(reason) = reason
            && lock(&self.state).pending.remove(&id).is_some()
        {
            self.sink.publish(UiOutput::Dismiss { id, reason });
        }
        answer
    }

    /// Delivers a frontend answer; the first answer wins, later ones are ignored.
    pub fn respond(&self, response: UiResponse) {
        let pending = lock(&self.state).pending.remove(&response.id);
        if let Some((tx, _)) = pending {
            let _ = tx.send(response.answer);
            self.sink.publish(UiOutput::Dismiss {
                id: response.id,
                reason: DismissReason::Answered,
            });
        }
    }

    /// Publishes a fire-and-forget request (dropped without UI) and retains keyed state.
    pub fn fire(&self, extension: Option<&str>, method: &str, params: JsonObject) {
        if !self.has_ui {
            return;
        }
        let request = self.request(extension, method, params);
        if let Some(key) = retention_key(&request) {
            lock(&self.state).retained.insert(key, request.clone());
        }
        self.sink.publish(UiOutput::Request(request));
    }

    /// Retained fire-and-forget state plus open dialogs, for a frontend that attaches late.
    #[must_use]
    pub fn snapshot(&self) -> Vec<UiRequest> {
        let state = lock(&self.state);
        state
            .retained
            .values()
            .cloned()
            .chain(state.pending.values().map(|(_, request)| request.clone()))
            .collect()
    }

    /// Cancels every open dialog (session shutdown).
    pub fn cancel_all(&self) {
        let pending: Vec<_> = lock(&self.state).pending.drain().collect();
        for (id, (tx, _)) in pending {
            let _ = tx.send(UiAnswer::Cancelled);
            self.sink.publish(UiOutput::Dismiss {
                id,
                reason: DismissReason::Cancelled,
            });
        }
    }
}

/// Key under which a fire-and-forget request replaces earlier state.
fn retention_key(request: &UiRequest) -> Option<String> {
    let field = match request.method.as_str() {
        "setStatus" => "statusKey",
        "setWidget" => "widgetKey",
        "setFrame" => "frameKey",
        "setHeader" | "setFooter" => "key",
        "openPanel" | "updatePanel" | "closePanel" => "panelId",
        "setTitle" | "setWorkingMessage" | "setShortcuts" => "",
        _ => return None,
    };
    let key = if field.is_empty() {
        String::new()
    } else {
        request.params.get(field)?.as_str()?.to_owned()
    };
    let family = match request.method.as_str() {
        "openPanel" | "updatePanel" | "closePanel" => "panel",
        other => other,
    };
    Some(format!(
        "{family}:{}/{key}",
        request.extension_id.as_deref().unwrap_or("")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<UiOutput>>);

    impl UiSink for Recorder {
        fn publish(&self, output: UiOutput) {
            lock(&self.0).push(output);
        }
    }

    fn params(value: JsonValue) -> JsonObject {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn request_and_response_wire_forms() {
        let request = UiRequest {
            id: "1".into(),
            method: "select".into(),
            params: params(json!({"title":"Pick","options":["a","b"]})),
            extension_id: Some("ext".into()),
            session_id: None,
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            value,
            json!({"type":"extension_ui_request","id":"1","method":"select","title":"Pick","options":["a","b"],"extensionId":"ext"})
        );
        assert_eq!(serde_json::from_value::<UiRequest>(value).unwrap(), request);
        let response: UiResponse = serde_json::from_value(
            json!({"type":"extension_ui_response","id":"1","confirmed":true}),
        )
        .unwrap();
        assert_eq!(response.answer, UiAnswer::Confirmed(true));
    }

    #[tokio::test]
    async fn first_answer_wins_and_dismisses() {
        let sink = Arc::new(Recorder::default());
        let bridge = UiBridge::new(true, "s", sink.clone());
        let dialog = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .dialog(
                        Some("e"),
                        "confirm",
                        params(json!({"title":"t","message":"m"})),
                        None,
                    )
                    .await
            })
        };
        tokio::task::yield_now().await;
        let id = loop {
            if let Some(UiOutput::Request(request)) = lock(&sink.0).first().cloned() {
                break request.id;
            }
            tokio::task::yield_now().await;
        };
        bridge.respond(UiResponse {
            id: id.clone(),
            answer: UiAnswer::Confirmed(true),
        });
        bridge.respond(UiResponse {
            id,
            answer: UiAnswer::Confirmed(false),
        });
        assert_eq!(dialog.await.unwrap(), UiAnswer::Confirmed(true));
        assert_eq!(lock(&sink.0).len(), 2);
    }

    #[tokio::test]
    async fn timeout_and_no_ui() {
        let sink = Arc::new(Recorder::default());
        let bridge = UiBridge::new(true, "s", sink.clone());
        let answer = bridge
            .dialog(
                None,
                "input",
                params(json!({"title":"t","timeout":10})),
                None,
            )
            .await;
        assert_eq!(answer, UiAnswer::Cancelled);
        assert!(matches!(
            lock(&sink.0).last(),
            Some(UiOutput::Dismiss {
                reason: DismissReason::Timeout,
                ..
            })
        ));
        let headless = UiBridge::new(false, "s", sink);
        assert_eq!(
            headless
                .dialog(None, "select", JsonObject::new(), None)
                .await,
            UiAnswer::Cancelled
        );
    }

    #[test]
    fn retains_keyed_state() {
        let sink = Arc::new(Recorder::default());
        let bridge = UiBridge::new(true, "s", sink);
        bridge.fire(
            Some("e"),
            "setStatus",
            params(json!({"statusKey":"k","statusText":"1"})),
        );
        bridge.fire(
            Some("e"),
            "setStatus",
            params(json!({"statusKey":"k","statusText":"2"})),
        );
        bridge.fire(Some("e"), "notify", params(json!({"message":"hi"})));
        let snapshot = bridge.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].params["statusText"], json!("2"));
    }
}
