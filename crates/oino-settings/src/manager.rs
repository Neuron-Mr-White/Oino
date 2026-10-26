//! [`SettingsManager`] (pi `SettingsManager`): layered load, trust gating, scoped writes.

use std::path::{Path, PathBuf};

use oino_ai::{ThinkingLevel, Transport};
use serde_json::Value;

use crate::error::{SettingsError, SettingsErrorKind, SettingsScope};
use crate::json::{JsonObject, get_path, migrate_settings, set_path, strip_bom};
use crate::paths::{SETTINGS_FILE, home_dir, lexical_normalize, project_dir};
use crate::storage::Storage;
use crate::types::{
    CacheWarmingMode, PackageSource, ProjectTrustDefault, QueueMode, ResourceKind, UiDensity,
};
use crate::view::Settings;

/// Global + project settings with pi merge, migration and write semantics.
///
/// - Reads: [`Self::settings`] gives the typed [`Settings`] view (project overrides global,
///   nested objects deep-merged, `defaultTools` modifiers appended).
/// - Writes go to exactly one [`SettingsScope`]. Each write re-reads the file under an exclusive
///   lock on `<file>.lock`, migrates it, replaces only the written key path (unknown keys and
///   concurrent edits to other keys survive) and replaces the file atomically.
/// - A layer whose file failed to load is never written (pi parity: a broken file is not
///   clobbered); the in-memory change still applies.
/// - Load errors are collected and returned by [`Self::drain_errors`]; write errors are returned
///   directly.
#[derive(Debug)]
pub struct SettingsManager {
    storage: Storage,
    view: Settings,
    project_trusted: bool,
    global_load_failed: bool,
    project_load_failed: bool,
    errors: Vec<SettingsError>,
}

impl SettingsManager {
    /// Load `<agent_dir>/settings.json` and, when `project_trusted`, `<cwd>/.oino/settings.json`
    /// (pi `SettingsManager.create`). Relative inputs resolve against the process cwd.
    #[must_use]
    pub fn create(cwd: &Path, agent_dir: &Path, project_trusted: bool) -> Self {
        let absolute = |path: &Path| {
            std::path::absolute(path)
                .map_or_else(|_| path.to_path_buf(), |path| lexical_normalize(&path))
        };
        let storage = Storage::Files {
            global: absolute(agent_dir).join(SETTINGS_FILE),
            project: project_dir(&absolute(cwd)).join(SETTINGS_FILE),
        };
        Self::from_storage(storage, project_trusted)
    }

    /// An in-memory manager seeded with `initial` as the global layer (pi `inMemory`); the
    /// project is trusted (pi default) and starts empty.
    #[must_use]
    pub fn in_memory(initial: JsonObject) -> Self {
        let mut initial = initial;
        migrate_settings(&mut initial);
        let content = serde_json::to_string_pretty(&Value::Object(initial)).unwrap_or_default();
        Self::from_storage(
            Storage::Memory {
                global: Some(content),
                project: None,
            },
            true,
        )
    }

    fn from_storage(storage: Storage, project_trusted: bool) -> Self {
        let mut errors = Vec::new();
        let (global, global_load_failed) = match load_layer(&storage, SettingsScope::Global) {
            Ok(global) => (global, false),
            Err(kind) => {
                errors.push(error(&storage, SettingsScope::Global, kind));
                (JsonObject::new(), true)
            }
        };
        let (project, project_load_failed) = if project_trusted {
            match load_layer(&storage, SettingsScope::Project) {
                Ok(project) => (project, false),
                Err(kind) => {
                    errors.push(error(&storage, SettingsScope::Project, kind));
                    (JsonObject::new(), true)
                }
            }
        } else {
            (JsonObject::new(), false)
        };
        Self {
            storage,
            view: Settings::from_layers(global, project, JsonObject::new(), home_dir()),
            project_trusted,
            global_load_failed,
            project_load_failed,
            errors,
        }
    }

    /// The typed view of the current layers.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.view
    }

    /// An owned snapshot (for blocking tasks or other threads).
    #[must_use]
    pub fn snapshot(&self) -> Settings {
        self.view.clone()
    }

    /// File path of a layer (`None` for in-memory storage).
    #[must_use]
    pub fn path(&self, scope: SettingsScope) -> Option<&Path> {
        self.storage.path(scope)
    }

    /// Whether project settings are loaded and writable.
    #[must_use]
    pub fn is_project_trusted(&self) -> bool {
        self.project_trusted
    }

    /// Change the trust state: untrusted clears the project layer, trusted loads it
    /// (pi `setProjectTrusted`).
    pub fn set_project_trusted(&mut self, trusted: bool) {
        if self.project_trusted == trusted {
            return;
        }
        self.project_trusted = trusted;
        let project = if trusted {
            match load_layer(&self.storage, SettingsScope::Project) {
                Ok(project) => {
                    self.project_load_failed = false;
                    project
                }
                Err(kind) => {
                    self.project_load_failed = true;
                    self.record(SettingsScope::Project, kind);
                    JsonObject::new()
                }
            }
        } else {
            self.project_load_failed = false;
            JsonObject::new()
        };
        self.rebuild(None, Some(project));
    }

    /// Re-read both layers for the current trust state; a layer that fails to load keeps its
    /// previous content and records an error (pi `reload`).
    pub fn reload(&mut self) {
        let global = match load_layer(&self.storage, SettingsScope::Global) {
            Ok(global) => {
                self.global_load_failed = false;
                Some(global)
            }
            Err(kind) => {
                self.global_load_failed = true;
                self.record(SettingsScope::Global, kind);
                None
            }
        };
        let project = if self.project_trusted {
            match load_layer(&self.storage, SettingsScope::Project) {
                Ok(project) => {
                    self.project_load_failed = false;
                    Some(project)
                }
                Err(kind) => {
                    self.project_load_failed = true;
                    self.record(SettingsScope::Project, kind);
                    None
                }
            }
        } else {
            Some(JsonObject::new())
        };
        self.rebuild(global, project);
    }

    /// Layer session-only overrides on top of the merged settings (pi `applyOverrides`; kept
    /// across later writes and reloads).
    pub fn apply_overrides(&mut self, overrides: &JsonObject) {
        let merged = crate::json::deep_merge_settings(self.view.overrides(), overrides);
        self.view = Settings::from_layers(
            self.view.global().clone(),
            self.view.project().clone(),
            merged,
            self.view.home().map(Path::to_path_buf),
        );
    }

    /// Take the collected load diagnostics (pi `drainErrors`).
    pub fn drain_errors(&mut self) -> Vec<SettingsError> {
        std::mem::take(&mut self.errors)
    }

    /// Set (`Some`) or delete (`None`) the value at `path` in one scope and persist that path.
    ///
    /// # Errors
    ///
    /// [`SettingsErrorKind::ProjectUntrusted`] for project writes while untrusted; lock, read,
    /// parse or write failures of the target file.
    pub fn set(
        &mut self,
        scope: SettingsScope,
        path: &[&str],
        value: Option<Value>,
    ) -> Result<(), SettingsError> {
        self.set_many(scope, &[(path, value)])
    }

    /// Several [`Self::set`] edits applied and persisted in one locked write.
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_many(
        &mut self,
        scope: SettingsScope,
        edits: &[(&[&str], Option<Value>)],
    ) -> Result<(), SettingsError> {
        if scope == SettingsScope::Project && !self.project_trusted {
            return Err(self.error(scope, SettingsErrorKind::ProjectUntrusted));
        }
        let mut layer = self.view.layer(scope).clone();
        for (path, value) in edits {
            set_path(&mut layer, path, value.clone());
        }
        match scope {
            SettingsScope::Global => self.rebuild(Some(layer.clone()), None),
            SettingsScope::Project => self.rebuild(None, Some(layer.clone())),
        }
        let load_failed = match scope {
            SettingsScope::Global => self.global_load_failed,
            SettingsScope::Project => self.project_load_failed,
        };
        if load_failed {
            return Ok(());
        }
        self.storage
            .update(scope, |current| {
                let mut file = match current {
                    Some(content) if !content.is_empty() => parse_layer(content)?,
                    _ => JsonObject::new(),
                };
                for (path, _) in edits {
                    set_path(&mut file, path, get_path(&layer, path).cloned());
                }
                serde_json::to_string_pretty(&Value::Object(file))
                    .map_err(|e| SettingsErrorKind::Write(e.to_string()))
            })
            .map_err(|kind| self.error(scope, kind))
    }

    /// `defaultProvider` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_default_provider(&mut self, provider: &str) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["defaultProvider"],
            Some(provider.into()),
        )
    }

    /// `defaultModel` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_default_model(&mut self, model_id: &str) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["defaultModel"],
            Some(model_id.into()),
        )
    }

    /// `defaultProvider` + `defaultModel` in one write (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_default_model_and_provider(
        &mut self,
        provider: &str,
        model_id: &str,
    ) -> Result<(), SettingsError> {
        self.set_many(
            SettingsScope::Global,
            &[
                (&["defaultProvider"], Some(provider.into())),
                (&["defaultModel"], Some(model_id.into())),
            ],
        )
    }

    /// `defaultThinkingLevel` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_default_thinking_level(
        &mut self,
        level: ThinkingLevel,
    ) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["defaultThinkingLevel"],
            Some(to_value(level)),
        )
    }

    /// `modelThinkingLevels["provider/modelId"] = level` (global; whole map rewritten like pi).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_model_thinking_level(
        &mut self,
        provider: &str,
        model_id: &str,
        level: ThinkingLevel,
    ) -> Result<(), SettingsError> {
        let mut levels = self.global_object("modelThinkingLevels");
        levels.insert(format!("{provider}/{model_id}"), to_value(level));
        self.set(
            SettingsScope::Global,
            &["modelThinkingLevels"],
            Some(Value::Object(levels)),
        )
    }

    /// Remove one `modelThinkingLevels` entry; the map is deleted when it becomes empty.
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn remove_model_thinking_level(
        &mut self,
        provider: &str,
        model_id: &str,
    ) -> Result<(), SettingsError> {
        if !self.view.global().contains_key("modelThinkingLevels") {
            return Ok(());
        }
        let mut levels = self.global_object("modelThinkingLevels");
        levels.shift_remove(&format!("{provider}/{model_id}"));
        let value = (!levels.is_empty()).then_some(Value::Object(levels));
        self.set(SettingsScope::Global, &["modelThinkingLevels"], value)
    }

    /// `steeringMode` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_steering_mode(&mut self, mode: QueueMode) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["steeringMode"],
            Some(to_value(mode)),
        )
    }

    /// `followUpMode` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_follow_up_mode(&mut self, mode: QueueMode) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["followUpMode"],
            Some(to_value(mode)),
        )
    }

    /// `transport` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_transport(&mut self, transport: Transport) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["transport"],
            Some(to_value(transport)),
        )
    }

    /// `compaction.enabled` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_compaction_enabled(&mut self, enabled: bool) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["compaction", "enabled"],
            Some(enabled.into()),
        )
    }

    /// `retry.enabled` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_retry_enabled(&mut self, enabled: bool) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["retry", "enabled"],
            Some(enabled.into()),
        )
    }

    /// `httpIdleTimeoutMs` (global; `0` disables).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_http_idle_timeout_ms(&mut self, timeout_ms: u64) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["httpIdleTimeoutMs"],
            Some(timeout_ms.into()),
        )
    }

    /// `cacheWarming` (global only).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_cache_warming_mode(&mut self, mode: CacheWarmingMode) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["cacheWarming"],
            Some(to_value(mode)),
        )
    }

    /// `showCacheMissNotices` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_show_cache_miss_notices(&mut self, show: bool) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["showCacheMissNotices"],
            Some(show.into()),
        )
    }

    /// `shellPath` (global; `None` deletes).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_shell_path(&mut self, path: Option<&str>) -> Result<(), SettingsError> {
        self.set(SettingsScope::Global, &["shellPath"], path.map(Value::from))
    }

    /// `shellCommandPrefix` (global; `None` deletes).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_shell_command_prefix(&mut self, prefix: Option<&str>) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["shellCommandPrefix"],
            prefix.map(Value::from),
        )
    }

    /// `defaultProjectTrust` (global only).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_default_project_trust(
        &mut self,
        trust: ProjectTrustDefault,
    ) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["defaultProjectTrust"],
            Some(to_value(trust)),
        )
    }

    /// `enableSkillCommands` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_enable_skill_commands(&mut self, enabled: bool) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["enableSkillCommands"],
            Some(enabled.into()),
        )
    }

    /// `enabledModels` (global; `None` deletes).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_enabled_models(&mut self, patterns: Option<&[String]>) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["enabledModels"],
            patterns.map(Into::into),
        )
    }

    /// `images.autoResize` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_image_auto_resize(&mut self, enabled: bool) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["images", "autoResize"],
            Some(enabled.into()),
        )
    }

    /// `images.blockImages` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_block_images(&mut self, blocked: bool) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["images", "blockImages"],
            Some(blocked.into()),
        )
    }

    /// `packages` of one scope.
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_packages(
        &mut self,
        scope: SettingsScope,
        packages: &[PackageSource],
    ) -> Result<(), SettingsError> {
        let value = serde_json::to_value(packages)
            .map_err(|e| self.error(scope, SettingsErrorKind::Write(e.to_string())))?;
        self.set(scope, &["packages"], Some(value))
    }

    /// A resource array (`extensions`, `skills`, `prompts`, `themes`) of one scope.
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_resource_paths(
        &mut self,
        scope: SettingsScope,
        kind: ResourceKind,
        paths: &[String],
    ) -> Result<(), SettingsError> {
        self.set(scope, &[kind.key()], Some(paths.into()))
    }

    /// `deviceId`, generated (UUID v4) and persisted globally on first use (pi
    /// `getOrCreateDeviceId`; project settings are ignored).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn get_or_create_device_id(&mut self) -> Result<String, SettingsError> {
        if let Some(id) = self.view.device_id() {
            return Ok(id.to_owned());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.set(
            SettingsScope::Global,
            &["deviceId"],
            Some(id.clone().into()),
        )?;
        Ok(id)
    }

    /// One field of `extensionSettings.<namespace>` in one scope; `None` deletes it (a deleted
    /// project field falls back to global/defaults: "use default").
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_extension_setting(
        &mut self,
        scope: SettingsScope,
        namespace: &str,
        key: &str,
        value: Option<Value>,
    ) -> Result<(), SettingsError> {
        self.set(scope, &["extensionSettings", namespace, key], value)
    }

    /// The whole `extensionSettings.<namespace>` object of one scope; `None` deletes it.
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_extension_settings(
        &mut self,
        scope: SettingsScope,
        namespace: &str,
        value: Option<Value>,
    ) -> Result<(), SettingsError> {
        self.set(scope, &["extensionSettings", namespace], value)
    }

    /// `disabledExtensions` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_disabled_extensions(&mut self, ids: &[String]) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["disabledExtensions"],
            Some(ids.into()),
        )
    }

    /// `ui.density` (global).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_ui_density(&mut self, density: UiDensity) -> Result<(), SettingsError> {
        self.set(
            SettingsScope::Global,
            &["ui", "density"],
            Some(to_value(density)),
        )
    }

    /// An untyped frontend key (global; `None` deletes).
    ///
    /// # Errors
    ///
    /// See [`Self::set`].
    pub fn set_frontend(&mut self, key: &str, value: Option<Value>) -> Result<(), SettingsError> {
        self.set(SettingsScope::Global, &[key], value)
    }

    fn global_object(&self, key: &str) -> JsonObject {
        self.view
            .global()
            .get(key)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    fn rebuild(&mut self, global: Option<JsonObject>, project: Option<JsonObject>) {
        let global = global.unwrap_or_else(|| self.view.global().clone());
        let project = project.unwrap_or_else(|| self.view.project().clone());
        self.view = Settings::from_layers(
            global,
            project,
            self.view.overrides().clone(),
            self.view.home().map(Path::to_path_buf),
        );
    }

    fn error(&self, scope: SettingsScope, kind: SettingsErrorKind) -> SettingsError {
        error(&self.storage, scope, kind)
    }

    fn record(&mut self, scope: SettingsScope, kind: SettingsErrorKind) {
        let error = self.error(scope, kind);
        self.errors.push(error);
    }
}

fn error(storage: &Storage, scope: SettingsScope, kind: SettingsErrorKind) -> SettingsError {
    SettingsError {
        scope,
        path: storage.path(scope).map(PathBuf::from),
        kind,
    }
}

fn load_layer(storage: &Storage, scope: SettingsScope) -> Result<JsonObject, SettingsErrorKind> {
    match storage.read(scope)? {
        Some(content) if !content.is_empty() => parse_layer(&content),
        _ => Ok(JsonObject::new()),
    }
}

/// Parse (BOM stripped) and migrate a settings file.
fn parse_layer(content: &str) -> Result<JsonObject, SettingsErrorKind> {
    match serde_json::from_str(strip_bom(content)) {
        Ok(Value::Object(mut object)) => {
            migrate_settings(&mut object);
            Ok(object)
        }
        Ok(_) => Err(SettingsErrorKind::NotAnObject),
        Err(error) => Err(SettingsErrorKind::Parse(error.to_string())),
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
