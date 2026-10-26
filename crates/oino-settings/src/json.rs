//! JSON-object helpers: pi `deepMergeSettings`, `migrateSettings`, path edits and JS-style
//! value coercions used by the typed getters.

use serde_json::{Map, Value};

use crate::tools::merge_default_tools;

/// Settings JSON object (insertion-ordered, `preserve_order`).
pub type JsonObject = Map<String, Value>;

/// pi `deepMergeObjects`: `overrides` wins; nested objects (not arrays) merge recursively.
/// Keys keep their first-seen position, new keys are appended (JS spread order).
#[must_use]
pub fn deep_merge_objects(base: &JsonObject, overrides: &JsonObject) -> JsonObject {
    let mut result = base.clone();
    for (key, override_value) in overrides {
        let merged = match (base.get(key), override_value) {
            (Some(Value::Object(base_obj)), Value::Object(override_obj)) => {
                Value::Object(deep_merge_objects(base_obj, override_obj))
            }
            _ => override_value.clone(),
        };
        result.insert(key.clone(), merged);
    }
    result
}

/// pi `deepMergeSettings`: [`deep_merge_objects`] plus the `defaultTools` rule (a list of only
/// `+name`/`-name` entries is appended to the inherited list instead of replacing it).
#[must_use]
pub fn deep_merge_settings(base: &JsonObject, overrides: &JsonObject) -> JsonObject {
    let mut merged = deep_merge_objects(base, overrides);
    if let Some(tools) =
        merge_default_tools(base.get("defaultTools"), overrides.get("defaultTools"))
    {
        merged.insert("defaultTools".to_owned(), tools);
    }
    merged
}

/// pi `SettingsManager.migrateSettings`: rewrite legacy keys in place.
///
/// - `queueMode` → `steeringMode` (only when `steeringMode` is absent);
/// - boolean `websockets` → `transport` (`websocket` / `sse`) when `transport` is absent;
/// - object `skills: {enableSkillCommands, customDirectories}` → `enableSkillCommands` + array;
/// - `retry.maxDelayMs` → `retry.provider.maxRetryDelayMs` (always removes `maxDelayMs`).
pub fn migrate_settings(settings: &mut JsonObject) {
    if !settings.contains_key("steeringMode")
        && let Some(mode) = settings.shift_remove("queueMode")
    {
        settings.insert("steeringMode".to_owned(), mode);
    }

    if !settings.contains_key("transport")
        && let Some(Value::Bool(websockets)) = settings.get("websockets").cloned()
    {
        let transport = if websockets { "websocket" } else { "sse" };
        settings.insert("transport".to_owned(), Value::from(transport));
        settings.shift_remove("websockets");
    }

    if let Some(Value::Object(skills)) = settings.get("skills").cloned() {
        if let Some(enable) = skills.get("enableSkillCommands")
            && !settings.contains_key("enableSkillCommands")
        {
            settings.insert("enableSkillCommands".to_owned(), enable.clone());
        }
        match skills.get("customDirectories") {
            Some(Value::Array(dirs)) if !dirs.is_empty() => {
                settings.insert("skills".to_owned(), Value::Array(dirs.clone()));
            }
            _ => {
                settings.shift_remove("skills");
            }
        }
    }

    if let Some(Value::Object(retry)) = settings.get_mut("retry") {
        let provider_missing_delay = match retry.get("provider") {
            Some(Value::Object(provider)) => {
                matches!(provider.get("maxRetryDelayMs"), None | Some(Value::Null))
            }
            _ => true,
        };
        if let Some(delay @ Value::Number(_)) = retry.get("maxDelayMs").cloned()
            && provider_missing_delay
        {
            let mut provider = match retry.get("provider") {
                Some(Value::Object(provider)) => provider.clone(),
                _ => JsonObject::new(),
            };
            provider.insert("maxRetryDelayMs".to_owned(), delay);
            retry.insert("provider".to_owned(), Value::Object(provider));
        }
        retry.shift_remove("maxDelayMs");
    }
}

/// The value at `path` (object keys), if every step is an object.
#[must_use]
pub fn get_path<'a>(object: &'a JsonObject, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    let mut current = object.get(*first)?;
    for key in rest {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

/// Set (`Some`) or remove (`None`) the value at `path`, creating intermediate objects and
/// replacing non-object intermediates. Removal keeps sibling order.
pub fn set_path(object: &mut JsonObject, path: &[&str], value: Option<Value>) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut current = object;
    for key in parents {
        let entry = current
            .entry((*key).to_owned())
            .or_insert_with(|| Value::Object(JsonObject::new()));
        if !entry.is_object() {
            if value.is_none() {
                return;
            }
            *entry = Value::Object(JsonObject::new());
        }
        let Value::Object(next) = entry else {
            return;
        };
        current = next;
    }
    match value {
        Some(value) => {
            current.insert((*last).to_owned(), value);
        }
        None => {
            current.shift_remove(*last);
        }
    }
}

/// Strip a leading UTF-8 byte order mark (pi `stripBom`).
#[must_use]
pub fn strip_bom(content: &str) -> &str {
    content.strip_prefix('\u{feff}').unwrap_or(content)
}

/// JavaScript `String(value)` for diagnostics (`[object Object]`, joined arrays, JS numbers).
#[must_use]
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_number(n),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn js_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    match n.as_f64() {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "integral f64 within i64 range is printed like JS"
        )]
        Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
        Some(f) => f.to_string(),
        None => n.to_string(),
    }
}

/// A JSON number as a non-negative integer, flooring finite fractions (pi `Math.floor`).
#[must_use]
pub fn as_non_negative_u64(value: &Value) -> Option<u64> {
    if let Some(u) = value.as_u64() {
        return Some(u);
    }
    let f = value.as_f64()?;
    if f.is_finite() && f >= 0.0 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "finite, non-negative, floored; saturates above u64::MAX"
        )]
        return Some(f.floor() as u64);
    }
    None
}

/// `Number.isSafeInteger(value) && value >= 0`.
#[must_use]
pub fn as_safe_non_negative_integer(value: &Value) -> Option<u64> {
    const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
    if let Some(u) = value.as_u64() {
        return (u <= MAX_SAFE_INTEGER).then_some(u);
    }
    let f = value.as_f64()?;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "range-checked against MAX_SAFE_INTEGER before the cast"
    )]
    if f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f <= MAX_SAFE_INTEGER as f64 {
        return Some(f as u64);
    }
    None
}

/// The string items of a JSON array (other items skipped); `None` when not an array.
#[must_use]
pub fn string_array(value: &Value) -> Option<Vec<String>> {
    value.as_array().map(|items| {
        items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn obj(value: Value) -> JsonObject {
        match value {
            Value::Object(map) => map,
            _ => unreachable!("test literal is an object"),
        }
    }

    #[test]
    fn deep_merge_recurses_objects_and_replaces_arrays() {
        let base = obj(json!({"a": {"x": 1, "y": 2}, "arr": [1], "k": 1}));
        let over = obj(json!({"a": {"y": 3, "z": 4}, "arr": [2], "n": null}));
        let merged = deep_merge_objects(&base, &over);
        assert_eq!(
            Value::Object(merged),
            json!({"a": {"x": 1, "y": 3, "z": 4}, "arr": [2], "k": 1, "n": null})
        );
    }

    #[test]
    fn migrations_match_pi() {
        let mut s = obj(json!({
            "queueMode": "all",
            "websockets": true,
            "skills": {"enableSkillCommands": false, "customDirectories": ["a"]},
            "retry": {"maxDelayMs": 5, "provider": {"timeoutMs": 1}}
        }));
        migrate_settings(&mut s);
        assert_eq!(
            Value::Object(s),
            json!({
                "skills": ["a"],
                "retry": {"provider": {"timeoutMs": 1, "maxRetryDelayMs": 5}},
                "steeringMode": "all",
                "transport": "websocket",
                "enableSkillCommands": false
            })
        );
        let mut kept =
            obj(json!({"queueMode": "all", "steeringMode": "one-at-a-time", "skills": {}}));
        migrate_settings(&mut kept);
        assert_eq!(
            Value::Object(kept),
            json!({"queueMode": "all", "steeringMode": "one-at-a-time"})
        );
    }

    #[test]
    fn set_path_creates_and_removes() {
        let mut s = obj(json!({"a": 1, "b": {"c": 2, "d": 3}}));
        set_path(&mut s, &["b", "c"], None);
        set_path(&mut s, &["x", "y"], Some(json!(true)));
        set_path(&mut s, &["a"], None);
        assert_eq!(Value::Object(s), json!({"b": {"d": 3}, "x": {"y": true}}));
    }

    #[test]
    fn js_string_matches_javascript() {
        assert_eq!(js_string(&json!(1.0)), "1");
        assert_eq!(js_string(&json!(-1.5)), "-1.5");
        assert_eq!(js_string(&json!({"a": 1})), "[object Object]");
        assert_eq!(js_string(&json!([1, null, "x"])), "1,,x");
    }
}
