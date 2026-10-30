//! The shared error envelope (`AppError`) and `ProtoError` mapping.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable machine-readable error codes shared across host, core and UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum AppErrorCode {
    E_PROTO_VERSION,
    E_SCHEMA,
    E_UNAUTHORIZED,
    E_NOT_FOUND,
    E_UNSUPPORTED,
    E_TRANSPORT,
    E_TIMEOUT,
    E_MUX_UNAVAILABLE,
    E_BOARD_UNAVAILABLE,
    E_BOARD_REJECTED,
    E_CHAT_UNAVAILABLE,
    E_BLOB_REJECTED,
    E_BLOB_UNAVAILABLE,
    E_PAIRING_INVALID,
    E_HOSTKEY_MISMATCH,
    E_SETTINGS,
    E_INTERNAL,
    /// `fs_write`'s `base_hash` no longer matches the file on disk.
    E_CONFLICT,
    /// `fs_read` on a file that isn't valid UTF-8: route to the previewer/`blob_get`.
    E_BINARY,
    /// A path falls outside the host's allowed scope (not under `$HOME`
    /// and `outside:true` wasn't set, or it tries to escape it via a symlink).
    E_SCOPE,
    /// UNI-213: port forwarding is switched off on the host (the default)
    /// or not granted to this device.
    E_FORWARD_DISABLED,
    /// UNI-215: the remote browser is switched off on the host (the default)
    /// or not granted to this device.
    E_BROWSER_DISABLED,
    /// UNI-215: no Chromium-family browser found, or it failed to start /
    /// crashed (`detail.reason`: `not_found` | `launch_failed` | `crashed`).
    E_BROWSER_UNAVAILABLE,
    /// UNI-216: desktop streaming is switched off on the host (the default)
    /// or not granted to this device.
    E_DESKTOP_DISABLED,
    /// UNI-216: the desktop can't be streamed (`detail.reason`:
    /// `not_compiled` | `no_display` | `denied` | `capture_failed` |
    /// `encoder_failed` | `busy`).
    E_DESKTOP_UNAVAILABLE,
    /// UNI-217: a git command failed (not a repo, conflict, push rejected,
    /// auth needed…); `message` is git's own text, `hint` the next step.
    E_GIT,
}

impl AppErrorCode {
    /// Default retryability for this code, ignoring any explicit flag
    /// carried on an `AppErrorDto`.
    pub fn default_retryable(self) -> bool {
        matches!(
            self,
            AppErrorCode::E_TRANSPORT
                | AppErrorCode::E_TIMEOUT
                | AppErrorCode::E_MUX_UNAVAILABLE
                | AppErrorCode::E_BOARD_UNAVAILABLE
                | AppErrorCode::E_CHAT_UNAVAILABLE
                | AppErrorCode::E_BLOB_UNAVAILABLE
        )
    }
}

/// A single `path` segment: either a string key or a numeric index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathSegment {
    Key(String),
    Index(i64),
}

/// A non-empty error message. Deserializes with validation, mirroring
/// the TypeScript `z.string().min(1)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Message(String);

impl Message {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Builds a `Message` from any string, falling back to a generic
    /// placeholder if it is empty (never panics).
    pub fn new_or_fallback(value: impl Into<String>) -> Self {
        let value = value.into();
        if value.is_empty() {
            Message("Internal error".to_string())
        } else {
            Message(value)
        }
    }
}

impl TryFrom<String> for Message {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            Err("message must not be empty".to_string())
        } else {
            Ok(Message(value))
        }
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Message::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The one error envelope used everywhere (host -> core -> UI). See
/// `unipi-app/DESIGN.md` §5.5. Mirrors
/// `packages/protocol/src/error.ts`'s `AppErrorSchema`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppErrorDto {
    pub code: AppErrorCode,
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub path: Option<Vec<PathSegment>>,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<Value>,
}

impl AppErrorDto {
    pub fn new(code: AppErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        AppErrorDto {
            code,
            message: Message::new_or_fallback(message),
            hint: None,
            path: None,
            retryable,
            detail: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_path(mut self, path: Vec<PathSegment>) -> Self {
        self.path = Some(path);
        self
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Validates shape constraints that `serde` alone doesn't express:
    /// non-empty `message`. Call this after deserializing if strict
    /// validation (beyond type-checking) is required.
    pub fn is_valid(&self) -> bool {
        !self.message.as_str().is_empty()
    }
}

/// Per-layer protocol errors (transport, mux, board, settings,
/// pairing, schema) that map onto stable `AppErrorDto` codes.
#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("protocol version mismatch: {0}")]
    ProtoVersion(String),

    #[error("schema validation failed at {path:?}: {message}")]
    Schema {
        message: String,
        path: Vec<PathSegment>,
        detail: Option<String>,
    },

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("timed out: {0}")]
    Timeout(String),

    #[error("multiplexer unavailable: {0}")]
    MuxUnavailable(String),

    #[error("board unavailable: {0}")]
    BoardUnavailable(String),

    #[error("board rejected: {0}")]
    BoardRejected(String),

    #[error("pairing invalid: {0}")]
    PairingInvalid(String),

    #[error("host key mismatch: {0}")]
    HostkeyMismatch(String),

    #[error("settings error: {0}")]
    Settings(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl std::fmt::Display for PathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSegment::Key(k) => write!(f, "{k}"),
            PathSegment::Index(i) => write!(f, "{i}"),
        }
    }
}

impl ProtoError {
    /// Builds a `Schema` error from a `serde_path_to_error::Error`,
    /// capturing the exact failing path, mirroring the TypeScript
    /// `toAppError`'s handling of `ZodError`.
    pub fn from_path_error<E: std::fmt::Display>(err: serde_path_to_error::Error<E>) -> Self {
        let path = err
            .path()
            .iter()
            .filter_map(|segment| match segment {
                serde_path_to_error::Segment::Seq { index } => {
                    Some(PathSegment::Index(*index as i64))
                }
                serde_path_to_error::Segment::Map { key } => Some(PathSegment::Key(key.clone())),
                serde_path_to_error::Segment::Enum { variant } => {
                    Some(PathSegment::Key(variant.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let message = err.inner().to_string();
        ProtoError::Schema {
            message,
            path,
            detail: None,
        }
    }

    /// Converts this error into the shared wire-level `AppErrorDto`.
    pub fn to_app_error(&self) -> AppErrorDto {
        match self {
            ProtoError::ProtoVersion(msg) => {
                AppErrorDto::new(AppErrorCode::E_PROTO_VERSION, msg.clone(), false)
            }
            ProtoError::Schema {
                message,
                path,
                detail,
            } => {
                let mut dto = AppErrorDto::new(AppErrorCode::E_SCHEMA, message.clone(), false);
                if !path.is_empty() {
                    dto = dto.with_path(path.clone());
                }
                if let Some(detail) = detail {
                    dto = dto.with_detail(Value::String(detail.clone()));
                }
                dto
            }
            ProtoError::Unauthorized(msg) => {
                AppErrorDto::new(AppErrorCode::E_UNAUTHORIZED, msg.clone(), false)
            }
            ProtoError::NotFound(msg) => {
                AppErrorDto::new(AppErrorCode::E_NOT_FOUND, msg.clone(), false)
            }
            ProtoError::Unsupported(msg) => {
                AppErrorDto::new(AppErrorCode::E_UNSUPPORTED, msg.clone(), false)
            }
            ProtoError::Transport(msg) => {
                AppErrorDto::new(AppErrorCode::E_TRANSPORT, msg.clone(), true)
            }
            ProtoError::Timeout(msg) => {
                AppErrorDto::new(AppErrorCode::E_TIMEOUT, msg.clone(), true)
            }
            ProtoError::MuxUnavailable(msg) => {
                AppErrorDto::new(AppErrorCode::E_MUX_UNAVAILABLE, msg.clone(), true)
            }
            ProtoError::BoardUnavailable(msg) => {
                AppErrorDto::new(AppErrorCode::E_BOARD_UNAVAILABLE, msg.clone(), true)
            }
            ProtoError::BoardRejected(msg) => {
                AppErrorDto::new(AppErrorCode::E_BOARD_REJECTED, msg.clone(), false)
            }
            ProtoError::PairingInvalid(msg) => {
                AppErrorDto::new(AppErrorCode::E_PAIRING_INVALID, msg.clone(), false)
            }
            ProtoError::HostkeyMismatch(msg) => {
                AppErrorDto::new(AppErrorCode::E_HOSTKEY_MISMATCH, msg.clone(), false)
            }
            ProtoError::Settings(msg) => {
                AppErrorDto::new(AppErrorCode::E_SETTINGS, msg.clone(), false)
            }
            ProtoError::Internal(msg) => {
                AppErrorDto::new(AppErrorCode::E_INTERNAL, msg.clone(), false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_error_maps_to_e_schema_with_path() {
        let err = ProtoError::Schema {
            message: "expected string, got number".to_string(),
            path: vec![PathSegment::Key("cols".to_string())],
            detail: None,
        };
        let dto = err.to_app_error();
        assert_eq!(dto.code, AppErrorCode::E_SCHEMA);
        assert_eq!(dto.path, Some(vec![PathSegment::Key("cols".to_string())]));
        assert!(!dto.retryable);
    }

    #[test]
    fn transport_and_timeout_are_retryable_by_default() {
        assert!(AppErrorCode::E_TRANSPORT.default_retryable());
        assert!(AppErrorCode::E_TIMEOUT.default_retryable());
        assert!(AppErrorCode::E_MUX_UNAVAILABLE.default_retryable());
        assert!(AppErrorCode::E_BOARD_UNAVAILABLE.default_retryable());
    }

    #[test]
    fn internal_and_schema_are_not_retryable_by_default() {
        assert!(!AppErrorCode::E_INTERNAL.default_retryable());
        assert!(!AppErrorCode::E_SCHEMA.default_retryable());
        assert!(!AppErrorCode::E_PROTO_VERSION.default_retryable());
    }

    #[test]
    fn serializes_with_optional_fields_omitted() {
        let dto = AppErrorDto::new(AppErrorCode::E_TIMEOUT, "timed out", true);
        let json = serde_json::to_string(&dto).expect("serialize");
        assert!(!json.contains("hint"));
        assert!(!json.contains("path"));
        assert!(!json.contains("detail"));
    }

    #[test]
    fn new_falls_back_to_placeholder_for_empty_message() {
        let dto = AppErrorDto::new(AppErrorCode::E_INTERNAL, "", false);
        assert!(dto.is_valid());
        assert!(!dto.message.as_str().is_empty());
    }

    #[test]
    fn deserializing_an_empty_message_fails() {
        let json = r#"{"code":"E_INTERNAL","message":"","retryable":false}"#;
        let result: Result<AppErrorDto, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
