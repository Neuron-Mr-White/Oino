//! JSON aliases and serde helpers that keep the wire format identical to pi (JavaScript).

/// Arbitrary JSON value (pi `JsonValue` / `unknown`).
pub type JsonValue = serde_json::Value;

/// JSON object with insertion order preserved (pi `JsonObject`, `Record<string, unknown>`).
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

/// Largest integer a JavaScript `number` represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Converts an integral `f64` to `i64` when JavaScript would print it without a fraction.
#[allow(
    clippy::cast_possible_truncation,
    reason = "value is integral and within ±2^53, so the cast is exact"
)]
fn js_integer(value: f64) -> Option<i64> {
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER)
        .then_some(value as i64)
}

/// Serialises `f64` like `JSON.stringify`: integral values are written without `.0`.
pub(crate) mod js_f64 {
    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "signature required by serde(serialize_with)"
    )]
    pub(crate) fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        match super::js_integer(*value) {
            Some(int) => serializer.serialize_i64(int),
            None => serializer.serialize_f64(*value),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        f64::deserialize(deserializer)
    }
}

/// [`js_f64`] for `Option<f64>` fields.
pub(crate) mod js_f64_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(
        clippy::ref_option,
        reason = "signature required by serde(serialize_with)"
    )]
    pub(crate) fn serialize<S: Serializer>(
        value: &Option<f64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(v) => super::js_f64::serialize(v, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<f64>, D::Error> {
        Option::<f64>::deserialize(deserializer)
    }
}

/// `deserialize_with` helper that keeps an explicit JSON `null` as `Some(Value::Null)`, so
/// optional `details`/`data` fields round-trip exactly (absent ⇒ `None` via `#[serde(default)]`).
///
/// # Errors
///
/// Propagates the deserializer's error.
pub fn keep_null<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<JsonValue>, D::Error> {
    <JsonValue as serde::Deserialize>::deserialize(deserializer).map(Some)
}

/// Compares two JSON values the way pi golden tests do (quality §4.3): objects ignore key
/// order, numbers compare by numeric value (`1` equals `1.0`), arrays compare in order.
#[must_use]
pub fn json_semantic_eq(left: &JsonValue, right: &JsonValue) -> bool {
    use serde_json::Value;
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x.to_bits() == y.to_bits() || x == y,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| json_semantic_eq(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, x)| b.get(key).is_some_and(|y| json_semantic_eq(x, y)))
        }
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semantic_eq_ignores_key_order_and_number_repr() {
        assert!(json_semantic_eq(
            &json!({"a": 1, "b": [1.0, {"c": null}]}),
            &json!({"b": [1, {"c": null}], "a": 1.0})
        ));
        assert!(!json_semantic_eq(&json!([1, 2]), &json!([2, 1])));
        assert!(!json_semantic_eq(
            &json!({"a": 1}),
            &json!({"a": 1, "b": 2})
        ));
    }

    #[test]
    fn js_integer_matches_json_stringify() {
        assert_eq!(js_integer(3.0), Some(3));
        assert_eq!(js_integer(0.015), None);
        assert_eq!(js_integer(f64::NAN), None);
        assert_eq!(js_integer(1e300), None);
    }
}
