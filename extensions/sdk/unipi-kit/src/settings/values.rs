//! Resolved settings values (`defaults ⊕ global ⊕ project`, runtime §10.3).
//!
//! The host passes resolved values per namespace in `InitInput.settings`; afterwards
//! `oino_settings_changed {namespace, keys}` tells the owner to re-read, which [`Store`] does
//! from `control.get-settings()["extensionSettings"][namespace]` layered over the registered
//! defaults. Reads never fail: missing or malformed layers fall back to defaults (unipi
//! `getSettings` never throws).

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Value at a dot-path key (unipi `getField`).
#[must_use]
pub fn get_field<'a>(settings: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.')
        .try_fold(settings, |cur, seg| cur.as_object()?.get(seg))
}

/// Copy with `key` set (unipi `setField`; intermediate non-objects are replaced).
#[must_use]
pub fn set_field(settings: &Value, key: &str, value: Value) -> Value {
    let mut out = if settings.is_object() {
        settings.clone()
    } else {
        Value::Object(Map::new())
    };
    let segments: Vec<&str> = key.split('.').collect();
    let mut cursor = &mut out;
    for seg in &segments[..segments.len().saturating_sub(1)] {
        let Value::Object(map) = cursor else { break };
        let next = map
            .entry((*seg).to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        cursor = next;
    }
    if let (Value::Object(map), Some(last)) = (cursor, segments.last()) {
        map.insert((*last).to_owned(), value);
    }
    out
}

/// Deep merge: objects merge key by key, everything else in `over` replaces `base`.
#[must_use]
pub fn deep_merge(base: &Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out = b.clone();
            for (k, v) in o {
                let merged = match out.get(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (_, Value::Null) => base.clone(),
        _ => over.clone(),
    }
}

/// One namespace's resolved values with typed accessors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values(pub Value);

impl Values {
    /// Raw value at `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        get_field(&self.0, key)
    }

    /// Boolean (`default` when missing or not a bool).
    #[must_use]
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    /// String (`default` when missing or not a string).
    #[must_use]
    pub fn str<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).and_then(Value::as_str).unwrap_or(default)
    }

    /// Number.
    #[must_use]
    pub fn f64(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(Value::as_f64).unwrap_or(default)
    }

    /// Non-negative integer.
    #[must_use]
    pub fn u64(&self, key: &str, default: u64) -> u64 {
        self.get(key).and_then(Value::as_u64).unwrap_or(default)
    }

    /// String list (non-strings dropped).
    #[must_use]
    pub fn strings(&self, key: &str) -> Vec<String> {
        self.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Decodes the whole namespace (`None` on mismatch).
    #[must_use]
    pub fn decode<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.0.clone()).ok()
    }
}

/// Resolved values of the namespaces one extension owns.
#[derive(Debug, Default)]
pub struct Store {
    inner: RefCell<BTreeMap<String, (Value, Value)>>,
}

impl Store {
    /// Declares a namespace and its defaults (values = defaults until loaded).
    pub fn declare(&self, namespace: &str, defaults: Value) {
        self.inner
            .borrow_mut()
            .insert(namespace.to_owned(), (defaults.clone(), defaults));
    }

    /// Applies host values (`{namespace: {...}}`, e.g. `Init::settings()`) over the defaults.
    pub fn load(&self, resolved: &Value) {
        let mut inner = self.inner.borrow_mut();
        for (ns, (defaults, values)) in inner.iter_mut() {
            let layer = resolved.get(ns).cloned().unwrap_or(Value::Null);
            *values = deep_merge(defaults, &layer);
        }
    }

    /// Applies a pi settings object (`{extensionSettings: {ns: {...}}}`) over the defaults.
    pub fn load_from_settings(&self, settings: &Value) {
        self.load(settings.get("extensionSettings").unwrap_or(&Value::Null));
    }

    /// Re-reads from the host (`control.get-settings`); keeps current values on failure.
    pub fn refresh(&self) {
        if let Ok(settings) = oino_extension::control::settings()
            && settings.get("extensionSettings").is_some()
        {
            self.load_from_settings(&settings);
        }
    }

    /// Handles an `oino_settings_changed` payload; returns the namespace when it is ours.
    pub fn on_changed(&self, payload: &Value) -> Option<String> {
        let ns = payload.get("namespace")?.as_str()?.to_owned();
        if !self.inner.borrow().contains_key(&ns) {
            return None;
        }
        self.refresh();
        Some(ns)
    }

    /// Current values of `namespace` (`{}` when undeclared).
    #[must_use]
    pub fn get(&self, namespace: &str) -> Values {
        Values(
            self.inner
                .borrow()
                .get(namespace)
                .map_or_else(|| Value::Object(Map::new()), |(_, v)| v.clone()),
        )
    }

    /// Overrides one key in memory (tests and optimistic updates before the host echoes).
    pub fn set_local(&self, namespace: &str, key: &str, value: Value) {
        if let Some((_, v)) = self.inner.borrow_mut().get_mut(namespace) {
            *v = set_field(v, key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dot_paths() {
        let v = json!({"a": {"b": {"c": 1}}, "x": 2});
        assert_eq!(get_field(&v, "a.b.c"), Some(&json!(1)));
        assert_eq!(get_field(&v, "x.y"), None);
        assert_eq!(get_field(&v, "missing"), None);
        let w = set_field(&v, "a.b.d", json!(true));
        assert_eq!(w["a"]["b"], json!({"c": 1, "d": true}));
        assert_eq!(v["a"]["b"], json!({"c": 1}), "no mutation");
        let w = set_field(&v, "x.y", json!(3));
        assert_eq!(w["x"], json!({"y": 3}));
        assert_eq!(set_field(&Value::Null, "k", json!(1)), json!({"k": 1}));
    }

    #[test]
    fn merge_layers_project_wins() {
        let defaults = json!({"enabled": true, "crab": "auto", "nested": {"a": 1, "b": 2}});
        let global = json!({"crab": "blocks", "nested": {"a": 5}});
        let project = json!({"enabled": false, "nested": {"b": 9}});
        let v = deep_merge(&deep_merge(&defaults, &global), &project);
        assert_eq!(
            v,
            json!({"enabled": false, "crab": "blocks", "nested": {"a": 5, "b": 9}})
        );
        assert_eq!(deep_merge(&defaults, &Value::Null), defaults);
    }

    #[test]
    fn store_layers_host_values_over_defaults() {
        let s = Store::default();
        s.declare(
            "hints",
            json!({"enabled": true, "header": true, "crab": "auto"}),
        );
        assert!(s.get("hints").bool("enabled", false));
        s.load(&json!({"hints": {"enabled": false}, "other": {"x": 1}}));
        let v = s.get("hints");
        assert!(!v.bool("enabled", true));
        assert_eq!(v.str("crab", "?"), "auto");
        s.load_from_settings(&json!({"extensionSettings": {"hints": {"crab": "image"}}}));
        assert_eq!(s.get("hints").str("crab", "?"), "image");
        assert!(
            s.get("hints").bool("enabled", false),
            "reload starts from defaults"
        );
        s.set_local("hints", "header", json!(false));
        assert!(!s.get("hints").bool("header", true));
        assert_eq!(s.get("nope"), Values(json!({})));
        assert_eq!(s.on_changed(&json!({"namespace": "other"})), None);
    }

    #[test]
    fn typed_accessors() {
        let v = Values(json!({"n": 5, "f": 1.5, "l": ["a", 1, "b"], "s": "x"}));
        assert_eq!(v.u64("n", 0), 5);
        assert!((v.f64("f", 0.0) - 1.5).abs() < f64::EPSILON);
        assert_eq!(v.strings("l"), ["a", "b"]);
        assert_eq!(v.str("missing", "d"), "d");
        assert_eq!(v.u64("s", 7), 7);
    }
}
