//! [`RawJson`]: canonical JSON text carried across the WIT boundary (`type json = string`).

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// Canonical JSON text of a pi/Oino serde type (D5). Cheap to forward, parsed on demand.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawJson(pub Box<RawValue>);

impl RawJson {
    /// Parses `text`, rejecting invalid JSON.
    ///
    /// # Errors
    ///
    /// `text` is not valid JSON.
    pub fn parse(text: impl Into<String>) -> Result<Self, serde_json::Error> {
        RawValue::from_string(text.into()).map(Self)
    }

    /// Serialises any value.
    ///
    /// # Errors
    ///
    /// The value cannot be serialised (non-string map keys, failing `Serialize`).
    pub fn from_value<T: Serialize + ?Sized>(value: &T) -> Result<Self, serde_json::Error> {
        serde_json::value::to_raw_value(value).map(Self)
    }

    /// `null`.
    #[must_use]
    pub fn null() -> Self {
        Self::from_json_value(&serde_json::Value::Null)
    }

    /// Serialises a [`serde_json::Value`] (infallible).
    #[must_use]
    pub fn from_json_value(value: &serde_json::Value) -> Self {
        // A `Value` always serialises; fall back to `null` defensively.
        serde_json::value::to_raw_value(value)
            .map_or_else(|_| Self(RawValue::NULL.to_owned()), Self)
    }

    /// The JSON text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.get()
    }

    /// Consumes into the JSON text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0.get().to_owned()
    }

    /// Decodes into `T`.
    ///
    /// # Errors
    ///
    /// The JSON does not match `T`.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(self.as_str())
    }

    /// Decodes into a [`serde_json::Value`] (`null` on failure, which cannot happen for
    /// validated text).
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::from_str(self.as_str()).unwrap_or(serde_json::Value::Null)
    }
}

impl fmt::Debug for RawJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for RawJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq for RawJson {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str() || self.to_value() == other.to_value()
    }
}

impl From<serde_json::Value> for RawJson {
    fn from(value: serde_json::Value) -> Self {
        Self::from_json_value(&value)
    }
}

impl From<&serde_json::Value> for RawJson {
    fn from(value: &serde_json::Value) -> Self {
        Self::from_json_value(value)
    }
}

#[cfg(test)]
mod tests {
    use super::RawJson;

    #[test]
    fn round_trips_and_compares_semantically() {
        let a = RawJson::parse(r#"{"a":1,"b":[true]}"#).unwrap();
        let b = RawJson::parse(r#"{ "b": [true], "a": 1 }"#).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_value()["a"], 1);
        assert!(RawJson::parse("{").is_err());
        assert_eq!(RawJson::null().as_str(), "null");
    }
}
