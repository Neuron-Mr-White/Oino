//! Event payloads. Payloads are pi-shaped JSON (they carry pi's `type` field).

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Result;
pub use crate::bindings::oino::extension::registry::EventKind;

/// One delivered event.
#[derive(Clone, Debug)]
pub struct Event {
    /// Kind.
    pub kind: EventKind,
    /// Payload JSON.
    pub payload: Value,
}

impl Event {
    /// Decodes the payload into a typed struct.
    ///
    /// # Errors
    ///
    /// The payload does not match `T`.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }

    /// A payload field.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.payload.get(key)
    }
}

/// Return types accepted from event handlers: `()` = no result (pi `undefined`), `Option<T>`,
/// or a JSON [`Value`].
pub trait IntoEventResult {
    /// Converts into the optional JSON result.
    fn into_event_result(self) -> Option<Value>;
}

impl IntoEventResult for () {
    fn into_event_result(self) -> Option<Value> {
        None
    }
}

impl<T: Serialize> IntoEventResult for Option<T> {
    fn into_event_result(self) -> Option<Value> {
        self.and_then(|v| serde_json::to_value(v).ok())
    }
}

impl IntoEventResult for Value {
    fn into_event_result(self) -> Option<Value> {
        Some(self)
    }
}

/// Wire name (pi `type`) of an event kind.
#[must_use]
pub fn wire_name(kind: EventKind) -> String {
    let debug = format!("{kind:?}");
    let name = debug.rsplit("::").next().unwrap_or(&debug);
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
