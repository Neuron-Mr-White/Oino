//! Extension UI state (ui-protocol §2–§6, §9.2).
//!
//! Two reducers over the same dialog/effect vocabulary:
//!
//! - [`RegistryMirror`] is the v2 frontend state (§9.2): apply a `UiSnapshot`, then a stream
//!   of `UiPatch`es with a monotonic `rev`, and read entries per slot (§6.4), the dialog
//!   queue (§3.2) and the registry meta. A revision gap or an inapplicable op is refused so
//!   the caller can send `_oino/ui_resync`.
//! - [`UiState`] is the transitional v1 reducer over `extension_ui_request` /
//!   `extension_ui_dismiss` (first answer wins). It stays until the TUI reads the registry
//!   (WP-U4); core no longer publishes those retained requests.
//!
//! Fire-and-forget methods update retained state (status by key, widgets by placement,
//! frames, header/footer, title, working message, panels, shortcuts); dialogs queue up and
//! leave when answered locally or dismissed by core. One-shot effects (notify, editor text,
//! OS notifications, clipboard, URLs) are returned to the caller.

use std::collections::VecDeque;

use indexmap::IndexMap;
use oino_ai::{ImageContent, JsonObject, JsonValue};
use oino_core::ui::UiRequest;
use oino_ui::patch::{PatchError, apply};
use oino_ui::slot::{
    EntryMetaPatch, RegistryOp, Scope, Slot, UiEntry, UiMeta, UiMetaPatch, UiPatch, UiSnapshot,
    UiView,
};

use crate::ous::{OusNode, Tone};

fn s(params: &JsonObject, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn b(params: &JsonObject, key: &str) -> bool {
    params
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn arr<'a>(params: &'a JsonObject, key: &str) -> &'a [JsonValue] {
    params
        .get(key)
        .and_then(JsonValue::as_array)
        .map_or(&[], Vec::as_slice)
}

fn vs(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

/// Effective key `<extensionId>/<key>`.
fn scoped(request: &UiRequest, key: &str) -> String {
    format!("{}/{key}", request.extension_id.as_deref().unwrap_or(""))
}

/// Severity of a `notify`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyLevel {
    /// Info.
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
}

/// One-shot effect of a fire-and-forget request.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEffect {
    /// `notify`.
    Notify {
        /// Level.
        level: NotifyLevel,
        /// Text.
        message: String,
    },
    /// `set_editor_text`.
    SetEditorText(String),
    /// `insertEditorText`.
    InsertEditorText(String),
    /// `insertEditorToken`.
    InsertToken(EditorToken),
    /// `osNotify`.
    OsNotify {
        /// Title.
        title: String,
        /// Body.
        body: String,
    },
    /// `openUrl`.
    OpenUrl(String),
    /// `clipboardWrite`.
    ClipboardWrite(String),
    /// `openSettings`.
    OpenSettings {
        /// Namespace filter.
        namespace: Option<String>,
        /// Search query.
        query: Option<String>,
    },
    /// `setTheme`.
    SetTheme(String),
}

/// Attachment token (`insertEditorToken`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorToken {
    /// Token id.
    pub token_id: String,
    /// `image` | `file` | `text`.
    pub kind: String,
    /// Label shown in the composer.
    pub label: String,
    /// Image data.
    pub data: Option<ImageContent>,
    /// File path.
    pub path: Option<String>,
}

/// Status entry (`setStatus`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusItem {
    /// Text.
    pub text: String,
    /// Tone.
    pub tone: Tone,
    /// Higher first.
    pub priority: i64,
}

/// Widget placement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Above the composer.
    #[default]
    AboveEditor,
    /// Below the composer.
    BelowEditor,
    /// Header area.
    Header,
    /// Footer area.
    Footer,
}

/// Widget (`setWidget`).
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    /// Placement.
    pub placement: Placement,
    /// Plain lines.
    pub lines: Vec<String>,
    /// OUS spec (preferred).
    pub spec: Option<OusNode>,
}

/// Frame segment alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SegmentAlign {
    /// Left.
    #[default]
    Left,
    /// Right.
    Right,
}

/// Frame segment (`setFrame`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Text.
    pub text: String,
    /// Tone.
    pub tone: Tone,
    /// Icon name.
    pub icon: Option<String>,
    /// Lower priorities drop first.
    pub priority: i64,
    /// Alignment.
    pub align: SegmentAlign,
    /// Minimum width.
    pub min_width: Option<u16>,
}

/// Composer frame (`setFrame`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// Segments on the top border.
    pub top: Vec<Segment>,
    /// Segments on the bottom border.
    pub bottom: Vec<Segment>,
}

/// Panel placement (ui-protocol §5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelPlacement {
    /// Modal over the transcript.
    #[default]
    Overlay,
    /// GPUI side pane (TUI: overlay).
    SidePane,
    /// Replaces the composer.
    EditorReplace,
    /// GPUI bottom row (TUI: editor replace).
    BottomPane,
    /// Full screen.
    Fullscreen,
}

/// Panel key binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelKey {
    /// Key id.
    pub key: String,
    /// Action fired.
    pub action_id: String,
    /// Hint label.
    pub label: String,
}

/// Open panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    /// Panel id.
    pub panel_id: String,
    /// Owning extension.
    pub extension_id: Option<String>,
    /// Placement.
    pub placement: PanelPlacement,
    /// Title.
    pub title: Option<String>,
    /// Spec.
    pub spec: OusNode,
    /// Key bindings.
    pub keys: Vec<PanelKey>,
    /// Takes focus.
    pub focus: bool,
    /// Escape closes.
    pub close_on_escape: bool,
}

/// Extension shortcut (`setShortcuts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Shortcut id.
    pub id: String,
    /// Declared key ids.
    pub keys: Vec<String>,
    /// Description.
    pub description: String,
    /// Extension.
    pub extension_id: String,
}

/// Option of a `select` dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    /// Label (the answer value for `select`).
    pub label: String,
    /// Description.
    pub description: Option<String>,
    /// Value (`multiSelect`, falls back to label).
    pub value: Option<String>,
    /// Pre-selected (`multiSelect`).
    pub selected: bool,
}

/// Form question option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormOption {
    /// Label.
    pub label: String,
    /// Description.
    pub description: Option<String>,
    /// Value (falls back to label).
    pub value: Option<String>,
    /// `end_turn` | `new_session`.
    pub action: Option<String>,
    /// Text placed into the composer.
    pub prefill: Option<String>,
}

/// Form question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormQuestion {
    /// Question id.
    pub id: String,
    /// Question text.
    pub question: String,
    /// Short header.
    pub header: Option<String>,
    /// Options (0–9).
    pub options: Vec<FormOption>,
    /// Multiple choice.
    pub multi_select: bool,
    /// Free-text "Other".
    pub other: bool,
}

/// Approval option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalOption {
    /// Option id.
    pub option_id: String,
    /// Label.
    pub label: String,
    /// `allow_once` | `allow_always` | `reject_once` | `reject_always`.
    pub kind: String,
}

/// Typed dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogKind {
    /// `select`.
    Select {
        /// Title.
        title: String,
        /// Options.
        options: Vec<ChoiceOption>,
    },
    /// `confirm`.
    Confirm {
        /// Title.
        title: String,
        /// Message.
        message: String,
    },
    /// `input`.
    Input {
        /// Title.
        title: String,
        /// Placeholder.
        placeholder: Option<String>,
        /// Prefill.
        prefill: Option<String>,
        /// Mask input.
        secret: bool,
    },
    /// `editor`.
    Editor {
        /// Title.
        title: String,
        /// Prefill.
        prefill: String,
        /// Language hint.
        language: Option<String>,
    },
    /// `multiSelect`.
    MultiSelect {
        /// Title.
        title: String,
        /// Options.
        options: Vec<ChoiceOption>,
        /// Minimum selections.
        min: Option<usize>,
        /// Maximum selections.
        max: Option<usize>,
    },
    /// `form`.
    Form {
        /// Title.
        title: String,
        /// Questions.
        questions: Vec<FormQuestion>,
        /// Submit label.
        submit_label: Option<String>,
    },
    /// `approval`.
    Approval {
        /// Title.
        title: String,
        /// Tool name.
        tool_name: Option<String>,
        /// Summary.
        summary: String,
        /// Detail spec.
        detail: Option<OusNode>,
        /// Options.
        options: Vec<ApprovalOption>,
        /// Feedback placeholder (feedback enabled when `Some`).
        feedback: Option<String>,
    },
    /// `custom`.
    Custom {
        /// Title.
        title: Option<String>,
        /// Spec.
        spec: OusNode,
        /// `small` | `medium` | `large` | `full`.
        size: Option<String>,
    },
    /// `clipboardReadImage`.
    ClipboardReadImage,
    /// `editExternally`.
    EditExternally {
        /// Text.
        text: String,
        /// File extension hint.
        extension_hint: Option<String>,
    },
    /// A dialog method this frontend does not know (answered as cancelled).
    Unsupported {
        /// Method.
        method: String,
    },
}

/// Queued dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    /// Request id.
    pub id: String,
    /// Owning extension.
    pub extension_id: Option<String>,
    /// Timeout (ms) declared by the request.
    pub timeout_ms: Option<u64>,
    /// Content.
    pub kind: DialogKind,
}

fn choice_options(params: &JsonObject) -> Vec<ChoiceOption> {
    let details = arr(params, "optionDetails");
    arr(params, "options")
        .iter()
        .enumerate()
        .map(|(i, option)| match option {
            JsonValue::String(label) => ChoiceOption {
                label: label.clone(),
                description: details.get(i).and_then(|d| vs(d, "description")),
                value: None,
                selected: false,
            },
            other => ChoiceOption {
                label: vs(other, "label").unwrap_or_default(),
                description: vs(other, "description"),
                value: vs(other, "value"),
                selected: other
                    .get("selected")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
            },
        })
        .collect()
}

fn usize_param(params: &JsonObject, key: &str) -> Option<usize> {
    params
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

/// Parses a dialog request.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one arm per ui-protocol dialog method"
)]
pub fn parse_dialog(request: &UiRequest) -> Dialog {
    let p = &request.params;
    let title = s(p, "title").unwrap_or_default();
    let kind = match request.method.as_str() {
        "select" => DialogKind::Select {
            title,
            options: choice_options(p),
        },
        "confirm" => DialogKind::Confirm {
            title,
            message: s(p, "message").unwrap_or_default(),
        },
        "input" => DialogKind::Input {
            title,
            placeholder: s(p, "placeholder"),
            prefill: s(p, "prefill"),
            secret: b(p, "secret"),
        },
        "editor" => DialogKind::Editor {
            title,
            prefill: s(p, "prefill").unwrap_or_default(),
            language: s(p, "language"),
        },
        "multiSelect" => DialogKind::MultiSelect {
            title,
            options: choice_options(p),
            min: usize_param(p, "min"),
            max: usize_param(p, "max"),
        },
        "form" => DialogKind::Form {
            title,
            questions: arr(p, "questions")
                .iter()
                .map(|q| FormQuestion {
                    id: vs(q, "id").unwrap_or_default(),
                    question: vs(q, "question").unwrap_or_default(),
                    header: vs(q, "header"),
                    options: q
                        .get("options")
                        .and_then(JsonValue::as_array)
                        .map(|options| {
                            options
                                .iter()
                                .take(9)
                                .map(|o| FormOption {
                                    label: vs(o, "label").unwrap_or_default(),
                                    description: vs(o, "description"),
                                    value: vs(o, "value"),
                                    action: vs(o, "action"),
                                    prefill: vs(o, "prefill"),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    multi_select: q
                        .get("multiSelect")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false),
                    other: q.get("other").and_then(JsonValue::as_bool).unwrap_or(false),
                })
                .collect(),
            submit_label: s(p, "submitLabel"),
        },
        "approval" => DialogKind::Approval {
            title,
            tool_name: s(p, "toolName"),
            summary: s(p, "summary").unwrap_or_default(),
            detail: p.get("detail").map(OusNode::from_json),
            options: arr(p, "options")
                .iter()
                .map(|o| ApprovalOption {
                    option_id: vs(o, "optionId").unwrap_or_default(),
                    label: vs(o, "label").unwrap_or_default(),
                    kind: vs(o, "kind").unwrap_or_default(),
                })
                .collect(),
            feedback: p
                .get("feedback")
                .map(|f| vs(f, "placeholder").unwrap_or_else(|| "Feedback (optional)".into())),
        },
        "custom" => DialogKind::Custom {
            title: s(p, "title"),
            spec: p
                .get("spec")
                .map_or(OusNode::Empty { message: None }, OusNode::from_json),
            size: s(p, "size"),
        },
        "clipboardReadImage" => DialogKind::ClipboardReadImage,
        "editExternally" => DialogKind::EditExternally {
            text: s(p, "text").unwrap_or_default(),
            extension_hint: s(p, "extensionHint"),
        },
        other => DialogKind::Unsupported {
            method: other.to_owned(),
        },
    };
    Dialog {
        id: request.id.clone(),
        extension_id: request.extension_id.clone(),
        timeout_ms: p.get("timeout").and_then(JsonValue::as_u64),
        kind,
    }
}

fn segments(params: &JsonObject, key: &str) -> Vec<Segment> {
    arr(params, key)
        .iter()
        .map(|seg| Segment {
            text: vs(seg, "text").unwrap_or_default(),
            tone: Tone::parse(seg.get("tone").and_then(JsonValue::as_str).unwrap_or("")),
            icon: vs(seg, "icon"),
            priority: seg.get("priority").and_then(JsonValue::as_i64).unwrap_or(0),
            align: if seg.get("align").and_then(JsonValue::as_str) == Some("right") {
                SegmentAlign::Right
            } else {
                SegmentAlign::Left
            },
            min_width: seg
                .get("minWidth")
                .and_then(JsonValue::as_u64)
                .and_then(|n| u16::try_from(n).ok()),
        })
        .collect()
}

fn panel_placement(params: &JsonObject) -> PanelPlacement {
    match params.get("placement").and_then(JsonValue::as_str) {
        Some("sidePane") => PanelPlacement::SidePane,
        Some("editorReplace") => PanelPlacement::EditorReplace,
        Some("bottomPane") => PanelPlacement::BottomPane,
        Some("fullscreen") => PanelPlacement::Fullscreen,
        _ => PanelPlacement::Overlay,
    }
}

fn panel_keys(params: &JsonObject) -> Vec<PanelKey> {
    arr(params, "keys")
        .iter()
        .map(|k| PanelKey {
            key: vs(k, "key").unwrap_or_default(),
            action_id: vs(k, "actionId").unwrap_or_default(),
            label: vs(k, "label").unwrap_or_default(),
        })
        .collect()
}

/// The one-shot effect of a fire-and-forget request (`notify`, `set_editor_text`, `openUrl`,
/// clipboard, OS notifications, settings, theme); `None` for retained or unknown methods.
fn one_shot(request: &UiRequest) -> Option<UiEffect> {
    let p = &request.params;
    Some(match request.method.as_str() {
        "notify" => UiEffect::Notify {
            level: match p.get("notifyType").and_then(JsonValue::as_str) {
                Some("warning") => NotifyLevel::Warning,
                Some("error") => NotifyLevel::Error,
                _ => NotifyLevel::Info,
            },
            message: s(p, "message").unwrap_or_default(),
        },
        "set_editor_text" => UiEffect::SetEditorText(s(p, "text").unwrap_or_default()),
        "insertEditorText" => UiEffect::InsertEditorText(s(p, "text").unwrap_or_default()),
        "insertEditorToken" => {
            let token = p.get("token").cloned().unwrap_or(JsonValue::Null);
            UiEffect::InsertToken(EditorToken {
                token_id: vs(&token, "tokenId").unwrap_or_default(),
                kind: vs(&token, "kind").unwrap_or_else(|| "text".into()),
                label: vs(&token, "label").unwrap_or_default(),
                data: token
                    .get("data")
                    .and_then(|d| serde_json::from_value(d.clone()).ok()),
                path: vs(&token, "path"),
            })
        }
        "osNotify" => UiEffect::OsNotify {
            title: s(p, "title").unwrap_or_default(),
            body: s(p, "body").unwrap_or_default(),
        },
        "openUrl" => UiEffect::OpenUrl(s(p, "url").unwrap_or_default()),
        "clipboardWrite" => UiEffect::ClipboardWrite(s(p, "text").unwrap_or_default()),
        "openSettings" => UiEffect::OpenSettings {
            namespace: s(p, "namespace"),
            query: s(p, "query"),
        },
        "setTheme" => UiEffect::SetTheme(s(p, "name").unwrap_or_default()),
        _ => return None,
    })
}

/// Retained extension UI state of one session.
#[derive(Clone, Debug, Default)]
pub struct UiState {
    /// Status entries by effective key.
    pub statuses: IndexMap<String, StatusItem>,
    /// Widgets by effective key.
    pub widgets: IndexMap<String, Widget>,
    /// Frames by effective key.
    pub frames: IndexMap<String, Frame>,
    /// Header specs by effective key.
    pub headers: IndexMap<String, OusNode>,
    /// Footer specs by effective key.
    pub footers: IndexMap<String, OusNode>,
    /// Terminal/window title.
    pub title: Option<String>,
    /// Working message override.
    pub working_message: Option<String>,
    /// Working indicator visibility (pi `setWorkingVisible`).
    pub working_visible: bool,
    /// Open panels; the last is on top.
    pub panels: Vec<Panel>,
    /// Pending dialogs; the front is shown.
    pub dialogs: VecDeque<Dialog>,
    /// Extension shortcuts.
    pub shortcuts: Vec<Shortcut>,
}

impl UiState {
    /// Empty state (working indicator visible).
    #[must_use]
    pub fn new() -> Self {
        Self {
            working_visible: true,
            ..Self::default()
        }
    }

    /// Applies one request; returns a one-shot effect when the method has one.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per ui-protocol §4 fire-and-forget method"
    )]
    pub fn apply(&mut self, request: &UiRequest) -> Option<UiEffect> {
        let p = &request.params;
        if oino_core::ui::is_dialog(&request.method) {
            if !self.dialogs.iter().any(|d| d.id == request.id) {
                self.dialogs.push_back(parse_dialog(request));
            }
            return None;
        }
        if let Some(effect) = one_shot(request) {
            return Some(effect);
        }
        match request.method.as_str() {
            "setStatus" => {
                let key = scoped(request, &s(p, "statusKey").unwrap_or_default());
                match s(p, "statusText") {
                    Some(text) => {
                        self.statuses.insert(
                            key,
                            StatusItem {
                                text,
                                tone: Tone::parse(
                                    p.get("tone").and_then(JsonValue::as_str).unwrap_or(""),
                                ),
                                priority: p
                                    .get("priority")
                                    .and_then(JsonValue::as_i64)
                                    .unwrap_or(0),
                            },
                        );
                    }
                    None => {
                        self.statuses.shift_remove(&key);
                    }
                }
            }
            "setWidget" => {
                let key = scoped(request, &s(p, "widgetKey").unwrap_or_default());
                let lines = p.get("widgetLines").and_then(JsonValue::as_array);
                let spec = p.get("widgetSpec").filter(|v| !v.is_null());
                if lines.is_none() && spec.is_none() {
                    self.widgets.shift_remove(&key);
                } else {
                    let placement = match p.get("widgetPlacement").and_then(JsonValue::as_str) {
                        Some("belowEditor") => Placement::BelowEditor,
                        Some("header") => Placement::Header,
                        Some("footer") => Placement::Footer,
                        _ => Placement::AboveEditor,
                    };
                    self.widgets.insert(
                        key,
                        Widget {
                            placement,
                            lines: lines
                                .map(|l| {
                                    l.iter()
                                        .map(|v| v.as_str().unwrap_or_default().to_owned())
                                        .collect()
                                })
                                .unwrap_or_default(),
                            spec: spec.map(OusNode::from_json),
                        },
                    );
                }
            }
            "setFrame" => {
                let key = scoped(request, &s(p, "frameKey").unwrap_or_default());
                if p.get("top").is_none() && p.get("bottom").is_none() {
                    self.frames.shift_remove(&key);
                } else {
                    self.frames.insert(
                        key,
                        Frame {
                            top: segments(p, "top"),
                            bottom: segments(p, "bottom"),
                        },
                    );
                }
            }
            "setHeader" | "setFooter" => {
                let key = scoped(request, &s(p, "key").unwrap_or_default());
                let target = if request.method == "setHeader" {
                    &mut self.headers
                } else {
                    &mut self.footers
                };
                match p.get("spec").filter(|v| !v.is_null()) {
                    Some(spec) => {
                        target.insert(key, OusNode::from_json(spec));
                    }
                    None => {
                        target.shift_remove(&key);
                    }
                }
            }
            "setTitle" => self.title = s(p, "title"),
            "setWorkingMessage" => {
                if p.contains_key("message") {
                    self.working_message = s(p, "message");
                }
                if let Some(visible) = p.get("visible").and_then(JsonValue::as_bool) {
                    self.working_visible = visible;
                }
            }
            "openPanel" => {
                let panel_id = s(p, "panelId").unwrap_or_default();
                self.panels.retain(|panel| panel.panel_id != panel_id);
                self.panels.push(Panel {
                    panel_id,
                    extension_id: request.extension_id.clone(),
                    placement: panel_placement(p),
                    title: s(p, "title"),
                    spec: p
                        .get("spec")
                        .map_or(OusNode::Empty { message: None }, OusNode::from_json),
                    keys: panel_keys(p),
                    focus: p.get("focus").and_then(JsonValue::as_bool).unwrap_or(true),
                    close_on_escape: p
                        .get("closeOnEscape")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(true),
                });
            }
            "updatePanel" => {
                let panel_id = s(p, "panelId").unwrap_or_default();
                if let Some(panel) = self
                    .panels
                    .iter_mut()
                    .find(|panel| panel.panel_id == panel_id)
                {
                    if let Some(spec) = p.get("spec") {
                        panel.spec = OusNode::from_json(spec);
                    }
                    if p.contains_key("title") {
                        panel.title = s(p, "title");
                    }
                    if p.contains_key("keys") {
                        panel.keys = panel_keys(p);
                    }
                }
            }
            "closePanel" => {
                let panel_id = s(p, "panelId").unwrap_or_default();
                self.panels.retain(|panel| panel.panel_id != panel_id);
            }
            "setShortcuts" => {
                self.shortcuts = arr(p, "shortcuts")
                    .iter()
                    .map(|sc| Shortcut {
                        id: vs(sc, "id").unwrap_or_default(),
                        keys: match sc.get("keys") {
                            Some(JsonValue::String(k)) => vec![k.clone()],
                            Some(JsonValue::Array(keys)) => keys
                                .iter()
                                .filter_map(|k| k.as_str().map(str::to_owned))
                                .collect(),
                            _ => Vec::new(),
                        },
                        description: vs(sc, "description").unwrap_or_default(),
                        extension_id: vs(sc, "extensionId")
                            .or_else(|| request.extension_id.clone())
                            .unwrap_or_default(),
                    })
                    .collect();
            }
            _ => {}
        }
        None
    }

    /// Removes a dialog (answered here or dismissed by core).
    pub fn dismiss(&mut self, id: &str) -> Option<Dialog> {
        let index = self.dialogs.iter().position(|d| d.id == id)?;
        self.dialogs.remove(index)
    }

    /// The dialog to show.
    #[must_use]
    pub fn active_dialog(&self) -> Option<&Dialog> {
        self.dialogs.front()
    }

    /// The top panel that takes focus.
    #[must_use]
    pub fn focused_panel(&self) -> Option<&Panel> {
        self.panels.iter().rev().find(|p| p.focus)
    }

    /// Statuses sorted for display: priority (higher first), then key (pi sorts by key).
    #[must_use]
    pub fn sorted_statuses(&self) -> Vec<(&str, &StatusItem)> {
        let mut items: Vec<(&str, &StatusItem)> =
            self.statuses.iter().map(|(k, v)| (k.as_str(), v)).collect();
        items.sort_by(|a, b| b.1.priority.cmp(&a.1.priority).then_with(|| a.0.cmp(b.0)));
        items
    }

    /// Widgets of `placement` in insertion order.
    #[must_use]
    pub fn widgets_at(&self, placement: Placement) -> Vec<&Widget> {
        self.widgets
            .values()
            .filter(|w| w.placement == placement)
            .collect()
    }

    /// Clears everything (session replaced).
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

// ── registry mirror (§6.1, §6.4, §9.2) ───────────────────────────────────────

/// One `turnFooter` cache invalidation (§6.7): drop the cached footers of `entry_id`, of
/// `turns` only when set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnFooterInvalidation {
    /// The `turnFooter` entry whose caches are stale.
    pub entry_id: String,
    /// Turn entry ids to drop; absent = every turn.
    pub turns: Option<Vec<String>>,
}

/// Why a registry update was refused. The frontend answers by sending
/// `_oino/ui_resync{scope, rev}` (§9.2) and replacing its state with the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorError {
    /// The envelope belongs to the other registry scope.
    Scope {
        /// The mirror's scope.
        expected: Scope,
        /// The envelope's scope.
        got: Scope,
    },
    /// `rev` is not the previous revision + 1 (or the patch was already applied).
    RevGap {
        /// `last + 1`.
        expected: u64,
        /// The envelope's `rev`.
        got: u64,
    },
    /// An op addressed an entry the mirror does not hold.
    MissingEntry(String),
    /// A `tree` op addressed an entry whose view is not a tree.
    NotATree(String),
    /// A node op could not be applied to the retained tree (§4.7).
    Patch {
        /// Entry id.
        id: String,
        /// The op error (diagnostic text of §4.8).
        message: String,
    },
}

impl MirrorError {
    fn patch(id: &str, error: PatchError) -> Self {
        Self::Patch {
            id: id.to_owned(),
            message: error.to_string(),
        }
    }
}

/// What one applied patch produced for the frontend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatchOutcome {
    /// One-shot effects (`notify`, `set_editor_text`, `openUrl`, …); never retained (§2 L0).
    pub effects: Vec<UiEffect>,
    /// `turnFooter` caches to drop (§6.7).
    pub invalidations: Vec<TurnFooterInvalidation>,
}

/// What a reconciled snapshot changed relative to the mirror.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotChange {
    /// Entries that appeared.
    pub added: Vec<String>,
    /// Entries whose content differs (re-render them).
    pub changed: Vec<String>,
    /// Entries that are gone: close their view and drop local state (§6.6).
    pub removed: Vec<String>,
}

/// Open dialogs in arrival order (§3.2: they queue behind each other and show "1 of N").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DialogQueue {
    dialogs: VecDeque<Dialog>,
}

impl DialogQueue {
    /// Queues a dialog; `false` when one with the same id is already queued (first answer
    /// wins, so a repeated `dialog` op changes nothing).
    pub fn push(&mut self, dialog: Dialog) -> bool {
        if self.dialogs.iter().any(|queued| queued.id == dialog.id) {
            return false;
        }
        self.dialogs.push_back(dialog);
        true
    }

    /// Removes a dialog (answered here or dismissed by core).
    pub fn remove(&mut self, id: &str) -> Option<Dialog> {
        let index = self.dialogs.iter().position(|dialog| dialog.id == id)?;
        self.dialogs.remove(index)
    }

    /// The dialog to show (the oldest one).
    #[must_use]
    pub fn active(&self) -> Option<&Dialog> {
        self.dialogs.front()
    }

    /// The "1 of N" pager of the front dialog: 1-based position and queue length.
    #[must_use]
    pub fn position(&self) -> Option<(usize, usize)> {
        (!self.dialogs.is_empty()).then_some((1, self.dialogs.len()))
    }

    /// Queued dialogs in arrival order.
    pub fn iter(&self) -> impl Iterator<Item = &Dialog> {
        self.dialogs.iter()
    }

    /// Number of queued dialogs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dialogs.len()
    }

    /// Whether no dialog is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dialogs.is_empty()
    }

    /// Empties the queue.
    pub fn clear(&mut self) {
        self.dialogs.clear();
    }
}

/// Frontend mirror of one registry scope (§9.2): a `UiSnapshot` plus a stream of `UiPatch`es
/// with a monotonic `rev`.
///
/// Entries are read per slot in registry order (`order` desc, then `seq` asc, §6.4); stack
/// slots (`overlay`, `editorReplace`) use [`stack`](Self::stack). The dialog queue and the
/// registry meta live here too, so a frontend holds one value per scope.
#[derive(Clone, Debug, Default)]
pub struct RegistryMirror {
    scope: Scope,
    session_id: Option<String>,
    rev: u64,
    entries: IndexMap<String, UiEntry>,
    dialogs: DialogQueue,
    meta: UiMeta,
}

impl RegistryMirror {
    /// An empty mirror (`Scope::Session` unless set otherwise).
    #[must_use]
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            ..Self::default()
        }
    }

    /// Registry scope of this mirror.
    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Revision of the state (`rev != last + 1` on the next patch ⇒ resync, §9.2).
    #[must_use]
    pub const fn rev(&self) -> u64 {
        self.rev
    }

    /// Session the registry belongs to.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// One entry by id.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&UiEntry> {
        self.entries.get(id)
    }

    /// Entries in registration order (registry order not implied; sort with
    /// [`entries_for`](Self::entries_for), §9.2).
    pub fn entries(&self) -> impl Iterator<Item = &UiEntry> {
        self.entries.values()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the scope holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries of one slot in registry order (`order` desc, then `seq` asc, §6.4).
    #[must_use]
    pub fn entries_for(&self, slot: Slot) -> Vec<&UiEntry> {
        let mut entries: Vec<&UiEntry> = self
            .entries
            .values()
            .filter(|entry| entry.slot == slot)
            .collect();
        entries.sort_by(|a, b| b.order.cmp(&a.order).then(a.seq.cmp(&b.seq)));
        entries
    }

    /// Entries of a stack slot bottom → top: most recently opened or focused last (§6.4).
    /// The last element is what a frontend shows.
    #[must_use]
    pub fn stack(&self, slot: Slot) -> Vec<&UiEntry> {
        let mut entries: Vec<&UiEntry> = self
            .entries
            .values()
            .filter(|entry| entry.slot == slot)
            .collect();
        entries.sort_by_key(|entry| (entry.focus == Some(true), entry.seq));
        entries
    }

    /// The visible entry of a stack slot, if any.
    #[must_use]
    pub fn stack_top(&self, slot: Slot) -> Option<&UiEntry> {
        self.stack(slot).pop()
    }

    /// Registry meta: title, working message/visibility, shortcuts (§9.2).
    #[must_use]
    pub const fn meta(&self) -> &UiMeta {
        &self.meta
    }

    /// Open dialogs.
    #[must_use]
    pub const fn dialogs(&self) -> &DialogQueue {
        &self.dialogs
    }

    /// The dialog to show (§3.2).
    #[must_use]
    pub fn active_dialog(&self) -> Option<&Dialog> {
        self.dialogs.active()
    }

    /// Whether a patch at `rev` must be refused because updates were missed (§9.2).
    #[must_use]
    pub const fn needs_resync(&self, rev: u64) -> bool {
        rev != self.rev.saturating_add(1)
    }

    /// Removes a dialog answered locally (core dismisses it for every frontend separately).
    pub fn dismiss_dialog(&mut self, id: &str) -> Option<Dialog> {
        self.dialogs.remove(id)
    }

    /// Reconciles the mirror with a full snapshot (first load and `_oino/ui_resync`, §9.3).
    ///
    /// Entries are matched by id, so local state of surviving entries lives on; a snapshot of
    /// another session replaces the registry. Dialogs answered locally but still open in core
    /// reappear here until core dismisses them (first answer wins, §3).
    ///
    /// # Errors
    ///
    /// [`MirrorError::Scope`] when the snapshot belongs to the other scope.
    pub fn apply_snapshot(&mut self, snapshot: UiSnapshot) -> Result<SnapshotChange, MirrorError> {
        if snapshot.scope != self.scope {
            return Err(MirrorError::Scope {
                expected: self.scope,
                got: snapshot.scope,
            });
        }
        let mut change = SnapshotChange::default();
        if self.session_id.is_some() && self.session_id != snapshot.session_id {
            change.removed.extend(self.entries.keys().cloned());
            self.entries.clear();
            self.dialogs.clear();
        }
        let present: std::collections::HashSet<String> = snapshot
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        for entry in snapshot.entries {
            match self.entries.get(&entry.id) {
                None => change.added.push(entry.id.clone()),
                Some(old) if *old != entry => change.changed.push(entry.id.clone()),
                Some(_) => {}
            }
            self.entries.insert(entry.id.clone(), entry);
        }
        self.entries.retain(|id, _| {
            let keep = present.contains(id);
            if !keep {
                change.removed.push(id.clone());
            }
            keep
        });
        self.dialogs.clear();
        for request in &snapshot.dialogs {
            self.dialogs.push(parse_registry_dialog(request));
        }
        self.session_id = snapshot.session_id;
        self.meta = snapshot.meta;
        self.rev = snapshot.rev;
        Ok(change)
    }

    /// Applies one patch atomically: either every op is applicable and the state advances to
    /// `patch.rev`, or nothing changes and the caller resyncs (§9.2).
    ///
    /// # Errors
    ///
    /// [`MirrorError`] for a wrong scope, a revision gap and the ops that address missing or
    /// non-tree entries with unusable node ops.
    pub fn apply_patch(&mut self, patch: &UiPatch) -> Result<PatchOutcome, MirrorError> {
        if patch.scope != self.scope {
            return Err(MirrorError::Scope {
                expected: self.scope,
                got: patch.scope,
            });
        }
        if self.needs_resync(patch.rev) {
            return Err(MirrorError::RevGap {
                expected: self.rev.saturating_add(1),
                got: patch.rev,
            });
        }
        let prepared = patch
            .ops
            .iter()
            .map(|op| self.prepare(op))
            .collect::<Result<Vec<_>, _>>()?;
        let mut outcome = PatchOutcome::default();
        for step in prepared {
            self.commit(step, &mut outcome);
        }
        self.rev = patch.rev;
        Ok(outcome)
    }

    /// Validates and computes one op without touching the mirror (atomicity).
    fn prepare(&self, op: &RegistryOp) -> Result<Prepared, MirrorError> {
        Ok(match op {
            RegistryOp::Put { entry } => Prepared::Put(Box::new(entry.clone())),
            RegistryOp::Del { id } => Prepared::Del(id.clone()),
            RegistryOp::Meta { id, set } => {
                if !self.entries.contains_key(id) {
                    return Err(MirrorError::MissingEntry(id.clone()));
                }
                Prepared::Meta(id.clone(), set.clone())
            }
            RegistryOp::Tree { id, rev, ops } => {
                let Some(entry) = self.entries.get(id) else {
                    return Err(MirrorError::MissingEntry(id.clone()));
                };
                let UiView::Tree { root } = &entry.view else {
                    return Err(MirrorError::NotATree(id.clone()));
                };
                let root =
                    apply(root, ops).map_err(|error| MirrorError::patch(id, error))?;
                Prepared::Tree {
                    id: id.clone(),
                    rev: *rev,
                    root: Box::new(root),
                }
            }
            RegistryOp::Invalidate { id, turns } => {
                Prepared::Invalidate(TurnFooterInvalidation {
                    entry_id: id.clone(),
                    turns: turns.clone(),
                })
            }
            RegistryOp::Dialog { request } => {
                Prepared::Dialog(Box::new(parse_registry_dialog(request)))
            }
            RegistryOp::Dismiss { id, .. } => Prepared::Dismiss(id.clone()),
            RegistryOp::SessionMeta { set } => Prepared::SessionMeta(set.clone()),
            RegistryOp::Effect { request } => Prepared::Effect(effect_from_registry(request)),
        })
    }

    /// Applies one prepared op.
    fn commit(&mut self, step: Prepared, outcome: &mut PatchOutcome) {
        match step {
            Prepared::Put(entry) => {
                self.entries.insert(entry.id.clone(), *entry);
            }
            Prepared::Del(id) => {
                self.entries.shift_remove(&id);
            }
            Prepared::Meta(id, set) => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    if let Some(title) = set.title {
                        entry.title = Some(title);
                    }
                    if let Some(icon) = set.icon {
                        entry.icon = Some(icon);
                    }
                    if let Some(badge) = set.badge {
                        entry.badge = Some(badge);
                    }
                    if let Some(order) = set.order {
                        entry.order = order;
                    }
                    if let Some(keys) = set.keys {
                        entry.keys = keys;
                    }
                    if let Some(tone) = set.tone {
                        entry.tone = Some(tone);
                    }
                    if let Some(focus) = set.focus {
                        entry.focus = Some(focus);
                    }
                    // `meta` carries no `rev` (§9.2); core advances the entry rev on every
                    // change (§6.1), so the mirror does the same for its cache keys.
                    entry.rev = entry.rev.saturating_add(1);
                }
            }
            Prepared::Tree { id, rev, root } => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.view = UiView::Tree { root: *root };
                    entry.rev = rev;
                }
            }
            Prepared::Invalidate(invalidation) => outcome.invalidations.push(invalidation),
            Prepared::Dialog(dialog) => {
                self.dialogs.push(*dialog);
            }
            Prepared::Dismiss(id) => {
                self.dialogs.remove(&id);
            }
            Prepared::SessionMeta(set) => {
                if let Some(title) = set.title {
                    self.meta.title = Some(title);
                }
                if let Some(message) = set.working_message {
                    self.meta.working_message = Some(message);
                }
                if let Some(visible) = set.working_visible {
                    self.meta.working_visible = Some(visible);
                }
                if let Some(shortcuts) = set.shortcuts {
                    self.meta.shortcuts = shortcuts;
                }
            }
            Prepared::Effect(effect) => {
                if let Some(effect) = effect {
                    outcome.effects.push(effect);
                }
            }
        }
    }
}

/// One validated patch op, ready to commit (§9.2 `RegistryOp`).
enum Prepared {
    Put(Box<UiEntry>),
    Del(String),
    Meta(String, EntryMetaPatch),
    Tree {
        id: String,
        rev: u64,
        root: Box<oino_ui::node::Node>,
    },
    Invalidate(TurnFooterInvalidation),
    Dialog(Box<Dialog>),
    Dismiss(String),
    SessionMeta(UiMetaPatch),
    Effect(Option<UiEffect>),
}

/// The pi-shaped request behind a registry-form request (`{id, method, …params}`, §9.2).
fn registry_request(request: &oino_ui::slot::UiRequest) -> Option<UiRequest> {
    let mut fields = request.fields.clone();
    let method = fields.remove("method")?.as_str()?.to_owned();
    let extension_id = fields
        .remove("extensionId")
        .and_then(|value| value.as_str().map(str::to_owned));
    let session_id = fields
        .remove("sessionId")
        .and_then(|value| value.as_str().map(str::to_owned));
    Some(UiRequest {
        id: request.id.clone(),
        method,
        params: fields,
        extension_id,
        session_id,
    })
}

/// Parses a registry-form dialog (`UiSnapshot.dialogs`, `op:"dialog"`) into the typed dialog a
/// frontend renders.
#[must_use]
pub fn parse_registry_dialog(request: &oino_ui::slot::UiRequest) -> Dialog {
    registry_request(request).map_or_else(
        || Dialog {
            id: request.id.clone(),
            extension_id: None,
            timeout_ms: None,
            kind: DialogKind::Unsupported {
                method: String::new(),
            },
        },
        |request| parse_dialog(&request),
    )
}

/// The one-shot effect of a registry-form `effect` op; `None` for retained or unknown
/// methods.
fn effect_from_registry(request: &oino_ui::slot::UiRequest) -> Option<UiEffect> {
    registry_request(request).as_ref().and_then(one_shot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(method: &str, params: JsonValue) -> UiRequest {
        UiRequest {
            id: format!("id-{method}"),
            method: method.into(),
            params: serde_json::from_value(params).unwrap(),
            extension_id: Some("ext".into()),
            session_id: None,
        }
    }

    #[test]
    fn retained_state_and_effects() {
        let mut ui = UiState::new();
        ui.apply(&request(
            "setStatus",
            json!({"statusKey":"a","statusText":"one","priority":1}),
        ));
        ui.apply(&request(
            "setStatus",
            json!({"statusKey":"b","statusText":"two","priority":5}),
        ));
        assert_eq!(ui.sorted_statuses()[0].1.text, "two");
        ui.apply(&request("setStatus", json!({"statusKey":"b"})));
        assert_eq!(ui.statuses.len(), 1);
        ui.apply(&request(
            "setWidget",
            json!({"widgetKey":"w","widgetLines":["x"],"widgetPlacement":"belowEditor"}),
        ));
        assert_eq!(ui.widgets_at(Placement::BelowEditor).len(), 1);
        ui.apply(&request("setWidget", json!({"widgetKey":"w"})));
        assert!(ui.widgets.is_empty());
        assert_eq!(
            ui.apply(&request(
                "notify",
                json!({"message":"hi","notifyType":"warning"})
            )),
            Some(UiEffect::Notify {
                level: NotifyLevel::Warning,
                message: "hi".into()
            })
        );
        ui.apply(&request(
            "openPanel",
            json!({"panelId":"ext/p","spec":{"t":"text","value":"x"},"placement":"editorReplace"}),
        ));
        ui.apply(&request(
            "updatePanel",
            json!({"panelId":"ext/p","title":"T"}),
        ));
        assert_eq!(ui.panels[0].title.as_deref(), Some("T"));
        assert_eq!(ui.panels[0].placement, PanelPlacement::EditorReplace);
        ui.apply(&request("closePanel", json!({"panelId":"ext/p"})));
        assert_eq!(ui.panels.len(), 0);
    }

    #[test]
    fn dialogs_queue_and_dismiss() {
        let mut ui = UiState::new();
        ui.apply(&request("select", json!({"title":"Pick","options":["a","b"],"optionDetails":[{"label":"a","description":"first"}]})));
        let mut second = request("confirm", json!({"title":"Sure?","message":"really"}));
        second.id = "c2".into();
        ui.apply(&second);
        assert_eq!(ui.dialogs.len(), 2);
        let DialogKind::Select { options, .. } = &ui.active_dialog().unwrap().kind else {
            panic!()
        };
        assert_eq!(options[0].description.as_deref(), Some("first"));
        ui.dismiss("id-select");
        assert!(matches!(
            ui.active_dialog().unwrap().kind,
            DialogKind::Confirm { .. }
        ));
    }
}
