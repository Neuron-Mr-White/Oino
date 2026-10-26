//! Extension settings registry (runtime §10.3, D11): namespaces declared by extensions
//! (ui-protocol §6.6 schema + defaults), layered reads, scoped writes, secret fields and change
//! notifications.
//!
//! - **Layering**: `defaults ⊕ global ⊕ project` where the layers are
//!   `extensionSettings.<namespace>` of `<agent>/settings.json` and the trusted
//!   `.oino/settings.json`, read through the shared [`SettingsManager`] (pi layering, locked
//!   atomic writes, unknown keys preserved). Legacy flat dotted keys (`"events.x": true`) are
//!   expanded into nested objects on read and removed on write.
//! - **Keys** are dot paths into the namespace object (`events.agentEnd` →
//!   `extensionSettings.notify.events.agentEnd`).
//! - **Secret fields** (`type: "secret"`) never reach `settings.json`: they live in
//!   `<agent>/secrets.json` at `<owner-ext-id>/<namespace>/<key>`. Frontends see
//!   [`SECRET_MASK`] for a set secret; the owning extension (and extensions matching the
//!   namespace's `secretReaders` patterns) read the value.
//! - **Writes** come from frontends ([`ExtensionSettings::set`]) or from the owning extension
//!   ([`ExtensionSettings::set_as`], WIT `registry.set-extension-setting`). Each successful write
//!   publishes a [`SettingsChange`]; the services deliver it to the owner as
//!   `oino_settings_changed {namespace, keys, scope}`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};
use oino_settings::json::{deep_merge_objects, get_path};
use oino_settings::{SecretStore, SettingsError, SettingsManager, SettingsScope};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// What frontends see for a secret field that holds a value. Writing it back is a no-op.
pub const SECRET_MASK: &str = "••••••";

/// Settings key holding every namespace (`extensionSettings`).
pub const EXTENSION_SETTINGS_KEY: &str = "extensionSettings";

const CHANGE_CAPACITY: usize = 64;

/// Which values a read returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsLayer {
    /// `defaults ⊕ global ⊕ project` (with secrets).
    #[default]
    Effective,
    /// The global file's values only (with secrets).
    Global,
    /// The project file's values only.
    Project,
}

/// Hub scope of a field (`scope` in the schema).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FieldScope {
    #[default]
    Both,
    Global,
    Project,
}

/// Validation facts about one schema field.
#[derive(Clone, Debug, Default)]
struct FieldInfo {
    kind: String,
    scope: FieldScope,
    options: Option<Vec<String>>,
    allow_custom: bool,
}

/// One registered namespace.
#[derive(Clone, Debug)]
struct Namespace {
    owner: String,
    schema: JsonValue,
    defaults: JsonObject,
    fields: HashMap<String, FieldInfo>,
    secret_readers: Vec<String>,
}

impl Namespace {
    fn secret_keys(&self) -> impl Iterator<Item = &str> {
        self.fields
            .iter()
            .filter(|(_, f)| f.kind == "secret")
            .map(|(k, _)| k.as_str())
    }

    fn may_read_secrets(&self, reader: &str) -> bool {
        reader == self.owner
            || self.secret_readers.iter().any(|pattern| {
                pattern == "*"
                    || pattern == reader
                    || pattern
                        .strip_suffix('*')
                        .is_some_and(|prefix| reader.starts_with(prefix))
            })
    }
}

/// A successful write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChange {
    /// Namespace.
    pub namespace: String,
    /// Dot-path keys written.
    pub keys: Vec<String>,
    /// Scope written (secrets are always global).
    pub scope: SettingsScope,
    /// Owning extension (`None` for an unregistered namespace).
    pub extension_id: Option<String>,
}

/// Failure of a registry operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExtensionSettingsError {
    /// Malformed namespace declaration, name or key.
    #[error("{0}")]
    Invalid(String),
    /// The namespace is owned by another extension.
    #[error("settings namespace \"{namespace}\" belongs to {owner}")]
    NotOwner {
        /// Namespace.
        namespace: String,
        /// Its owner.
        owner: String,
    },
    /// Extensions may only write namespaces they registered.
    #[error("settings namespace \"{0}\" is not registered")]
    UnknownNamespace(String),
    /// The value does not match the field type.
    #[error("invalid value for {key}: {message}")]
    InvalidValue {
        /// Key.
        key: String,
        /// Reason.
        message: String,
    },
    /// The field cannot be written in this scope.
    #[error("{key} can only be set in the {allowed} scope")]
    Scope {
        /// Key.
        key: String,
        /// Allowed scope.
        allowed: SettingsScope,
    },
    /// Settings file failure (includes untrusted project writes).
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// Secrets file failure.
    #[error("secrets store: {0}")]
    Secrets(#[from] std::io::Error),
}

/// The registry. One per [`crate::services::Services`]; cheap to share behind an `Arc`.
pub struct ExtensionSettings {
    settings: Arc<Mutex<SettingsManager>>,
    secrets: Option<SecretStore>,
    namespaces: RwLock<IndexMap<String, Namespace>>,
    changes: broadcast::Sender<SettingsChange>,
}

impl std::fmt::Debug for ExtensionSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionSettings")
            .field("namespaces", &self.read().len())
            .finish_non_exhaustive()
    }
}

impl ExtensionSettings {
    /// Registry over `settings`; secret fields go to `secrets` (`None`: secret fields are not
    /// writable and read as unset, e.g. in-memory test services).
    #[must_use]
    pub fn new(settings: Arc<Mutex<SettingsManager>>, secrets: Option<SecretStore>) -> Self {
        Self {
            settings,
            secrets,
            namespaces: RwLock::default(),
            changes: broadcast::channel(CHANGE_CAPACITY).0,
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, IndexMap<String, Namespace>> {
        self.namespaces
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn manager(&self) -> std::sync::MutexGuard<'_, SettingsManager> {
        self.settings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers (or replaces) a namespace declared by `extension`. `extensionId` in the
    /// schema is set to `extension`; field `default`s fill missing `defaults` keys.
    ///
    /// # Errors
    ///
    /// [`ExtensionSettingsError::Invalid`] for a malformed declaration,
    /// [`ExtensionSettingsError::NotOwner`] when another extension registered the name.
    pub fn register(
        &self,
        extension: &str,
        mut schema: JsonValue,
    ) -> Result<(), ExtensionSettingsError> {
        let JsonValue::Object(object) = &mut schema else {
            return Err(ExtensionSettingsError::Invalid(
                "settings namespace must be an object".into(),
            ));
        };
        let name = object
            .get("namespace")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        validate_name(&name)?;
        object.insert(
            "extensionId".into(),
            JsonValue::String(extension.to_owned()),
        );
        let mut defaults = object
            .get("defaults")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let mut fields = HashMap::new();
        if let Some(sections) = object.get("sections") {
            collect_fields(sections, &mut fields, &mut defaults);
        }
        let secret_readers = object
            .get("secretReaders")
            .and_then(JsonValue::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(JsonValue::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut namespaces = self
            .namespaces
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = namespaces.get(&name)
            && existing.owner != extension
        {
            return Err(ExtensionSettingsError::NotOwner {
                namespace: name,
                owner: existing.owner.clone(),
            });
        }
        namespaces.insert(
            name,
            Namespace {
                owner: extension.to_owned(),
                schema,
                defaults,
                fields,
                secret_readers,
            },
        );
        Ok(())
    }

    /// The settings manager the registry writes through.
    #[must_use]
    pub fn manager_handle(&self) -> &Arc<Mutex<SettingsManager>> {
        &self.settings
    }

    /// pi `getSettings()`: the merged settings object.
    #[must_use]
    pub fn merged_settings(&self) -> JsonObject {
        self.manager().settings().merged().clone()
    }

    /// Schemas of every registered namespace, in registration order (`get_settings_schema`).
    #[must_use]
    pub fn schema(&self) -> Vec<JsonValue> {
        self.read().values().map(|n| n.schema.clone()).collect()
    }

    /// Schema of one namespace.
    #[must_use]
    pub fn namespace(&self, namespace: &str) -> Option<JsonValue> {
        self.read().get(namespace).map(|n| n.schema.clone())
    }

    /// Owning extension of a namespace.
    #[must_use]
    pub fn owner(&self, namespace: &str) -> Option<String> {
        self.read().get(namespace).map(|n| n.owner.clone())
    }

    /// Namespaces owned by `extension`.
    #[must_use]
    pub fn owned_by(&self, extension: &str) -> Vec<String> {
        self.read()
            .iter()
            .filter(|(_, n)| n.owner == extension)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Values for frontends (`get_extension_settings`): set secrets read as [`SECRET_MASK`].
    /// Unregistered namespaces read their raw layers.
    #[must_use]
    pub fn values(&self, namespace: &str, layer: SettingsLayer) -> JsonValue {
        self.resolve(namespace, layer, false)
    }

    /// Values for extension `reader` (WIT `registry.extension-settings`): secret fields are
    /// resolved when `reader` owns the namespace or matches its `secretReaders`, otherwise
    /// they are absent.
    #[must_use]
    pub fn values_for(&self, reader: &str, namespace: &str, layer: SettingsLayer) -> JsonValue {
        let reveal = self
            .read()
            .get(namespace)
            .is_some_and(|n| n.may_read_secrets(reader));
        let mut values = self.resolve(namespace, layer, reveal);
        if !reveal && let Some(ns) = self.read().get(namespace) {
            for key in ns.secret_keys() {
                remove_path(&mut values, key);
            }
        }
        values
    }

    /// `{namespace: effective values}` of every namespace present in the settings files or
    /// registered, as extension `reader` sees them (`InitInput.settings`).
    #[must_use]
    pub fn all_for(&self, reader: &str) -> JsonValue {
        let mut names: Vec<String> = self.read().keys().cloned().collect();
        {
            let manager = self.manager();
            let merged = manager.settings().merged();
            if let Some(JsonValue::Object(all)) = merged.get(EXTENSION_SETTINGS_KEY) {
                names.extend(
                    all.keys()
                        .filter(|k| !names.contains(k))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
        }
        JsonValue::Object(
            names
                .into_iter()
                .map(|ns| {
                    let v = self.values_for(reader, &ns, SettingsLayer::Effective);
                    (ns, v)
                })
                .collect(),
        )
    }

    fn resolve(&self, namespace: &str, layer: SettingsLayer, reveal: bool) -> JsonValue {
        let registered = self.read().get(namespace).cloned();
        let (global, project) = {
            let manager = self.manager();
            let view = manager.settings();
            let layer_of = |object: &JsonObject| {
                expand_dotted(
                    get_path(object, &[EXTENSION_SETTINGS_KEY, namespace])
                        .and_then(JsonValue::as_object)
                        .cloned()
                        .unwrap_or_default(),
                )
            };
            (
                layer_of(view.global()),
                if manager.is_project_trusted() {
                    layer_of(view.project())
                } else {
                    JsonObject::new()
                },
            )
        };
        let mut values = match layer {
            SettingsLayer::Effective => {
                let defaults = registered
                    .as_ref()
                    .map(|n| n.defaults.clone())
                    .unwrap_or_default();
                deep_merge_objects(&deep_merge_objects(&defaults, &global), &project)
            }
            SettingsLayer::Global => global,
            SettingsLayer::Project => project,
        };
        if let (Some(ns), Some(store), false) =
            (&registered, &self.secrets, layer == SettingsLayer::Project)
        {
            for key in ns.secret_keys() {
                match store.get(&secret_key(&ns.owner, namespace, key)) {
                    Ok(Some(secret)) => {
                        let shown = if reveal {
                            secret
                        } else {
                            SECRET_MASK.to_owned()
                        };
                        set_path_value(&mut values, key, JsonValue::String(shown));
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!("reading secret {namespace}.{key}: {error}"),
                }
            }
        }
        JsonValue::Object(values)
    }

    /// Frontend write (`set_extension_setting` / `unset_extension_setting`): `None` (or JSON
    /// `null`) deletes the key in `scope` ("use default"). Unregistered namespaces accept plain
    /// values.
    ///
    /// # Errors
    ///
    /// Invalid keys or values, scope violations, untrusted project writes, I/O failures.
    pub fn set(
        &self,
        namespace: &str,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> Result<(), ExtensionSettingsError> {
        self.write(None, namespace, key, value, scope)
    }

    /// Write by extension `writer` (WIT `registry.set-extension-setting`): only namespaces it
    /// registered.
    ///
    /// # Errors
    ///
    /// See [`Self::set`]; [`ExtensionSettingsError::UnknownNamespace`] /
    /// [`ExtensionSettingsError::NotOwner`] for namespaces it does not own.
    pub fn set_as(
        &self,
        writer: &str,
        namespace: &str,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> Result<(), ExtensionSettingsError> {
        self.write(Some(writer), namespace, key, value, scope)
    }

    fn write(
        &self,
        writer: Option<&str>,
        namespace: &str,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> Result<(), ExtensionSettingsError> {
        validate_name(namespace)?;
        let segments = key_segments(key)?;
        let registered = self.read().get(namespace).cloned();
        if let Some(writer) = writer {
            match &registered {
                None => {
                    return Err(ExtensionSettingsError::UnknownNamespace(namespace.into()));
                }
                Some(ns) if ns.owner != writer => {
                    return Err(ExtensionSettingsError::NotOwner {
                        namespace: namespace.into(),
                        owner: ns.owner.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        let value = value.filter(|v| !v.is_null());
        let field = registered.as_ref().and_then(|n| n.fields.get(key)).cloned();
        if let Some(field) = &field {
            check_scope(key, field.scope, scope)?;
            if let Some(value) = &value {
                check_value(key, field, value)?;
            }
        }
        let owner = registered.as_ref().map(|n| n.owner.clone());
        if let (Some(field), Some(owner)) = (&field, &owner)
            && field.kind == "secret"
        {
            let Some(store) = &self.secrets else {
                return Err(ExtensionSettingsError::Invalid(
                    "secret settings are not available in this host".into(),
                ));
            };
            let at = secret_key(owner, namespace, key);
            match value.as_ref().and_then(JsonValue::as_str) {
                Some(SECRET_MASK) => return Ok(()),
                Some(secret) if !secret.is_empty() => store.set(&at, secret)?,
                _ => {
                    store.delete(&at)?;
                }
            }
            self.publish(namespace, key, SettingsScope::Global, owner.clone().into());
            return Ok(());
        }
        let mut path: Vec<&str> = vec![EXTENSION_SETTINGS_KEY, namespace];
        path.extend(segments.iter().copied());
        let flat = [EXTENSION_SETTINGS_KEY, namespace, key];
        let mut edits: Vec<(&[&str], Option<JsonValue>)> = vec![(&path, value)];
        if segments.len() > 1 {
            edits.push((&flat, None));
        }
        self.manager().set_many(scope, &edits)?;
        self.publish(namespace, key, scope, owner);
        Ok(())
    }

    /// Writes several keys of one namespace in one locked file write (one change event).
    /// Secret fields are not accepted here.
    ///
    /// # Errors
    ///
    /// See [`Self::set_as`].
    pub fn set_many_as(
        &self,
        writer: Option<&str>,
        namespace: &str,
        values: &JsonObject,
        scope: SettingsScope,
    ) -> Result<(), ExtensionSettingsError> {
        validate_name(namespace)?;
        let registered = self.read().get(namespace).cloned();
        if let Some(writer) = writer {
            match &registered {
                None => return Err(ExtensionSettingsError::UnknownNamespace(namespace.into())),
                Some(ns) if ns.owner != writer => {
                    return Err(ExtensionSettingsError::NotOwner {
                        namespace: namespace.into(),
                        owner: ns.owner.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        let mut paths = Vec::new();
        for (key, value) in values {
            let segments = key_segments(key)?;
            if let Some(field) = registered.as_ref().and_then(|n| n.fields.get(key)) {
                if field.kind == "secret" {
                    return Err(ExtensionSettingsError::Invalid(format!(
                        "{key} is a secret field; write it on its own"
                    )));
                }
                check_scope(key, field.scope, scope)?;
                if !value.is_null() {
                    check_value(key, field, value)?;
                }
            }
            let mut path: Vec<String> = vec![EXTENSION_SETTINGS_KEY.into(), namespace.into()];
            path.extend(segments.iter().map(|s| (*s).to_owned()));
            paths.push((path, Some(value.clone()).filter(|v| !v.is_null())));
            if segments.len() > 1 {
                paths.push((
                    vec![EXTENSION_SETTINGS_KEY.into(), namespace.into(), key.clone()],
                    None,
                ));
            }
        }
        if paths.is_empty() {
            return Ok(());
        }
        let borrowed: Vec<Vec<&str>> = paths
            .iter()
            .map(|(p, _)| p.iter().map(String::as_str).collect())
            .collect();
        let edits: Vec<(&[&str], Option<JsonValue>)> = borrowed
            .iter()
            .zip(&paths)
            .map(|(p, (_, v))| (p.as_slice(), v.clone()))
            .collect();
        self.manager().set_many(scope, &edits)?;
        let _ = self.changes.send(SettingsChange {
            namespace: namespace.into(),
            keys: values.keys().cloned().collect(),
            scope,
            extension_id: registered.map(|n| n.owner),
        });
        Ok(())
    }

    fn publish(&self, namespace: &str, key: &str, scope: SettingsScope, owner: Option<String>) {
        let _ = self.changes.send(SettingsChange {
            namespace: namespace.into(),
            keys: vec![key.into()],
            scope,
            extension_id: owner,
        });
    }

    /// Change notifications (frontends refresh the hub; the services forward them to owners).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<SettingsChange> {
        self.changes.subscribe()
    }
}

/// Secrets-file key of a secret field.
#[must_use]
pub fn secret_key(owner: &str, namespace: &str, key: &str) -> String {
    format!("{owner}/{namespace}/{key}")
}

fn validate_name(name: &str) -> Result<(), ExtensionSettingsError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(ExtensionSettingsError::Invalid(format!(
            "invalid settings namespace \"{name}\""
        )))
    }
}

fn key_segments(key: &str) -> Result<Vec<&str>, ExtensionSettingsError> {
    let segments: Vec<&str> = key.split('.').collect();
    if key.is_empty() || segments.iter().any(|s| s.is_empty()) {
        return Err(ExtensionSettingsError::Invalid(format!(
            "invalid settings key \"{key}\""
        )));
    }
    Ok(segments)
}

fn check_scope(
    key: &str,
    field: FieldScope,
    scope: SettingsScope,
) -> Result<(), ExtensionSettingsError> {
    match (field, scope) {
        (FieldScope::Global, SettingsScope::Project) => Err(ExtensionSettingsError::Scope {
            key: key.into(),
            allowed: SettingsScope::Global,
        }),
        (FieldScope::Project, SettingsScope::Global) => Err(ExtensionSettingsError::Scope {
            key: key.into(),
            allowed: SettingsScope::Project,
        }),
        _ => Ok(()),
    }
}

fn check_value(
    key: &str,
    field: &FieldInfo,
    value: &JsonValue,
) -> Result<(), ExtensionSettingsError> {
    let bad = |message: &str| {
        Err(ExtensionSettingsError::InvalidValue {
            key: key.into(),
            message: message.into(),
        })
    };
    match field.kind.as_str() {
        "boolean" if !value.is_boolean() => bad("expected a boolean"),
        "number" if !value.is_number() => bad("expected a number"),
        "string" | "secret" | "model" if !value.is_string() => bad("expected a string"),
        "multiselect" | "order"
            if !value
                .as_array()
                .is_some_and(|a| a.iter().all(JsonValue::is_string)) =>
        {
            bad("expected a list of strings")
        }
        "enum" => match (value.as_str(), &field.options) {
            (None, _) => bad("expected a string"),
            (Some(v), Some(options)) if !field.allow_custom && !options.iter().any(|o| o == v) => {
                bad(&format!("expected one of {}", options.join(", ")))
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Walks `sections[].fields[]` (and `page` sub-sections) collecting field facts and seeding
/// `defaults` from field `default`s that the defaults object does not cover.
fn collect_fields(
    sections: &JsonValue,
    fields: &mut HashMap<String, FieldInfo>,
    defaults: &mut JsonObject,
) {
    for section in sections.as_array().into_iter().flatten() {
        for field in section
            .get("fields")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            let Some(key) = field.get("key").and_then(JsonValue::as_str) else {
                continue;
            };
            let kind = field
                .get("type")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned();
            if kind == "page" {
                if let Some(sub) = field.get("sections") {
                    collect_fields(sub, fields, defaults);
                }
                continue;
            }
            if kind == "action" {
                continue;
            }
            let scope = match field.get("scope").and_then(JsonValue::as_str) {
                Some("global") => FieldScope::Global,
                Some("project") => FieldScope::Project,
                _ => FieldScope::Both,
            };
            let options = field.get("options").and_then(JsonValue::as_array).map(|a| {
                a.iter()
                    .filter_map(|o| {
                        o.as_str()
                            .or_else(|| o.get("value").and_then(JsonValue::as_str))
                            .map(ToOwned::to_owned)
                    })
                    .collect()
            });
            let allow_custom = field
                .get("allowCustom")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            if kind != "secret"
                && let Some(default) = field.get("default")
                && get_dotted(defaults, key).is_none()
            {
                set_path_value(defaults, key, default.clone());
            }
            fields.insert(
                key.to_owned(),
                FieldInfo {
                    kind,
                    scope,
                    options,
                    allow_custom,
                },
            );
        }
    }
}

fn get_dotted<'a>(object: &'a JsonObject, key: &str) -> Option<&'a JsonValue> {
    let segments: Vec<&str> = key.split('.').collect();
    get_path(object, &segments)
}

/// Sets `value` at dot path `key`, replacing non-object intermediates.
fn set_path_value(object: &mut JsonObject, key: &str, value: JsonValue) {
    let mut segments = key.split('.').peekable();
    let mut cursor = object;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            cursor.insert(segment.to_owned(), value);
            return;
        }
        let next = cursor
            .entry(segment.to_owned())
            .or_insert_with(|| JsonValue::Object(JsonObject::new()));
        if !next.is_object() {
            *next = JsonValue::Object(JsonObject::new());
        }
        let JsonValue::Object(map) = next else {
            return;
        };
        cursor = map;
    }
}

fn remove_path(value: &mut JsonValue, key: &str) {
    let segments: Vec<&str> = key.split('.').collect();
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut cursor = value;
    for segment in parents {
        match cursor.get_mut(*segment) {
            Some(next) => cursor = next,
            None => return,
        }
    }
    if let JsonValue::Object(map) = cursor {
        map.shift_remove(*last);
    }
}

/// Expands legacy flat dotted keys (`{"a.b": 1}` → `{"a": {"b": 1}}`); nested values win over
/// flat ones for the same path.
fn expand_dotted(object: JsonObject) -> JsonObject {
    let (flat, nested): (Vec<_>, Vec<_>) = object.into_iter().partition(|(k, _)| k.contains('.'));
    let mut out = JsonObject::new();
    for (key, value) in flat {
        set_path_value(&mut out, &key, value);
    }
    deep_merge_objects(&out, &nested.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn registry(dir: &std::path::Path) -> ExtensionSettings {
        let manager = SettingsManager::create(&dir.join("project"), &dir.join("agent"), true);
        ExtensionSettings::new(
            Arc::new(Mutex::new(manager)),
            Some(SecretStore::new(&dir.join("agent"))),
        )
    }

    fn notify_schema() -> JsonValue {
        json!({
            "namespace": "notify", "title": "Notify", "extensionId": "spoofed",
            "defaults": {"events": {"agentEnd": true}, "platform": "native"},
            "sections": [{"id": "s", "title": "S", "fields": [
                {"key": "events.agentEnd", "type": "boolean", "label": "Agent end"},
                {"key": "platform", "type": "enum", "label": "P", "options": [{"value": "native", "label": "N"}, {"value": "ntfy", "label": "ntfy"}]},
                {"key": "ntfy.token", "type": "secret", "label": "Token", "scope": "global"},
                {"key": "adv", "type": "page", "label": "Adv", "sections": [{"id": "a", "title": "A", "fields": [
                    {"key": "timeoutMs", "type": "number", "label": "T", "default": 5, "scope": "project"}
                ]}]}
            ]}]
        })
    }

    #[test]
    fn layers_defaults_global_project_and_expands_flat_keys() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        reg.register("unipi.notify", notify_schema()).unwrap();
        assert_eq!(reg.schema()[0]["extensionId"], "unipi.notify");
        assert_eq!(
            reg.values("notify", SettingsLayer::Effective),
            json!({"events": {"agentEnd": true}, "platform": "native", "timeoutMs": 5})
        );
        reg.set(
            "notify",
            "platform",
            Some(json!("ntfy")),
            SettingsScope::Global,
        )
        .unwrap();
        reg.set(
            "notify",
            "timeoutMs",
            Some(json!(9)),
            SettingsScope::Project,
        )
        .unwrap();
        reg.manager()
            .set(
                SettingsScope::Global,
                &["extensionSettings", "notify", "events.agentEnd"],
                Some(json!(false)),
            )
            .unwrap();
        let v = reg.values("notify", SettingsLayer::Effective);
        assert_eq!(v["platform"], "ntfy");
        assert_eq!(v["timeoutMs"], 9);
        assert_eq!(v["events"]["agentEnd"], false);
        assert_eq!(
            reg.values("notify", SettingsLayer::Project),
            json!({"timeoutMs": 9})
        );
        // A nested write removes the legacy flat key.
        reg.set(
            "notify",
            "events.agentEnd",
            Some(json!(true)),
            SettingsScope::Global,
        )
        .unwrap();
        let global: JsonValue = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("agent/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            global["extensionSettings"]["notify"],
            json!({"platform": "ntfy", "events": {"agentEnd": true}})
        );
        // `None` = use default.
        reg.set("notify", "timeoutMs", None, SettingsScope::Project)
            .unwrap();
        assert_eq!(
            reg.values("notify", SettingsLayer::Effective)["timeoutMs"],
            5
        );
    }

    #[test]
    fn secrets_go_to_the_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        let mut schema = notify_schema();
        schema["secretReaders"] = json!(["unipi.*"]);
        reg.register("unipi.notify", schema).unwrap();
        reg.set(
            "notify",
            "ntfy.token",
            Some(json!("tok")),
            SettingsScope::Global,
        )
        .unwrap();
        let file =
            std::fs::read_to_string(dir.path().join("agent/settings.json")).unwrap_or_default();
        assert!(!file.contains("tok"));
        let secrets = SecretStore::new(&dir.path().join("agent"));
        assert_eq!(
            secrets
                .get("unipi.notify/notify/ntfy.token")
                .unwrap()
                .as_deref(),
            Some("tok")
        );
        assert_eq!(
            reg.values("notify", SettingsLayer::Effective)["ntfy"]["token"],
            SECRET_MASK
        );
        let own = reg.values_for("unipi.notify", "notify", SettingsLayer::Effective);
        assert_eq!(own["ntfy"]["token"], "tok");
        let shared = reg.values_for("unipi.core", "notify", SettingsLayer::Effective);
        assert_eq!(shared["ntfy"]["token"], "tok");
        let other = reg.values_for("acme.x", "notify", SettingsLayer::Effective);
        assert!(other.get("ntfy").and_then(|n| n.get("token")).is_none());
        // Writing the mask back is a no-op; empty deletes.
        reg.set(
            "notify",
            "ntfy.token",
            Some(json!(SECRET_MASK)),
            SettingsScope::Global,
        )
        .unwrap();
        assert_eq!(
            secrets
                .get("unipi.notify/notify/ntfy.token")
                .unwrap()
                .as_deref(),
            Some("tok")
        );
        reg.set(
            "notify",
            "ntfy.token",
            Some(json!("")),
            SettingsScope::Global,
        )
        .unwrap();
        assert_eq!(secrets.get("unipi.notify/notify/ntfy.token").unwrap(), None);
    }

    #[test]
    fn ownership_validation_and_events() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        reg.register("unipi.notify", notify_schema()).unwrap();
        assert!(matches!(
            reg.register("other", notify_schema()),
            Err(ExtensionSettingsError::NotOwner { .. })
        ));
        assert!(matches!(
            reg.register("x", json!({"namespace": "bad name"})),
            Err(ExtensionSettingsError::Invalid(_))
        ));
        let mut rx = reg.subscribe();
        assert!(matches!(
            reg.set_as(
                "other",
                "notify",
                "platform",
                Some(json!("ntfy")),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::NotOwner { .. })
        ));
        assert!(matches!(
            reg.set_as("other", "nope", "k", Some(json!(1)), SettingsScope::Global),
            Err(ExtensionSettingsError::UnknownNamespace(_))
        ));
        assert!(matches!(
            reg.set(
                "notify",
                "platform",
                Some(json!("smoke")),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::InvalidValue { .. })
        ));
        assert!(matches!(
            reg.set(
                "notify",
                "events.agentEnd",
                Some(json!("yes")),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::InvalidValue { .. })
        ));
        assert!(matches!(
            reg.set(
                "notify",
                "ntfy.token",
                Some(json!("t")),
                SettingsScope::Project
            ),
            Err(ExtensionSettingsError::Scope { .. })
        ));
        assert!(matches!(
            reg.set("notify", "a..b", Some(json!(1)), SettingsScope::Global),
            Err(ExtensionSettingsError::Invalid(_))
        ));
        reg.set_as(
            "unipi.notify",
            "notify",
            "platform",
            Some(json!("ntfy")),
            SettingsScope::Global,
        )
        .unwrap();
        let change = rx.try_recv().unwrap();
        assert_eq!(
            change,
            SettingsChange {
                namespace: "notify".into(),
                keys: vec!["platform".into()],
                scope: SettingsScope::Global,
                extension_id: Some("unipi.notify".into()),
            }
        );
        assert!(rx.try_recv().is_err(), "failed writes publish nothing");
        let mut batch = JsonObject::new();
        batch.insert("platform".into(), json!("native"));
        batch.insert("rules".into(), json!([{"tool": "bash"}]));
        reg.set_many_as(
            Some("unipi.notify"),
            "notify",
            &batch,
            SettingsScope::Global,
        )
        .unwrap();
        assert_eq!(rx.try_recv().unwrap().keys, ["platform", "rules"]);
        assert_eq!(
            reg.values("notify", SettingsLayer::Global)["rules"],
            json!([{"tool": "bash"}])
        );
        assert_eq!(reg.owned_by("unipi.notify"), ["notify"]);
        assert_eq!(reg.all_for("unipi.notify")["notify"]["platform"], "native");
    }

    #[test]
    fn untrusted_project_writes_fail_and_project_layer_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SettingsManager::create(&dir.path().join("p"), &dir.path().join("a"), false);
        let reg = ExtensionSettings::new(Arc::new(Mutex::new(manager)), None);
        reg.register("e", notify_schema()).unwrap();
        assert!(matches!(
            reg.set(
                "notify",
                "platform",
                Some(json!("ntfy")),
                SettingsScope::Project
            ),
            Err(ExtensionSettingsError::Settings(_))
        ));
        assert!(matches!(
            reg.set(
                "notify",
                "ntfy.token",
                Some(json!("t")),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::Invalid(_))
        ));
    }
}
