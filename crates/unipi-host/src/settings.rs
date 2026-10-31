//! Host settings (host.md §9.9): `host_settings_get` / `host_settings_set` over
//! `<host dir>/config.json`.
//!
//! The file is read on every request (owner edits and `unipi-host …` CLI
//! commands apply without a restart). `set` merges one dotted key into the
//! file, validates the *whole* merged object with the same typed parsers the
//! rest of the host uses (forwarding, push, fs scope, term cap, update mode,
//! relay), then writes atomically — so a wrong-typed value is `E_SCHEMA` and
//! nothing changes.
//!
//! Blocks owned by other modules (`browser`, `sharing`, `cron`) accept any
//! leaf under their prefix and are stored as given; their own loaders are
//! their schema (and they read the file fresh on every request too).
//!
//! Secrets: `config.json` has none today (the notify E2E key and extension
//! secrets live in `<agent>/secrets.json`), but a secret-looking value is
//! masked in every reply anyway — a future key must not leak by accident.

use serde_json::{Map, Value};

use unipi_proto::{AppErrorCode, AppErrorDto, HostSettingsSetRequest, SECRET_MASK};

use crate::data_dir::HostPaths;

fn schema(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
}

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false)
}

/// Reads `config.json` as an object (`{}` for a missing/empty/non-object
/// file). Never masks; used by [`get`] after masking and by [`set`] before
/// writing.
fn read_object(paths: &HostPaths) -> Result<Map<String, Value>, AppErrorDto> {
    let raw: Option<Value> = crate::data_dir::read_json(&paths.config()).map_err(internal)?;
    Ok(match raw {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    })
}

/// `host_settings_get`: the whole object, secret-looking values masked.
pub fn get(paths: &HostPaths) -> Result<Value, AppErrorDto> {
    let mut value = Value::Object(read_object(paths)?);
    mask(&mut value);
    Ok(value)
}

/// `host_settings_set`: patch one dotted key and return the new object
/// (masked), or `E_SCHEMA` with the offending key when the merged file does
/// not validate.
pub fn set(paths: &HostPaths, request: &HostSettingsSetRequest) -> Result<Value, AppErrorDto> {
    let key = request.key.as_str();
    let mut object = read_object(paths)?;
    insert(&mut object, key, request.value.clone())?;
    let merged = Value::Object(object);
    validate(key, &merged)?;
    crate::data_dir::write_json(&paths.config(), &merged).map_err(internal)?;
    let mut reply = merged;
    mask(&mut reply);
    Ok(reply)
}

/// Sets `object.a.b = value` for `key = "a.b"`, creating intermediate
/// objects and replacing a scalar where an object is needed.
fn insert(object: &mut Map<String, Value>, key: &str, value: Value) -> Result<(), AppErrorDto> {
    let mut parts: Vec<&str> = key.split('.').collect();
    let leaf = parts
        .pop()
        .ok_or_else(|| schema("host settings key is empty"))?;
    if leaf.is_empty() {
        return Err(schema(format!(
            "host settings key `{key}` has an empty part"
        )));
    }
    let mut cursor = object;
    for part in parts {
        let entry = cursor
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cursor = entry.as_object_mut().expect("object just ensured");
    }
    cursor.insert(leaf.to_string(), value);
    Ok(())
}

/// Validates the merged object with the block parsers this host reads. Only
/// the blocks that are present are checked (a file may hold just `relay`).
fn validate(key: &str, merged: &Value) -> Result<(), AppErrorDto> {
    let bad = |what: String| schema(format!("host settings `{key}`: {what}"));
    let object = merged
        .as_object()
        .ok_or_else(|| bad("config.json must be a JSON object".into()))?;

    if let Some(relay) = object.get("relay") {
        let text = relay
            .as_str()
            .ok_or_else(|| bad("relay must be a string".into()))?;
        unipi_proto::RelaySetting::try_from(text.to_string()).map_err(bad)?;
    }
    if let Some(relay_only) = object.get("relay_only") {
        if !relay_only.is_boolean() {
            return Err(bad("relay_only must be a boolean".into()));
        }
    }
    if let Some(forwarding) = object.get("forwarding") {
        serde_json::from_value::<crate::forward::ForwardingConfig>(forwarding.clone())
            .map_err(|e| bad(format!("forwarding: {e}")))?;
    }
    if let Some(push) = object.get("push") {
        serde_json::from_value::<crate::push::PushConfig>(push.clone())
            .map_err(|e| bad(format!("push: {e}")))?;
    }
    if let Some(fs) = object.get("fs") {
        serde_json::from_value::<crate::fs_scope::FsAccessConfig>(fs.clone())
            .map_err(|e| bad(format!("fs: {e}")))?;
    }
    if let Some(lsp) = object.get("lsp") {
        // app spec §3.12 (WP-A21): `lsp.enabled`, `lsp.idle_minutes`,
        // `lsp.servers.<language>.command` — the block's own loader is
        // the schema, like browser/sharing/cron.
        serde_json::from_value::<crate::lsp::LspConfig>(lsp.clone())
            .map_err(|e| bad(format!("lsp: {e}")))?;
    }
    if let Some(term) = object.get("term") {
        let term = serde_json::from_value::<crate::term::TermConfig>(term.clone())
            .map_err(|e| bad(format!("term: {e}")))?;
        if !(1..=crate::term::MAX_TERMS_LIMIT).contains(&term.max) {
            return Err(bad(format!(
                "term.max must be 1..={}",
                crate::term::MAX_TERMS_LIMIT
            )));
        }
    }
    if let Some(update) = object.get("update") {
        serde_json::from_value::<crate::update::UpdateConfig>(update.clone())
            .map_err(|e| bad(format!("update: {e}")))?;
    }
    for block in ["browser", "sharing", "cron"] {
        if let Some(value) = object.get(block) {
            if !value.is_object() {
                return Err(bad(format!("{block} must be an object")));
            }
        }
    }
    Ok(())
}

/// Recursively replaces secret-looking values with [`SECRET_MASK`].
fn mask(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if is_secret_key(key) && !child.is_null() {
                    *child = Value::String(SECRET_MASK.to_string());
                } else {
                    mask(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask),
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower == "apikey"
        || lower == "api_key"
        || lower == "privatekey"
}

/// The `relay` / `relay_only` values the endpoint is started with (the
/// `--relay` / `--relay-only` flags override them).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelayConfig {
    /// `"n0"` or a self-hosted URL; `None` = the compiled-in default.
    pub relay: Option<String>,
    pub relay_only: bool,
}

impl RelayConfig {
    pub fn load(paths: &HostPaths) -> RelayConfig {
        let Ok(object) = read_object(paths) else {
            return RelayConfig::default();
        };
        RelayConfig {
            relay: object
                .get("relay")
                .and_then(Value::as_str)
                .map(str::to_string),
            relay_only: object
                .get("relay_only")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(key: &str, value: Value) -> HostSettingsSetRequest {
        HostSettingsSetRequest {
            key: key.to_string().try_into().unwrap(),
            value,
        }
    }

    #[test]
    fn missing_file_reads_as_an_empty_object() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        assert_eq!(get(&paths).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn set_creates_blocks_and_keeps_other_keys() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        std::fs::write(
            paths.config(),
            r#"{"forwarding":{"enabled":true,"ports":[5173]},"unknown":"kept"}"#,
        )
        .unwrap();
        let reply = set(&paths, &request("term.max", serde_json::json!(32))).unwrap();
        assert_eq!(reply["term"]["max"], 32);
        assert_eq!(reply["forwarding"]["enabled"], true);
        assert_eq!(reply["unknown"], "kept");
        // The file itself is the merged object (unmasked, complete).
        let raw: Value = serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
        assert_eq!(raw["term"]["max"], 32);
        assert_eq!(raw["unknown"], "kept");
    }

    #[test]
    fn wrong_typed_values_are_rejected_and_change_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        std::fs::write(paths.config(), r#"{"term":{"max":16}}"#).unwrap();
        let cases = [
            ("relay", serde_json::json!("http://insecure.example")),
            ("relay_only", serde_json::json!("yes")),
            ("forwarding", serde_json::json!("on")),
            ("forwarding.ports", serde_json::json!("8080")),
            ("push.enabled", serde_json::json!(1)),
            ("fs.allow", serde_json::json!("/data")),
            ("term.max", serde_json::json!(0)),
            ("update.mode", serde_json::json!("later")),
            ("browser", serde_json::json!("headless")),
        ];
        for (key, value) in cases {
            let error = set(&paths, &request(key, value.clone())).unwrap_err();
            assert_eq!(error.code, AppErrorCode::E_SCHEMA, "{key}: {value}");
        }
        let raw: Value = serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
        assert_eq!(raw, serde_json::json!({"term":{"max":16}}));
    }

    #[test]
    fn relay_may_be_n0_or_an_https_url() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        assert!(set(&paths, &request("relay", serde_json::json!("n0"))).is_ok());
        assert!(set(
            &paths,
            &request("relay", serde_json::json!("https://relay.example"))
        )
        .is_ok());
        let relay = RelayConfig::load(&paths);
        assert_eq!(relay.relay.as_deref(), Some("https://relay.example"));
        assert!(!relay.relay_only);
    }

    #[test]
    fn secret_looking_values_are_masked_in_replies_only() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        std::fs::write(
            paths.config(),
            r#"{"browser":{"enabled":true,"apiToken":"tk_secret"},"sharing":{"nested":[{"password":"hunter2"}]}}"#,
        )
        .unwrap();
        let reply = get(&paths).unwrap();
        assert_eq!(reply["browser"]["apiToken"], SECRET_MASK);
        assert_eq!(reply["sharing"]["nested"][0]["password"], SECRET_MASK);
        assert_eq!(reply["browser"]["enabled"], true);
        // Reading never rewrites the file.
        let raw: Value = serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
        assert_eq!(raw["browser"]["apiToken"], "tk_secret");
    }

    #[test]
    fn setting_a_leaf_inside_an_untyped_block_is_stored_as_given() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        let reply = set(&paths, &request("cron.concurrency", serde_json::json!(3))).unwrap();
        assert_eq!(reply["cron"]["concurrency"], 3);
    }
}
