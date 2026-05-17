#![forbid(unsafe_code)]

use oino_tui::{ChatStyle, CollapseMode};
use oino_types::ThinkingLevel;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::fs;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettings {
    pub model: Option<String>,
    pub thinking_level: Option<ThinkingLevel>,
    pub thinking_collapse_mode: Option<CollapseMode>,
    pub tool_collapse_mode: Option<CollapseMode>,
    pub chat_style: Option<ChatStyle>,
}

impl UserSettings {
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
        }
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
