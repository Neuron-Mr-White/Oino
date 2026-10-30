//! Placements, slots and the entry registry wire types (§6.1, §9.2, §4.6).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Loose JSON object (`UiRequest.fields`, `UiAction.values`).
pub type JsonMap = serde_json::Map<String, Json>;

use crate::extras::icon::Icon;
use crate::node::{ActionId, Node};
use crate::patch::NodeOp;
use crate::style::Color;
use crate::text::Line;

/// Slot = placement (§6.2): where an entry renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Slot {
    /// Stack of modals / dialogs (top = most recently opened).
    Overlay,
    /// Right pane tabs (Z-5).
    SidePane,
    /// Bottom panel tabs (terminal et al).
    BottomPane,
    /// Replaces the composer (work tray, /btw).
    EditorReplace,
    /// Sidebar page / transcript takeover.
    Fullscreen,
    /// Widget above the composer (default `setWidget` placement).
    #[default]
    Widget,
    /// Widget below the composer (desktop only).
    WidgetBelow,
    /// Status bar segments (`setStatus` / `setFrame` / `setFooter` segments).
    Status,
    /// End-of-turn renderers (§6.7).
    TurnFooter,
    /// Band above the transcript (`setHeader`).
    Header,
    /// Footer rows (`setFooter{spec}`), folded into the status bar in the app.
    Footer,
    /// One Settings section per namespace (§7).
    Settings,
}

/// Placement values of `openPanel` (identical set to [`Slot`], §6.2).
pub type Placement = Slot;

/// `setWidget` placement (§6.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetPlacement {
    /// Above the composer (default).
    #[default]
    AboveEditor,
    /// Below the composer (desktop only).
    BelowEditor,
}

/// Reserved first-party components (§8); the app renders its native component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReservedComponent {
    /// Kanban board (`unipi.kanboard/board`).
    Board,
    /// Cron jobs (`unipi.cron/jobs`).
    Cron,
    /// Files tree + editor (`host/files`).
    Files,
    /// Git pane (`host/git`).
    Git,
    /// Browser pane (`unipi.browser/browser`).
    Browser,
    /// Terminal (`host/terminal`).
    Terminal,
    /// Session info modal (`unipi.info-screen/dashboard`).
    SessionInfo,
    /// Long-horizon status (`unipi.long-horizon/visualize`).
    LhStatus,
    /// Turn file summary (`unipi.file-summary/turn-files`).
    TurnFiles,
}

/// Entry size of an overlay (`custom` dialog / `overlay` panel, §6.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntrySize {
    /// 420 px (desktop).
    Small,
    /// 560 px.
    #[default]
    Medium,
    /// 800 px.
    Large,
    /// Full window / sheet.
    Full,
}

/// Registry scope (§2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Per session: entries of the session's extension instances.
    #[default]
    Session,
    /// Per host: reserved components, settings namespaces.
    Host,
}

/// A key binding of an entry (§6.1 `keys`) or a session shortcut (§9.2 `meta`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBinding {
    /// Key id (pi `KeyId`).
    pub key: String,
    /// Action fired (absent: `$key` with the key id as value, §4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionId>,
    /// Label shown in the entry footer chips.
    pub label: String,
}

/// What an entry renders (§6.1 `view`).
#[allow(
    clippy::large_enum_variant,
    reason = "an entry holds one view; boxing the tree root would force a clone per render"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum UiView {
    /// A validated node tree.
    Tree {
        /// The root node.
        root: Node,
    },
    /// pi `setWidget` string arrays.
    Lines {
        /// Plain lines.
        lines: Vec<String>,
    },
    /// `setStatus` / `setFrame` segment text.
    Status {
        /// Segment text.
        text: Line,
    },
    /// A reserved first-party component (§8).
    Reserved {
        /// Which native component the app renders.
        component: ReservedComponent,
        /// Tree for frontends without the component (§8 rule 3).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<Box<Node>>,
        /// Component parameters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<Json>,
    },
    /// A turn-footer renderer registration (§6.7).
    Renderer {
        /// Renderer key.
        key: String,
    },
    /// One Settings section (§7).
    Settings {
        /// The namespace descriptor.
        namespace: crate::settings::SettingsNamespace,
    },
}

/// One registry entry (§6.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiEntry {
    /// `"<extensionId>/<name>"` or a reserved id (§8).
    pub id: String,
    /// Owning extension (`"host"` for host-owned reserved screens).
    pub extension_id: String,
    /// Registry scope.
    pub scope: Scope,
    /// Where the entry renders.
    pub slot: Slot,
    /// Sort weight (default 0; order desc, then seq asc, §6.4).
    #[serde(default)]
    pub order: i32,
    /// Registration sequence, assigned by core; stable across updates (§6.4).
    pub seq: u64,
    /// Tab / page title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tab / page icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Tab badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Line>,
    /// Tab tone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Color>,
    /// What the entry renders.
    pub view: UiView,
    /// Key bindings active while focus is inside the entry (§4.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyBinding>,
    /// Activate on open when the user is on that session (§6.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<bool>,
    /// Esc closes the entry (default `true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_on_escape: Option<bool>,
    /// Overlay width (`custom` dialogs / `overlay` panels, §6.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<EntrySize>,
    /// Last change of this entry (cache key for frontends).
    pub rev: u64,
}

/// pi-shaped dialog request kept opaquely in snapshots (`{id, method, …}`, §9.2
/// `dialogs`); core owns the exact pi records (pi parity).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiRequest {
    /// Request id (pi request shape).
    pub id: String,
    /// Remaining fields (`method`, params, Oino extras) passed through verbatim.
    #[serde(flatten)]
    pub fields: JsonMap,
}

/// Registry meta (§9.2): session title, working message and shortcuts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiMeta {
    /// Terminal / window title (`setTitle`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Working line text (`setWorkingMessage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_message: Option<String>,
    /// Working line visibility (`setWorkingMessageVisible`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_visible: Option<bool>,
    /// Session shortcuts (`setShortcuts`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<KeyBinding>,
}

/// Partial registry meta for `meta` / `sessionMeta` ops (§9.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiMetaPatch {
    /// Set the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Set the working message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_message: Option<String>,
    /// Set the working visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_visible: Option<bool>,
    /// Set the shortcuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Vec<KeyBinding>>,
}

/// Partial entry-meta `set` for the `meta` op (§9.2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMetaPatch {
    /// Set the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Set the icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Set the badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Line>,
    /// Set the sort weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    /// Set the key bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<KeyBinding>>,
    /// Set the tone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Color>,
    /// Set the focus flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<bool>,
}

/// Why a dialog op was dismissed (§9.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DismissReason {
    /// The dialog was answered.
    Answered,
    /// It timed out.
    Timeout,
    /// It was cancelled (user or withdrawal).
    #[default]
    Cancelled,
}

/// One registry mutation (§9.2 `RegistryOp`).
#[allow(
    clippy::large_enum_variant,
    reason = "registry ops are built once per batch; boxing the entry adds noise"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum RegistryOp {
    /// Add or replace a whole entry.
    Put {
        /// The entry.
        entry: UiEntry,
    },
    /// Delete an entry (frontends close its view and drop local state, §6.6).
    Del {
        /// Entry id.
        id: String,
    },
    /// Patch entry meta without touching the view.
    Meta {
        /// Entry id.
        id: String,
        /// Changed meta fields.
        set: EntryMetaPatch,
    },
    /// Apply node ops to an entry's tree (§4.7).
    Tree {
        /// Entry id.
        id: String,
        /// The entry's new `rev` (cache key).
        rev: u64,
        /// Node ops, applied in order (§4.7).
        ops: Vec<NodeOp>,
    },
    /// Invalidate turn-footer caches (§6.7).
    Invalidate {
        /// Entry id.
        id: String,
        /// Turns invalidated (absent = all).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turns: Option<Vec<String>>,
    },
    /// A dialog opened (also sent as `extension_ui_request` for pi clients, §9.2).
    Dialog {
        /// The pi-shaped request.
        request: UiRequest,
    },
    /// A dialog was dismissed (§9.2).
    Dismiss {
        /// Dialog id.
        id: String,
        /// Why.
        reason: DismissReason,
    },
    /// Patch registry meta.
    SessionMeta {
        /// Changed meta fields.
        set: UiMetaPatch,
    },
    /// One-shot effect (`notify`, `set_editor_text`, `openUrl`, …); never in
    /// snapshots (§2 L0).
    Effect {
        /// The pi-shaped request.
        request: UiRequest,
    },
}

/// Full registry state for a scope (§9.2 `UiSnapshot`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSnapshot {
    /// Which registry this describes.
    pub scope: Scope,
    /// Session id (session scope only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Monotonic revision.
    pub rev: u64,
    /// Entries (registry order not implied; frontends sort by §6.4).
    #[serde(default)]
    pub entries: Vec<UiEntry>,
    /// Open dialogs, pi request shape (first-answer-wins state, §3).
    #[serde(default)]
    pub dialogs: Vec<UiRequest>,
    /// Registry meta.
    #[serde(default)]
    pub meta: UiMeta,
}

/// Incremental registry update (§9.2 `UiPatch`); `rev` = previous + 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPatch {
    /// Which registry this mutates.
    pub scope: Scope,
    /// Session id (session scope only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `previous + 1`; a gap triggers `_oino/ui_resync` (§9.3).
    pub rev: u64,
    /// Ops, applied in order.
    #[serde(default)]
    pub ops: Vec<RegistryOp>,
}

/// Where an action came from (§4.6 `source`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionSource {
    /// The terminal UI.
    Tui,
    /// The unipi app (desktop or phone).
    #[default]
    App,
    /// An RPC client.
    Rpc,
}

/// An interaction event from a frontend (§4.6). Emitted to the owning extension
/// as the `oino_ui_action` event (`type` added by the event envelope);
/// `panelId` keeps the v1 field name (the entry id) so old handlers keep working.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiAction {
    /// The entry id (v1 name kept).
    pub panel_id: String,
    /// Action id (§4.6).
    pub action_id: String,
    /// Primary value (item key, submitted text, bool, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Json>,
    /// Values of every keyed `Input` / `Toggle` / `Select` in the entry,
    /// sent with form-style submissions (§4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<JsonMap>,
    /// Path of the node that fired the action (§4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<crate::patch::NodePath>,
    /// Which frontend sent it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ActionSource>,
}

/// Event type name of [`UiAction`] on the wire.
pub const UI_ACTION_EVENT_TYPE: &str = "oino_ui_action";

/// `_oino/ui_visible` (§6.5): the frontend reports visibility; core aggregates
/// `$show` / `$hide` over frontends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiVisible {
    /// Entry id.
    pub entry_id: String,
    /// Visible in this frontend?
    pub visible: bool,
}

/// Frontend capabilities sent in `_oino/hello` (§10).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCapabilities {
    /// Protocol version (2).
    pub protocol: u8,
    /// Dialogs the frontend renders (pi method names, §10).
    pub dialogs: Vec<String>,
    /// Slots the frontend hosts.
    pub slots: Vec<Slot>,
    /// Reserved components implemented.
    pub reserved: Vec<ReservedComponent>,
    /// Node coverage: `"all"` or `"none"` (text fallback, §10).
    pub nodes: String,
    /// Inline images.
    pub images: bool,
    /// OS notifications.
    pub os_notify: bool,
    /// Clipboard access.
    pub clipboard: bool,
}

impl Default for UiCapabilities {
    fn default() -> Self {
        Self::full()
    }
}

impl UiCapabilities {
    /// The full v2 capability set of §10.
    #[must_use]
    pub fn full() -> Self {
        Self {
            protocol: 2,
            dialogs: [
                "select",
                "confirm",
                "input",
                "editor",
                "multiSelect",
                "form",
                "approval",
                "custom",
                "clipboardReadImage",
                "editExternally",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            slots: [
                Slot::Overlay,
                Slot::SidePane,
                Slot::BottomPane,
                Slot::EditorReplace,
                Slot::Fullscreen,
                Slot::Widget,
                Slot::WidgetBelow,
                Slot::Status,
                Slot::TurnFooter,
                Slot::Header,
                Slot::Footer,
                Slot::Settings,
            ]
            .into_iter()
            .collect(),
            reserved: [
                ReservedComponent::Board,
                ReservedComponent::Cron,
                ReservedComponent::Files,
                ReservedComponent::Git,
                ReservedComponent::Browser,
                ReservedComponent::Terminal,
                ReservedComponent::SessionInfo,
                ReservedComponent::LhStatus,
                ReservedComponent::TurnFiles,
            ]
            .into_iter()
            .collect(),
            nodes: "all".to_owned(),
            images: true,
            os_notify: true,
            clipboard: true,
        }
    }
}

/// `openPanel` payload (§9.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelOpen {
    /// Entry id (default `"<ext>/panel-<n>"`, v1 U8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_id: Option<String>,
    /// Placement.
    pub placement: Placement,
    /// Tab / page title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tab / page icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Tab badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Line>,
    /// Sort weight (§6.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    /// Root node (ignored for reserved ids, §8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Node>,
    /// Key bindings (§4.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyBinding>,
    /// Activate the entry (§6.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<bool>,
    /// Esc closes (default `true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_on_escape: Option<bool>,
    /// Overlay width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<EntrySize>,
    /// Reserved-component parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Json>,
}

/// `updatePanel` payload (§9.1): a whole tree (core diffs) or explicit ops, plus
/// optional meta.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelUpdate {
    /// Whole tree (core diffs, §4.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Node>,
    /// Explicit ops (§4.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<Vec<NodeOp>>,
    /// Set the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Set the icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Set the badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Line>,
    /// Set the sort weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    /// Set the key bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<KeyBinding>>,
    /// Set the fallback tree of a reserved entry (§8 rule 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Node>,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use super::*;
    use crate::extras::log::Log;

    #[test]
    fn action_event_matches_spec_example() {
        let raw = r#"{"panelId":"unipi.kanboard/board","actionId":"open-task",
            "value":"T-12","path":["main","tasks"],"values":{"filter":"open"},
            "source":"app"}"#;
        let action: UiAction = serde_json::from_str(raw).unwrap_or_default();
        assert_eq!(action.panel_id, "unipi.kanboard/board");
        assert_eq!(action.action_id, "open-task");
        assert_eq!(action.source, Some(ActionSource::App));
    }

    #[test]
    fn snapshot_and_patch_round_trip() {
        let snapshot = UiSnapshot {
            scope: Scope::Session,
            session_id: Some("s1".into()),
            rev: 7,
            entries: vec![UiEntry {
                id: "ext/board".into(),
                extension_id: "ext".into(),
                scope: Scope::Session,
                slot: Slot::SidePane,
                order: 0,
                seq: 1,
                title: Some("Board".into()),
                icon: Some(Icon::Board),
                badge: None,
                tone: None,
                view: UiView::Tree {
                    root: Log::new(["a"]).into(),
                },
                keys: vec![KeyBinding {
                    key: "enter".into(),
                    action: Some("open".into()),
                    label: "open".into(),
                }],
                focus: Some(true),
                close_on_escape: None,
                size: None,
                rev: 3,
            }],
            dialogs: vec![UiRequest {
                id: "1".into(),
                fields: serde_json::Map::from_iter([
                    ("method".to_owned(), Json::String("select".into())),
                    ("title".to_owned(), Json::String("Pick".into())),
                ]),
            }],
            meta: UiMeta {
                title: Some("fix: bug".into()),
                working_message: None,
                working_visible: None,
                shortcuts: Vec::new(),
            },
        };
        let json = serde_json::to_value(&snapshot).unwrap_or(Json::Null);
        let back: UiSnapshot = serde_json::from_value(json).unwrap_or_default();
        assert_eq!(snapshot, back);

        let patch = UiPatch {
            scope: Scope::Session,
            session_id: Some("s1".into()),
            rev: 8,
            ops: vec![
                RegistryOp::Put {
                    entry: snapshot.entries[0].clone(),
                },
                RegistryOp::Tree {
                    id: "ext/board".into(),
                    rev: 4,
                    ops: vec![],
                },
                RegistryOp::Del {
                    id: "ext/old".into(),
                },
                RegistryOp::Invalidate {
                    id: "ext/tf".into(),
                    turns: Some(vec!["t1".into()]),
                },
                RegistryOp::Dismiss {
                    id: "1".into(),
                    reason: DismissReason::Answered,
                },
                RegistryOp::SessionMeta {
                    set: UiMetaPatch {
                        title: Some("x".into()),
                        ..UiMetaPatch::default()
                    },
                },
                RegistryOp::Effect {
                    request: UiRequest::default(),
                },
                RegistryOp::Meta {
                    id: "ext/board".into(),
                    set: EntryMetaPatch {
                        order: Some(5),
                        ..EntryMetaPatch::default()
                    },
                },
            ],
        };
        let json = serde_json::to_value(&patch).unwrap_or(Json::Null);
        let back: UiPatch = serde_json::from_value(json).unwrap_or_default();
        assert_eq!(patch, back);
    }

    #[test]
    fn capabilities_round_trip() {
        let caps = UiCapabilities::full();
        let json = serde_json::to_value(&caps).unwrap_or(Json::Null);
        let back: UiCapabilities = serde_json::from_value(json).unwrap_or_default();
        assert_eq!(caps, back);
        assert_eq!(caps.protocol, 2);
    }
}
