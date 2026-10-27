//! Viewer stream and user input (browser-control.md §8.5, §9).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::status::{BrowserStatus, TabInfo};

/// Base64 serde for byte payloads.
pub(crate) mod b64 {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(data))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// A JPEG frame shared without copies in-process (base64 over RPC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameData(pub Arc<Vec<u8>>);

impl Serialize for FrameData {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        b64::serialize(&self.0, s)
    }
}

impl<'de> Deserialize<'de> for FrameData {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        b64::deserialize(d).map(|v| Self(Arc::new(v)))
    }
}

/// A viewer's viewport.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    /// CSS width.
    pub width: u32,
    /// CSS height.
    pub height: u32,
    /// Device scale factor.
    pub scale: f64,
    /// Mobile emulation.
    #[serde(default)]
    pub mobile: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            scale: 1.0,
            mobile: false,
        }
    }
}

/// Events of a session's viewers (frames are per attached view).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserViewEvent {
    /// A screencast frame.
    #[serde(rename_all = "camelCase")]
    Frame {
        /// View id.
        view_id: u64,
        /// Tab.
        tab: String,
        /// Frame sequence (ack it with `view_ack`).
        seq: u64,
        /// Page CSS width the frame shows.
        css_width: f64,
        /// Page CSS height the frame shows.
        css_height: f64,
        /// JPEG bytes.
        jpeg: FrameData,
    },
    /// Page state.
    #[serde(rename_all = "camelCase")]
    Page {
        /// Tab.
        tab: String,
        /// URL.
        url: String,
        /// Title.
        title: String,
        /// Loading.
        loading: bool,
        /// Back possible.
        can_back: bool,
        /// Forward possible.
        can_forward: bool,
    },
    /// The session's tabs changed.
    Tabs {
        /// Every tab of the session.
        tabs: Vec<TabInfo>,
    },
    /// Agent / take-over state of a tab.
    #[serde(rename_all = "camelCase")]
    Agent {
        /// Tab.
        tab: String,
        /// An agent command runs.
        driving: bool,
        /// The user has control.
        taken_over: bool,
    },
    /// Show this tab (agent opened/selected it or asks for a handoff).
    Reveal {
        /// Tab.
        tab: String,
        /// `agent_opened|agent_selected|handoff|user`.
        reason: String,
    },
    /// A JS dialog is open.
    #[serde(rename_all = "camelCase")]
    Dialog {
        /// Tab.
        tab: String,
        /// `alert|confirm|prompt|beforeunload`.
        dialog: String,
        /// Message.
        message: String,
        /// Default prompt text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_prompt: Option<String>,
    },
    /// The dialog was answered.
    DialogClosed {
        /// Tab.
        tab: String,
    },
    /// The agent asks the user to act (`browser_handoff`).
    Handoff {
        /// Tab.
        tab: String,
        /// Request text.
        message: String,
        /// Still pending (false: answered).
        pending: bool,
    },
    /// Download progress.
    Download {
        /// Tab.
        tab: String,
        /// File name.
        name: String,
        /// `started|completed|canceled`.
        state: String,
    },
    /// A tab closed.
    Closed {
        /// Tab.
        tab: String,
        /// `user|agent|limit|idle|crash|exit`.
        reason: String,
    },
    /// Hover cursor style (`pointer|text|default|grab|…`).
    Cursor {
        /// Tab.
        tab: String,
        /// CSS cursor.
        cursor: String,
    },
    /// Service status changed.
    Status {
        /// New status.
        status: Box<BrowserStatus>,
    },
}

/// Mouse event kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseEventType {
    /// Button down.
    Down,
    /// Button up.
    Up,
    /// Move.
    Move,
}

/// Key event kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyEventType {
    /// Key down (with text when it produces characters).
    Down,
    /// Key up.
    Up,
}

/// Modifier bit flags (CDP: Alt=1, Ctrl=2, Meta=4, Shift=8).
pub mod modifiers {
    /// Alt.
    pub const ALT: u8 = 1;
    /// Control.
    pub const CTRL: u8 = 2;
    /// Meta.
    pub const META: u8 = 4;
    /// Shift.
    pub const SHIFT: u8 = 8;
}

/// User input forwarded from a viewer (`_oino/browser_input`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserInput {
    /// Mouse.
    #[serde(rename_all = "camelCase")]
    Mouse {
        /// Kind.
        event: MouseEventType,
        /// Page CSS x.
        x: f64,
        /// Page CSS y.
        y: f64,
        /// Button.
        #[serde(default)]
        button: crate::command::MouseButton,
        /// Click count.
        #[serde(default)]
        click_count: u32,
        /// Modifier bits.
        #[serde(default)]
        modifiers: u8,
    },
    /// Wheel (pixel deltas).
    Wheel {
        /// X.
        x: f64,
        /// Y.
        y: f64,
        /// Horizontal delta.
        dx: f64,
        /// Vertical delta.
        dy: f64,
        /// Modifier bits.
        #[serde(default)]
        modifiers: u8,
    },
    /// A key (Playwright/DOM `key` name, e.g. `Enter`, `a`, `ArrowLeft`).
    Key {
        /// Down/up.
        event: KeyEventType,
        /// DOM key.
        key: String,
        /// Produced text (down only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// Modifier bits.
        #[serde(default)]
        modifiers: u8,
    },
    /// Insert committed text.
    Text {
        /// Text.
        text: String,
    },
    /// IME composition.
    #[serde(rename_all = "camelCase")]
    Ime {
        /// Marked text.
        text: String,
        /// Selection start.
        selection_start: u32,
        /// Selection end.
        selection_end: u32,
    },
    /// Paste OS clipboard text.
    ClipboardPaste {
        /// Text.
        text: String,
    },
    /// Navigate (URL bar).
    Goto {
        /// URL.
        url: String,
    },
    /// History / reload / stop.
    History {
        /// `back|forward|reload|stop`.
        action: String,
    },
    /// Viewer viewport changed (Fit to pane).
    Resize {
        /// New viewport.
        viewport: Viewport,
    },
}

/// Frontend → service requests (`_oino/browser_*`, browser-control.md §8.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FrontendRequest {
    /// Open a user tab in the session (`_oino/browser_tab_open`).
    TabOpen {
        /// URL.
        url: String,
        /// Reveal it.
        #[serde(default)]
        reveal: bool,
    },
    /// Close a tab.
    TabClose {
        /// Tab.
        tab: String,
    },
    /// Forward user input (`_oino/browser_input`; also URL bar / history).
    Input {
        /// Tab.
        tab: String,
        /// Input.
        input: BrowserInput,
    },
    /// Take over (`on`) or give back.
    Takeover {
        /// Tab.
        tab: String,
        /// Take over.
        on: bool,
    },
    /// Answer a handoff card.
    HandoffDone {
        /// Tab.
        tab: String,
        /// `true` = Done, `false` = Cancel.
        done: bool,
    },
    /// Answer a JS dialog from the viewer.
    DialogAnswer {
        /// Tab.
        tab: String,
        /// Accept.
        accept: bool,
        /// Prompt text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Selected text of the page (clipboard bridge).
    CopySelection {
        /// Tab.
        tab: String,
    },
    /// CSS cursor at a point (cursor probe).
    Cursor {
        /// Tab.
        tab: String,
        /// X.
        x: f64,
        /// Y.
        y: f64,
    },
    /// Acknowledge a frame.
    ViewAck {
        /// View.
        view_id: u64,
        /// Sequence.
        seq: u64,
    },
    /// Stop a view.
    ViewDetach {
        /// View.
        view_id: u64,
    },
    /// Read or set site permissions of an origin.
    SitePermissions {
        /// Origin.
        origin: String,
        /// `(perm, allow|block|ask)`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set: Option<(String, String)>,
    },
    /// Profiles: `list|create|remove|use`.
    Profiles {
        /// Action.
        op: String,
        /// Name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_is_base64() {
        let e = BrowserViewEvent::Frame {
            view_id: 1,
            tab: "t1".into(),
            seq: 3,
            css_width: 1280.0,
            css_height: 800.0,
            jpeg: FrameData(Arc::new(vec![1, 2, 3])),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["jpeg"], json!("AQID"));
        assert_eq!(v["kind"], json!("frame"));
        let back: BrowserViewEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn input_wire() {
        let i: BrowserInput = serde_json::from_value(
            json!({"type":"mouse","event":"down","x":1,"y":2,"button":"left","clickCount":1}),
        )
        .unwrap();
        assert!(matches!(i, BrowserInput::Mouse { click_count: 1, .. }));
    }
}
