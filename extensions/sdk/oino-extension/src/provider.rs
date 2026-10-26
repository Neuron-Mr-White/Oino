//! Provider components (feature `provider`): implement [`Provider`] and return it from
//! `Extension::provider`.

use futures::future::LocalBoxFuture;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::provider_host as wph;

/// A provider event stream handed to the host.
pub struct ProviderStream {
    pub(crate) reader: wit_bindgen::StreamReader<String>,
}

/// Writes `ProviderEvent` JSON items.
pub struct EventSink(wit_bindgen::StreamWriter<String>);

impl EventSink {
    /// Sends one event; `false` when the host dropped the stream (cancelled).
    pub async fn send(&mut self, event: &impl serde::Serialize) -> bool {
        let Ok(text) = serde_json::to_string(event) else {
            return false;
        };
        self.0.write_one(text).await.is_none()
    }
}

impl ProviderStream {
    /// Spawns `producer` writing events; returns the stream for the host.
    pub fn spawn<F, Fut>(producer: F) -> Self
    where
        F: FnOnce(EventSink) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let (tx, rx) = crate::bindings::wit_stream::new::<String>();
        wit_bindgen::spawn_local(producer(EventSink(tx)));
        Self { reader: rx }
    }
}

/// Provider exports.
pub trait Provider: 'static {
    /// `stream(ProviderStreamRequest)`.
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>>;
    /// `Vec<Model>` JSON for `provider_id`.
    fn list_models(
        &'static self,
        provider_id: String,
        credential: Option<Value>,
    ) -> LocalBoxFuture<'static, Result<Value>>;
    /// OAuth login → credential JSON.
    fn oauth_login(&'static self, _provider_id: String) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async { Err(crate::Error::msg("oauth not supported")) })
    }
    /// OAuth refresh → credential JSON.
    fn oauth_refresh(
        &'static self,
        _provider_id: String,
        _credential: Value,
    ) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async { Err(crate::Error::msg("oauth not supported")) })
    }
}

/// `provider-host.before-request`: lets `before_provider_*` handlers replace payload/headers.
pub async fn before_request(
    request_id: &str,
    payload: &Value,
    headers: Vec<(String, String)>,
) -> (Value, Vec<(String, String)>) {
    let (p, h) = wph::before_request(request_id.to_owned(), payload.to_string(), headers).await;
    (
        serde_json::from_str(&p).unwrap_or_else(|_| payload.clone()),
        h,
    )
}
/// `provider-host.after-response`.
pub fn after_response(request_id: &str, status: u16, headers: &[(String, String)]) {
    wph::after_response(request_id, status, headers);
}
/// `provider-host.stream-event`.
pub fn stream_event(request_id: &str, data: &Value) {
    wph::stream_event(request_id, &data.to_string());
}
