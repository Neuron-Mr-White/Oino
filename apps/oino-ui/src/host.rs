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
    /// Display name (manifest `name`; empty → the id).
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Permission summary (`network`, `process`, `secrets`…).
    pub permissions: Vec<String>,
    /// Enabled (not in `disabledExtensions`).
    pub enabled: bool,
    /// Package directory (README, Open folder).
    pub root: Option<PathBuf>,
}

impl ExtensionInfo {
    /// An info with only the required facts (the rest default: enabled, no extras).
    #[must_use]
    pub fn basic(
        id: &str,
        version: &str,
        kind: &str,
        bundled: bool,
        error: Option<String>,
    ) -> Self {
        Self {
            id: id.to_owned(),
            version: version.to_owned(),
            kind: kind.to_owned(),
            bundled,
            error,
            name: String::new(),
            description: String::new(),
            permissions: Vec::new(),
            enabled: true,
            root: None,
        }
    }

    /// The package README, when the package ships one.
    #[must_use]
    pub fn readme(&self) -> Option<String> {
        let root = self.root.as_ref()?;
        ["README.md", "readme.md", "README"]
            .iter()
            .find_map(|f| std::fs::read_to_string(root.join(f)).ok())
    }
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

/// Theme preference (design.md §2.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    /// Follow the window appearance (dark when unknown).
    #[default]
    System,
    /// Oino Light.
    Light,
    /// Oino Dark.
    Dark,
}

/// Reduce motion (design.md §2.10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReduceMotion {
    /// Follow the OS (`OINO_REDUCE_MOTION=1` forces it where GPUI has no OS signal).
    #[default]
    FollowOs,
    /// Always reduce.
    On,
    /// Never reduce.
    Off,
}

impl ReduceMotion {
    /// Whether motion is reduced.
    #[must_use]
    pub fn resolve(self) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::FollowOs => std::env::var("OINO_REDUCE_MOTION").is_ok_and(|v| v == "1"),
        }
    }
}

/// Status panel mode (design.md §4.19).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusPanelMode {
    /// Expanded at ≥ 1280 px, mini below.
    #[default]
    Auto,
    /// Always expanded.
    Expanded,
    /// Always the mini capsule.
    Mini,
}

/// Tool row density (unipi utility render styles, design.md §4.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolDensity {
    /// One line, no detail until expanded.
    #[default]
    Simple,
    /// One line plus a short preview for edits and shell commands.
    Regular,
    /// Details expanded.
    Advanced,
}

impl ToolDensity {
    /// Wire name (`render{density}`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Regular => "regular",
            Self::Advanced => "advanced",
        }
    }
}

/// Thinking block display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThinkingBlocks {
    /// One collapsed line (default).
    #[default]
    Collapsed,
    /// Expanded.
    Expanded,
    /// Not shown (`hideThinkingBlock`).
    Hidden,
}

/// Desktop-only preferences (`~/.oino/desktop.json`); agent settings stay in
/// `settings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent persisted preferences"
)]
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
    /// UI font size in px (12–18).
    pub ui_font_size: u8,
    /// Code / diff / log font size in px (10–20).
    pub code_font_size: u8,
    /// Reduce motion.
    pub reduce_motion: ReduceMotion,
    /// Status panel mode.
    pub status_panel: StatusPanelMode,
    /// Tool row density.
    pub tool_density: ToolDensity,
    /// Thinking blocks.
    pub thinking_blocks: ThinkingBlocks,
    /// Onboarding finished (or skipped).
    pub onboarded: bool,
    /// Palette history (most recent first, max 20).
    pub palette_history: Vec<String>,
    /// Linux: run on native Wayland, which disables the embedded browser (design.md A1.4;
    /// the default runs on X11/XWayland so `WebKitGTK` can embed).
    pub native_wayland: bool,
    /// Opt-in desktop mascot window (design.md A1.2, unipi §1.5.1).
    pub mascot_window: bool,
    /// Mascot crab size in px: 72 / 96 / 128.
    pub mascot_size: u8,
    /// Sidebar row density: compact rows when `true`.
    pub sidebar_compact: bool,
    /// Session list column width in px (A2.8: 280–560, default 400).
    pub list_width: u16,
    /// The session file shown last; reopened at startup instead of a new empty session
    /// (GA-11).
    pub last_session: Option<String>,
    /// The working directory of [`Self::last_session`].
    pub last_session_cwd: Option<String>,
    /// Folder new sessions start in (Settings → General); `None` = the host default.
    pub default_cwd: Option<String>,
    /// Reopen the last session at startup (GA-11; Settings → General → Startup).
    pub reopen_last: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            hide_thinking: true,
            compact: false,
            expand_tools: false,
            recent_cwds: Vec::new(),
            ui_font_size: 14,
            code_font_size: 12,
            reduce_motion: ReduceMotion::FollowOs,
            status_panel: StatusPanelMode::Auto,
            tool_density: ToolDensity::Simple,
            thinking_blocks: ThinkingBlocks::Collapsed,
            onboarded: false,
            palette_history: Vec::new(),
            native_wayland: false,
            mascot_window: false,
            mascot_size: 96,
            sidebar_compact: false,
            list_width: 400,
            last_session: None,
            last_session_cwd: None,
            default_cwd: None,
            reopen_last: true,
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

    /// Records a palette query (max 20, most recent first).
    pub fn push_palette_history(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.palette_history.retain(|q| q != query);
        self.palette_history.insert(0, query.to_owned());
        self.palette_history.truncate(20);
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

    /// Enables / disables an extension (`disabledExtensions`; applies to new sessions).
    ///
    /// # Errors
    /// The host cannot change extension state, or the settings write failed.
    fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<(), HostError> {
        let _ = (id, enabled);
        Err(HostError::Session(
            "this host cannot enable or disable extensions".into(),
        ))
    }

    /// Installs the extension package at `path` (a directory with `extension.json`) into the
    /// user extensions folder; returns its id.
    fn install_extension(&self, path: PathBuf) -> BoxFuture<'static, Result<String, HostError>> {
        let _ = path;
        Box::pin(async {
            Err(HostError::Session(
                "this host cannot install extensions".into(),
            ))
        })
    }

    /// The user extensions folder (`~/.oino/extensions`).
    fn extensions_dir(&self) -> Option<PathBuf> {
        None
    }

    /// Shuts every live session down (`session_shutdown{reason:"quit"}`, child processes
    /// reaped); called once when the app quits. Runs on the tokio runtime.
    fn shutdown(&self) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_recent_cwds() {
        let parsed: AppSettings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(parsed.theme, ThemePref::Light);
        assert!(parsed.hide_thinking);
        assert_eq!(parsed.ui_font_size, 14);
        assert_eq!(AppSettings::default().theme, ThemePref::System);
        let mut h = AppSettings::default();
        for i in 0..25 {
            h.push_palette_history(&format!("q{i}"));
        }
        h.push_palette_history("q3");
        assert_eq!(h.palette_history.len(), 20);
        assert_eq!(h.palette_history[0], "q3");
        let mut s = AppSettings::default();
        for i in 0..12 {
            s.touch_cwd(&format!("/p{i}"));
        }
        s.touch_cwd("/p5");
        assert_eq!(s.recent_cwds.len(), 10);
        assert_eq!(s.recent_cwds[0], "/p5");
    }
}
