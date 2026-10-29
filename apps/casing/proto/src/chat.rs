//! Start and close pi chats from the app (UNI-260, proto 1.12.0). Mirrors
//! `packages/protocol/src/chat.ts`; see docs/m5/PROTOCOL.md §10.
//!
//! - `chat_new{target, cwd?, outside?, name?}` → the host opens a herdr tab
//!   (in an existing workspace or a new one) or a tmux window/session in
//!   `cwd`, types the PC's pi launcher (`chat.launcher`, default `pi`) into
//!   it, waits for that pi's app-bridge record and answers
//!   `ok{data: ChatNewData}` (the pid to `chat_open`).
//! - `chat_close{pid, close_tab}` → stops that pi gracefully (SIGTERM, which
//!   pi turns into a normal quit) and, with `close_tab`, closes its herdr
//!   tab / tmux window (only its pane when the tab has others). →
//!   `ok{data: ChatCloseData}`.
//! - `chat_config` → `ok{data: ChatConfigData}`; `chat_config_set{launcher}`
//!   stores a new launcher (`""` = back to the default) and answers the same.
//!
//! Older hosts answer all four with `E_SCHEMA` (unknown variant).

use serde::{Deserialize, Serialize};

use crate::control::{HerdrId, MuxKind, SessionName};

/// Longest launcher command.
pub const CHAT_LAUNCHER_MAX: usize = 256;
/// The launcher when the PC has none configured.
pub const CHAT_LAUNCHER_DEFAULT: &str = "pi";

/// The command typed into the new pane to start pi (`pi`, `unipi`,
/// `~/bin/pi --model x`): 0..=256 chars, one line, no control characters.
/// Empty only in `chat_config_set` (= reset to the default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ChatLauncher(String);

impl ChatLauncher {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ChatLauncher {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.chars().count() > CHAT_LAUNCHER_MAX {
            return Err(format!("launcher must be at most {CHAT_LAUNCHER_MAX} characters"));
        }
        if value.chars().any(char::is_control) {
            return Err("launcher must be one line without control characters".into());
        }
        Ok(ChatLauncher(value.trim().to_string()))
    }
}

impl<'de> Deserialize<'de> for ChatLauncher {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ChatLauncher::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Where the new chat goes. Tagged on `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatNewTarget {
    /// A new tab in an existing herdr workspace.
    HerdrWorkspace { session: SessionName, workspace_id: HerdrId },
    /// A new herdr workspace (label defaults to the folder name).
    HerdrNewWorkspace {
        session: SessionName,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        label: Option<SessionName>,
    },
    /// A new window in an existing tmux session.
    TmuxWindow { session: SessionName },
    /// A new tmux session.
    TmuxNewSession { name: SessionName },
}

impl ChatNewTarget {
    /// The multiplexer session it touches (protected sessions refuse).
    pub fn session(&self) -> &str {
        match self {
            ChatNewTarget::HerdrWorkspace { session, .. }
            | ChatNewTarget::HerdrNewWorkspace { session, .. }
            | ChatNewTarget::TmuxWindow { session } => session.as_str(),
            ChatNewTarget::TmuxNewSession { name } => name.as_str(),
        }
    }
    pub fn mux(&self) -> MuxKind {
        match self {
            ChatNewTarget::HerdrWorkspace { .. } | ChatNewTarget::HerdrNewWorkspace { .. } => MuxKind::Herdr,
            ChatNewTarget::TmuxWindow { .. } | ChatNewTarget::TmuxNewSession { .. } => MuxKind::Tmux,
        }
    }
}

/// `ok.data` for `chat_new`: the started pi and where it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatNewData {
    /// The pi's pid (= its bridge): `chat_open{pid}`.
    pub pid: u32,
    pub mux: MuxKind,
    /// herdr / tmux session name.
    pub session: String,
    /// herdr workspace id, or the tmux window id (`@3`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workspace_id: Option<String>,
    /// herdr tab id (absent for tmux).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tab_id: Option<String>,
    /// herdr pane id or tmux pane id (`%7`).
    pub pane_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
}

/// What `chat_close` closed besides stopping pi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatClosed {
    /// Nothing (close_tab was false, or the pane was already gone).
    None,
    /// Only the pane (its tab/window had other panes).
    Pane,
    /// The herdr tab.
    Tab,
    /// The tmux window.
    Window,
}

/// `ok.data` for `chat_close`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatCloseData {
    /// pi exited (it may have been gone already).
    pub stopped: bool,
    pub closed: ChatClosed,
}

/// `ok.data` for `chat_config` / `chat_config_set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatConfigData {
    /// The command new chats run (never empty).
    pub launcher: String,
    /// `launcher` is the built-in default (nothing configured).
    #[serde(default)]
    pub default: bool,
    /// Multiplexers chat_new can use on this PC right now.
    #[serde(default)]
    pub herdr: bool,
    #[serde(default)]
    pub tmux: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_rules() {
        assert_eq!(ChatLauncher::try_from(" pi ".to_string()).unwrap().as_str(), "pi");
        assert!(ChatLauncher::try_from("".to_string()).is_ok());
        assert!(ChatLauncher::try_from("pi\nrm -rf /".to_string()).is_err());
        assert!(ChatLauncher::try_from("x".repeat(257)).is_err());
    }

    #[test]
    fn targets_parse() {
        let t: ChatNewTarget = serde_json::from_str(r#"{"kind":"herdr_workspace","session":"default","workspace_id":"w4E"}"#).unwrap();
        assert_eq!(t.session(), "default");
        assert_eq!(t.mux(), MuxKind::Herdr);
        let t: ChatNewTarget = serde_json::from_str(r#"{"kind":"tmux_new_session","name":"work"}"#).unwrap();
        assert_eq!((t.session(), t.mux()), ("work", MuxKind::Tmux));
        assert!(serde_json::from_str::<ChatNewTarget>(r#"{"kind":"herdr_workspace","session":"default"}"#).is_err());
    }
}
