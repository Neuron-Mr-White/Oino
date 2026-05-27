#![forbid(unsafe_code)]

use crate::notify::NotifySettings;
use oino_extension_core::ExtensionPolicySettings;
use oino_tui::{ChatStyle, CollapseMode, KeymapConfig, ThemeSettings};
use oino_types::{CompactMethod, ThinkingLevel};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};
use tokio::fs;

/// Compaction settings controlling auto-compaction behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactSettings {
    /// Auto-compact when context exceeds this percentage of the model's context window.
    /// `None` disables auto-compaction.
    pub threshold_pct: Option<u8>,
    /// Compaction method: `vcc` (deterministic) or `llm` (LLM-based summarization).
    pub method: CompactMethod,
    /// Whether auto-compaction is enabled.
    pub auto: bool,
    /// Model for LLM compaction. `None` or `"inherit"` means use the main chat model.
    pub model: Option<String>,
    /// Path to a custom LLM compaction prompt file. `None` uses the built-in default.
    pub prompt: Option<String>,
}

impl Default for CompactSettings {
    fn default() -> Self {
        Self {
            threshold_pct: Some(80),
            method: CompactMethod::Vcc,
            auto: true,
            model: None,
            prompt: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettings {
    pub model: Option<String>,
    pub thinking_level: Option<ThinkingLevel>,
    pub thinking_collapse_mode: Option<CollapseMode>,
    pub tool_collapse_mode: Option<CollapseMode>,
    pub chat_style: Option<ChatStyle>,
    pub keymap: Option<KeymapConfig>,
    pub theme: ThemeSettings,
    pub notify: NotifySettings,
    pub btw_model: Option<String>,
    pub tools: BTreeMap<String, bool>,
    pub extensions: ExtensionPolicySettings,
    pub compact: CompactSettings,
}

impl UserSettings {
    #[cfg(test)]
    #[must_use]
    pub fn from_current(
        model: impl Into<String>,
        thinking_level: ThinkingLevel,
        thinking_collapse_mode: CollapseMode,
        tool_collapse_mode: CollapseMode,
        chat_style: ChatStyle,
    ) -> Self {
        Self {
            model: Some(model.into()),
            thinking_level: Some(thinking_level),
            thinking_collapse_mode: Some(thinking_collapse_mode),
            tool_collapse_mode: Some(tool_collapse_mode),
            chat_style: Some(chat_style),
            keymap: None,
            theme: ThemeSettings::default(),
            notify: NotifySettings::default(),
            btw_model: None,
            tools: BTreeMap::new(),
            extensions: ExtensionPolicySettings::default(),
            compact: CompactSettings::default(),
        }
    }

    pub fn apply_current(
        &mut self,
        model: impl Into<String>,
        thinking_level: ThinkingLevel,
        thinking_collapse_mode: CollapseMode,
        tool_collapse_mode: CollapseMode,
        chat_style: ChatStyle,
    ) {
        self.model = Some(model.into());
        self.thinking_level = Some(thinking_level);
        self.thinking_collapse_mode = Some(thinking_collapse_mode);
        self.tool_collapse_mode = Some(tool_collapse_mode);
        self.chat_style = Some(chat_style);
    }

    pub async fn load_default() -> io::Result<Self> {
        load_from_path(&settings_path()?).await
    }

    pub async fn save_default(&self) -> io::Result<()> {
        save_to_path(self, &settings_path()?).await
    }
}

pub async fn load_from_path(path: &Path) -> io::Result<UserSettings> {
    match fs::read_to_string(path).await {
        Ok(text) => serde_json::from_str::<UserSettings>(&text)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(UserSettings::default()),
        Err(err) => Err(err),
    }
}

pub async fn save_to_path(settings: &UserSettings, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    fs::write(path, text).await
}

fn settings_path() -> io::Result<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "home directory unavailable",
        ));
    };
    Ok(home.join(".oino").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_settings_default() {
        let settings = CompactSettings::default();
        assert_eq!(settings.threshold_pct, Some(80));
        assert_eq!(settings.method, CompactMethod::Vcc);
        assert!(settings.auto);
        assert_eq!(settings.model, None);
        assert_eq!(settings.prompt, None);
    }

    #[tokio::test]
    async fn settings_round_trip_to_json_file() {
        let path =
            std::env::temp_dir().join(format!("oino-user-settings-{}.json", std::process::id()));
        let _ = fs::remove_file(&path).await;
        let settings = UserSettings::from_current(
            "anthropic/claude-3.5-sonnet",
            ThinkingLevel::High,
            CollapseMode::Truncate,
            CollapseMode::Collapse,
            ChatStyle::Agentic,
        );
        if let Err(err) = save_to_path(&settings, &path).await {
            panic!("save settings failed: {err}");
        }
        let loaded = match load_from_path(&path).await {
            Ok(settings) => settings,
            Err(err) => panic!("load settings failed: {err}"),
        };
        assert_eq!(loaded, settings);
        let _ = fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn theme_settings_round_trip_with_user_settings() {
        let path = std::env::temp_dir().join(format!(
            "oino-theme-user-settings-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path).await;
        let mut settings = UserSettings::default();
        settings.theme.set_active("Oino Aurora");
        settings
            .theme
            .overrides
            .insert("app.bg".into(), "#08111f".into());
        if let Err(err) = save_to_path(&settings, &path).await {
            panic!("save settings failed: {err}");
        }
        let loaded = match load_from_path(&path).await {
            Ok(settings) => settings,
            Err(err) => panic!("load settings failed: {err}"),
        };
        assert_eq!(loaded.theme.active.as_deref(), Some("oino-aurora"));
        assert_eq!(
            loaded.theme.overrides.get("app.bg").map(String::as_str),
            Some("#08111f")
        );
        let _ = fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn extension_policy_settings_round_trip_with_user_settings() {
        let path = std::env::temp_dir().join(format!(
            "oino-extension-user-settings-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path).await;
        let extension_id = match oino_extension_core::ExtensionId::new("acme.process") {
            Ok(extension_id) => extension_id,
            Err(err) => panic!("extension id should be valid: {err}"),
        };
        let mut settings = UserSettings::default();
        settings.extensions.extensions.insert(
            extension_id.clone(),
            oino_extension_core::PolicyToggle::Enabled,
        );
        if let Err(err) = save_to_path(&settings, &path).await {
            panic!("save settings failed: {err}");
        }
        let loaded = match load_from_path(&path).await {
            Ok(settings) => settings,
            Err(err) => panic!("load settings failed: {err}"),
        };
        assert_eq!(
            loaded.extensions.extensions.get(&extension_id),
            Some(&oino_extension_core::PolicyToggle::Enabled)
        );
        let _ = fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn missing_settings_file_loads_defaults() {
        let path = std::env::temp_dir().join(format!(
            "oino-missing-user-settings-{}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path).await;
        let loaded = match load_from_path(&path).await {
            Ok(settings) => settings,
            Err(err) => panic!("load default settings failed: {err}"),
        };
        assert_eq!(loaded, UserSettings::default());
    }
}
