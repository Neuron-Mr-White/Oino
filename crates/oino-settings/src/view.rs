//! [`Settings`]: an immutable, typed view over the global, project and merged layers
//! (pi `SettingsManager` getters).

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use oino_ai::{ThinkingBudgets, ThinkingLevel, Transport};
use serde_json::Value;

use crate::error::{InvalidSettingError, SettingsScope};
use crate::json::{
    JsonObject, as_non_negative_u64, as_safe_non_negative_integer, deep_merge_objects,
    deep_merge_settings, get_path, js_string, string_array,
};
use crate::paths::normalize_path;
use crate::tools::resolve_default_tools;
use crate::types::{
    BranchSummarySettings, CacheWarmingMode, CompactionSettings, ImageSettings, PackageSource,
    ProjectTrustDefault, ProviderRetrySettings, QueueMode, ResourceKind, RetrySettings,
    ServeSettings, UiDensity, defaults, parse_enum,
};

/// Compaction token fields that support per-model overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionTokenField {
    /// `reserveTokens`.
    ReserveTokens,
    /// `keepRecentTokens`.
    KeepRecentTokens,
}

impl CompactionTokenField {
    const fn key(self) -> &'static str {
        match self {
            Self::ReserveTokens => "reserveTokens",
            Self::KeepRecentTokens => "keepRecentTokens",
        }
    }

    const fn default_value(self) -> u64 {
        match self {
            Self::ReserveTokens => defaults::COMPACTION_RESERVE_TOKENS,
            Self::KeepRecentTokens => defaults::COMPACTION_KEEP_RECENT_TOKENS,
        }
    }
}

/// Typed, layered settings snapshot. Cheap to clone relative to file I/O; hand it to blocking
/// tasks instead of the manager.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    global: JsonObject,
    project: JsonObject,
    overrides: JsonObject,
    merged: JsonObject,
    home: Option<PathBuf>,
}

impl Settings {
    /// Build a view from raw layers (`project` overrides `global`, then `overrides`).
    #[must_use]
    pub fn from_layers(
        global: JsonObject,
        project: JsonObject,
        overrides: JsonObject,
        home: Option<PathBuf>,
    ) -> Self {
        let merged = deep_merge_settings(&deep_merge_settings(&global, &project), &overrides);
        Self {
            global,
            project,
            overrides,
            merged,
            home,
        }
    }

    /// The effective object (pi `getSettings`).
    #[must_use]
    pub fn merged(&self) -> &JsonObject {
        &self.merged
    }

    /// The global layer (pi `getGlobalSettings`).
    #[must_use]
    pub fn global(&self) -> &JsonObject {
        &self.global
    }

    /// The project layer; empty when untrusted (pi `getProjectSettings`).
    #[must_use]
    pub fn project(&self) -> &JsonObject {
        &self.project
    }

    /// One layer.
    #[must_use]
    pub fn layer(&self, scope: SettingsScope) -> &JsonObject {
        match scope {
            SettingsScope::Global => &self.global,
            SettingsScope::Project => &self.project,
        }
    }

    pub(crate) fn overrides(&self) -> &JsonObject {
        &self.overrides
    }

    /// The effective value at a key path.
    #[must_use]
    pub fn value(&self, path: &[&str]) -> Option<&Value> {
        get_path(&self.merged, path)
    }

    /// An untyped (frontend-owned) key such as `theme` or `markdown`, effective value.
    #[must_use]
    pub fn frontend(&self, key: &str) -> Option<Value> {
        self.merged.get(key).cloned()
    }

    fn str_at(&self, path: &[&str]) -> Option<&str> {
        self.value(path).and_then(Value::as_str)
    }

    fn bool_at(&self, path: &[&str], default: bool) -> bool {
        self.value(path).and_then(Value::as_bool).unwrap_or(default)
    }

    fn u64_at(&self, path: &[&str], default: u64) -> u64 {
        self.value(path)
            .and_then(as_non_negative_u64)
            .unwrap_or(default)
    }

    fn path_at(&self, key: &str) -> Option<PathBuf> {
        self.merged
            .get(key)
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(|path| normalize_path(path, self.home.as_deref()))
    }

    /// `defaultProvider`.
    #[must_use]
    pub fn default_provider(&self) -> Option<&str> {
        self.str_at(&["defaultProvider"])
    }

    /// `defaultModel`.
    #[must_use]
    pub fn default_model(&self) -> Option<&str> {
        self.str_at(&["defaultModel"])
    }

    /// `defaultThinkingLevel` (unset → `None`; the runtime then uses
    /// [`defaults::THINKING_LEVEL`]).
    #[must_use]
    pub fn default_thinking_level(&self) -> Option<ThinkingLevel> {
        self.value(&["defaultThinkingLevel"]).and_then(parse_enum)
    }

    /// `modelThinkingLevels["provider/modelId"]`.
    #[must_use]
    pub fn model_thinking_level(&self, provider: &str, model_id: &str) -> Option<ThinkingLevel> {
        let key = format!("{provider}/{model_id}");
        self.value(&["modelThinkingLevels", &key])
            .and_then(parse_enum)
    }

    /// Every valid `modelThinkingLevels` entry (pi `getAllModelThinkingLevels`).
    #[must_use]
    pub fn model_thinking_levels(&self) -> IndexMap<String, ThinkingLevel> {
        self.value(&["modelThinkingLevels"])
            .and_then(Value::as_object)
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|(key, level)| parse_enum(level).map(|level| (key.clone(), level)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `thinkingBudgets`.
    #[must_use]
    pub fn thinking_budgets(&self) -> Option<ThinkingBudgets> {
        let budgets = self.value(&["thinkingBudgets"])?.as_object()?;
        let field = |key: &str| budgets.get(key).and_then(as_non_negative_u64);
        Some(ThinkingBudgets {
            minimal: field("minimal"),
            low: field("low"),
            medium: field("medium"),
            high: field("high"),
        })
    }

    /// `enabledModels` patterns.
    #[must_use]
    pub fn enabled_models(&self) -> Option<Vec<String>> {
        self.value(&["enabledModels"]).and_then(string_array)
    }

    /// `steeringMode` (default `one-at-a-time`).
    #[must_use]
    pub fn steering_mode(&self) -> QueueMode {
        self.value(&["steeringMode"])
            .and_then(parse_enum)
            .unwrap_or_default()
    }

    /// `followUpMode` (default `one-at-a-time`).
    #[must_use]
    pub fn follow_up_mode(&self) -> QueueMode {
        self.value(&["followUpMode"])
            .and_then(parse_enum)
            .unwrap_or_default()
    }

    /// Resolved `defaultTools` selection; `None` when no layer sets it (pi `getDefaultTools`).
    #[must_use]
    pub fn default_tools(&self) -> Option<Vec<String>> {
        let tools = self.merged.get("defaultTools")?;
        Some(resolve_default_tools(
            &string_array(tools).unwrap_or_default(),
        ))
    }

    /// `sessionDir`, tilde-expanded (relative paths stay relative to the caller's cwd).
    #[must_use]
    pub fn session_dir(&self) -> Option<PathBuf> {
        self.path_at("sessionDir")
    }

    /// `compaction.enabled` (default `true`).
    #[must_use]
    pub fn compaction_enabled(&self) -> bool {
        self.bool_at(&["compaction", "enabled"], defaults::COMPACTION_ENABLED)
    }

    /// One compaction token setting: model override, then ordinary value, then default
    /// (pi `getCompactionTokenSetting`).
    ///
    /// # Errors
    ///
    /// pi's validation errors for non-integer values or non-object model overrides.
    pub fn compaction_token(
        &self,
        field: CompactionTokenField,
        model: Option<(&str, &str)>,
    ) -> Result<u64, InvalidSettingError> {
        let key = field.key();
        let ordinary = self.value(&["compaction", key]);
        let ordinary = match ordinary {
            None => None,
            Some(value) => Some(as_safe_non_negative_integer(value).ok_or_else(|| {
                InvalidSettingError::new(format!(
                    "Invalid compaction.{key} setting: {}. Expected a non-negative safe integer.",
                    js_string(value)
                ))
            })?),
        };
        let Some((provider, id)) = model else {
            return Ok(ordinary.unwrap_or(field.default_value()));
        };
        let model_key = format!("{provider}/{id}");
        let entry = self.value(&["compaction", "modelOverrides", &model_key]);
        let override_value = match entry {
            None => None,
            Some(Value::Object(entry)) => match entry.get(key) {
                None => None,
                Some(value) => Some(as_safe_non_negative_integer(value).ok_or_else(|| {
                    InvalidSettingError::new(format!(
                        "Invalid compaction.modelOverrides[\"{model_key}\"].{key} setting: {}. Expected a non-negative safe integer.",
                        js_string(value)
                    ))
                })?),
            },
            Some(other) => {
                return Err(InvalidSettingError::new(format!(
                    "Invalid compaction.modelOverrides[\"{model_key}\"] setting: {}. Expected an object.",
                    js_string(other)
                )));
            }
        };
        Ok(override_value.or(ordinary).unwrap_or(field.default_value()))
    }

    /// Resolved compaction settings for an optional `(provider, modelId)`.
    ///
    /// # Errors
    ///
    /// See [`Self::compaction_token`].
    pub fn compaction_settings(
        &self,
        model: Option<(&str, &str)>,
    ) -> Result<CompactionSettings, InvalidSettingError> {
        Ok(CompactionSettings {
            enabled: self.compaction_enabled(),
            reserve_tokens: self.compaction_token(CompactionTokenField::ReserveTokens, model)?,
            keep_recent_tokens: self
                .compaction_token(CompactionTokenField::KeepRecentTokens, model)?,
        })
    }

    /// `branchSummary.*`.
    #[must_use]
    pub fn branch_summary_settings(&self) -> BranchSummarySettings {
        BranchSummarySettings {
            reserve_tokens: self.u64_at(
                &["branchSummary", "reserveTokens"],
                defaults::BRANCH_SUMMARY_RESERVE_TOKENS,
            ),
            skip_prompt: self.bool_at(
                &["branchSummary", "skipPrompt"],
                defaults::BRANCH_SUMMARY_SKIP_PROMPT,
            ),
        }
    }

    /// `retry.enabled` (default `true`).
    #[must_use]
    pub fn retry_enabled(&self) -> bool {
        self.bool_at(&["retry", "enabled"], defaults::RETRY_ENABLED)
    }

    /// `retry.{enabled, maxRetries, baseDelayMs, maxAgentDelayMs}`.
    #[must_use]
    pub fn retry_settings(&self) -> RetrySettings {
        RetrySettings {
            enabled: self.retry_enabled(),
            max_retries: self.u64_at(&["retry", "maxRetries"], defaults::RETRY_MAX_RETRIES),
            base_delay_ms: self.u64_at(&["retry", "baseDelayMs"], defaults::RETRY_BASE_DELAY_MS),
            max_agent_delay_ms: self.u64_at(
                &["retry", "maxAgentDelayMs"],
                defaults::RETRY_MAX_AGENT_DELAY_MS,
            ),
        }
    }

    /// `retry.provider.*`.
    #[must_use]
    pub fn provider_retry_settings(&self) -> ProviderRetrySettings {
        let field = |key: &str| {
            self.value(&["retry", "provider", key])
                .and_then(as_non_negative_u64)
        };
        ProviderRetrySettings {
            timeout_ms: field("timeoutMs"),
            max_retries: field("maxRetries"),
            max_retry_delay_ms: field("maxRetryDelayMs")
                .unwrap_or(defaults::RETRY_PROVIDER_MAX_RETRY_DELAY_MS),
        }
    }

    /// `transport` (default `auto`).
    #[must_use]
    pub fn transport(&self) -> Transport {
        self.value(&["transport"])
            .and_then(parse_enum)
            .unwrap_or(Transport::Auto)
    }

    /// `httpProxy`, global layer only (pi reads `getGlobalSettings().httpProxy`).
    #[must_use]
    pub fn http_proxy(&self) -> Option<&str> {
        self.global.get("httpProxy").and_then(Value::as_str)
    }

    /// `httpIdleTimeoutMs` (`0` = disabled; default 300000).
    ///
    /// # Errors
    ///
    /// `Invalid httpIdleTimeoutMs setting: …` for unparsable values.
    pub fn http_idle_timeout_ms(&self) -> Result<u64, InvalidSettingError> {
        Ok(
            parse_timeout_setting(self.merged.get("httpIdleTimeoutMs"), "httpIdleTimeoutMs")?
                .unwrap_or(defaults::HTTP_IDLE_TIMEOUT_MS),
        )
    }

    /// `websocketConnectTimeoutMs` (`None` = provider default).
    ///
    /// # Errors
    ///
    /// `Invalid websocketConnectTimeoutMs setting: …` for unparsable values.
    pub fn websocket_connect_timeout_ms(&self) -> Result<Option<u64>, InvalidSettingError> {
        parse_timeout_setting(
            self.merged.get("websocketConnectTimeoutMs"),
            "websocketConnectTimeoutMs",
        )
    }

    /// `cacheWarming`, global layer only (default `streaming`).
    #[must_use]
    pub fn cache_warming_mode(&self) -> CacheWarmingMode {
        self.global
            .get("cacheWarming")
            .and_then(parse_enum)
            .unwrap_or_default()
    }

    /// `showCacheMissNotices` (default `false`).
    #[must_use]
    pub fn show_cache_miss_notices(&self) -> bool {
        self.bool_at(&["showCacheMissNotices"], defaults::SHOW_CACHE_MISS_NOTICES)
    }

    /// `shellPath`, tilde-expanded.
    #[must_use]
    pub fn shell_path(&self) -> Option<PathBuf> {
        self.path_at("shellPath")
    }

    /// `shellCommandPrefix`.
    #[must_use]
    pub fn shell_command_prefix(&self) -> Option<&str> {
        self.str_at(&["shellCommandPrefix"])
    }

    /// `images.*`.
    #[must_use]
    pub fn image_settings(&self) -> ImageSettings {
        ImageSettings {
            auto_resize: self.bool_at(&["images", "autoResize"], defaults::IMAGES_AUTO_RESIZE),
            block_images: self.bool_at(&["images", "blockImages"], defaults::IMAGES_BLOCK_IMAGES),
        }
    }

    /// `enableSkillCommands` (default `true`).
    #[must_use]
    pub fn enable_skill_commands(&self) -> bool {
        self.bool_at(&["enableSkillCommands"], defaults::ENABLE_SKILL_COMMANDS)
    }

    /// `defaultProjectTrust`, global layer only (default `ask`).
    #[must_use]
    pub fn default_project_trust(&self) -> ProjectTrustDefault {
        self.global
            .get("defaultProjectTrust")
            .and_then(parse_enum)
            .unwrap_or_default()
    }

    /// `deviceId`, global layer only.
    #[must_use]
    pub fn device_id(&self) -> Option<&str> {
        self.global.get("deviceId").and_then(Value::as_str)
    }

    /// Effective `packages` (project replaces global, pi `getPackages`).
    #[must_use]
    pub fn packages(&self) -> Vec<PackageSource> {
        parse_packages(self.merged.get("packages"))
    }

    /// `packages` of one layer (resolution combines both layers, project first).
    #[must_use]
    pub fn packages_in(&self, scope: SettingsScope) -> Vec<PackageSource> {
        parse_packages(self.layer(scope).get("packages"))
    }

    /// Effective resource array (project replaces global, pi `getSkillPaths` & co.).
    #[must_use]
    pub fn resource_paths(&self, kind: ResourceKind) -> Vec<String> {
        self.merged
            .get(kind.key())
            .and_then(string_array)
            .unwrap_or_default()
    }

    /// The raw entries of one layer's resource array. Resource resolution combines both
    /// layers: project entries resolve against `<cwd>/.oino`, global ones against the agent dir.
    #[must_use]
    pub fn resource_entries(&self, kind: ResourceKind, scope: SettingsScope) -> Vec<String> {
        self.layer(scope)
            .get(kind.key())
            .and_then(string_array)
            .unwrap_or_default()
    }

    /// `extensionSettings.<namespace>` layered as `defaults ⊕ global ⊕ project` (D11).
    #[must_use]
    pub fn extension_settings(&self, namespace: &str, defaults: Option<&Value>) -> Value {
        let layer = |object: &JsonObject| {
            get_path(object, &["extensionSettings", namespace])
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        let base = defaults
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let merged = deep_merge_objects(&base, &layer(&self.global));
        let merged = deep_merge_objects(&merged, &layer(&self.project));
        Value::Object(deep_merge_objects(&merged, &layer(&self.overrides)))
    }

    /// `disabledExtensions` (default `[]`).
    #[must_use]
    pub fn disabled_extensions(&self) -> Vec<String> {
        self.merged
            .get("disabledExtensions")
            .and_then(string_array)
            .unwrap_or_default()
    }

    /// `serve.*` (defaults `127.0.0.1:7420`, `true`, `false`).
    #[must_use]
    pub fn serve_settings(&self) -> ServeSettings {
        ServeSettings {
            bind: self
                .str_at(&["serve", "bind"])
                .unwrap_or(defaults::SERVE_BIND)
                .to_owned(),
            tls: self.bool_at(&["serve", "tls"], defaults::SERVE_TLS),
            allow_remote: self.bool_at(&["serve", "allowRemote"], defaults::SERVE_ALLOW_REMOTE),
        }
    }

    /// `ui.density` (default `regular`).
    #[must_use]
    pub fn ui_density(&self) -> UiDensity {
        self.value(&["ui", "density"])
            .and_then(parse_enum)
            .unwrap_or_default()
    }

    /// Home directory used for `~` expansion.
    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }
}

fn parse_packages(value: Option<&Value>) -> Vec<PackageSource> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// pi `parseHttpIdleTimeoutMs`: numbers ≥ 0 (floored), numeric strings, `"disabled"` → 0.
#[must_use]
pub fn parse_timeout_ms(value: &Value) -> Option<u64> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.eq_ignore_ascii_case("disabled") {
                return Some(0);
            }
            if trimmed.is_empty() {
                return None;
            }
            let number: f64 = trimmed.parse().ok()?;
            serde_json::Number::from_f64(number)
                .and_then(|n| as_non_negative_u64(&Value::Number(n)))
        }
        Value::Number(_) => as_non_negative_u64(value),
        _ => None,
    }
}

fn parse_timeout_setting(
    value: Option<&Value>,
    name: &str,
) -> Result<Option<u64>, InvalidSettingError> {
    match value {
        None => Ok(None),
        Some(value) => parse_timeout_ms(value).map(Some).ok_or_else(|| {
            InvalidSettingError::new(format!("Invalid {name} setting: {}", js_string(value)))
        }),
    }
}
