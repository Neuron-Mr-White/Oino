//! Frontend-agnostic UI (ui-protocol v2): dialogs, slots (panels, widgets, status segments,
//! turn footers), settings namespaces, editor, device integration.
//!
//! Node trees are built with the re-exported builder crate [`widgets`] (`oino_ui`, ui-protocol
//! §11); this module adds the slot calls on top of it. The pi-shaped helpers (`select`,
//! `confirm`, `input`, `notify`, `status`, `widget_lines`) keep their 1.x wire form, so pi
//! parity (§D-U1) holds.
//!
//! ```
//! use oino_extension::ui::{self, widgets::prelude::*, Placement};
//!
//! struct Task { id: String, title: String, status: String, priority: String }
//!
//! fn board_panel(tasks: &[Task]) -> Node {
//!     let header = Paragraph::new(Line::from(vec![
//!         "Board ".bold(),
//!         Span::from(format!("{} open", tasks.len())).fg(Color::Muted),
//!     ]));
//!     let list = List::new(tasks.iter().map(|t| {
//!         ListItem::new(t.title.as_str())
//!             .key(&t.id)
//!             .detail(Line::from(t.status.as_str()).fg(Color::Dim))
//!             .badge(Span::from(t.priority.as_str()).bg(Color::WarningBg))
//!     }))
//!     .highlight_symbol("› ")
//!     .on_select("select-task")
//!     .on_activate("open-task")
//!     .block(Block::bordered().title("Tasks").border_type(BorderType::Rounded));
//!     Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)])
//!         .children([
//!             header.into(),
//!             list.into(),
//!             KeyHints::new([("enter", "open"), ("n", "new")]).hint_action("n", "new-task").into(),
//!         ])
//!         .into()
//! }
//!
//! fn show(tasks: &[Task], lines: Vec<String>) -> oino_extension::Result<()> {
//!     let id = ui::Panel::new("tasks").placement(Placement::SidePane).title("Tasks")
//!         .icon(Icon::List).spec(board_panel(tasks)).focus(true).open()?;
//!     ui::Panel::update(&id).spec(board_panel(tasks)).send()?;            // whole tree, core diffs
//!     ui::Panel::update(&id)
//!         .patch([NodeOp::append(["log"], PatchField::Lines, lines)])
//!         .send()?;                                                       // hot path
//!     ui::widget("progress", Some(Gauge::default().ratio(0.4).label("40%").into()))?;
//!     ui::status("mode", Some(Line::from("Graph 3/7").fg(Color::Info)))?;
//!     ui::register_turn_footer("files")?;                                 // + guest render export
//!     Ok(())
//! }
//! ```
//!
//! Panels opened by `Panel::open` live until [`Panel::close`], the user closes them (`$close`)
//! or the extension instance is disposed (ui-protocol §6.6). Reserved ids (§8) ignore `spec`.

use serde::Serialize;
use serde_json::{Map, Value, json};

pub use oino_ui::extras::icon::Icon;
pub use oino_ui::settings::SettingsNamespace;
pub use oino_ui::slot::{
    EntrySize, KeyBinding, PanelOpen, PanelUpdate, Placement, ReservedComponent, Slot, UiAction,
    UiCapabilities, UiEntry, UiPatch, UiSnapshot, UiView,
};
pub use oino_ui::{Color, Line, Node, Span, Stylize, Text};

use oino_ui::patch::NodeOp;

use crate::Result;
use crate::bindings::oino::extension::registry as wr;
use crate::bindings::oino::extension::ui as wu;
use crate::error::parse;

/// The node builder crate (ui-protocol §11): widgets, text, style, layout, patch ops, slots.
///
/// `use oino_extension::ui::widgets::prelude::*;` brings the ratatui-mirroring builder into
/// scope; `widgets::slot` (re-exported here as [`Placement`], [`Slot`], …) has the entry model.
pub use oino_ui as widgets;

/// Raises a dialog (`{method, ...}` per ui-protocol §3) and waits for the answer.
pub async fn dialog(request: &Value) -> Result<Value> {
    parse(&wu::dialog(request.to_string(), None).await?)
}

/// pi `ctx.ui.select` → chosen value (`None` when dismissed).
pub async fn select(title: &str, options: &[&str]) -> Result<Option<String>> {
    let answer = dialog(&json!({"method": "select", "title": title, "options": options})).await?;
    Ok(answer
        .get("value")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

/// pi `ctx.ui.confirm`.
pub async fn confirm(title: &str, message: &str) -> Result<bool> {
    let answer = dialog(&json!({"method": "confirm", "title": title, "message": message})).await?;
    Ok(answer
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// pi `ctx.ui.input`.
pub async fn input(title: &str, placeholder: Option<&str>) -> Result<Option<String>> {
    let answer =
        dialog(&json!({"method": "input", "title": title, "placeholder": placeholder})).await?;
    Ok(answer
        .get("value")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

/// Fire-and-forget request (`notify`, `setStatus`, `setWidget`, `setTitle`, …, §3) or a
/// registry call (`invalidateTurnFooter{key, turnEntryIds?}`, `closePanel`, …).
pub fn fire(request: &Value) -> Result<()> {
    Ok(wu::fire(&request.to_string())?)
}

/// pi `ctx.ui.notify(message, type)`.
pub fn notify(message: &str, kind: &str) -> Result<()> {
    fire(&json!({"method": "notify", "message": message, "notifyType": kind}))
}

/// Content of a status segment (`setStatus`, ui-protocol §3).
#[derive(Clone, Debug, PartialEq)]
pub enum StatusText {
    /// pi 1.4.0 `statusText` (byte-for-byte parity, D-U1).
    Text(String),
    /// Oino v2 `spans` (one [`Line`] of styled spans).
    Line(Line),
    /// Absent text: clears the segment.
    Clear,
}

impl From<&str> for StatusText {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for StatusText {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&String> for StatusText {
    fn from(text: &String) -> Self {
        Self::Text(text.clone())
    }
}

impl From<Line> for StatusText {
    fn from(line: Line) -> Self {
        Self::Line(line)
    }
}

impl From<&Line> for StatusText {
    fn from(line: &Line) -> Self {
        Self::Line(line.clone())
    }
}

impl<T: Into<Self>> From<Option<T>> for StatusText {
    fn from(text: Option<T>) -> Self {
        text.map_or(Self::Clear, Into::into)
    }
}

/// pi `ctx.ui.setStatus(key, text)` / Oino styled status segment: absent text clears it.
///
/// `status("mode", Some("Graph 3/7"))`, `status("mode", None)` (pi form) and
/// `status("mode", Some(Line::from("Graph 3/7").fg(Color::Info)))` (v2 form) all work.
pub fn status(key: &str, text: impl Into<StatusText>) -> Result<()> {
    fire(&status_request(key, text.into())?)
}

/// Clears the status segment `key` (no `None::<&str>` type annotation needed).
pub fn clear_status(key: &str) -> Result<()> {
    status(key, StatusText::Clear)
}

fn status_request(key: &str, text: StatusText) -> Result<Value> {
    let mut request = json!({"method": "setStatus", "statusKey": key});
    match text {
        StatusText::Text(text) => request["statusText"] = json!(text),
        StatusText::Line(line) => request["spans"] = serde_json::to_value(line)?,
        StatusText::Clear => {}
    }
    Ok(request)
}

/// pi `ctx.ui.setWidget(key, lines)` (absent lines clear it).
pub fn widget_lines(key: &str, lines: Option<&[String]>) -> Result<()> {
    let mut request = json!({"method": "setWidget", "widgetKey": key});
    if let Some(lines) = lines {
        request["widgetLines"] = json!(lines);
    }
    fire(&request)
}

/// A v2 widget above the composer (`setWidget{widgetSpec}`, absent node removes it).
pub fn widget(key: &str, spec: Option<Node>) -> Result<()> {
    fire(&widget_request(key, spec)?)
}

fn widget_request(key: &str, spec: Option<Node>) -> Result<Value> {
    let mut request = json!({"method": "setWidget", "widgetKey": key});
    if let Some(node) = spec {
        request["widgetSpec"] = serde_json::to_value(node)?;
    }
    Ok(request)
}

/// Renders `key` at the end of every turn (ui-protocol §6.7): registers the `turnFooter`
/// renderer `{target: "turnFooter", key}`; core creates the entry `<extension>/turnFooter:<key>`
/// and fronts the guest `render` export.
///
/// Registering the *spec* is step one — the guest must also answer `render` for
/// `{target:"turnFooter", key}`, which [`crate::Init::renderer`] wires up with
/// `RendererTarget::TurnFooter`. Invalidating a rendered footer:
/// `fire(&json!({"method":"invalidateTurnFooter","key":"files"}))`.
pub fn register_turn_footer(key: &str) -> Result<()> {
    wr::register_renderer(&wr::RendererSpec {
        target: wr::RendererTarget::TurnFooter,
        key: key.to_owned(),
    });
    Ok(())
}

/// Registers an OSS-2 settings namespace at runtime (ui-protocol §7); the manifest's static
/// `ui.settings` list is the other path. A namespace registered twice is rejected.
pub fn register_settings(namespace: &SettingsNamespace) -> Result<()> {
    Ok(wr::register_settings(&serde_json::to_string(namespace)?)?)
}

/// [`register_settings`] for a namespace built as JSON (e.g. `unipi_kit::settings`).
pub fn register_settings_json(namespace: &Value) -> Result<()> {
    Ok(wr::register_settings(&namespace.to_string())?)
}

/// Inserts `value` under `key` when set (`None` leaves the field out of the request).
fn insert<T: Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<&T>) -> Result<()> {
    if let Some(value) = value {
        map.insert(key.to_owned(), serde_json::to_value(value)?);
    }
    Ok(())
}

/// A panel (`openPanel` / `updatePanel`, ui-protocol §6): chainable, then [`Panel::open`],
/// [`Panel::send`] or [`Panel::close`].
///
/// ```no_run
/// # use oino_extension::ui::{self, widgets::prelude::*};
/// # fn run() -> oino_extension::Result<()> {
/// let id = ui::Panel::new("todos").placement(ui::Placement::SidePane)
///     .title("Todos").spec(Paragraph::new("nothing yet")).open()?;
/// ui::Panel::update(&id).title("Todos 2").send()?;
/// ui::Panel::update(&id).close();
/// # Ok(()) }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Panel {
    id: Option<String>,
    placement: Option<Placement>,
    title: Option<String>,
    icon: Option<Icon>,
    badge: Option<Line>,
    order: Option<i32>,
    spec: Option<Node>,
    patch: Option<Vec<NodeOp>>,
    keys: Option<Vec<KeyBinding>>,
    focus: Option<bool>,
    close_on_escape: Option<bool>,
    size: Option<EntrySize>,
    params: Option<Value>,
    fallback: Option<Node>,
}

impl Panel {
    /// Opens a panel with explicit entry id (`"<extensionId>/<name>"`; reserved ids §8) in the
    /// `overlay` slot unless [`Panel::placement`] says otherwise.
    #[must_use]
    pub fn new(panel_id: impl AsRef<str>) -> Self {
        Self {
            id: Some(panel_id.as_ref().to_owned()),
            placement: Some(Placement::Overlay),
            ..Self::default()
        }
    }

    /// Updates (or re-opens) an existing entry: whole trees are diffed by core, [`Panel::patch`]
    /// sends explicit ops instead (§4.7).
    #[must_use]
    pub fn update(panel_id: impl AsRef<str>) -> Self {
        Self {
            id: Some(panel_id.as_ref().to_owned()),
            ..Self::default()
        }
    }

    /// Slot the entry renders in (§6.2).
    #[must_use]
    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }

    /// Tab / page title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Tab / page icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Tab badge.
    #[must_use]
    pub fn badge(mut self, badge: impl Into<Line>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Sort weight (§6.4; third-party entries clamp to `[-100, 100]`).
    #[must_use]
    pub fn order(mut self, order: i32) -> Self {
        self.order = Some(order);
        self
    }

    /// Root node (whole tree; core diffs it).
    #[must_use]
    pub fn spec(mut self, spec: impl Into<Node>) -> Self {
        self.spec = Some(spec.into());
        self
    }

    /// Explicit patch ops for [`Panel::send`] (§4.7).
    #[must_use]
    pub fn patch(mut self, ops: impl IntoIterator<Item = NodeOp>) -> Self {
        self.patch = Some(ops.into_iter().collect());
        self
    }

    /// Entry key bindings (§4.6).
    #[must_use]
    pub fn keys(mut self, keys: impl IntoIterator<Item = KeyBinding>) -> Self {
        self.keys = Some(keys.into_iter().collect());
        self
    }

    /// Activates the entry on open (§6.5).
    #[must_use]
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus = Some(focus);
        self
    }

    /// Esc closes the entry (default `true`).
    #[must_use]
    pub fn close_on_escape(mut self, close: bool) -> Self {
        self.close_on_escape = Some(close);
        self
    }

    /// Overlay width (§6.1 `size`).
    #[must_use]
    pub fn size(mut self, size: EntrySize) -> Self {
        self.size = Some(size);
        self
    }

    /// Reserved-component parameters (§8).
    #[must_use]
    pub fn params(mut self, params: Value) -> Self {
        self.params = Some(params);
        self
    }

    /// Tree rendered when a reserved component is unavailable (TUI / gated owners, §8 rule 3).
    #[must_use]
    pub fn fallback(mut self, fallback: impl Into<Node>) -> Self {
        self.fallback = Some(fallback.into());
        self
    }

    /// `openPanel` → the entry id core assigned.
    pub fn open(self) -> Result<String> {
        Ok(wu::panel_open(&self.open_request()?.to_string())?)
    }

    fn open_request(&self) -> Result<Value> {
        let mut request = Map::new();
        insert(&mut request, "panelId", self.id.as_ref())?;
        insert(&mut request, "placement", self.placement.as_ref())?;
        insert(&mut request, "title", self.title.as_ref())?;
        insert(&mut request, "icon", self.icon.as_ref())?;
        insert(&mut request, "badge", self.badge.as_ref())?;
        insert(&mut request, "order", self.order.as_ref())?;
        insert(&mut request, "spec", self.spec.as_ref())?;
        insert(&mut request, "keys", self.keys.as_ref())?;
        insert(&mut request, "focus", self.focus.as_ref())?;
        insert(&mut request, "closeOnEscape", self.close_on_escape.as_ref())?;
        insert(&mut request, "size", self.size.as_ref())?;
        insert(&mut request, "params", self.params.as_ref())?;
        Ok(Value::Object(request))
    }

    /// `updatePanel` for this entry: tree, patch and/or meta.
    pub fn send(self) -> Result<()> {
        let (id, request) = self.update_request()?;
        Ok(wu::panel_update(&id, &request.to_string())?)
    }

    fn update_request(&self) -> Result<(String, Value)> {
        let Some(id) = self.id.clone() else {
            return Err(crate::Error::msg("panel update without an id"));
        };
        let mut request = Map::new();
        insert(&mut request, "spec", self.spec.as_ref())?;
        insert(&mut request, "patch", self.patch.as_ref())?;
        insert(&mut request, "title", self.title.as_ref())?;
        insert(&mut request, "icon", self.icon.as_ref())?;
        insert(&mut request, "badge", self.badge.as_ref())?;
        insert(&mut request, "order", self.order.as_ref())?;
        insert(&mut request, "keys", self.keys.as_ref())?;
        insert(&mut request, "fallback", self.fallback.as_ref())?;
        Ok((id, Value::Object(request)))
    }

    /// `closePanel`: frontends drop the entry and its local state (§6.6).
    pub fn close(self) {
        if let Some(id) = self.id {
            wu::panel_close(&id);
        }
    }
}

/// Opens a panel from a raw request (§9.1) → entry id.
pub fn panel_open(spec: &Value) -> Result<String> {
    Ok(wu::panel_open(&spec.to_string())?)
}

/// Updates a panel from a raw request (§9.1).
pub fn panel_update(panel_id: &str, spec: &Value) -> Result<()> {
    Ok(wu::panel_update(panel_id, &spec.to_string())?)
}

pub use wu::{editor_insert, editor_set, editor_text, panel_close};

/// OS notification (permission `osNotify`).
pub fn os_notify(title: &str, body: &str, urgency: &str, action_url: Option<&str>) -> Result<()> {
    Ok(wu::os_notify(title, body, urgency, action_url)?)
}
/// Clipboard text (permission `clipboard.write`).
pub fn clipboard_write_text(text: &str) -> Result<()> {
    Ok(wu::clipboard_write_text(text)?)
}
/// Clipboard image (permission `clipboard.read`).
pub async fn clipboard_read_image() -> Result<Option<Value>> {
    wu::clipboard_read_image()
        .await?
        .map(|t| parse(&t))
        .transpose()
}
/// Opens a URL (permission `openUrl`).
pub fn open_url(url: &str) -> Result<()> {
    Ok(wu::open_url(url)?)
}
/// External editor (permission `editExternally`).
pub async fn edit_externally(text: &str, extension_hint: Option<&str>) -> Result<Option<String>> {
    Ok(wu::edit_externally(text.to_owned(), extension_hint.map(ToOwned::to_owned)).await?)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use oino_ui::patch::{NodeOp, PatchField};
    use oino_ui::prelude::{Paragraph, Span};
    use serde_json::json;

    use super::*;

    #[test]
    fn open_request_carries_the_v2_slot_fields() {
        let request = Panel::new("ext/tasks")
            .placement(Placement::SidePane)
            .title("Tasks")
            .icon(Icon::List)
            .badge(Span::from("2"))
            .spec(Paragraph::new("nothing"))
            .keys([KeyBinding {
                key: "enter".into(),
                action: Some("open".into()),
                label: "open".into(),
            }])
            .focus(true)
            .close_on_escape(false)
            .order(5)
            .size(EntrySize::Large)
            .params(json!({"task": "T-12"}))
            .open_request()
            .unwrap_or_default();
        assert_eq!(request["panelId"], "ext/tasks");
        assert_eq!(request["placement"], "sidePane");
        assert_eq!(request["title"], "Tasks");
        assert_eq!(request["icon"], "list");
        assert_eq!(request["spec"]["t"], "Paragraph");
        assert_eq!(request["keys"][0]["action"], "open");
        assert_eq!(request["focus"], true);
        assert_eq!(request["closeOnEscape"], false);
        assert_eq!(request["order"], 5);
        assert_eq!(request["size"], "large");
        assert_eq!(request["params"]["task"], "T-12");
        assert_eq!(
            Panel::new("ext/x").open_request().unwrap_or_default()["placement"],
            "overlay",
            "panels default to the overlay slot"
        );
    }

    #[test]
    fn update_request_sends_ops_or_a_tree_but_never_a_placement() {
        let (id, request) = Panel::update("ext/log")
            .patch([NodeOp::append(["log"], PatchField::Lines, ["a".to_owned()])])
            .title("Log")
            .update_request()
            .unwrap_or_default();
        assert_eq!(id, "ext/log");
        assert_eq!(request["patch"][0]["op"], "append");
        assert_eq!(request["patch"][0]["field"], "lines");
        assert_eq!(request["patch"][0]["items"], json!(["a"]));
        assert_eq!(request["title"], "Log");
        assert_eq!(request.get("spec"), None);
        assert_eq!(request.get("placement"), None);

        let (_, request) = Panel::update("ext/log")
            .spec(Paragraph::new("tree"))
            .fallback(Paragraph::new("fallback"))
            .update_request()
            .unwrap_or_default();
        assert_eq!(request["spec"]["t"], "Paragraph");
        assert_eq!(request["fallback"]["t"], "Paragraph");
        assert!(Panel::default().update_request().is_err(), "no id");
    }

    #[test]
    fn status_keeps_the_pi_form_and_accepts_a_line() {
        let request =
            status_request("mode", StatusText::from(Some("Graph 3/7"))).unwrap_or_default();
        assert_eq!(request["method"], "setStatus");
        assert_eq!(request["statusKey"], "mode");
        assert_eq!(request["statusText"], "Graph 3/7");
        assert_eq!(request.get("spans"), None);

        let request =
            status_request("mode", Some(String::from("owned")).into()).unwrap_or_default();
        assert_eq!(request["statusText"], "owned");

        let request = status_request(
            "mode",
            StatusText::from(Some(Line::from("Graph 3/7").fg(Color::Info))),
        )
        .unwrap_or_default();
        assert_eq!(request.get("statusText"), None);
        assert_eq!(request["spans"]["spans"][0], "Graph 3/7");
        assert_eq!(request["spans"]["style"]["fg"], "info");

        for clear in [
            status_request("mode", StatusText::Clear).unwrap_or_default(),
            status_request("mode", None::<&str>.into()).unwrap_or_default(),
            status_request("mode", StatusText::from(Option::<Line>::None)).unwrap_or_default(),
        ] {
            assert_eq!(clear["method"], "setStatus");
            assert_eq!(clear.get("statusText"), None);
            assert_eq!(clear.get("spans"), None);
        }
    }

    #[test]
    fn widget_prefers_a_node_and_still_takes_pi_lines() {
        let request =
            widget_request("progress", Some(Paragraph::new("40%").into())).unwrap_or_default();
        assert_eq!(request["method"], "setWidget");
        assert_eq!(request["widgetKey"], "progress");
        assert_eq!(request["widgetSpec"]["t"], "Paragraph");
        assert_eq!(request.get("widgetLines"), None);

        let request = widget_request("progress", None).unwrap_or_default();
        assert_eq!(request.get("widgetSpec"), None);
        assert_eq!(request.get("widgetLines"), None);
    }
}
