//! Service status and tab descriptions (browser-control.md §4.1, §6.2).

use serde::{Deserialize, Serialize};

/// Why the browser is unavailable (`backend_unavailable{reason}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// `browser.enabled = false`.
    Disabled,
    /// No Chrome-family browser found.
    NoEngine,
    /// `engine = cloak` but `CloakBrowser` is not installed.
    CloakNotInstalled,
    /// The browser failed to start.
    LaunchFailed,
    /// The browser crashed.
    Crashed,
    /// The browser restarted; the tab handle is stale.
    Restarted,
    /// Too many running profiles.
    TooMany,
    /// `display = window` without a graphical session.
    NoDisplay,
}

impl UnavailableReason {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NoEngine => "no_engine",
            Self::CloakNotInstalled => "cloak_not_installed",
            Self::LaunchFailed => "launch_failed",
            Self::Crashed => "crashed",
            Self::Restarted => "restarted",
            Self::TooMany => "too_many",
            Self::NoDisplay => "no_display",
        }
    }
}

/// Who opened a tab.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TabOwner {
    /// The agent of a session.
    Agent {
        /// Session id.
        session: String,
    },
    /// The user, from a session's pane (`None`: no session, never claimable).
    User {
        /// Session id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
}

impl TabOwner {
    /// The owning session.
    #[must_use]
    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Agent { session } => Some(session),
            Self::User { session } => session.as_deref(),
        }
    }

    /// Opened by an agent.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }
}

/// Tab lifecycle (§6.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabLifecycle {
    /// Agent-controlled.
    #[default]
    Active,
    /// Waiting for the user (`browser_handoff`, `keep as handoff`).
    Handoff,
    /// A result for the user.
    Deliverable,
    /// A user tab.
    User,
    /// Closed.
    Closed,
}

impl TabLifecycle {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Handoff => "handoff",
            Self::Deliverable => "deliverable",
            Self::User => "user",
            Self::Closed => "closed",
        }
    }
}

/// Who drives a tab right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlledBy {
    /// The agent may act.
    #[default]
    Agent,
    /// The user took over.
    User,
}

/// One tab, as frontends and `browser_tabs list` see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools, reason = "independent wire flags")]
pub struct TabInfo {
    /// Logical id (`t3`).
    pub id: String,
    /// Full URL (frontends of the owning session only).
    pub url: String,
    /// Title.
    pub title: String,
    /// Owner.
    pub owner: TabOwner,
    /// Lifecycle.
    pub lifecycle: TabLifecycle,
    /// Driver.
    pub controlled_by: ControlledBy,
    /// Loading.
    #[serde(default)]
    pub loading: bool,
    /// History back possible.
    #[serde(default)]
    pub can_back: bool,
    /// History forward possible.
    #[serde(default)]
    pub can_forward: bool,
    /// Profile.
    pub profile: String,
    /// An agent command runs on it now.
    #[serde(default)]
    pub driving: bool,
}

/// One profile (§4.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    /// Name.
    pub name: String,
    /// Its browser runs.
    pub running: bool,
    /// Domains with imported cookies.
    #[serde(default)]
    pub cookie_domains: Vec<String>,
}

/// `browser.status` / `_oino/browser_status`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    /// `browser.enabled`.
    pub enabled: bool,
    /// Enabled and an engine resolves.
    pub available: bool,
    /// Why not available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<UnavailableReason>,
    /// `chrome` / `cloak` / `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Product name ("Google Chrome", "Chromium", "`CloakBrowser`").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// Browser version once running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Executable path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// `CloakBrowser` engine.
    pub stealth: bool,
    /// Why less stealthy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_note: Option<String>,
    /// Effective display (`hidden`, `headless`, `window`).
    pub display: String,
    /// Profiles.
    #[serde(default)]
    pub profiles: Vec<ProfileInfo>,
    /// The caller session's profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_profile: Option<String>,
    /// The caller session's tabs.
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
}

impl BrowserStatus {
    /// A short one-line summary ("Google Chrome · headless · profile default").
    #[must_use]
    pub fn summary(&self) -> String {
        if !self.available {
            return format!(
                "Browser unavailable ({})",
                self.reason.map_or("unknown", UnavailableReason::as_str)
            );
        }
        format!(
            "{} · {} · profile {}",
            self.product.as_deref().unwrap_or("Chrome"),
            self.display,
            self.session_profile.as_deref().unwrap_or("default")
        )
    }
}
