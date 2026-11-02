//! Extension settings registry (runtime §10.3, D11) and the core `oino.*` namespaces
//! (ui-protocol v2 §7): namespaces declared by extensions and by core, layered
//! reads, scoped writes, secret fields and change notifications.
//!
//! - **Dialect**: OSS-2 (§7). Wire schemas are parsed into
//!   [`oino_ui::settings::SettingsNamespace`]; `resolve` / `simplify` / `check_value` come from
//!   `oino_ui::settings` so core, the TUI hub and the app agree. v1 §6.6 namespaces load
//!   unchanged.
//! - **Layering**: `defaults ⊕ global ⊕ project` where the layers are
//!   `extensionSettings.<namespace>` of `<agent>/settings.json` and the trusted
//!   `.oino/settings.json`, read through the shared [`SettingsManager`] (layered file
//!   writes, unknown keys preserved). Legacy flat dotted keys (`"events.x": true`) are
//!   expanded into nested objects on read and removed on write. Invalid stored values fall
//!   back to the next layer down and produce a [`oino_ui::settings::Diagnostic`]
//!   ([`ExtensionSettings::resolved`]); `resolve` never fails.
//! - **Core namespaces**: `oino.model`, `oino.compaction`, … describe the agent settings in the same
//!   dialect (owner `"oino"`, values live at the top level of the settings files) so frontends
//!   render one Settings screen. Writes go through the same commands.
//! - **Keys** are dot paths into the namespace object (`events.agentEnd` →
//!   `extensionSettings.notify.events.agentEnd`).
//! - **Default writes unset** (§7.2): writing a value equal to the layer below deletes the key
//!   instead of storing it.
//! - **Secret fields** (`type: "secret"`) never reach `settings.json`: they live in
//!   `<agent>/secrets.json` at `<owner-ext-id>/<namespace>/<key>`. Frontends see
//!   [`SECRET_MASK`] for a set secret; the owning extension (and extensions matching the
//!   namespace's `secretReaders` patterns) read the value.
//! - **Writes** come from frontends ([`ExtensionSettings::set`]) or from the owning extension
//!   ([`ExtensionSettings::set_as`], WIT `registry.set-extension-setting`). Each successful write
//!   publishes a [`SettingsChange`]; the services deliver it to the owner as
//!   `oino_settings_changed {namespace, keys, scope}`.

use std::sync::{Arc, Mutex, PoisonError, RwLock};

use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};
use oino_settings::json::{deep_merge_objects, get_path};
use oino_settings::{SecretStore, SettingsError, SettingsManager, SettingsScope};
use oino_ui::settings::{
    Field, FieldScope, FieldType, Layers, SettingsNamespace, SettingsSection, resolve_with,
    simplify_key,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// What frontends see for a secret field that holds a value. Writing it back is a no-op.
pub const SECRET_MASK: &str = "••••••";

/// Settings key holding every namespace (`extensionSettings`).
pub const EXTENSION_SETTINGS_KEY: &str = "extensionSettings";

/// Owner id of the core namespaces (the settings groups).
pub const CORE_OWNER: &str = "oino";

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

/// Where a namespace's values are stored.
#[derive(Clone, Debug)]
enum Root {
    /// `extensionSettings.<namespace>` of the settings files.
    Extension,
    /// Top-level settings; field keys are relative to `prefix` (`None` = top-level names).
    Core { prefix: Option<String> },
}

/// One registered namespace.
#[derive(Clone, Debug)]
struct Namespace {
    owner: String,
    schema: JsonValue,
    /// Parsed OSS-2 view; `None` when the schema is not a valid declaration (values then pass
    /// through unvalidated, v1 behaviour).
    typed: Option<SettingsNamespace>,
    /// Keys of `type: "secret"` fields.
    secret_keys: Vec<String>,
    root: Root,
    secret_readers: Vec<String>,
}

impl Namespace {
    fn defaults_value_object(&self) -> JsonObject {
        self.defaults_value()
            .as_object()
            .cloned()
            .unwrap_or_default()
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

    fn field(&self, key: &str) -> Option<Field> {
        self.typed.as_ref()?.field(key).cloned()
    }

    fn is_secret(&self, key: &str) -> bool {
        self.secret_keys.iter().any(|secret| secret == key)
    }

    fn defaults_value(&self) -> JsonValue {
        self.typed
            .as_ref()
            .and_then(|ns| ns.defaults.clone())
            .unwrap_or(JsonValue::Null)
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
            .field("secrets", &self.secrets.is_some())
            .finish_non_exhaustive()
    }
}

impl ExtensionSettings {
    /// A registry over `settings`; secrets go to `secrets` (absent in secret-less hosts).
    /// The core `oino.*` namespaces (settings groups, §7) are registered up front.
    #[must_use]
    pub fn new(settings: Arc<Mutex<SettingsManager>>, secrets: Option<SecretStore>) -> Self {
        let (changes, _) = broadcast::channel(CHANGE_CAPACITY);
        let registry = Self {
            settings,
            secrets,
            namespaces: RwLock::new(IndexMap::new()),
            changes,
        };
        for (namespace, prefix) in core_namespaces() {
            let schema = serde_json::to_value(&namespace).unwrap_or(JsonValue::Null);
            let root = Root::Core { prefix };
            if let Err(error) = registry.insert(CORE_OWNER, schema, Some(namespace), root) {
                tracing::warn!("registering core settings namespace: {error}");
            }
        }
        registry
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, IndexMap<String, Namespace>> {
        self.namespaces
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn manager(&self) -> std::sync::MutexGuard<'_, SettingsManager> {
        self.settings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `schema` for `extension`: the namespace id must be unique per host and owned
    /// by one extension (a second owner is rejected). The stored schema's `extensionId` is set
    /// to `extension`.
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
        let name = {
            let JsonValue::Object(object) = &mut schema else {
                return Err(ExtensionSettingsError::Invalid(
                    "settings namespace must be an object".into(),
                ));
            };
            object.insert(
                "extensionId".into(),
                JsonValue::String(extension.to_owned()),
            );
            object
                .get("namespace")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let typed = serde_json::from_value::<SettingsNamespace>(schema.clone()).ok();
        if typed.is_none() {
            tracing::warn!(
                "settings namespace {name}: schema is not OSS-2; values are passed through unvalidated"
            );
        }
        self.insert(extension, schema, typed, Root::Extension)
    }

    fn insert(
        &self,
        extension: &str,
        schema: JsonValue,
        typed: Option<SettingsNamespace>,
        root: Root,
    ) -> Result<(), ExtensionSettingsError> {
        let JsonValue::Object(object) = &schema else {
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
        let secret_keys = match &typed {
            Some(ns) => ns
                .fields()
                .into_iter()
                .filter(|field| field.type_ == FieldType::Secret)
                .map(|field| field.key.clone())
                .collect(),
            None => secret_keys_of(&schema),
        };
        let secret_readers = object
            .get("secretReaders")
            .and_then(JsonValue::as_array)
            .map(|readers| {
                readers
                    .iter()
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
                typed,
                secret_keys,
                root,
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

    /// The merged settings object.
    #[must_use]
    pub fn merged_settings(&self) -> JsonObject {
        self.manager().settings().merged().clone()
    }

    /// Schemas of every registered namespace (core `oino.*` first), in registration order
    /// (`get_settings_schema`).
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
        self.resolve(namespace, layer, false).values
    }

    /// Values plus the stored values `resolve` rejected (§7.1 diagnostics) — the reply of
    /// `get_extension_settings`.
    #[must_use]
    pub fn resolved(&self, namespace: &str, layer: SettingsLayer) -> oino_ui::settings::Resolved {
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
        let mut values = self.resolve(namespace, layer, reveal).values;
        if !reveal && let Some(ns) = self.read().get(namespace) {
            for key in &ns.secret_keys {
                remove_path(&mut values, key);
            }
        }
        values
    }

    /// `{namespace: effective values}` of every namespace present in the settings files or
    /// registered, as extension `reader` sees them (`InitInput.settings`). Core `oino.*`
    /// namespaces are frontend-only and stay out of the extension payload.
    #[must_use]
    pub fn all_for(&self, reader: &str) -> JsonValue {
        let core: Vec<String> = self
            .read()
            .iter()
            .filter(|(_, n)| matches!(n.root, Root::Core { .. }))
            .map(|(k, _)| k.clone())
            .collect();
        let mut names: Vec<String> = self
            .read()
            .keys()
            .filter(|name| !core.contains(name))
            .cloned()
            .collect();
        {
            let manager = self.manager();
            let merged = manager.settings().merged();
            if let Some(JsonValue::Object(all)) = merged.get(EXTENSION_SETTINGS_KEY) {
                names.extend(
                    all.keys()
                        .filter(|k| !names.contains(k) && !core.contains(k))
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

    /// The stored layer object of `namespace` in one settings file (`Root::Extension`:
    /// `extensionSettings.<namespace>`).
    fn layer(registered: Option<&Namespace>, object: &JsonObject, namespace: &str) -> JsonObject {
        let value = match registered.map(|ns| &ns.root) {
            Some(Root::Core {
                prefix: Some(prefix),
            }) => get_path(object, &prefix.split('.').collect::<Vec<_>>()),
            Some(Root::Core { prefix: None }) => return object.clone(),
            _ => get_path(object, &[EXTENSION_SETTINGS_KEY, namespace]),
        };
        value
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default()
    }

    /// Global and project layer objects of `namespace` (project empty when untrusted), with
    /// legacy flat dotted keys expanded; core namespaces are projected to their fields.
    fn stored_layers(
        &self,
        registered: Option<&Namespace>,
        namespace: &str,
    ) -> (JsonObject, JsonObject) {
        let manager = self.manager();
        let view = manager.settings();
        let layer_of = |object: &JsonObject| {
            let layer = Self::layer(registered, object, namespace);
            match registered {
                Some(ns) if matches!(ns.root, Root::Core { .. }) => project_fields(ns, &layer),
                _ => expand_dotted(layer),
            }
        };
        let global = layer_of(view.global());
        let project = if manager.is_project_trusted() {
            layer_of(view.project())
        } else {
            JsonObject::new()
        };
        (global, project)
    }

    fn resolve(
        &self,
        namespace: &str,
        layer: SettingsLayer,
        reveal: bool,
    ) -> oino_ui::settings::Resolved {
        let registered = self.read().get(namespace).cloned();
        let (global, project) = self.stored_layers(registered.as_ref(), namespace);
        let resolved = match (&registered, layer) {
            (_, SettingsLayer::Global) => oino_ui::settings::Resolved {
                values: JsonValue::Object(global),
                diagnostics: Vec::new(),
            },
            (_, SettingsLayer::Project) => oino_ui::settings::Resolved {
                values: JsonValue::Object(project),
                diagnostics: Vec::new(),
            },
            (None, SettingsLayer::Effective) => oino_ui::settings::Resolved {
                values: JsonValue::Object(deep_merge_objects(&global, &project)),
                diagnostics: Vec::new(),
            },
            (Some(ns), SettingsLayer::Effective) => match &ns.typed {
                Some(typed) => {
                    let defaults = ns.defaults_value();
                    resolve_with(
                        typed,
                        Layers {
                            defaults: &defaults,
                            global: &JsonValue::Object(global.clone()),
                            project: &JsonValue::Object(project.clone()),
                        },
                        &check_field,
                    )
                }
                None => oino_ui::settings::Resolved {
                    values: JsonValue::Object(deep_merge_objects(
                        &deep_merge_objects(
                            &registered
                                .as_ref()
                                .map_or_else(JsonObject::new, Namespace::defaults_value_object),
                            &global,
                        ),
                        &project,
                    )),
                    diagnostics: Vec::new(),
                },
            },
        };
        let mut values = resolved.values;
        if let (Some(ns), Some(store), false) =
            (&registered, &self.secrets, layer == SettingsLayer::Project)
        {
            for key in &ns.secret_keys {
                match store.get(&secret_key(&ns.owner, namespace, key)) {
                    Ok(Some(secret)) => {
                        let shown = if reveal {
                            secret
                        } else {
                            SECRET_MASK.to_owned()
                        };
                        set_value_path(&mut values, key, JsonValue::String(shown));
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!("reading secret {namespace}.{key}: {error}"),
                }
            }
        }
        oino_ui::settings::Resolved {
            values,
            diagnostics: resolved.diagnostics,
        }
    }

    /// Frontend write (`set_extension_setting` / `unset_extension_setting`): `None` (or JSON
    /// `null`) deletes the key in `scope` ("use default"). Writing the value of the layer below
    /// also unsets (§7.2). Unregistered namespaces accept plain values.
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
        key_segments(key)?;
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
        let field = registered.as_ref().and_then(|ns| ns.field(key));
        if let Some(field) = &field {
            check_scope(key, field.scope(), scope)?;
            if let Some(value) = &value {
                check_field(field, value).map_err(|message| {
                    ExtensionSettingsError::InvalidValue {
                        key: key.into(),
                        message,
                    }
                })?;
            }
        }
        let owner = registered.as_ref().map(|n| n.owner.clone());
        if registered.as_ref().is_some_and(|ns| ns.is_secret(key)) {
            let Some(store) = &self.secrets else {
                return Err(ExtensionSettingsError::Invalid(
                    "secret settings are not available in this host".into(),
                ));
            };
            let at = secret_key(owner.as_deref().unwrap_or(CORE_OWNER), namespace, key);
            match value.as_ref().and_then(JsonValue::as_str) {
                Some(SECRET_MASK) => return Ok(()),
                Some(secret) if !secret.is_empty() => store.set(&at, secret)?,
                _ => {
                    store.delete(&at)?;
                }
            }
            self.publish(namespace, key, SettingsScope::Global, owner);
            return Ok(());
        }
        let (path, flat, stored) = self.plan(namespace, registered.as_ref(), key, value, scope);
        let path_ref: Vec<&str> = path.iter().map(String::as_str).collect();
        let flat_ref: Vec<&str> = flat
            .as_ref()
            .map(|flat| flat.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let mut edits: Vec<(&[&str], Option<JsonValue>)> = vec![(path_ref.as_slice(), stored)];
        if flat.is_some() {
            edits.push((flat_ref.as_slice(), None));
        }
        self.manager().set_many(scope, &edits)?;
        self.publish(namespace, key, scope, owner);
        Ok(())
    }

    /// The edit paths and the stored value of one key (`None` in `stored` = unset).
    fn plan(
        &self,
        namespace: &str,
        registered: Option<&Namespace>,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> (Vec<String>, Option<Vec<String>>, Option<JsonValue>) {
        let segments: Vec<&str> = key.split('.').collect();
        let base: Vec<String> = match registered.map(|ns| &ns.root) {
            Some(Root::Core {
                prefix: Some(prefix),
            }) => prefix.split('.').map(ToOwned::to_owned).collect(),
            Some(Root::Core { prefix: None }) => Vec::new(),
            _ => vec![EXTENSION_SETTINGS_KEY.into(), namespace.into()],
        };
        let mut path = base.clone();
        path.extend(segments.iter().map(|segment| (*segment).to_owned()));
        let flat = (segments.len() > 1).then(|| {
            let mut flat = base;
            flat.push(key.to_owned());
            flat
        });
        let stored = match (&value, registered.and_then(|ns| ns.field(key))) {
            (Some(value), Some(field)) => {
                let below = self.below(namespace, registered, scope, key);
                simplify_key(&field, below.as_ref(), value)
            }
            _ => value,
        };
        (path, flat, stored)
    }

    /// Effective value of `key` one layer below a `scope` write (§7.2): the defaults for a
    /// global write, defaults ⊕ global for a project write.
    fn below(
        &self,
        namespace: &str,
        registered: Option<&Namespace>,
        scope: SettingsScope,
        key: &str,
    ) -> Option<JsonValue> {
        let registered = registered?;
        let typed = registered.typed.as_ref()?;
        let (global, _) = self.stored_layers(Some(registered), namespace);
        let defaults = registered.defaults_value();
        let empty = JsonObject::new();
        let layers = match scope {
            SettingsScope::Global => Layers {
                defaults: &defaults,
                global: &JsonValue::Object(empty),
                project: &JsonValue::Null,
            },
            SettingsScope::Project => Layers {
                defaults: &defaults,
                global: &JsonValue::Object(global),
                project: &JsonValue::Null,
            },
        };
        let resolved = resolve_with(typed, layers, &check_field);
        resolved
            .values
            .as_object()
            .and_then(|values| get_dotted(values, key))
            .cloned()
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
            if registered.as_ref().is_some_and(|ns| ns.is_secret(key)) {
                return Err(ExtensionSettingsError::Invalid(format!(
                    "{key} is a secret field; write it on its own"
                )));
            }
            let (path, flat, stored) = self.plan(
                namespace,
                registered.as_ref(),
                key,
                Some(value.clone()).filter(|v| !v.is_null()),
                scope,
            );
            paths.push((path, stored));
            if let Some(flat) = flat {
                paths.push((flat, None));
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

/// Keeps only the namespace's field values of a core namespace's layer (`Root::Core`).
fn project_fields(namespace: &Namespace, layer: &JsonObject) -> JsonObject {
    let Some(typed) = &namespace.typed else {
        return layer.clone();
    };
    let mut out = JsonObject::new();
    for field in typed.fields() {
        if let Some(value) = get_dotted(layer, &field.key) {
            set_path_value(&mut out, &field.key, value.clone());
        }
    }
    out
}

/// OSS-2 value check plus the regex parts `oino_ui` (serde-only) leaves to the host.
fn check_field(field: &Field, value: &JsonValue) -> Result<(), String> {
    oino_ui::settings::check_value(field, value)?;
    if let Some(pattern) = &field.props.pattern {
        let regex = regex::Regex::new(pattern)
            .map_err(|error| format!("invalid pattern {pattern}: {error}"))?;
        if let Some(text) = value.as_str()
            && !regex.is_match(text)
        {
            return Err(format!("must match {pattern}"));
        }
    }
    if field.type_ == FieldType::Dict
        && let Some(pattern) = &field.props.key_pattern
    {
        let regex = regex::Regex::new(pattern)
            .map_err(|error| format!("invalid keyPattern {pattern}: {error}"))?;
        if let Some(entries) = value.as_object()
            && let Some(key) = entries.keys().find(|key| !regex.is_match(key))
        {
            return Err(format!("key {key} does not match {pattern}"));
        }
    }
    Ok(())
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

/// [`set_path_value`] on a whole value.
fn set_value_path(value: &mut JsonValue, key: &str, new: JsonValue) {
    if let JsonValue::Object(object) = value {
        set_path_value(object, key, new);
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

/// Secret field keys of a schema that does not parse as OSS-2 (v1 raw walk).
fn secret_keys_of(schema: &JsonValue) -> Vec<String> {
    fn walk(sections: &JsonValue, out: &mut Vec<String>) {
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
                match field.get("type").and_then(JsonValue::as_str) {
                    Some("secret") => out.push(key.to_owned()),
                    Some("page") => {
                        if let Some(sub) = field.get("sections") {
                            walk(sub, out);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Some(sections) = schema.get("sections") {
        walk(sections, &mut out);
    }
    out
}

/// A settings field of a core namespace (compact builder for the tables below).
fn core_field(key: &str, label: &str, type_: FieldType) -> Field {
    Field::new(key, type_, label)
}

fn core_boolean(key: &str, label: &str, default: bool) -> Field {
    let mut field = core_field(key, label, FieldType::Boolean);
    field.default = Some(JsonValue::Bool(default));
    field
}

fn core_choice(key: &str, label: &str, options: &[&str], default: &str) -> Field {
    let mut field = core_field(key, label, FieldType::Enum);
    field.props.options = options
        .iter()
        .map(|option| oino_ui::settings::Choice {
            value: JsonValue::String((*option).to_owned()),
            label: (*option).to_owned(),
            description: None,
        })
        .collect();
    field.default = Some(JsonValue::String(default.to_owned()));
    field
}

fn core_number(key: &str, label: &str, default: u64, unit: Option<&str>) -> Field {
    let mut field = core_field(key, label, FieldType::Number);
    field.default = Some(JsonValue::from(default));
    field.props.integer = Some(true);
    field.props.min = Some(serde_json::Number::from(0u64));
    field.props.unit = unit.map(ToOwned::to_owned);
    field
}

fn core_string(key: &str, label: &str) -> Field {
    core_field(key, label, FieldType::String)
}

/// One page of core fields.
fn core_section(id: &str, title: &str, fields: Vec<Field>) -> SettingsSection {
    SettingsSection {
        id: id.to_owned(),
        title: title.to_owned(),
        description: None,
        advanced: None,
        fields,
    }
}

fn core_namespace(
    namespace: &str,
    title: &str,
    order: i32,
    description: &str,
    sections: Vec<SettingsSection>,
) -> SettingsNamespace {
    SettingsNamespace {
        namespace: namespace.to_owned(),
        title: title.to_owned(),
        extension_id: CORE_OWNER.to_owned(),
        icon: None,
        order: Some(order),
        description: Some(description.to_owned()),
        defaults: None,
        sections,
    }
}

/// The `oino.*` namespaces of §7 with their storage prefix (settings top-level key).
#[allow(
    clippy::too_many_lines,
    reason = "declarative table of the settings groups"
)]
fn core_namespaces() -> Vec<(SettingsNamespace, Option<String>)> {
    use oino_settings::types::defaults;
    let mut list = vec![
        (
            core_namespace(
                "oino.model",
                "Model & thinking",
                10,
                "Startup model, scoped models and reasoning budgets",
                vec![core_section(
                    "model",
                    "Model",
                    vec![
                        {
                            // The picker field: stores the model id; the app writes `defaultProvider` beside it.
                            let mut field = core_field("defaultModel", "Default model", FieldType::Model);
                            field.description = Some("The model new chats start on. Empty = the first model of a working provider.".into());
                            field
                        },
                        core_choice(
                            "defaultThinkingLevel",
                            "Thinking level",
                            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
                            "medium",
                        ),
                        {
                            let mut field =
                                core_field("enabledModels", "Scoped models", FieldType::List);
                            field.props.item =
                                Some(Box::new(core_field("model", "Model", FieldType::String)));
                            field
                        },
                        {
                            let mut field = core_field(
                                "modelThinkingLevels",
                                "Per-model thinking levels",
                                FieldType::Dict,
                            );
                            field.props.value = Some(Box::new(core_field(
                                "level",
                                "Thinking level",
                                FieldType::String,
                            )));
                            field
                        },
                        {
                            // One token budget per level (core's `ThinkingBudgets`); blank = the provider default.
                            let mut field = core_field("thinkingBudgets", "Thinking budgets", FieldType::Json);
                            field.description = Some("Token budget per thinking level. Blank = the provider default.".into());
                            field.props.schema = Some(serde_json::json!({
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "minimal": {"type": "integer", "minimum": 0, "title": "Minimal", "unit": "tokens"},
                                    "low": {"type": "integer", "minimum": 0, "title": "Low", "unit": "tokens"},
                                    "medium": {"type": "integer", "minimum": 0, "title": "Medium", "unit": "tokens"},
                                    "high": {"type": "integer", "minimum": 0, "title": "High", "unit": "tokens"}
                                }
                            }));
                            field
                        },
                    ],
                )],
            ),
            None,
        ),
        (
            core_namespace(
                "oino.compaction",
                "Compaction",
                20,
                "Automatic context compaction",
                vec![core_section(
                    "compaction",
                    "Compaction",
                    vec![
                        core_boolean("enabled", "Auto-compact", defaults::COMPACTION_ENABLED),
                        core_number(
                            "reserveTokens",
                            "Reserved tokens",
                            defaults::COMPACTION_RESERVE_TOKENS,
                            Some("tokens"),
                        ),
                        core_number(
                            "keepRecentTokens",
                            "Recent tokens kept verbatim",
                            defaults::COMPACTION_KEEP_RECENT_TOKENS,
                            Some("tokens"),
                        ),
                    ],
                )],
            ),
            Some("compaction".into()),
        ),
        (
            core_namespace(
                "oino.branch-summary",
                "Branch summaries",
                30,
                "Summaries when navigating the session tree",
                vec![core_section(
                    "branch",
                    "Branch summaries",
                    vec![
                        core_number(
                            "reserveTokens",
                            "Reserved tokens",
                            defaults::BRANCH_SUMMARY_RESERVE_TOKENS,
                            Some("tokens"),
                        ),
                        core_boolean(
                            "skipPrompt",
                            "Skip the prompt",
                            defaults::BRANCH_SUMMARY_SKIP_PROMPT,
                        ),
                    ],
                )],
            ),
            Some("branchSummary".into()),
        ),
        (
            core_namespace(
                "oino.retry",
                "Retry",
                40,
                "Automatic retry of transient provider errors",
                vec![core_section(
                    "retry",
                    "Retry",
                    vec![
                        core_boolean("enabled", "Auto-retry", defaults::RETRY_ENABLED),
                        core_number(
                            "maxRetries",
                            "Maximum retries",
                            defaults::RETRY_MAX_RETRIES,
                            None,
                        ),
                        core_number(
                            "baseDelayMs",
                            "Base delay",
                            defaults::RETRY_BASE_DELAY_MS,
                            Some("ms"),
                        ),
                        core_number(
                            "maxAgentDelayMs",
                            "Maximum delay",
                            defaults::RETRY_MAX_AGENT_DELAY_MS,
                            Some("ms"),
                        ),
                        core_number(
                            "provider.maxRetryDelayMs",
                            "Maximum provider delay",
                            defaults::RETRY_PROVIDER_MAX_RETRY_DELAY_MS,
                            Some("ms"),
                        ),
                    ],
                )],
            ),
            Some("retry".into()),
        ),
        (
            core_namespace(
                "oino.agent",
                "Session & queue",
                50,
                "Queued message delivery, sessions and skills",
                vec![core_section(
                    "agent",
                    "Agent",
                    vec![
                        core_choice(
                            "steeringMode",
                            "Steering delivery",
                            &["one-at-a-time", "all"],
                            "one-at-a-time",
                        ),
                        core_choice(
                            "followUpMode",
                            "Follow-up delivery",
                            &["one-at-a-time", "all"],
                            "one-at-a-time",
                        ),
                        {
                            let mut field = core_field(
                                "defaultProjectTrust",
                                "Default project trust",
                                FieldType::Enum,
                            );
                            field.props.options = ["ask", "always", "never"]
                                .iter()
                                .map(|option| oino_ui::settings::Choice {
                                    value: JsonValue::String((*option).to_owned()),
                                    label: (*option).to_owned(),
                                    description: None,
                                })
                                .collect();
                            field.default = Some(JsonValue::String("ask".into()));
                            field.scope = Some(FieldScope::Global);
                            field
                        },
                        core_boolean(
                            "enableSkillCommands",
                            "Skill commands",
                            defaults::ENABLE_SKILL_COMMANDS,
                        ),
                        {
                            let mut field = core_string("sessionDir", "Session storage folder");
                            field.description = Some(
                                "Where chat history files are saved. Empty = ~/.oino/sessions (default)."
                                    .into(),
                            );
                            field.props.format = Some(oino_ui::settings::StringFormat::Path);
                            field
                        },
                        core_boolean("quietStartup", "Quiet startup", false),
                    ],
                )],
            ),
            None,
        ),
        (
            core_namespace(
                "oino.transport",
                "Provider transport",
                60,
                "Streaming transport, timeouts and prompt caching",
                vec![core_section(
                    "transport",
                    "Transport",
                    vec![
                        {
                            let mut field = core_field("transport", "Transport", FieldType::Enum);
                            field.props.options = ["auto", "sse", "websocket", "websocket-cached"]
                                .iter()
                                .map(|option| oino_ui::settings::Choice {
                                    value: JsonValue::String((*option).to_owned()),
                                    label: (*option).to_owned(),
                                    description: None,
                                })
                                .collect();
                            field.default = Some(JsonValue::String("auto".into()));
                            field
                        },
                        core_number(
                            "httpIdleTimeoutMs",
                            "HTTP idle timeout",
                            defaults::HTTP_IDLE_TIMEOUT_MS,
                            Some("ms"),
                        ),
                        core_choice(
                            "cacheWarming",
                            "Prompt-cache warming",
                            &["off", "streaming", "idle"],
                            "streaming",
                        ),
                        core_boolean(
                            "showCacheMissNotices",
                            "Cache-miss notices",
                            defaults::SHOW_CACHE_MISS_NOTICES,
                        ),
                    ],
                )],
            ),
            None,
        ),
        (
            core_namespace(
                "oino.images",
                "Images",
                70,
                "Image handling",
                vec![core_section(
                    "images",
                    "Images",
                    vec![
                        core_boolean("autoResize", "Auto-resize", defaults::IMAGES_AUTO_RESIZE),
                        core_boolean("blockImages", "Block images", defaults::IMAGES_BLOCK_IMAGES),
                    ],
                )],
            ),
            Some("images".into()),
        ),
        (
            core_namespace(
                "oino.shell",
                "Shell",
                80,
                "The bash tool's shell",
                vec![core_section(
                    "shell",
                    "Shell",
                    vec![
                        {
                            let mut field = core_string("shellPath", "Shell path");
                            field.props.format = Some(oino_ui::settings::StringFormat::Path);
                            field
                        },
                        core_string("shellCommandPrefix", "Command prefix"),
                    ],
                )],
            ),
            None,
        ),
        (
            core_namespace(
                "oino.tui",
                "Terminal UI",
                90,
                "Density, theme and transcript preferences",
                vec![core_section(
                    "tui",
                    "TUI",
                    vec![
                        core_choice("theme", "Theme", &["dark", "light"], "dark"),
                        core_boolean("hideThinkingBlock", "Hide thinking", false),
                        core_choice(
                            "doubleEscapeAction",
                            "Double-escape action",
                            &["tree", "fork", "none"],
                            "tree",
                        ),
                        core_choice(
                            "treeFilterMode",
                            "Tree filter",
                            &["default", "no-tools", "user-only", "labeled-only", "all"],
                            "default",
                        ),
                        core_choice(
                            "tuiMode",
                            "TUI mode",
                            &["fullscreen", "regular"],
                            "fullscreen",
                        ),
                    ],
                )],
            ),
            None,
        ),
        (
            core_namespace(
                "oino.ui",
                "Interface",
                95,
                "Frontend density hint",
                vec![core_section(
                    "ui",
                    "Interface",
                    vec![core_choice(
                        "density",
                        "Transcript density",
                        &["simple", "regular", "advanced"],
                        "regular",
                    )],
                )],
            ),
            Some("ui".into()),
        ),
        (
            core_namespace(
                "oino.serve",
                "Serve",
                100,
                "oino serve listener",
                vec![core_section(
                    "serve",
                    "Serve",
                    vec![
                        {
                            let mut field = core_string("bind", "Listen address");
                            field.default = Some(JsonValue::String(defaults::SERVE_BIND.into()));
                            field.scope = Some(FieldScope::Global);
                            field
                        },
                        {
                            let mut field = core_boolean("tls", "TLS", defaults::SERVE_TLS);
                            field.scope = Some(FieldScope::Global);
                            field
                        },
                        {
                            let mut field = core_boolean(
                                "allowRemote",
                                "Allow remote clients",
                                defaults::SERVE_ALLOW_REMOTE,
                            );
                            field.scope = Some(FieldScope::Global);
                            field
                        },
                    ],
                )],
            ),
            Some("serve".into()),
        ),
    ];
    list.sort_by_key(|(namespace, _)| namespace.order);
    list
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
        reg.register("acme.notify", notify_schema()).unwrap();
        let schema = reg.schema();
        let notify = schema
            .iter()
            .find(|ns| ns["namespace"] == "notify")
            .expect("notify is registered");
        assert_eq!(notify["extensionId"], "acme.notify");
        assert_eq!(
            schema[0]["extensionId"], CORE_OWNER,
            "core namespaces come first"
        );
        assert_eq!(reg.owner("notify").as_deref(), Some("acme.notify"));
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
        // Writing the value of the layer below unsets the key (OSS-2 §7 simplify).
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
            global["extensionSettings"]["notify"]["platform"], "ntfy",
            "the namespace default is not written back"
        );
        assert!(
            global["extensionSettings"]["notify"]["events"]
                .get("agentEnd")
                .is_none(),
            "the default-valued key is unset: {global}"
        );
        assert_eq!(
            reg.values("notify", SettingsLayer::Effective)["events"]["agentEnd"],
            true
        );
        // A nested write removes the legacy flat key.
        reg.set(
            "notify",
            "events.agentEnd",
            Some(json!(false)),
            SettingsScope::Global,
        )
        .unwrap();
        let global: JsonValue = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("agent/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            global["extensionSettings"]["notify"],
            json!({"platform": "ntfy", "events": {"agentEnd": false}})
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
        schema["secretReaders"] = json!(["acme.*"]);
        reg.register("acme.notify", schema).unwrap();
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
                .get("acme.notify/notify/ntfy.token")
                .unwrap()
                .as_deref(),
            Some("tok")
        );
        assert_eq!(
            reg.values("notify", SettingsLayer::Effective)["ntfy"]["token"],
            SECRET_MASK
        );
        let own = reg.values_for("acme.notify", "notify", SettingsLayer::Effective);
        assert_eq!(own["ntfy"]["token"], "tok");
        let shared = reg.values_for("acme.core", "notify", SettingsLayer::Effective);
        assert_eq!(shared["ntfy"]["token"], "tok");
        let other = reg.values_for("other.x", "notify", SettingsLayer::Effective);
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
                .get("acme.notify/notify/ntfy.token")
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
        assert_eq!(secrets.get("acme.notify/notify/ntfy.token").unwrap(), None);
    }

    #[test]
    fn ownership_validation_and_events() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        reg.register("acme.notify", notify_schema()).unwrap();
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
            "acme.notify",
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
                extension_id: Some("acme.notify".into()),
            }
        );
        assert!(rx.try_recv().is_err(), "failed writes publish nothing");
        let mut batch = JsonObject::new();
        batch.insert("platform".into(), json!("native"));
        batch.insert("rules".into(), json!([{"tool": "bash"}]));
        reg.set_many_as(Some("acme.notify"), "notify", &batch, SettingsScope::Global)
            .unwrap();
        assert_eq!(rx.try_recv().unwrap().keys, ["platform", "rules"]);
        assert_eq!(
            reg.values("notify", SettingsLayer::Global)["rules"],
            json!([{"tool": "bash"}])
        );
        assert_eq!(reg.owned_by("acme.notify"), ["notify"]);
        assert_eq!(reg.all_for("acme.notify")["notify"]["platform"], "native");
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

    /// Writes through the manager (a file layer, like a hand-edited settings file).
    fn write_layer(reg: &ExtensionSettings, scope: SettingsScope, path: &[&str], value: JsonValue) {
        reg.manager().set(scope, path, Some(value)).unwrap();
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one scenario per core namespace guarantee"
    )]
    fn core_namespaces_resolve_pi_settings_and_write_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        // Registered up front, owned by core, sorted by order.
        let namespaces: Vec<String> = reg
            .schema()
            .iter()
            .filter_map(|ns| ns["namespace"].as_str().map(ToOwned::to_owned))
            .collect();
        assert_eq!(
            namespaces,
            [
                "oino.model",
                "oino.compaction",
                "oino.branch-summary",
                "oino.retry",
                "oino.agent",
                "oino.transport",
                "oino.images",
                "oino.shell",
                "oino.tui",
                "oino.ui",
                "oino.serve",
            ]
        );
        assert!(
            reg.owned_by(CORE_OWNER)
                .contains(&"oino.compaction".to_owned())
        );

        // Defaults come from `oino_settings::types::defaults`.
        assert_eq!(
            reg.values("oino.compaction", SettingsLayer::Effective),
            json!({"enabled": true, "reserveTokens": 16_384, "keepRecentTokens": 20_000})
        );
        // The global file is the top-level object, not `extensionSettings`.
        write_layer(
            &reg,
            SettingsScope::Global,
            &["compaction", "enabled"],
            json!(false),
        );
        write_layer(
            &reg,
            SettingsScope::Global,
            &["retry", "maxRetries"],
            json!(5),
        );
        write_layer(
            &reg,
            SettingsScope::Project,
            &["retry", "maxRetries"],
            json!(7),
        );
        assert_eq!(
            reg.values("oino.compaction", SettingsLayer::Effective)["enabled"],
            false
        );
        assert_eq!(
            reg.values("oino.retry", SettingsLayer::Effective)["maxRetries"],
            7
        );
        assert_eq!(
            reg.values("oino.retry", SettingsLayer::Global),
            json!({"maxRetries": 5}),
            "layer views are the stored namespace values, not the whole file"
        );

        // Frontend writes land at the top level and simplify a default back to an unset.
        reg.set(
            "oino.retry",
            "maxRetries",
            Some(json!(9)),
            SettingsScope::Global,
        )
        .unwrap();
        reg.set(
            "oino.agent",
            "steeringMode",
            Some(json!("all")),
            SettingsScope::Global,
        )
        .unwrap();
        reg.set(
            "oino.retry",
            "baseDelayMs",
            Some(json!(oino_settings::types::defaults::RETRY_BASE_DELAY_MS)),
            SettingsScope::Global,
        )
        .unwrap();
        let global: JsonValue = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("agent/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(global["retry"]["maxRetries"], 9);
        assert_eq!(global["steeringMode"], "all");
        assert!(
            global["retry"].get("baseDelayMs").is_none(),
            "writing the default unsets: {global}"
        );
        assert!(
            global
                .get("extensionSettings")
                .is_none_or(|v| v.get("oino.retry").is_none())
        );

        // Global-scope core fields reject project writes.
        assert!(matches!(
            reg.set(
                "oino.serve",
                "tls",
                Some(json!(false)),
                SettingsScope::Project
            ),
            Err(ExtensionSettingsError::Scope { .. })
        ));
        // Validation uses the OSS-2 field types.
        assert!(matches!(
            reg.set(
                "oino.compaction",
                "reserveTokens",
                Some(json!("lots")),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::InvalidValue { .. })
        ));
        // Extensions cannot write the core namespaces.
        assert!(matches!(
            reg.set_as(
                "acme.notify",
                "oino.retry",
                "enabled",
                Some(json!(false)),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::NotOwner { .. })
        ));
        // Core namespaces stay out of the extension payload.
        assert!(
            reg.all_for("acme.notify")
                .as_object()
                .is_some_and(|all| all.keys().all(|key| !key.starts_with("oino."))),
            "{:?}",
            reg.all_for("acme.notify")
        );
    }

    #[test]
    fn invalid_stored_values_fall_back_with_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        reg.register("acme.notify", notify_schema()).unwrap();
        write_layer(
            &reg,
            SettingsScope::Global,
            &["extensionSettings", "notify", "platform"],
            json!("ntfy"),
        );
        write_layer(
            &reg,
            SettingsScope::Project,
            &["extensionSettings", "notify", "platform"],
            json!("carrier-pigeon"),
        );
        write_layer(
            &reg,
            SettingsScope::Project,
            &["extensionSettings", "notify", "timeoutMs"],
            json!("soon"),
        );
        let resolved = reg.resolved("notify", SettingsLayer::Effective);
        assert_eq!(resolved.values["platform"], "ntfy");
        assert_eq!(resolved.values["timeoutMs"], 5, "field default");
        assert_eq!(
            resolved.diagnostics,
            vec![
                oino_ui::settings::Diagnostic {
                    namespace: "notify".into(),
                    key: "platform".into(),
                    message: "expected one of native, ntfy".into(),
                },
                oino_ui::settings::Diagnostic {
                    namespace: "notify".into(),
                    key: "timeoutMs".into(),
                    message: "expected a number".into(),
                },
            ]
        );
        // `resolve` never fails and the layer view stays the raw stored layer.
        assert_eq!(
            reg.values("notify", SettingsLayer::Project)["platform"],
            "carrier-pigeon"
        );
    }

    #[test]
    fn scope_layers_and_patterns_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        reg.register(
            "acme.workflow",
            json!({
                "namespace": "permission", "title": "Permission",
                "sections": [{"id": "s", "title": "S", "fields": [
                    {"key": "mode", "type": "enum", "label": "Mode",
                     "options": [{"value": "ask", "label": "Ask"}, {"value": "auto", "label": "Auto"}]},
                    {"key": "root", "type": "string", "label": "Root",
                     "pattern": "^/", "scope": "global"}
                ]}]
            }),
        )
        .unwrap();
        // A global-scope field ignores the project file (§7.1 layering).
        write_layer(
            &reg,
            SettingsScope::Global,
            &["extensionSettings", "permission", "root"],
            json!("/srv"),
        );
        write_layer(
            &reg,
            SettingsScope::Project,
            &["extensionSettings", "permission", "root"],
            json!("/tmp"),
        );
        assert_eq!(
            reg.values("permission", SettingsLayer::Effective)["root"],
            "/srv"
        );
        // A stored value violating the pattern falls back with a diagnostic.
        write_layer(
            &reg,
            SettingsScope::Global,
            &["extensionSettings", "permission", "root"],
            json!("relative"),
        );
        let resolved = reg.resolved("permission", SettingsLayer::Effective);
        assert!(resolved.values.get("root").is_none());
        assert_eq!(resolved.diagnostics[0].message, "must match ^/");
        // Core checks patterns on writes too.
        assert!(matches!(
            reg.set(
                "permission",
                "root",
                Some(json!("relative")),
                SettingsScope::Global
            ),
            Err(ExtensionSettingsError::InvalidValue { .. })
        ));
        reg.set(
            "permission",
            "mode",
            Some(json!("auto")),
            SettingsScope::Global,
        )
        .unwrap();
        assert_eq!(
            reg.values("permission", SettingsLayer::Effective)["mode"],
            "auto"
        );
    }

    #[test]
    fn v1_namespaces_resolve_like_the_plain_merge() {
        let dir = tempfile::tempdir().unwrap();
        let reg = registry(dir.path());
        // The v1 §6.6 shape the v1 kit's `settings::schema` emits, with every v1 field kind.
        let v1 = json!({
            "namespace": "notify", "title": "Notify", "extensionId": "acme.notify",
            "defaults": {"events": {"agentEnd": true}, "platform": "native"},
            "sections": [{"id": "events", "title": "Events", "fields": [
                {"key": "events.agentEnd", "type": "boolean", "label": "Agent"},
                {"key": "platform", "type": "enum", "label": "Platform",
                 "options": [{"value": "native", "label": "Native"}, "ntfy"]},
                {"key": "level", "type": "number", "label": "Level", "min": 1.0, "max": 9.0},
                {"key": "token", "type": "secret", "label": "Token", "scope": "global"},
                {"key": "columns", "type": "order", "label": "Columns",
                 "items": [{"value": "a", "label": "A"}]},
                {"key": "model", "type": "model", "label": "Model"},
                {"key": "name", "type": "string", "label": "Name", "multiline": false},
                {"key": "tags", "type": "multiselect", "label": "Tags",
                 "options": [{"value": "x", "label": "X"}]},
                {"key": "run", "type": "action", "label": "Run", "command": "run"},
                {"key": "adv", "type": "page", "label": "Adv", "sections": [
                    {"id": "a", "title": "A", "fields": [
                        {"key": "adv.timeout", "type": "number", "label": "Timeout"}
                    ]}
                ]}
            ]}]
        });
        reg.register("acme.notify", v1.clone()).unwrap();
        assert_eq!(reg.namespace("notify").unwrap()["sections"], v1["sections"]);
        let values = reg.values("notify", SettingsLayer::Effective);
        assert_eq!(values["events"]["agentEnd"], true);
        assert_eq!(values["platform"], "native");

        // Every v1 field writes and reads back (no OSS-2 syntax required).
        for (key, value) in [
            ("events.agentEnd", json!(false)),
            ("platform", json!("ntfy")),
            ("level", json!(7)),
            ("columns", json!(["a"])),
            ("model", json!("openai/gpt-5")),
            ("name", json!("board")),
            ("tags", json!(["x"])),
            ("adv.timeout", json!(30)),
        ] {
            reg.set("notify", key, Some(value.clone()), SettingsScope::Global)
                .unwrap();
            let values = reg.values("notify", SettingsLayer::Effective);
            let mut read = Some(&values);
            for segment in key.split('.') {
                read = read.and_then(|cursor| cursor.get(segment));
            }
            assert_eq!(read, Some(&value), "{key}");
        }
        // The invalid `level` write is rejected by the number bounds.
        assert!(matches!(
            reg.set("notify", "level", Some(json!(99)), SettingsScope::Global),
            Err(ExtensionSettingsError::InvalidValue { .. })
        ));
    }
}
