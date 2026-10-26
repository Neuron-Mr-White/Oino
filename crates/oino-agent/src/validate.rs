//! Tool-argument validation (pi-ai `validateToolArguments`).
//!
//! pi validates with `TypeBox` after two normalisation passes: optional properties set to `null`
//! are dropped, then primitives are coerced towards the schema type (`"5"` → `5`, `1` → `"1"`,
//! `"true"` → `true`, …). Error messages and paths follow `TypeBox` so the text the model sees is
//! identical to pi (golden-tested against the pi oracle).
//!
//! Supported keywords: `type`, `const`, `enum`, `minLength`, `maxLength`, `pattern`, `minimum`,
//! `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`, `items`, `prefixItems`,
//! `minItems`, `maxItems`, `uniqueItems`, `contains`, `properties`, `required`,
//! `additionalProperties`, `minProperties`, `maxProperties`, `allOf`, `anyOf`, `oneOf`, `not`,
//! local `$ref` (`#/$defs/…`, `#/definitions/…`) and boolean schemas. `format` is not checked.

use oino_ai::{JsonObject, JsonValue, ToolCall};
use regex::Regex;
use serde_json::Number;

/// pi `validateToolArguments`: returns the coerced arguments or pi's error text.
///
/// # Errors
///
/// `Invalid arguments for tool "{name}": {argumentsError}` when the provider could not parse
/// the arguments, or `Validation failed for tool "{name}":\n  - {path}: {message}…` followed by
/// the received arguments.
pub fn validate_tool_arguments(schema: &JsonValue, call: &ToolCall) -> Result<JsonObject, String> {
    if let Some(error) = &call.arguments_error {
        return Err(format!(
            "Invalid arguments for tool \"{}\": {error}",
            call.name
        ));
    }
    let mut args = JsonValue::Object(call.arguments.clone());
    normalize_optional_nulls(&mut args, schema, schema);
    let args = coerce(args, schema, schema);
    let errors = Validator::new(schema).errors(&args);
    if errors.is_empty() {
        return Ok(match args {
            JsonValue::Object(map) => map,
            _ => call.arguments.clone(),
        });
    }
    let list = errors
        .iter()
        .map(|error| format!("  - {}: {}", error.path, error.message))
        .collect::<Vec<_>>()
        .join("\n");
    let received = serde_json::to_string_pretty(&JsonValue::Object(call.arguments.clone()))
        .unwrap_or_else(|_| "{}".to_owned());
    Err(format!(
        "Validation failed for tool \"{}\":\n{list}\n\nReceived arguments:\n{received}",
        call.name
    ))
}

/// One validation failure with a TypeBox-style dotted path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    /// Dotted path (`root` for the top level).
    pub path: String,
    /// `TypeBox` message.
    pub message: String,
}

/// `true` when `value` satisfies `schema`.
#[must_use]
pub fn check(schema: &JsonValue, value: &JsonValue) -> bool {
    Validator::new(schema).errors(value).is_empty()
}

fn resolve<'a>(root: &'a JsonValue, schema: &'a JsonValue) -> &'a JsonValue {
    let mut current = schema;
    for _ in 0..32 {
        let Some(reference) = current.get("$ref").and_then(JsonValue::as_str) else {
            return current;
        };
        let Some(pointer) = reference.strip_prefix('#') else {
            return current;
        };
        match root.pointer(pointer) {
            Some(target) => current = target,
            None => return current,
        }
    }
    current
}

fn schema_types(schema: &JsonValue) -> Vec<&str> {
    match schema.get("type") {
        Some(JsonValue::String(kind)) => vec![kind.as_str()],
        Some(JsonValue::Array(kinds)) => kinds.iter().filter_map(JsonValue::as_str).collect(),
        _ => Vec::new(),
    }
}

fn matches_type(value: &JsonValue, kind: &str) -> bool {
    match kind {
        "number" => value.is_number(),
        "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

fn normalize_optional_nulls(value: &mut JsonValue, schema: &JsonValue, root: &JsonValue) {
    match value {
        JsonValue::Array(items) => match schema.get("items") {
            Some(JsonValue::Array(item_schemas)) => {
                for (item, item_schema) in items.iter_mut().zip(item_schemas) {
                    normalize_optional_nulls(item, item_schema, root);
                }
            }
            Some(item_schema) => {
                for item in items {
                    normalize_optional_nulls(item, item_schema, root);
                }
            }
            None => {}
        },
        JsonValue::Object(object) => {
            let Some(JsonValue::Object(properties)) = schema.get("properties") else {
                return;
            };
            let required: Vec<&str> = schema
                .get("required")
                .and_then(JsonValue::as_array)
                .map(|r| r.iter().filter_map(JsonValue::as_str).collect())
                .unwrap_or_default();
            for (key, property_schema) in properties {
                let Some(current) = object.get_mut(key) else {
                    continue;
                };
                let removable = current.is_null()
                    && !required.contains(&key.as_str())
                    && property_schema.get("$ref").is_none()
                    && !check_in(root, property_schema, &JsonValue::Null);
                if removable {
                    object.shift_remove(key);
                } else {
                    normalize_optional_nulls(current, property_schema, root);
                }
            }
        }
        _ => {}
    }
}

fn check_in(root: &JsonValue, schema: &JsonValue, value: &JsonValue) -> bool {
    let validator = Validator { root };
    let mut errors = Vec::new();
    validator.collect(schema, value, &mut Vec::new(), &mut errors);
    errors.is_empty()
}

/// pi `coerceWithJsonSchema` (does not follow `$ref`, like pi).
fn coerce(value: JsonValue, schema: &JsonValue, root: &JsonValue) -> JsonValue {
    let mut next = value;
    if let Some(JsonValue::Array(all)) = schema.get("allOf") {
        for nested in all {
            next = coerce(next, nested, root);
        }
    }
    for union in ["anyOf", "oneOf"] {
        if let Some(JsonValue::Array(members)) = schema.get(union) {
            next = coerce_union(next, members, root);
        }
    }
    let types = schema_types(schema);
    let matches_member = types.len() > 1 && types.iter().any(|kind| matches_type(&next, kind));
    if !types.is_empty() && !matches_member {
        for kind in &types {
            if let Some(candidate) = coerce_primitive(&next, kind) {
                next = candidate;
                break;
            }
        }
    }
    if types.contains(&"object")
        && let JsonValue::Object(object) = &mut next
    {
        coerce_object(object, schema, root);
    }
    if types.contains(&"array")
        && let JsonValue::Array(items) = &mut next
    {
        coerce_array(items, schema, root);
    }
    next
}

fn coerce_union(value: JsonValue, members: &[JsonValue], root: &JsonValue) -> JsonValue {
    if members.iter().any(|member| check_in(root, member, &value)) {
        return value;
    }
    for member in members {
        let candidate = coerce(value.clone(), member, root);
        if check_in(root, member, &candidate) {
            return candidate;
        }
    }
    value
}

fn coerce_object(object: &mut JsonObject, schema: &JsonValue, root: &JsonValue) {
    let properties = schema.get("properties").and_then(JsonValue::as_object);
    if let Some(properties) = properties {
        for (key, property_schema) in properties {
            if let Some(slot) = object.get_mut(key) {
                *slot = coerce(slot.take(), property_schema, root);
            }
        }
    }
    if let Some(additional @ JsonValue::Object(_)) = schema.get("additionalProperties") {
        for (key, slot) in object.iter_mut() {
            if properties.is_some_and(|p| p.contains_key(key)) {
                continue;
            }
            *slot = coerce(slot.take(), additional, root);
        }
    }
}

fn coerce_array(items: &mut [JsonValue], schema: &JsonValue, root: &JsonValue) {
    match schema.get("items") {
        Some(JsonValue::Array(item_schemas)) => {
            for (item, item_schema) in items.iter_mut().zip(item_schemas) {
                *item = coerce(item.take(), item_schema, root);
            }
        }
        Some(item_schema @ JsonValue::Object(_)) => {
            for item in items.iter_mut() {
                *item = coerce(item.take(), item_schema, root);
            }
        }
        _ => {}
    }
}

/// pi `coercePrimitiveByType`; `None` when the value is left unchanged.
fn coerce_primitive(value: &JsonValue, kind: &str) -> Option<JsonValue> {
    match (kind, value) {
        ("number" | "integer", JsonValue::Null) => Some(JsonValue::from(0)),
        ("number", JsonValue::String(text)) if !text.trim().is_empty() => js_to_number(text)
            .filter(|n| n.is_finite())
            .map(number_value),
        ("integer", JsonValue::String(text)) if !text.trim().is_empty() => js_to_number(text)
            .filter(|n| n.is_finite() && n.fract() == 0.0)
            .map(number_value),
        ("number" | "integer", JsonValue::Bool(flag)) => Some(JsonValue::from(u8::from(*flag))),
        ("boolean", JsonValue::Null) => Some(JsonValue::Bool(false)),
        ("boolean", JsonValue::String(text)) => match text.as_str() {
            "true" => Some(JsonValue::Bool(true)),
            "false" => Some(JsonValue::Bool(false)),
            _ => None,
        },
        ("boolean", JsonValue::Number(n)) => match n.as_f64() {
            Some(1.0) => Some(JsonValue::Bool(true)),
            Some(0.0) => Some(JsonValue::Bool(false)),
            _ => None,
        },
        ("string", JsonValue::Null) => Some(JsonValue::String(String::new())),
        ("string", JsonValue::Number(n)) => Some(JsonValue::String(js_number_text(n))),
        ("string", JsonValue::Bool(flag)) => Some(JsonValue::String(flag.to_string())),
        ("null", JsonValue::String(text)) if text.is_empty() => Some(JsonValue::Null),
        ("null", JsonValue::Bool(false)) => Some(JsonValue::Null),
        ("null", JsonValue::Number(n)) if n.as_f64() == Some(0.0) => Some(JsonValue::Null),
        _ => None,
    }
}

/// JSON number for `value`, integral when JavaScript would print no fraction.
#[allow(
    clippy::cast_possible_truncation,
    reason = "value is integral and within ±2^53, so the cast is exact"
)]
fn number_value(value: f64) -> JsonValue {
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        JsonValue::from(value as i64)
    } else {
        Number::from_f64(value).map_or(JsonValue::Null, JsonValue::Number)
    }
}

/// JavaScript `Number(text)`; `None` for `NaN`.
fn js_to_number(text: &str) -> Option<f64> {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if trimmed.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).ok().map(|n| {
                #[allow(clippy::cast_precision_loss, reason = "JavaScript numbers are f64")]
                let n = n as f64;
                n
            });
        }
    }
    let unsigned = trimmed.trim_start_matches(['+', '-']);
    if unsigned == "Infinity" && trimmed.len() - unsigned.len() <= 1 {
        return Some(if trimmed.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let valid = trimmed.len() - unsigned.len() <= 1
        && !unsigned.is_empty()
        && unsigned
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
        && unsigned
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '.');
    if !valid {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

/// JavaScript `String(number)`.
fn js_number_text(number: &Number) -> String {
    if number.is_i64() || number.is_u64() {
        return number.to_string();
    }
    let Some(value) = number.as_f64() else {
        return number.to_string();
    };
    js_f64_text(value)
}

fn js_f64_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        return format!("{value:.0}");
    }
    let magnitude = value.abs();
    if (1e-7..1e21).contains(&magnitude) {
        return format!("{value}");
    }
    let formatted = format!("{value:e}");
    match formatted.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => formatted,
    }
}

/// `(keyword, operator shown in the message, predicate)`.
type NumberCheck = (&'static str, &'static str, fn(f64, f64) -> bool);

/// Collection sizes as JavaScript numbers (exact below 2^53).
#[allow(clippy::cast_precision_loss, reason = "sizes stay far below 2^53")]
fn count_f64(count: usize) -> f64 {
    count as f64
}

struct Validator<'a> {
    root: &'a JsonValue,
}

impl<'a> Validator<'a> {
    fn new(root: &'a JsonValue) -> Self {
        Self { root }
    }

    fn errors(&self, value: &JsonValue) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        self.collect(self.root, value, &mut Vec::new(), &mut errors);
        errors
    }

    fn push(errors: &mut Vec<ValidationError>, path: &[String], message: String) {
        let path = if path.is_empty() {
            "root".to_owned()
        } else {
            path.join(".")
        };
        errors.push(ValidationError { path, message });
    }

    fn collect(
        &self,
        schema: &JsonValue,
        value: &JsonValue,
        path: &mut Vec<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        let schema = resolve(self.root, schema);
        match schema {
            JsonValue::Bool(false) => {
                Self::push(errors, path, "schema is false".to_owned());
                return;
            }
            JsonValue::Object(_) => {}
            _ => return,
        }
        let types = schema_types(schema);
        if !types.is_empty() && !types.iter().any(|kind| matches_type(value, kind)) {
            Self::push(errors, path, format!("must be {}", types.join(" or ")));
            return;
        }
        if let Some(expected) = schema.get("const")
            && !json_equal(expected, value)
        {
            Self::push(errors, path, "must be equal to constant".to_owned());
        }
        if let Some(JsonValue::Array(allowed)) = schema.get("enum")
            && !allowed.iter().any(|candidate| json_equal(candidate, value))
        {
            Self::push(
                errors,
                path,
                "must be equal to one of the allowed values".to_owned(),
            );
        }
        match value {
            JsonValue::String(text) => Self::string_keywords(schema, text, path, errors),
            JsonValue::Number(number) => {
                Self::number_keywords(schema, number.as_f64().unwrap_or(0.0), path, errors);
            }
            JsonValue::Array(items) => self.array_keywords(schema, items, path, errors),
            JsonValue::Object(object) => self.object_keywords(schema, object, path, errors),
            _ => {}
        }
        self.combinators(schema, value, path, errors);
    }

    fn string_keywords(
        schema: &JsonValue,
        text: &str,
        path: &[String],
        errors: &mut Vec<ValidationError>,
    ) {
        let length = text.encode_utf16().count();
        if let Some(min) = schema.get("minLength").and_then(JsonValue::as_f64)
            && count_f64(length) < min
        {
            Self::push(
                errors,
                path,
                format!("must not have fewer than {} characters", js_f64_text(min)),
            );
        }
        if let Some(max) = schema.get("maxLength").and_then(JsonValue::as_f64)
            && count_f64(length) > max
        {
            Self::push(
                errors,
                path,
                format!("must not have more than {} characters", js_f64_text(max)),
            );
        }
        if let Some(pattern) = schema.get("pattern").and_then(JsonValue::as_str)
            && Regex::new(pattern).is_ok_and(|re| !re.is_match(text))
        {
            Self::push(errors, path, format!("must match pattern \"{pattern}\""));
        }
    }

    fn number_keywords(
        schema: &JsonValue,
        number: f64,
        path: &[String],
        errors: &mut Vec<ValidationError>,
    ) {
        let bound = |key: &str| schema.get(key).and_then(JsonValue::as_f64);
        let checks: [NumberCheck; 4] = [
            ("minimum", ">=", |n, b| n >= b),
            ("maximum", "<=", |n, b| n <= b),
            ("exclusiveMinimum", ">", |n, b| n > b),
            ("exclusiveMaximum", "<", |n, b| n < b),
        ];
        for (key, operator, ok) in checks {
            if let Some(limit) = bound(key)
                && !ok(number, limit)
            {
                Self::push(
                    errors,
                    path,
                    format!("must be {operator} {}", js_f64_text(limit)),
                );
            }
        }
        if let Some(step) = bound("multipleOf")
            && step != 0.0
            && (number / step).fract() != 0.0
        {
            Self::push(
                errors,
                path,
                format!("must be multiple of {}", js_f64_text(step)),
            );
        }
    }

    fn array_keywords(
        &self,
        schema: &JsonValue,
        items: &[JsonValue],
        path: &mut Vec<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        let count = count_f64(items.len());
        if let Some(min) = schema.get("minItems").and_then(JsonValue::as_f64)
            && count < min
        {
            Self::push(
                errors,
                path,
                format!("must not have fewer than {} items", js_f64_text(min)),
            );
        }
        if let Some(max) = schema.get("maxItems").and_then(JsonValue::as_f64)
            && count > max
        {
            Self::push(
                errors,
                path,
                format!("must not have more than {} items", js_f64_text(max)),
            );
        }
        if schema.get("uniqueItems") == Some(&JsonValue::Bool(true)) {
            let duplicate = items
                .iter()
                .enumerate()
                .any(|(i, a)| items[i + 1..].iter().any(|b| json_equal(a, b)));
            if duplicate {
                Self::push(errors, path, "must not have duplicate items".to_owned());
            }
        }
        if let Some(contains) = schema.get("contains")
            && !items.iter().any(|item| check_in(self.root, contains, item))
        {
            Self::push(
                errors,
                path,
                "must contain at least 1 valid item".to_owned(),
            );
        }
        let prefix = match (schema.get("prefixItems"), schema.get("items")) {
            (Some(JsonValue::Array(prefix)), _) | (None, Some(JsonValue::Array(prefix))) => {
                Some(prefix.as_slice())
            }
            _ => None,
        };
        let rest = match schema.get("items") {
            Some(rest @ (JsonValue::Object(_) | JsonValue::Bool(_))) => Some(rest),
            _ => None,
        };
        for (index, item) in items.iter().enumerate() {
            let item_schema =
                prefix
                    .and_then(|p| p.get(index))
                    .or(if prefix.is_some() { None } else { rest });
            if let Some(item_schema) = item_schema {
                path.push(index.to_string());
                self.collect(item_schema, item, path, errors);
                path.pop();
            }
        }
    }

    fn object_keywords(
        &self,
        schema: &JsonValue,
        object: &JsonObject,
        path: &mut Vec<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(JsonValue::Array(required)) = schema.get("required") {
            let missing: Vec<&str> = required
                .iter()
                .filter_map(JsonValue::as_str)
                .filter(|key| !object.contains_key(*key))
                .collect();
            if let Some(first) = missing.first() {
                path.push((*first).to_owned());
                Self::push(
                    errors,
                    path,
                    format!("must have required properties {}", missing.join(", ")),
                );
                path.pop();
            }
        }
        let properties = schema.get("properties").and_then(JsonValue::as_object);
        if let Some(properties) = properties {
            for (key, property_schema) in properties {
                if let Some(property) = object.get(key) {
                    path.push(key.clone());
                    self.collect(property_schema, property, path, errors);
                    path.pop();
                }
            }
        }
        if let Some(additional) = schema.get("additionalProperties") {
            let extras: Vec<(&String, &JsonValue)> = object
                .iter()
                .filter(|(key, _)| !properties.is_some_and(|p| p.contains_key(*key)))
                .collect();
            let mut failed = false;
            for (key, extra) in extras {
                if additional == &JsonValue::Bool(true) {
                    continue;
                }
                let before = errors.len();
                path.push(key.clone());
                self.collect(additional, extra, path, errors);
                path.pop();
                failed |= errors.len() > before;
            }
            if failed {
                Self::push(
                    errors,
                    path,
                    "must not have additional properties".to_owned(),
                );
            }
        }
        let count = count_f64(object.len());
        if let Some(min) = schema.get("minProperties").and_then(JsonValue::as_f64)
            && count < min
        {
            Self::push(
                errors,
                path,
                format!("must not have fewer than {} properties", js_f64_text(min)),
            );
        }
        if let Some(max) = schema.get("maxProperties").and_then(JsonValue::as_f64)
            && count > max
        {
            Self::push(
                errors,
                path,
                format!("must not have more than {} properties", js_f64_text(max)),
            );
        }
    }

    fn combinators(
        &self,
        schema: &JsonValue,
        value: &JsonValue,
        path: &mut Vec<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(JsonValue::Array(all)) = schema.get("allOf") {
            for member in all {
                self.collect(member, value, path, errors);
            }
        }
        if let Some(JsonValue::Array(any)) = schema.get("anyOf") {
            let mut member_errors = Vec::new();
            let mut matched = false;
            for member in any {
                let before = member_errors.len();
                self.collect(member, value, path, &mut member_errors);
                if member_errors.len() == before {
                    matched = true;
                    break;
                }
            }
            if !matched {
                errors.extend(member_errors);
                Self::push(errors, path, "must match a schema in anyOf".to_owned());
            }
        }
        if let Some(JsonValue::Array(one)) = schema.get("oneOf") {
            let mut member_errors = Vec::new();
            let mut matches = 0;
            for member in one {
                let before = member_errors.len();
                self.collect(member, value, path, &mut member_errors);
                if member_errors.len() == before {
                    matches += 1;
                }
            }
            if matches != 1 {
                if matches == 0 {
                    errors.extend(member_errors);
                }
                Self::push(
                    errors,
                    path,
                    "must match exactly one schema in oneOf".to_owned(),
                );
            }
        }
        if let Some(not) = schema.get("not")
            && check_in(self.root, not, value)
        {
            Self::push(errors, path, "must not be valid".to_owned());
        }
    }
}

/// Deep equality where numbers compare by value (`1 == 1.0`) like JavaScript.
fn json_equal(left: &JsonValue, right: &JsonValue) -> bool {
    match (left, right) {
        (JsonValue::Number(a), JsonValue::Number(b)) => a.as_f64() == b.as_f64(),
        (JsonValue::Array(a), JsonValue::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| json_equal(x, y))
        }
        (JsonValue::Object(a), JsonValue::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, x)| b.get(key).is_some_and(|y| json_equal(x, y)))
        }
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_number_parsing() {
        assert_eq!(js_to_number(" 3 "), Some(3.0));
        assert_eq!(js_to_number("0x10"), Some(16.0));
        assert_eq!(js_to_number("1e3"), Some(1000.0));
        assert_eq!(js_to_number("abc"), None);
        assert_eq!(js_to_number("inf"), None);
        assert_eq!(js_to_number("Infinity"), Some(f64::INFINITY));
        assert_eq!(js_to_number("-.5"), Some(-0.5));
    }

    #[test]
    fn js_number_printing() {
        assert_eq!(js_f64_text(1.5), "1.5");
        assert_eq!(js_f64_text(1e21), "1e+21");
        assert_eq!(js_f64_text(3.0), "3");
    }
}
