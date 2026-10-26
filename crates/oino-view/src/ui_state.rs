//! Extension UI state (ui-protocol §2–§5): the reducer every frontend runs over
//! `extension_ui_request` / `extension_ui_dismiss`.
//!
//! Fire-and-forget methods update retained state (status by key, widgets by placement,
//! frames, header/footer, title, working message, panels, shortcuts); dialogs queue up and
//! leave when answered locally or dismissed by core (first answer wins). One-shot effects
//! (notify, editor text, OS notifications, clipboard, URLs) are returned to the caller.

use std::collections::VecDeque;

use indexmap::IndexMap;
use oino_ai::{ImageContent, JsonObject, JsonValue};
use oino_core::ui::UiRequest;

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
        match request.method.as_str() {
            "notify" => {
                let level = match p.get("notifyType").and_then(JsonValue::as_str) {
                    Some("warning") => NotifyLevel::Warning,
                    Some("error") => NotifyLevel::Error,
                    _ => NotifyLevel::Info,
                };
                return Some(UiEffect::Notify {
                    level,
                    message: s(p, "message").unwrap_or_default(),
                });
            }
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
            "set_editor_text" => {
                return Some(UiEffect::SetEditorText(s(p, "text").unwrap_or_default()));
            }
            "insertEditorText" => {
                return Some(UiEffect::InsertEditorText(s(p, "text").unwrap_or_default()));
            }
            "insertEditorToken" => {
                let token = p.get("token").cloned().unwrap_or(JsonValue::Null);
                return Some(UiEffect::InsertToken(EditorToken {
                    token_id: vs(&token, "tokenId").unwrap_or_default(),
                    kind: vs(&token, "kind").unwrap_or_else(|| "text".into()),
                    label: vs(&token, "label").unwrap_or_default(),
                    data: token
                        .get("data")
                        .and_then(|d| serde_json::from_value(d.clone()).ok()),
                    path: vs(&token, "path"),
                }));
            }
            "osNotify" => {
                return Some(UiEffect::OsNotify {
                    title: s(p, "title").unwrap_or_default(),
                    body: s(p, "body").unwrap_or_default(),
                });
            }
            "openUrl" => return Some(UiEffect::OpenUrl(s(p, "url").unwrap_or_default())),
            "clipboardWrite" => {
                return Some(UiEffect::ClipboardWrite(s(p, "text").unwrap_or_default()));
            }
            "openSettings" => {
                return Some(UiEffect::OpenSettings {
                    namespace: s(p, "namespace"),
                    query: s(p, "query"),
                });
            }
            "setTheme" => return Some(UiEffect::SetTheme(s(p, "name").unwrap_or_default())),
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
