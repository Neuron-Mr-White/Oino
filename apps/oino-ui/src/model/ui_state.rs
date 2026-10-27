//! Extension UI state of one session (ui-protocol §3–§5, §8): retained fire-and-forget
//! state, the dialog queue and one-shot effects.
//!
//! Keys are scoped per extension (`<extensionId>/<key>`). Dialogs queue in arrival order and
//! leave the queue when answered locally or dismissed by core (another frontend answered,
//! timeout, cancel). One-shot requests (`notify`, `osNotify`, `openUrl`, …) become
//! [`UiEffect`]s for the view to perform.

use std::collections::VecDeque;

use indexmap::IndexMap;
use oino_ai::JsonValue;
use oino_core::ui::UiRequest;

use crate::model::dialog::Dialog;
use crate::model::ous::{OusNode, Tone};

/// A status-bar entry (`setStatus`).
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// Text.
    pub text: String,
    /// Tone.
    pub tone: Tone,
    /// Higher first.
    pub priority: i64,
}

/// Where a widget renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetPlacement {
    /// Above the composer (default).
    AboveEditor,
    /// Below the composer.
    BelowEditor,
    /// Header area.
    Header,
    /// Footer area.
    Footer,
}

/// A widget (`setWidget`).
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    /// Placement.
    pub placement: WidgetPlacement,
    /// Plain lines.
    pub lines: Vec<String>,
    /// OUS spec (preferred).
    pub spec: Option<OusNode>,
}

/// Horizontal alignment of a frame segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Left group.
    Left,
    /// Right group.
    Right,
}

/// An editor frame / footer segment (`setFrame`, `setFooter` segments).
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// Segment id (`branch`, `context`, …) when the producer names it.
    pub id: Option<String>,
    /// Text.
    pub text: String,
    /// Styled spans (preferred over `text` when present).
    pub spans: Option<crate::model::ous::Rich>,
    /// Command run on click (design.md §10 R2), without the leading `/`.
    pub command: Option<String>,
    /// Tone.
    pub tone: Tone,
    /// Icon name.
    pub icon: Option<String>,
    /// Lower priorities drop first when narrow.
    pub priority: i64,
    /// Alignment.
    pub align: Align,
}

/// Editor frame of one extension.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Segments above the composer.
    pub top: Vec<Segment>,
    /// Segments below the composer.
    pub bottom: Vec<Segment>,
}

/// Panel placement (ui-protocol §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelPlacement {
    /// Modal over the transcript.
    Overlay,
    /// Side pane tab.
    SidePane,
    /// Replaces the composer.
    EditorReplace,
    /// Bottom dock.
    BottomPane,
    /// Window overlay.
    Fullscreen,
}

impl PanelPlacement {
    fn parse(text: Option<&str>) -> Self {
        match text {
            Some("sidePane") => Self::SidePane,
            Some("editorReplace") => Self::EditorReplace,
            Some("bottomPane") => Self::BottomPane,
            Some("fullscreen") => Self::Fullscreen,
            _ => Self::Overlay,
        }
    }
}

/// A panel key hint mapped to an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelKey {
    /// Key id.
    pub key: String,
    /// Action id.
    pub action_id: String,
    /// Label.
    pub label: String,
}

/// An extension panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    /// Panel id (`<extensionId>/<name>`).
    pub id: String,
    /// Title.
    pub title: String,
    /// Placement.
    pub placement: PanelPlacement,
    /// Body.
    pub spec: OusNode,
    /// Key hints.
    pub keys: Vec<PanelKey>,
    /// Close on Escape.
    pub close_on_escape: bool,
    /// Bumped on every `openPanel` (a reopened panel takes focus again).
    pub serial: u64,
}

/// An extension shortcut (`setShortcuts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Action id.
    pub id: String,
    /// Default keys (pi `KeyId`).
    pub keys: String,
    /// Description.
    pub description: String,
}

/// Severity of a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyKind {
    /// Info.
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
}

/// One-shot work for the view.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEffect {
    /// Toast.
    Notify {
        /// Message.
        message: String,
        /// Severity.
        kind: NotifyKind,
    },
    /// OS notification (when the window is unfocused).
    OsNotify {
        /// Title.
        title: String,
        /// Body.
        body: String,
    },
    /// Open a URL in the browser.
    OpenUrl(String),
    /// Write the clipboard.
    ClipboardWrite(String),
    /// Replace the composer text.
    SetEditorText(String),
    /// Insert text at the composer cursor.
    InsertEditorText(String),
    /// Insert an attachment token (label shown as a chip).
    InsertToken {
        /// Token id.
        token_id: String,
        /// Label.
        label: String,
    },
    /// Switch theme by name.
    SetTheme(String),
    /// Open the settings hub.
    OpenSettings {
        /// Namespace filter.
        namespace: Option<String>,
    },
    /// A new dialog is at the front of the queue (focus it).
    DialogOpened,
    /// A panel requested focus.
    FocusPanel(String),
}

/// Extension UI state of one session.
#[derive(Clone, Debug, Default)]
pub struct UiState {
    statuses: IndexMap<String, Status>,
    widgets: IndexMap<String, Widget>,
    frames: IndexMap<String, Frame>,
    headers: IndexMap<String, OusNode>,
    footers: IndexMap<String, OusNode>,
    footer_segments: IndexMap<String, Vec<Segment>>,
    panels: IndexMap<String, Panel>,
    dialogs: VecDeque<Dialog>,
    panel_serial: u64,
    /// Session title override (`setTitle`).
    pub title: Option<String>,
    /// Working message shown while streaming.
    pub working_message: Option<String>,
    /// Extension shortcuts.
    pub shortcuts: Vec<Shortcut>,
}

fn s(request: &UiRequest, key: &str) -> Option<String> {
    request
        .params
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn scoped(request: &UiRequest, key: &str) -> String {
    format!("{}/{key}", request.extension_id.as_deref().unwrap_or(""))
}

fn segments(value: Option<&JsonValue>) -> Vec<Segment> {
    value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|seg| {
            Some(Segment {
                id: seg.get("id").and_then(JsonValue::as_str).map(str::to_owned),
                text: seg.get("text")?.as_str()?.to_owned(),
                spans: seg
                    .get("spans")
                    .filter(|s| s.is_array())
                    .map(|s| crate::model::ous::rich(Some(s)))
                    .or_else(|| {
                        (seg.get("effect").and_then(JsonValue::as_str) == Some("rainbow"))
                            .then(|| wordmark(seg.get("text")?.as_str()?))
                            .flatten()
                    }),
                command: seg
                    .get("command")
                    .and_then(JsonValue::as_str)
                    .map(|c| c.trim_start_matches('/').to_owned()),
                tone: seg
                    .get("tone")
                    .and_then(JsonValue::as_str)
                    .map_or(Tone::Default, Tone::parse),
                icon: seg
                    .get("icon")
                    .and_then(JsonValue::as_str)
                    .map(str::to_owned),
                priority: seg.get("priority").and_then(JsonValue::as_i64).unwrap_or(0),
                align: if seg.get("align").and_then(JsonValue::as_str) == Some("right") {
                    Align::Right
                } else {
                    Align::Left
                },
            })
        })
        .collect()
}

/// The wordmark letters (A2.11): white, cyan, green, yellow, red, repeating — the GUI form of
/// the footer brand's `rainbow` effect.
fn wordmark(text: &str) -> Option<crate::model::ous::Rich> {
    const TONES: [Tone; 5] = [
        Tone::Default,
        Tone::Info,
        Tone::Success,
        Tone::Warning,
        Tone::Error,
    ];
    let mut i = 0;
    let spans: crate::model::ous::Rich = text
        .chars()
        .map(|c| {
            let tone = if c.is_alphanumeric() {
                let t = TONES[i % TONES.len()];
                i += 1;
                t
            } else {
                Tone::Muted
            };
            crate::model::ous::Span {
                text: c.to_string(),
                tone,
                bold: true,
                ..crate::model::ous::Span::default()
            }
        })
        .collect();
    (!spans.is_empty()).then_some(spans)
}

/// Whether a panel title is really a TUI key hint (`←→ tabs · esc close`).
fn hint_title(title: &str) -> bool {
    let t = title.to_lowercase();
    t.contains("esc close") || t.starts_with('\u{2190}') || t.starts_with('\u{2194}')
}

/// The GUI form of a panel (GA-22): composer-slot and overlay panels draw one legend from
/// their `keys` (plus Esc), so TUI `keyHints` rows in the spec go; a title that is only a
/// key hint becomes a readable title.
fn gui_panel(mut panel: Panel) -> Panel {
    if matches!(
        panel.placement,
        PanelPlacement::EditorReplace | PanelPlacement::Overlay | PanelPlacement::Fullscreen
    ) {
        panel.spec = panel.spec.without_key_hints();
    }
    if hint_title(&panel.title) {
        panel.title = if panel.id.ends_with("/tray") {
            "Work tray".to_owned()
        } else {
            String::new()
        };
    }
    panel
}

fn panel_keys(value: Option<&JsonValue>) -> Vec<PanelKey> {
    value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|k| {
            Some(PanelKey {
                key: k.get("key")?.as_str()?.to_owned(),
                action_id: k.get("actionId")?.as_str()?.to_owned(),
                label: k
                    .get("label")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}

/// Keeps the highest-priority segments whose total width (chars + `gap` between them) fits
/// in `width`, preserving their original order (ui-protocol §4.4).
#[must_use]
pub fn fit_segments(segments: &[Segment], width: usize, gap: usize) -> Vec<&Segment> {
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(segments[i].priority));
    let mut keep = vec![false; segments.len()];
    let mut used = 0usize;
    for i in order {
        let w = segments[i].text.chars().count() + if used == 0 { 0 } else { gap };
        if used + w <= width {
            used += w;
            keep[i] = true;
        }
    }
    segments
        .iter()
        .zip(keep)
        .filter_map(|(seg, k)| k.then_some(seg))
        .collect()
}

impl UiState {
    /// Empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Status entries, highest priority first (stable within equal priority).
    #[must_use]
    pub fn statuses(&self) -> Vec<&Status> {
        let mut out: Vec<&Status> = self.statuses.values().collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.priority));
        out
    }

    /// Widgets at `placement`, in registration order.
    pub fn widgets(&self, placement: WidgetPlacement) -> impl Iterator<Item = &Widget> {
        self.widgets
            .values()
            .filter(move |w| w.placement == placement)
    }

    /// All frame segments (top, bottom) merged across extensions.
    #[must_use]
    pub fn frame(&self) -> Frame {
        let mut merged = Frame::default();
        for frame in self.frames.values() {
            merged.top.extend(frame.top.iter().cloned());
            merged.bottom.extend(frame.bottom.iter().cloned());
        }
        merged
    }

    /// Header specs (`setHeader`).
    pub fn headers(&self) -> impl Iterator<Item = &OusNode> {
        self.headers.values()
    }

    /// Footer specs (`setFooter`).
    pub fn footers(&self) -> impl Iterator<Item = &OusNode> {
        self.footers.values()
    }

    /// Footer priority segments (`setFooter{segments}`, design.md §4.18), merged in
    /// registration order.
    pub fn footer_segments(&self) -> impl Iterator<Item = &Segment> {
        self.footer_segments.values().flatten()
    }

    /// Whether any footer content exists (segments or a spec).
    #[must_use]
    pub fn has_footer(&self) -> bool {
        !self.footers.is_empty()
    }

    /// Status entries with their scoped keys (`<extension>/<key>`).
    pub fn statuses_keyed(&self) -> impl Iterator<Item = (&str, &Status)> {
        self.statuses.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Widgets with their scoped keys.
    pub fn widgets_keyed(&self) -> impl Iterator<Item = (&str, &Widget)> {
        self.widgets.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Visits every frame segment (top then bottom) without cloning.
    pub fn frame_segments(&self) -> impl Iterator<Item = &Segment> {
        self.frames
            .values()
            .flat_map(|f| f.top.iter().chain(f.bottom.iter()))
    }

    /// Open panels in open order.
    pub fn panels(&self) -> impl Iterator<Item = &Panel> {
        self.panels.values()
    }

    /// Panel by id.
    #[must_use]
    pub fn panel(&self, id: &str) -> Option<&Panel> {
        self.panels.get(id)
    }

    /// The dialog to show (front of the queue).
    #[must_use]
    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialogs.front()
    }

    /// Number of queued dialogs.
    #[must_use]
    pub fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }

    /// Ids of the queued dialogs, front first (snapshot reconciliation, GA-30).
    #[must_use]
    pub fn dialog_ids(&self) -> Vec<String> {
        self.dialogs.iter().map(|d| d.id.clone()).collect()
    }

    /// Whether a dialog waits for the user ("attention" in the session list).
    #[must_use]
    pub fn needs_attention(&self) -> bool {
        !self.dialogs.is_empty()
    }

    /// Removes a dialog (answered here or dismissed by core). Returns whether it was queued.
    pub fn dismiss(&mut self, id: &str) -> bool {
        let before = self.dialogs.len();
        self.dialogs.retain(|d| d.id != id);
        before != self.dialogs.len()
    }

    /// Closes a panel locally (the view also sends `$close`).
    pub fn close_panel(&mut self, id: &str) -> bool {
        self.panels.shift_remove(id).is_some()
    }

    /// Applies one `extension_ui_request`.
    #[allow(clippy::too_many_lines, reason = "one arm per protocol method")]
    pub fn apply(&mut self, request: &UiRequest) -> Option<UiEffect> {
        if let Some(dialog) = Dialog::parse(request) {
            if self.dialogs.iter().any(|d| d.id == dialog.id) {
                return None;
            }
            self.dialogs.push_back(dialog);
            return (self.dialogs.len() == 1).then_some(UiEffect::DialogOpened);
        }
        let p = &request.params;
        match request.method.as_str() {
            "notify" => Some(UiEffect::Notify {
                message: s(request, "message").unwrap_or_default(),
                kind: match s(request, "notifyType").as_deref() {
                    Some("warning") => NotifyKind::Warning,
                    Some("error") => NotifyKind::Error,
                    _ => NotifyKind::Info,
                },
            }),
            "setStatus" => {
                let key = scoped(request, &s(request, "statusKey").unwrap_or_default());
                match s(request, "statusText") {
                    Some(text) => {
                        self.statuses.insert(
                            key,
                            Status {
                                text,
                                tone: s(request, "tone").map_or(Tone::Default, |t| Tone::parse(&t)),
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
                None
            }
            "setWidget" => {
                let key = scoped(request, &s(request, "widgetKey").unwrap_or_default());
                let lines: Option<Vec<String>> =
                    p.get("widgetLines").and_then(JsonValue::as_array).map(|l| {
                        l.iter()
                            .filter_map(JsonValue::as_str)
                            .map(str::to_owned)
                            .collect()
                    });
                let spec = p.get("widgetSpec").map(OusNode::parse);
                if lines.is_none() && spec.is_none() {
                    self.widgets.shift_remove(&key);
                } else {
                    let placement = match s(request, "widgetPlacement").as_deref() {
                        Some("belowEditor") => WidgetPlacement::BelowEditor,
                        Some("header") => WidgetPlacement::Header,
                        Some("footer") => WidgetPlacement::Footer,
                        _ => WidgetPlacement::AboveEditor,
                    };
                    self.widgets.insert(
                        key,
                        Widget {
                            placement,
                            lines: lines.unwrap_or_default(),
                            spec,
                        },
                    );
                }
                None
            }
            "setFrame" => {
                let key = scoped(request, &s(request, "frameKey").unwrap_or_default());
                let top = segments(p.get("top"));
                let bottom = segments(p.get("bottom"));
                if p.get("top").is_none() && p.get("bottom").is_none() {
                    self.frames.shift_remove(&key);
                } else {
                    self.frames.insert(key, Frame { top, bottom });
                }
                None
            }
            "setHeader" | "setFooter" => {
                let key = scoped(request, &s(request, "key").unwrap_or_default());
                let map = if request.method == "setHeader" {
                    &mut self.headers
                } else {
                    &mut self.footers
                };
                let is_footer = request.method == "setFooter";
                match p.get("spec") {
                    Some(spec) => {
                        map.insert(key.clone(), OusNode::parse(spec));
                    }
                    None => {
                        map.shift_remove(&key);
                    }
                }
                if is_footer {
                    let segs = segments(p.get("segments"));
                    if segs.is_empty() {
                        self.footer_segments.shift_remove(&key);
                    } else {
                        self.footer_segments.insert(key, segs);
                    }
                }
                None
            }
            "setTitle" => {
                self.title = s(request, "title").filter(|t| !t.is_empty());
                None
            }
            "setWorkingMessage" => {
                let visible = p.get("visible").and_then(JsonValue::as_bool) != Some(false);
                self.working_message = s(request, "message").filter(|_| visible);
                None
            }
            "openPanel" | "updatePanel" => {
                let id = s(request, "panelId")?;
                let existing = self.panels.get(&id).cloned();
                if request.method == "updatePanel" && existing.is_none() {
                    return None;
                }
                let serial = if request.method == "openPanel" {
                    self.panel_serial += 1;
                    self.panel_serial
                } else {
                    existing.as_ref().map_or(0, |e| e.serial)
                };
                let panel = Panel {
                    id: id.clone(),
                    title: s(request, "title")
                        .or_else(|| existing.as_ref().map(|e| e.title.clone()))
                        .unwrap_or_default(),
                    placement: if request.method == "openPanel" {
                        PanelPlacement::parse(p.get("placement").and_then(JsonValue::as_str))
                    } else {
                        existing
                            .as_ref()
                            .map_or(PanelPlacement::Overlay, |e| e.placement)
                    },
                    spec: p.get("spec").map_or_else(
                        || {
                            existing
                                .as_ref()
                                .map_or(OusNode::Empty { message: None }, |e| e.spec.clone())
                        },
                        OusNode::parse,
                    ),
                    keys: if p.contains_key("keys") {
                        panel_keys(p.get("keys"))
                    } else {
                        existing
                            .as_ref()
                            .map(|e| e.keys.clone())
                            .unwrap_or_default()
                    },
                    close_on_escape: p
                        .get("closeOnEscape")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or_else(|| existing.as_ref().is_none_or(|e| e.close_on_escape)),
                    serial,
                };
                let panel = gui_panel(panel);
                self.panels.insert(id.clone(), panel);
                (request.method == "openPanel"
                    && p.get("focus").and_then(JsonValue::as_bool) == Some(true))
                .then_some(UiEffect::FocusPanel(id))
            }
            "closePanel" => {
                if let Some(id) = s(request, "panelId") {
                    self.panels.shift_remove(&id);
                }
                None
            }
            "setShortcuts" => {
                self.shortcuts = p
                    .get("shortcuts")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|sc| {
                        Some(Shortcut {
                            id: sc.get("id")?.as_str()?.to_owned(),
                            // Core sends `keys` as a list of pi key ids (GA-43); a single
                            // string is accepted too. Several keys are joined with `,`.
                            keys: match sc.get("keys") {
                                Some(JsonValue::Array(keys)) => keys
                                    .iter()
                                    .filter_map(JsonValue::as_str)
                                    .collect::<Vec<_>>()
                                    .join(","),
                                Some(JsonValue::String(k)) => k.clone(),
                                _ => String::new(),
                            },
                            description: sc
                                .get("description")
                                .and_then(JsonValue::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect();
                None
            }
            "set_editor_text" => Some(UiEffect::SetEditorText(
                s(request, "text").unwrap_or_default(),
            )),
            "insertEditorText" => Some(UiEffect::InsertEditorText(
                s(request, "text").unwrap_or_default(),
            )),
            "insertEditorToken" => {
                let token = p.get("token")?;
                Some(UiEffect::InsertToken {
                    token_id: token.get("tokenId")?.as_str()?.to_owned(),
                    label: token
                        .get("label")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("attachment")
                        .to_owned(),
                })
            }
            "osNotify" => Some(UiEffect::OsNotify {
                title: s(request, "title").unwrap_or_default(),
                body: s(request, "body").unwrap_or_default(),
            }),
            "openUrl" => s(request, "url").map(UiEffect::OpenUrl),
            "clipboardWrite" => s(request, "text").map(UiEffect::ClipboardWrite),
            "setTheme" => s(request, "name").map(UiEffect::SetTheme),
            "openSettings" => Some(UiEffect::OpenSettings {
                namespace: s(request, "namespace"),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(value: JsonValue) -> UiRequest {
        let mut v = value;
        v["type"] = json!("extension_ui_request");
        if v.get("id").is_none() {
            v["id"] = json!("r");
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn tray_panels_get_a_title_and_one_legend() {
        let mut ui = UiState::new();
        ui.apply(&req(json!({"method":"openPanel","panelId":"unipi.core/tray","placement":"editorReplace",
            "title":"\u{2190}\u{2192} tabs \u{b7} esc close",
            "spec":{"t":"stack","children":[{"t":"text","value":"x"},{"t":"keyHints","hints":[{"keys":"esc","label":"close"}]}]}})));
        let panel = ui.panel("unipi.core/tray").unwrap();
        assert_eq!(panel.title, "Work tray");
        assert_eq!(panel.spec.to_lines(), vec!["x"]);
    }

    #[test]
    fn statuses_are_scoped_sorted_and_cleared() {
        let mut ui = UiState::new();
        ui.apply(&req(
            json!({"method":"setStatus","statusKey":"k","statusText":"low","extensionId":"a"}),
        ));
        ui.apply(&req(json!({"method":"setStatus","statusKey":"k","statusText":"high","priority":5,"tone":"warning","extensionId":"b"})));
        let texts: Vec<&str> = ui.statuses().iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["high", "low"]);
        assert_eq!(ui.statuses()[0].tone, Tone::Warning);
        ui.apply(&req(
            json!({"method":"setStatus","statusKey":"k","extensionId":"a"}),
        ));
        assert_eq!(ui.statuses().len(), 1);
    }

    #[test]
    fn widgets_frames_and_footers() {
        let mut ui = UiState::new();
        ui.apply(&req(
            json!({"method":"setWidget","widgetKey":"w","widgetLines":["a"],"extensionId":"x"}),
        ));
        ui.apply(&req(json!({"method":"setWidget","widgetKey":"f","widgetSpec":{"t":"text","value":"s"},"widgetPlacement":"footer"})));
        assert_eq!(ui.widgets(WidgetPlacement::AboveEditor).count(), 1);
        assert!(
            ui.widgets(WidgetPlacement::Footer)
                .next()
                .unwrap()
                .spec
                .is_some()
        );
        ui.apply(&req(
            json!({"method":"setWidget","widgetKey":"w","extensionId":"x"}),
        ));
        assert_eq!(ui.widgets(WidgetPlacement::AboveEditor).count(), 0);
        ui.apply(&req(json!({"method":"setFrame","frameKey":"f","bottom":[{"text":"main","priority":1,"align":"right"}]})));
        assert_eq!(ui.frame().bottom[0].align, Align::Right);
        ui.apply(&req(json!({"method":"setFrame","frameKey":"f"})));
        assert!(ui.frame().bottom.is_empty());
        ui.apply(&req(
            json!({"method":"setFooter","key":"k","spec":{"t":"text","value":"foot"}}),
        ));
        assert_eq!(ui.footers().count(), 1);
        ui.apply(&req(json!({"method":"setFooter","key":"k"})));
        assert_eq!(ui.footers().count(), 0);
        ui.apply(&req(json!({"method":"setFooter","key":"strip","spec":{"t":"text","value":"x"},
            "segments":[{"id":"ctx","text":"ctx 4%","priority":7,"align":"center","command":"/compact",
                         "spans":[{"text":"ctx ","dim":true},{"text":"4%"}]}]})));
        let seg = ui.footer_segments().next().unwrap();
        assert_eq!(seg.id.as_deref(), Some("ctx"));
        assert_eq!(seg.command.as_deref(), Some("compact"));
        assert_eq!(seg.spans.as_ref().unwrap().len(), 2);
        ui.apply(&req(json!({"method":"setFooter","key":"strip"})));
        assert_eq!(ui.footer_segments().count(), 0);
    }

    #[test]
    fn dialog_queue_and_dismiss() {
        let mut ui = UiState::new();
        let first = ui.apply(&req(
            json!({"id":"1","method":"confirm","title":"a","message":"m"}),
        ));
        assert_eq!(first, Some(UiEffect::DialogOpened));
        let second = ui.apply(&req(json!({"id":"2","method":"input","title":"b"})));
        assert_eq!(second, None, "queued behind the first");
        assert!(ui.needs_attention());
        assert_eq!(ui.dialog().unwrap().id, "1");
        assert!(ui.dismiss("1"));
        assert!(!ui.dismiss("1"));
        assert_eq!(ui.dialog().unwrap().id, "2");
    }

    #[test]
    fn panels_open_update_close() {
        let mut ui = UiState::new();
        let effect = ui.apply(&req(json!({"method":"openPanel","panelId":"p","title":"Info","placement":"sidePane","spec":{"t":"text","value":"v1"},"keys":[{"key":"r","actionId":"refresh","label":"Refresh"}],"focus":true})));
        assert_eq!(effect, Some(UiEffect::FocusPanel("p".into())));
        ui.apply(&req(
            json!({"method":"updatePanel","panelId":"p","spec":{"t":"text","value":"v2"}}),
        ));
        let panel = ui.panel("p").unwrap();
        assert_eq!(panel.placement, PanelPlacement::SidePane);
        assert_eq!(panel.title, "Info");
        assert_eq!(panel.keys[0].action_id, "refresh");
        assert_eq!(panel.spec.to_lines(), vec!["v2"]);
        ui.apply(&req(
            json!({"method":"updatePanel","panelId":"ghost","spec":{"t":"text","value":"x"}}),
        ));
        assert!(ui.panel("ghost").is_none());
        ui.apply(&req(json!({"method":"closePanel","panelId":"p"})));
        assert_eq!(ui.panels().count(), 0);
    }

    #[test]
    fn effects() {
        let mut ui = UiState::new();
        assert_eq!(
            ui.apply(&req(
                json!({"method":"notify","message":"hi","notifyType":"error"})
            )),
            Some(UiEffect::Notify {
                message: "hi".into(),
                kind: NotifyKind::Error
            })
        );
        assert_eq!(
            ui.apply(&req(json!({"method":"set_editor_text","text":"x"}))),
            Some(UiEffect::SetEditorText("x".into()))
        );
        assert_eq!(
            ui.apply(&req(json!({"method":"openUrl","url":"https://a"}))),
            Some(UiEffect::OpenUrl("https://a".into()))
        );
        ui.apply(&req(json!({"method":"setTitle","title":"T"})));
        assert_eq!(ui.title.as_deref(), Some("T"));
        ui.apply(&req(json!({"method":"setWorkingMessage","message":"busy"})));
        assert_eq!(ui.working_message.as_deref(), Some("busy"));
        ui.apply(&req(
            json!({"method":"setWorkingMessage","message":"busy","visible":false}),
        ));
        assert_eq!(ui.working_message, None);
        ui.apply(&req(json!({"method":"setShortcuts","shortcuts":[{"id":"x.go","keys":"alt+g","description":"Go"}]})));
        assert_eq!(ui.shortcuts[0].keys, "alt+g");
        ui.apply(&req(json!({"method":"setShortcuts","shortcuts":[{"id":"x.go","keys":["alt+g","ctrl+g"],"description":"Go"}]})));
        assert_eq!(ui.shortcuts[0].keys, "alt+g,ctrl+g");
    }

    #[test]
    fn segments_drop_lowest_priority_first() {
        let seg = |text: &str, priority| Segment {
            id: None,
            spans: None,
            command: None,
            text: text.into(),
            tone: Tone::Default,
            icon: None,
            priority,
            align: Align::Left,
        };
        let all = vec![seg("aaaa", 1), seg("bb", 9), seg("cccc", 5)];
        let kept: Vec<&str> = fit_segments(&all, 8, 1)
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(kept, vec!["bb", "cccc"]);
        assert_eq!(fit_segments(&all, 100, 1).len(), 3);
    }
}
