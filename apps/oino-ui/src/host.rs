//! [`SessionHost`]: what the views need from the app shell.
//!
//! The desktop shell implements it over an in-process `oino-core` (bundled providers and
//! unipi extensions); a mobile shell would implement it over `oino serve`. Futures are
//! `'static` and run on the shell's tokio runtime (`rt::Tokio`), never on the GPUI thread.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use oino_client::SessionBackend;
use serde::{Deserialize, Serialize};

use crate::model::sessions::SessionSummary;

/// Host failures shown to the user.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// Starting or opening a session failed.
    #[error("{0}")]
    Session(String),
    /// Listing or I/O failed.
    #[error("{0}")]
    Io(String),
}

/// Which session to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenRequest {
    /// A new session in `cwd`.
    New {
        /// Working directory.
        cwd: PathBuf,
    },
    /// An existing session file.
    Resume {
        /// Session file.
        path: PathBuf,
        /// Its working directory.
        cwd: PathBuf,
    },
}

/// An opened session.
#[derive(Clone)]
pub struct OpenedSession {
    /// Backend driving it.
    pub backend: Arc<dyn SessionBackend>,
    /// Working directory.
    pub cwd: String,
    /// Session file, when persisted.
    pub file: Option<PathBuf>,
}

impl std::fmt::Debug for OpenedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedSession")
            .field("session_id", &self.backend.session_id())
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

/// A loaded (or failed) extension, for the Extensions settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionInfo {
    /// Manifest id.
    pub id: String,
    /// Version.
    pub version: String,
    /// `provider` or `extension`.
    pub kind: String,
    /// Shipped inside the app.
    pub bundled: bool,
    /// Load error, if any.
    pub error: Option<String>,
}

/// Static facts about the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    /// Agent dir (`~/.oino`).
    pub agent_dir: PathBuf,
    /// Directory new sessions start in.
    pub default_cwd: PathBuf,
    /// App version.
    pub version: String,
    /// Backend label shown in the status bar (`in-process`, `demo`, `remote`).
    pub backend: String,
}

/// Theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    /// Light.
    Light,
    /// Dark.
    #[default]
    Dark,
}

/// Desktop-only preferences (`~/.oino/desktop.json`); agent settings stay in
/// `settings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Theme.
    pub theme: ThemePref,
    /// Collapse thinking blocks (pi `hideThinkingBlock`).
    pub hide_thinking: bool,
    /// Dense rows.
    pub compact: bool,
    /// Expand tool rows by default.
    pub expand_tools: bool,
    /// Recently used working directories (most recent first).
    pub recent_cwds: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemePref::Dark,
            hide_thinking: true,
            compact: false,
            expand_tools: false,
            recent_cwds: Vec::new(),
        }
    }
}

impl AppSettings {
    /// Moves `cwd` to the front of the recent list (max 10).
    pub fn touch_cwd(&mut self, cwd: &str) {
        self.recent_cwds.retain(|c| c != cwd);
        self.recent_cwds.insert(0, cwd.to_owned());
        self.recent_cwds.truncate(10);
    }
}

/// The app shell's services.
pub trait SessionHost: Send + Sync + 'static {
    /// Static facts.
    fn info(&self) -> HostInfo;

    /// Every persisted session (all workspaces).
    fn list_sessions(&self) -> BoxFuture<'static, Result<Vec<SessionSummary>, HostError>>;

    /// Opens or creates a session.
    fn open(&self, request: OpenRequest) -> BoxFuture<'static, Result<OpenedSession, HostError>>;

    /// Loaded extensions.
    fn extensions(&self) -> BoxFuture<'static, Vec<ExtensionInfo>>;

    /// Desktop preferences.
    fn app_settings(&self) -> AppSettings;

    /// Persists desktop preferences.
    fn save_app_settings(&self, settings: &AppSettings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_recent_cwds() {
        let parsed: AppSettings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(parsed.theme, ThemePref::Light);
        assert!(parsed.hide_thinking);
        let mut s = AppSettings::default();
        for i in 0..12 {
            s.touch_cwd(&format!("/p{i}"));
        }
        s.touch_cwd("/p5");
        assert_eq!(s.recent_cwds.len(), 10);
        assert_eq!(s.recent_cwds[0], "/p5");
    }
}
