//! Provider error texts: the Anthropic/`OpenAI` SDK `APIError` messages and pi-ai's
//! `utils/error-body.js` (`normalizeProviderError` + `formatProviderError`).

use serde_json::Value;

/// Maximum body characters kept in error texts.
pub const MAX_PROVIDER_ERROR_BODY_CHARS: usize = 4000;

/// The SDK error thrown for a failed request or an in-stream error payload.
#[derive(Clone, Debug, PartialEq)]
pub struct SdkError {
    /// HTTP status (absent for in-stream errors).
    pub status: Option<u16>,
    /// SDK `error.message`.
    pub message: String,
    /// SDK `error.error` (parsed error object), when it is a non-empty object.
    pub error: Option<Value>,
}

/// SDK `APIError.makeMessage(status, error, message)`.
fn make_message(status: Option<u16>, error: Option<&Value>, message: Option<&str>) -> String {
    let msg = match error {
        Some(error) => match error.get("message") {
            Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
            Some(other) if is_truthy(other) => Some(other.to_string()),
            _ => Some(error.to_string()),
        },
        None => message.filter(|m| !m.is_empty()).map(str::to_owned),
    };
    match (status, msg) {
        (Some(status), Some(msg)) => format!("{status} {msg}"),
        (Some(status), None) => format!("{status} status code (no body)"),
        (None, Some(msg)) => msg,
        (None, None) => "(no status code or body)".to_owned(),
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Anthropic SDK error for a non-2xx response (`error` = the whole JSON body).
#[must_use]
pub fn anthropic_status_error(status: u16, body: &str) -> SdkError {
    let json: Option<Value> = serde_json::from_str(body).ok();
    let message = make_message(Some(status), json.as_ref(), Some(body));
    SdkError {
        status: Some(status),
        message,
        error: json,
    }
}

/// `OpenAI` SDK error for a non-2xx response (`error` = `body.error`, or the body).
#[must_use]
pub fn openai_status_error(status: u16, body: &str) -> SdkError {
    let json: Option<Value> = serde_json::from_str(body).ok();
    let error = json.map(|json| match json.get("error") {
        Some(inner) if !inner.is_null() => inner.clone(),
        _ => json,
    });
    let message = make_message(Some(status), error.as_ref(), Some(body));
    SdkError {
        status: Some(status),
        message,
        error,
    }
}

/// `OpenAI` SDK error for an in-stream `error` payload (`data.error ?? data`).
#[must_use]
pub fn openai_stream_error(data: &Value) -> SdkError {
    let error = match data.get("error") {
        Some(inner) if !inner.is_null() => inner.clone(),
        _ => data.clone(),
    };
    SdkError {
        status: None,
        message: make_message(None, Some(&error), None),
        error: Some(error),
    }
}

fn truncate_error_text(text: &str, max: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= max {
        return text.to_owned();
    }
    format!(
        "{}... [truncated {} chars]",
        String::from_utf16_lossy(&units[..max]),
        units.len() - max
    )
}

/// pi `formatProviderError(normalizeProviderError(error), prefix)`.
#[must_use]
pub fn format_provider_error(error: &SdkError, prefix: Option<&str>) -> String {
    let body = error
        .error
        .as_ref()
        .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
        .map(Value::to_string)
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map(|text| truncate_error_text(&text, MAX_PROVIDER_ERROR_BODY_CHARS));
    let carries_body = body
        .as_ref()
        .is_none_or(|body| error.message.contains(body.as_str()));
    match (error.status, body) {
        (Some(status), Some(body)) if !carries_body => match prefix {
            Some(prefix) => format!("{prefix} ({status}): {body}"),
            None => format!("{status}: {body}"),
        },
        (Some(status), _) if prefix.is_some() => {
            format!(
                "{} ({status}): {}",
                prefix.unwrap_or_default(),
                error.message
            )
        }
        _ => error.message.clone(),
    }
}

/// SDK message for a transport failure (`APIConnectionError` / timeout).
#[must_use]
pub fn connection_error_message(detail: &str) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        "Request timed out.".to_owned()
    } else {
        "Connection error.".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_messages() {
        let error = openai_status_error(400, r#"{"error":{"message":"bad","type":"x"}}"#);
        assert_eq!(error.message, "400 bad");
        assert_eq!(
            format_provider_error(&error, None),
            r#"400: {"message":"bad","type":"x"}"#
        );
        let plain = openai_status_error(502, "Bad gateway");
        assert_eq!(
            format_provider_error(&plain, Some("OpenAI API error")),
            "OpenAI API error (502): 502 Bad gateway"
        );
        assert_eq!(
            openai_status_error(500, "").message,
            "500 status code (no body)"
        );
    }

    #[test]
    fn anthropic_messages() {
        let error = anthropic_status_error(
            429,
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#,
        );
        assert_eq!(
            error.message,
            r#"429 {"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#
        );
    }
}
