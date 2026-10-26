//! Typed access to validated tool arguments.

use oino_agent::ToolError;
use oino_ai::JsonObject;

/// A required string argument.
pub(crate) fn required_str<'a>(args: &'a JsonObject, name: &str) -> Result<&'a str, ToolError> {
    args.get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ToolError::new(format!("{name} must be a string")))
}

/// An optional string argument (`undefined` / `null` → `None`).
pub(crate) fn optional_str<'a>(args: &'a JsonObject, name: &str) -> Option<&'a str> {
    args.get(name).and_then(serde_json::Value::as_str)
}

/// An optional number argument.
pub(crate) fn optional_f64(args: &JsonObject, name: &str) -> Option<f64> {
    args.get(name).and_then(serde_json::Value::as_f64)
}

/// An optional boolean argument (JavaScript truthiness of booleans).
pub(crate) fn flag(args: &JsonObject, name: &str) -> bool {
    args.get(name)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Converts a non-negative JavaScript number to a count (`limit ?? DEFAULT`).
pub(crate) fn to_count(value: f64) -> u64 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the u64 range"
    )]
    let count = value.min(9_007_199_254_740_991.0) as u64;
    count
}
