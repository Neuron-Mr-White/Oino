//! `settings.json` key `browser` (browser-control.md §12).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `browser.engine`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineChoice {
    /// `CloakBrowser` when installed, else Chrome-family.
    #[default]
    Auto,
    /// System Chrome / Chromium / Edge / Brave.
    Chrome,
    /// `CloakBrowser` only.
    Cloak,
    /// `browser.executablePath` only.
    Path,
}

/// `browser.display`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayChoice {
    /// Hidden Xvfb display on Linux when it helps, else headless.
    #[default]
    Auto,
    /// Private Xvfb display (Linux).
    Hidden,
    /// `--headless=new`.
    Headless,
    /// A real window.
    Window,
}

/// `browser.humanize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Humanize {
    /// On for `CloakBrowser`.
    #[default]
    Auto,
    /// Off.
    Off,
    /// Normal timing.
    Default,
    /// Slower timing.
    Careful,
}

/// `browser.evaluate`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatePolicy {
    /// Run without asking.
    #[default]
    Allow,
    /// Ask every time.
    Ask,
    /// Refuse.
    Deny,
}

/// `browser.approvals`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approvals {
    /// Ask for sensitive actions (+ heuristics).
    #[default]
    Default,
    /// Trust the agent (uploads still ask).
    Never,
}

/// CDP transport (Unix default `pipe`; Windows always `port`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportChoice {
    /// Platform default.
    #[default]
    Auto,
    /// `--remote-debugging-pipe`.
    Pipe,
    /// Loopback port 0 + WebSocket.
    Port,
}

/// Effective browser settings.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors independent settings keys"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserSettings {
    /// Browser control on.
    pub enabled: bool,
    /// Engine.
    pub engine: EngineChoice,
    /// Explicit executable.
    pub executable_path: Option<PathBuf>,
    /// Display mode.
    pub display: DisplayChoice,
    /// Default profile of new sessions.
    pub profile: String,
    /// Running profiles cap.
    pub max_running: usize,
    /// Tabs per profile.
    pub max_tabs: usize,
    /// Idle quit (0 = never).
    pub idle_minutes: u64,
    /// Humanized input.
    pub humanize: Humanize,
    /// `browser_evaluate` policy.
    pub evaluate: EvaluatePolicy,
    /// Approval policy.
    pub approvals: Approvals,
    /// Agent navigation allow-list (globs, empty = all).
    pub allowed_origins: Vec<String>,
    /// Blocked origins (globs).
    pub blocked_origins: Vec<String>,
    /// Downloads root.
    pub downloads_dir: Option<PathBuf>,
    /// Turn-tail screenshot.
    pub turn_screenshot: bool,
    /// Ambient user-tab context.
    pub ambient_context: bool,
    /// Loopback CDP endpoint for external tools.
    pub external_cdp: bool,
    /// CDP transport override.
    pub transport: TransportChoice,
    /// Viewer frame-rate hint.
    pub viewer_max_fps: u32,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: EngineChoice::Auto,
            executable_path: None,
            display: DisplayChoice::Auto,
            profile: "default".into(),
            max_running: 3,
            max_tabs: 32,
            idle_minutes: 30,
            humanize: Humanize::Auto,
            evaluate: EvaluatePolicy::Allow,
            approvals: Approvals::Default,
            allowed_origins: Vec::new(),
            blocked_origins: Vec::new(),
            downloads_dir: None,
            turn_screenshot: true,
            ambient_context: true,
            external_cdp: false,
            transport: TransportChoice::Auto,
            viewer_max_fps: 30,
        }
    }
}

impl BrowserSettings {
    /// Reads the `browser` object of a settings document (unknown keys ignored, bad values
    /// fall back to defaults, numbers clamped to their documented ranges).
    #[must_use]
    pub fn from_settings(settings: &serde_json::Value) -> Self {
        let Some(obj) = settings.get("browser") else {
            return Self::default();
        };
        let mut out: Self = serde_json::from_value(obj.clone()).unwrap_or_else(|e| {
            tracing::warn!(%e, "settings: invalid `browser` section, using defaults");
            Self::default()
        });
        out.max_running = out.max_running.clamp(1, 8);
        out.max_tabs = out.max_tabs.clamp(4, 64);
        out.viewer_max_fps = out.viewer_max_fps.clamp(1, 60);
        if !valid_profile_name(&out.profile) {
            out.profile = "default".into();
        }
        out
    }

    /// Reads `<agent_dir>/settings.json` (missing / invalid ⇒ defaults).
    #[must_use]
    pub fn load(agent_dir: &std::path::Path) -> Self {
        let text = std::fs::read_to_string(agent_dir.join("settings.json")).unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        Self::from_settings(&value)
    }
}

/// Profile names: `[a-z0-9_-]{1,40}`.
#[must_use]
pub fn valid_profile_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_and_clamps() {
        let s = BrowserSettings::from_settings(&json!({"browser": {
            "engine": "chrome", "display": "headless", "maxTabs": 2, "profile": "Bad Name",
            "evaluate": "ask", "unknownKey": 1
        }}));
        assert_eq!(s.engine, EngineChoice::Chrome);
        assert_eq!(s.display, DisplayChoice::Headless);
        assert_eq!(s.max_tabs, 4);
        assert_eq!(s.profile, "default");
        assert_eq!(s.evaluate, EvaluatePolicy::Ask);
        assert_eq!(
            BrowserSettings::from_settings(&json!({})),
            BrowserSettings::default()
        );
        assert!(valid_profile_name("work-2"));
        assert!(!valid_profile_name(""));
    }
}
