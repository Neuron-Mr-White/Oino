//! [`BrowserCommand`]: the 19 `browser_*` tools (browser-control.md §5.2) plus host-only ops.
//!
//! Wire shape = the tool's argument object with an `op` tag (`browser_press_key {key}` ⇔
//! `{"op":"press_key","key":…}`), so the guest forwards arguments without re-modelling them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Prefix of every agent tool name.
pub const TOOL_PREFIX: &str = "browser_";

/// Names of the 19 agent tools, in spec order.
pub const TOOL_NAMES: [&str; 19] = [
    "browser_navigate",
    "browser_snapshot",
    "browser_click",
    "browser_hover",
    "browser_type",
    "browser_press_key",
    "browser_scroll",
    "browser_select_option",
    "browser_drag",
    "browser_file_upload",
    "browser_handle_dialog",
    "browser_take_screenshot",
    "browser_evaluate",
    "browser_wait_for",
    "browser_tabs",
    "browser_logs",
    "browser_downloads",
    "browser_handoff",
    "browser_profile",
];

/// `browser_navigate {history}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload.
    Reload,
}

/// Mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// Primary.
    #[default]
    Left,
    /// Secondary.
    Right,
    /// Middle.
    Middle,
}

/// Keyboard modifier (Playwright names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modifier {
    /// Alt / Option.
    Alt,
    /// Control.
    Control,
    /// Meta / Command.
    Meta,
    /// Shift.
    Shift,
    /// Meta on macOS, Control elsewhere.
    ControlOrMeta,
}

/// A viewport point in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// X.
    pub x: f64,
    /// Y.
    pub y: f64,
}

/// `browser_wait_for {ref_state.state}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefStateKind {
    /// Visible.
    Visible,
    /// Hidden or gone.
    Hidden,
    /// Enabled.
    Enabled,
    /// Removed from the DOM.
    Detached,
}

/// `browser_wait_for {ref_state}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefState {
    /// Ref from the latest snapshot.
    #[serde(rename = "ref")]
    pub target: String,
    /// State to wait for.
    pub state: RefStateKind,
}

/// `browser_wait_for {load}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitLoad {
    /// `DOMContentLoaded`.
    Domcontentloaded,
    /// `load`.
    Load,
    /// No requests for 500 ms.
    Networkidle,
}

/// `browser_tabs {action}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabsAction {
    /// Agent tabs + claimable user tabs.
    List,
    /// Make current and visible.
    Select,
    /// Open a new tab.
    New,
    /// Close one tab.
    Close,
    /// Take control of a user tab.
    Claim,
    /// Mark for the user.
    Keep,
}

/// `browser_tabs {as}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAs {
    /// A result for the user to look at.
    Deliverable,
    /// The user continues from here.
    Handoff,
}

/// `browser_logs {kind}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    /// Console messages.
    Console,
    /// Uncaught exceptions.
    Errors,
    /// Network requests.
    Network,
}

/// `browser_downloads {action}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadsAction {
    /// Downloads of this scope.
    List,
    /// Next completed download.
    Wait,
}

/// `browser_profile {action}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileAction {
    /// Engine, profiles, take-over state.
    Status,
    /// Switch this session's profile.
    Use,
    /// Create an empty profile.
    Create,
}

/// One browser command in the caller's scope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BrowserCommand {
    /// `browser_navigate`.
    Navigate {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// URL to open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// History action instead of a URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history: Option<HistoryAction>,
        /// Always open a new tab.
        #[serde(default)]
        new_tab: bool,
        /// Append a snapshot (default true).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<bool>,
    },
    /// `browser_snapshot`.
    Snapshot {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Limit to the subtree of this ref.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        /// Character budget (≤ 60000, default 24000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_chars: Option<u32>,
        /// Expand iframes (default true).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frames: Option<bool>,
    },
    /// `browser_click`.
    Click {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Target ref.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Viewport x (CSS px) when no ref.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        /// Viewport y (CSS px) when no ref.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        /// Button.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<MouseButton>,
        /// Double click.
        #[serde(default)]
        double: bool,
        /// Held modifiers.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<Modifier>,
        /// The model marks the action sensitive (approval).
        #[serde(default)]
        sensitive: bool,
        /// Append a snapshot.
        #[serde(default)]
        snapshot: bool,
    },
    /// `browser_hover`.
    Hover {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Target ref.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Viewport x.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        /// Viewport y.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    },
    /// `browser_type`.
    Type {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Target ref (else the focused element).
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Text to insert.
        text: String,
        /// Clear first (default true).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clear: Option<bool>,
        /// Press Enter afterwards.
        #[serde(default)]
        submit: bool,
        /// Sensitive (approval).
        #[serde(default)]
        sensitive: bool,
        /// Append a snapshot.
        #[serde(default)]
        snapshot: bool,
    },
    /// `browser_press_key`.
    PressKey {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Playwright key syntax (`Enter`, `Control+a`).
        key: String,
        /// Focus this ref first.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Sensitive (approval).
        #[serde(default)]
        sensitive: bool,
        /// Append a snapshot.
        #[serde(default)]
        snapshot: bool,
    },
    /// `browser_scroll`.
    Scroll {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Scroll this ref into view.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Horizontal wheel delta.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dx: Option<f64>,
        /// Vertical wheel delta.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dy: Option<f64>,
        /// Wheel x.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        /// Wheel y.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    },
    /// `browser_select_option`.
    SelectOption {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// The `<select>` ref.
        #[serde(rename = "ref")]
        target: String,
        /// Values or labels.
        values: Vec<String>,
    },
    /// `browser_drag`.
    Drag {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Source ref.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_ref: Option<String>,
        /// Target ref.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_ref: Option<String>,
        /// Source point.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Point>,
        /// Target point.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Point>,
    },
    /// `browser_file_upload`.
    FileUpload {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// File input (or chooser opener) ref.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Files to upload.
        paths: Vec<String>,
    },
    /// `browser_handle_dialog`.
    HandleDialog {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Accept or dismiss.
        accept: bool,
        /// Text for `prompt()`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_text: Option<String>,
    },
    /// `browser_take_screenshot`.
    TakeScreenshot {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Element clip.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Whole page.
        #[serde(default)]
        full_page: bool,
    },
    /// `browser_evaluate`.
    Evaluate {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// JS function source.
        function: String,
        /// Passed as the first argument.
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Await a returned promise (default true).
        #[serde(rename = "await", default, skip_serializing_if = "Option::is_none")]
        await_promise: Option<bool>,
    },
    /// `browser_wait_for`.
    WaitFor {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Text appears.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// Text disappears.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_gone: Option<String>,
        /// Element state.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_state: Option<RefState>,
        /// URL glob.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// Load state.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        load: Option<WaitLoad>,
        /// Fixed sleep (≤ 30000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time_ms: Option<u64>,
        /// Timeout (≤ 30000, default 5000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// `browser_tabs`.
    Tabs {
        /// Action.
        action: TabsAction,
        /// Tab id (select/close/claim/keep).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// URL (new).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// Keep kind.
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        keep_as: Option<KeepAs>,
    },
    /// `browser_logs`.
    Logs {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// Which buffer.
        kind: LogKind,
        /// Only entries after this sequence number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since_seq: Option<u64>,
        /// Substring or `/regex/`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
        /// ≤ 200, default 50.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// `browser_downloads`.
    Downloads {
        /// Action.
        action: DownloadsAction,
        /// Wait timeout (≤ 120000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// `browser_handoff`.
    Handoff {
        /// Tab id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        /// What the user should do.
        message: String,
        /// Block until Done/Cancel (default true).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<bool>,
        /// ≤ 1800, default 600.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_s: Option<u64>,
    },
    /// `browser_profile`.
    Profile {
        /// Action.
        action: ProfileAction,
        /// Profile name (use/create).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Host-only: [`crate::BrowserStatus`] in `details`.
    Status,
    /// Host-only: turn-tail screenshot of the current tab (§11.3).
    TurnShot {
        /// Turn identifier used for the file name.
        turn_id: String,
    },
    /// Host-only: reveal a tab in the viewers.
    Reveal {
        /// Tab id.
        tab: String,
    },
    /// Host-only: give a claimed user tab back to the user.
    Release {
        /// Tab id (else every claimed tab of the session).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
    },
    /// Host-only: the turn ended (cancels pending commands, nothing closes).
    TurnEnded,
    /// Host-only: the session closed.
    SessionClosed {
        /// Close its tabs (deleted session / `browserScope: own`).
        #[serde(default)]
        close_tabs: bool,
    },
    /// Host-only: close every agent tab of the session (`/browser close-tabs`).
    CloseAgentTabs,
}

impl BrowserCommand {
    /// Builds the command of agent tool `tool` from its arguments.
    ///
    /// # Errors
    ///
    /// Unknown tool name or arguments not matching the tool's schema.
    pub fn from_tool(tool: &str, args: &Value) -> Result<Self, serde_json::Error> {
        let op = tool.strip_prefix(TOOL_PREFIX).unwrap_or(tool);
        let mut object = match args {
            Value::Object(map) => map.clone(),
            Value::Null => Map::new(),
            other => {
                let mut m = Map::new();
                m.insert("value".to_owned(), other.clone());
                m
            }
        };
        object.insert("op".to_owned(), Value::String(op.to_owned()));
        serde_json::from_value(Value::Object(object))
    }

    /// The `op` wire name.
    #[must_use]
    pub fn op(&self) -> &'static str {
        match self {
            Self::Navigate { .. } => "navigate",
            Self::Snapshot { .. } => "snapshot",
            Self::Click { .. } => "click",
            Self::Hover { .. } => "hover",
            Self::Type { .. } => "type",
            Self::PressKey { .. } => "press_key",
            Self::Scroll { .. } => "scroll",
            Self::SelectOption { .. } => "select_option",
            Self::Drag { .. } => "drag",
            Self::FileUpload { .. } => "file_upload",
            Self::HandleDialog { .. } => "handle_dialog",
            Self::TakeScreenshot { .. } => "take_screenshot",
            Self::Evaluate { .. } => "evaluate",
            Self::WaitFor { .. } => "wait_for",
            Self::Tabs { .. } => "tabs",
            Self::Logs { .. } => "logs",
            Self::Downloads { .. } => "downloads",
            Self::Handoff { .. } => "handoff",
            Self::Profile { .. } => "profile",
            Self::Status => "status",
            Self::TurnShot { .. } => "turn_shot",
            Self::Reveal { .. } => "reveal",
            Self::Release { .. } => "release",
            Self::TurnEnded => "turn_ended",
            Self::SessionClosed { .. } => "session_closed",
            Self::CloseAgentTabs => "close_agent_tabs",
        }
    }

    /// The explicit `tab` argument, if any.
    #[must_use]
    pub fn tab(&self) -> Option<&str> {
        match self {
            Self::Navigate { tab, .. }
            | Self::Snapshot { tab, .. }
            | Self::Click { tab, .. }
            | Self::Hover { tab, .. }
            | Self::Type { tab, .. }
            | Self::PressKey { tab, .. }
            | Self::Scroll { tab, .. }
            | Self::SelectOption { tab, .. }
            | Self::Drag { tab, .. }
            | Self::FileUpload { tab, .. }
            | Self::HandleDialog { tab, .. }
            | Self::TakeScreenshot { tab, .. }
            | Self::Evaluate { tab, .. }
            | Self::WaitFor { tab, .. }
            | Self::Tabs { tab, .. }
            | Self::Logs { tab, .. }
            | Self::Handoff { tab, .. }
            | Self::Release { tab } => tab.as_deref(),
            Self::Reveal { tab } => Some(tab),
            _ => None,
        }
    }

    /// Commands that drive the page; refused while the user has taken over (§7.4).
    #[must_use]
    pub fn is_driving(&self) -> bool {
        matches!(
            self,
            Self::Navigate { .. }
                | Self::Click { .. }
                | Self::Hover { .. }
                | Self::Type { .. }
                | Self::PressKey { .. }
                | Self::Scroll { .. }
                | Self::SelectOption { .. }
                | Self::Drag { .. }
                | Self::FileUpload { .. }
                | Self::HandleDialog { .. }
                | Self::Evaluate { .. }
        )
    }

    /// Actions that change page state (turn-tail screenshot qualification, §11.3).
    #[must_use]
    pub fn is_action(&self) -> bool {
        self.is_driving() && !matches!(self, Self::Hover { .. })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_args_round_trip() {
        let cmd = BrowserCommand::from_tool(
            "browser_click",
            &json!({"ref": "e7", "modifiers": ["Shift"], "sensitive": true}),
        )
        .unwrap();
        assert_eq!(
            cmd,
            BrowserCommand::Click {
                tab: None,
                target: Some("e7".into()),
                x: None,
                y: None,
                button: None,
                double: false,
                modifiers: vec![Modifier::Shift],
                sensitive: true,
                snapshot: false,
            }
        );
        assert_eq!(
            serde_json::to_value(&cmd).unwrap(),
            json!({"op":"click","ref":"e7","double":false,"modifiers":["Shift"],"sensitive":true,"snapshot":false})
        );
        assert_eq!(cmd.op(), "click");
        assert!(cmd.is_driving());
    }

    #[test]
    fn every_tool_name_parses() {
        let minimal = |name: &str| match name {
            "browser_type" => json!({"text": "x"}),
            "browser_press_key" => json!({"key": "Enter"}),
            "browser_select_option" => json!({"ref": "e1", "values": ["a"]}),
            "browser_file_upload" => json!({"paths": ["/tmp/a"]}),
            "browser_handle_dialog" => json!({"accept": true}),
            "browser_evaluate" => json!({"function": "() => 1"}),
            "browser_tabs" | "browser_downloads" => json!({"action": "list"}),
            "browser_logs" => json!({"kind": "console"}),
            "browser_handoff" => json!({"message": "solve it"}),
            "browser_profile" => json!({"action": "status"}),
            _ => json!({}),
        };
        for name in TOOL_NAMES {
            let cmd = BrowserCommand::from_tool(name, &minimal(name)).unwrap();
            assert_eq!(format!("browser_{}", cmd.op()), name);
        }
        assert!(BrowserCommand::from_tool("browser_nope", &json!({})).is_err());
    }
}
