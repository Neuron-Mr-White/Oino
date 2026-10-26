//! `auth.json` credential shapes (pi `Credential`).

use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};
use serde::{Deserialize, Serialize};

/// A stored API key (`{"type":"api_key","key":…,"env":{…}}`).
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    /// Key or config value (`!command`, `$ENV`); absent for env-only credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Variables that take priority over the process environment for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<IndexMap<String, String>>,
    /// Unknown fields, preserved on write.
    #[serde(flatten)]
    pub extra: JsonObject,
}

impl std::fmt::Debug for ApiKeyCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyCredential")
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .field("env", &self.env.as_ref().map(IndexMap::len))
            .finish_non_exhaustive()
    }
}

/// A stored OAuth token set (`{"type":"oauth","access","refresh","expires",…}`).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthCredential {
    /// Access token (sent as the API key).
    pub access: String,
    /// Refresh token.
    pub refresh: String,
    /// Expiry, Unix milliseconds.
    pub expires: i64,
    /// Provider-specific fields (account ids, scopes, …), preserved.
    #[serde(flatten)]
    pub extra: JsonObject,
}

impl std::fmt::Debug for OAuthCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthCredential")
            .field("expires", &self.expires)
            .finish_non_exhaustive()
    }
}

/// A credential in `auth.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthCredential {
    /// API key.
    ApiKey(ApiKeyCredential),
    /// OAuth tokens.
    Oauth(OAuthCredential),
}

/// Credential kind (pi `AuthType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    /// `api_key`.
    ApiKey,
    /// `oauth`.
    Oauth,
}

impl CredentialType {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Oauth => "oauth",
        }
    }
}

impl AuthCredential {
    /// An API-key credential with just `key`.
    #[must_use]
    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey(ApiKeyCredential {
            key: Some(key.into()),
            ..ApiKeyCredential::default()
        })
    }

    /// The credential kind.
    #[must_use]
    pub fn kind(&self) -> CredentialType {
        match self {
            Self::ApiKey(_) => CredentialType::ApiKey,
            Self::Oauth(_) => CredentialType::Oauth,
        }
    }

    /// JSON form (`type` tag included).
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        serde_json::to_value(self).unwrap_or(JsonValue::Null)
    }

    /// Validates a raw `auth.json` entry like pi's `ReadOnlyAuthStorage`: API keys need an
    /// optional string `key` and string-valued `env`; OAuth needs string `access`/`refresh`
    /// and a finite numeric `expires`.
    #[must_use]
    pub fn from_value(value: &JsonValue) -> Option<Self> {
        let object = value.as_object()?;
        match object.get("type").and_then(JsonValue::as_str)? {
            "api_key" => {
                let key_ok = object.get("key").is_none_or(JsonValue::is_string);
                let env_ok = object.get("env").is_none_or(|env| {
                    env.as_object()
                        .is_some_and(|m| m.values().all(JsonValue::is_string))
                });
                if !(key_ok && env_ok) {
                    return None;
                }
                serde_json::from_value(value.clone()).ok()
            }
            "oauth" => {
                let access = object.get("access")?.as_str()?.to_owned();
                let refresh = object.get("refresh")?.as_str()?.to_owned();
                let expires = object.get("expires")?.as_f64().filter(|n| n.is_finite())?;
                let extra = object
                    .iter()
                    .filter(|(k, _)| {
                        !matches!(k.as_str(), "type" | "access" | "refresh" | "expires")
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                Some(Self::Oauth(OAuthCredential {
                    access,
                    refresh,
                    expires: js_ms(expires),
                    extra,
                }))
            }
            _ => None,
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "expiry timestamps are integral milliseconds well inside i64"
)]
fn js_ms(value: f64) -> i64 {
    value.trunc() as i64
}

/// Credential metadata (pi `CredentialInfo`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInfo {
    /// Provider id.
    pub provider_id: String,
    /// Raw `type` of the entry (`api_key`, `oauth`, or anything else stored).
    #[serde(rename = "type")]
    pub kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_pi_shapes() {
        let value = json!({"type": "oauth", "access": "a", "refresh": "r", "expires": 1.0e12, "accountId": "x"});
        let credential = AuthCredential::from_value(&value).unwrap();
        assert_eq!(
            credential.to_value(),
            json!({"type": "oauth", "access": "a", "refresh": "r", "expires": 1_000_000_000_000_i64, "accountId": "x"})
        );
        let value = json!({"type": "api_key", "key": "k", "env": {"A": "b"}});
        assert_eq!(
            AuthCredential::from_value(&value).unwrap().to_value(),
            value
        );
        assert!(AuthCredential::from_value(&json!({"type": "api_key", "key": 5})).is_none());
        assert!(AuthCredential::from_value(&json!({"type": "oauth", "access": "a"})).is_none());
        assert!(AuthCredential::from_value(&json!({"type": "other"})).is_none());
    }
}
