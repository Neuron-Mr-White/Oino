//! The per-frame element registry (docs/dev/v1/gui-driver.md).
//!
//! Interactive components wrap their element with [`probe`], which records the element's
//! bounds, role, label and state during prepaint — the phase GPUI already runs for layout.
//! The cost is a boolean test per probed element per frame while automation is off
//! ([`enabled`] is a relaxed atomic load); while it is on, one `Mutex` lock per element.
//!
//! A frame's records are swapped in by [`begin_frame`], called from the root view's render, so
//! the snapshot always describes the frame the window has just finished drawing (GPUI renders,
//! then lays out and prepaints, then paints, then calls `render` again for the next frame).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use gpui_kit::{
    App, Bounds, Element, ElementId, GlobalElementId, IntoElement, LayoutId, Pixels, Window,
    WindowId,
};

use serde_json::json;

/// Semantic role of an automatable element (what a driver selector may name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A labelled button.
    Button,
    /// An icon-only button.
    IconButton,
    /// A row of a menu.
    MenuRow,
    /// A tab (side pane, bottom frame, underline tabs).
    Tab,
    /// A switch (on/off).
    Switch,
    /// A checkbox or radio.
    Checkbox,
    /// A text input, textarea or composer.
    Input,
    /// A list row (session list, board lane, extension row, settings row).
    Row,
    /// A card (board card, docked card, result card).
    Card,
    /// One option of a dialog (approval / question / plan).
    Option,
    /// One option of a segmented control.
    Segment,
    /// A chip or toolbar pill.
    Chip,
    /// Read-only text the driver may assert on (a transcript row, a label).
    Text,
    /// Anything else that reacts to the pointer.
    Other,
}

impl Role {
    /// Wire name (`tree` output, selector `role:`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::IconButton => "icon_button",
            Self::MenuRow => "menu_row",
            Self::Tab => "tab",
            Self::Switch => "switch",
            Self::Checkbox => "checkbox",
            Self::Input => "input",
            Self::Row => "row",
            Self::Card => "card",
            Self::Option => "option",
            Self::Segment => "segment",
            Self::Chip => "chip",
            Self::Text => "text",
            Self::Other => "other",
        }
    }

    /// Parses a wire name.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "button" => Self::Button,
            "icon_button" => Self::IconButton,
            "menu_row" => Self::MenuRow,
            "tab" => Self::Tab,
            "switch" => Self::Switch,
            "checkbox" => Self::Checkbox,
            "input" => Self::Input,
            "row" => Self::Row,
            "card" => Self::Card,
            "option" => Self::Option,
            "segment" => Self::Segment,
            "chip" => Self::Chip,
            "text" => Self::Text,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// What a component tells the registry about itself.
#[derive(Clone, Debug)]
pub struct Meta {
    /// Semantic role.
    pub role: Role,
    /// Visible label / text (also matched by `wait_for` and `label` selectors).
    pub label: String,
    /// Accepts input (a disabled button is `false`).
    pub enabled: bool,
    /// Checked state (switches, checkboxes, menu checks).
    pub checked: Option<bool>,
    /// Selected state (tabs, segmented options, rows).
    pub selected: Option<bool>,
    /// Current text (inputs, composer draft).
    pub value: Option<String>,
    /// Keyboard focus handle, when the component has one.
    pub focus: Option<gpui_kit::FocusHandle>,
}

impl Meta {
    /// A meta with `role` and no label.
    #[must_use]
    pub fn new(role: Role) -> Self {
        Self {
            role,
            label: String::new(),
            enabled: true,
            checked: None,
            selected: None,
            value: None,
            focus: None,
        }
    }

    /// A labelled button.
    #[must_use]
    pub fn button(label: impl Into<String>) -> Self {
        Self::new(Role::Button).label(label)
    }

    /// A labelled icon-only button.
    #[must_use]
    pub fn icon_button(label: impl Into<String>) -> Self {
        Self::new(Role::IconButton).label(label)
    }

    /// A `role` with `label`.
    #[must_use]
    pub fn labelled(role: Role, label: impl Into<String>) -> Self {
        Self::new(role).label(label)
    }

    /// Sets the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Sets `enabled`.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }

    /// Sets `checked`.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Sets `checked` when `checked` is `Some`.
    #[must_use]
    pub fn maybe_checked(mut self, checked: Option<bool>) -> Self {
        self.checked = checked;
        self
    }

    /// Sets `selected`.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Sets `value`.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Sets the focus handle.
    #[must_use]
    pub fn focus(mut self, focus: gpui_kit::FocusHandle) -> Self {
        self.focus = Some(focus);
        self
    }
}

/// One registered element, as `tree` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Stable id: the element's own id, `#n`-suffixed when several share it.
    pub id: String,
    /// Full global element id path (`root.detail.send`).
    pub path: String,
    /// Semantic role.
    pub role: Role,
    /// Label / text.
    pub label: String,
    /// Value (inputs).
    pub value: Option<String>,
    /// Accepts input.
    pub enabled: bool,
    /// Has keyboard focus.
    pub focused: bool,
    /// Checked state.
    pub checked: Option<bool>,
    /// Selected state.
    pub selected: Option<bool>,
    /// Non-empty bounds intersecting the window viewport.
    pub visible: bool,
    /// Bounds in window px: `(x, y, width, height)`.
    pub bounds: (f32, f32, f32, f32),
}

impl Node {
    /// Centre of the element in window px.
    #[must_use]
    pub fn center(&self) -> (f32, f32) {
        (
            self.bounds.0 + self.bounds.2 / 2.0,
            self.bounds.1 + self.bounds.3 / 2.0,
        )
    }

    /// Whether `label` (case-insensitive substring) or `value` contains `text`.
    #[must_use]
    pub fn contains(&self, text: &str) -> bool {
        let needle = text.to_lowercase();
        self.label.to_lowercase().contains(&needle)
            || self
                .value
                .as_ref()
                .is_some_and(|v| v.to_lowercase().contains(&needle))
    }

    /// The `tree` JSON shape.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "id": self.id,
            "path": self.path,
            "role": self.role.as_str(),
            "label": self.label,
            "value": self.value,
            "enabled": self.enabled,
            "focused": self.focused,
            "checked": self.checked,
            "selected": self.selected,
            "visible": self.visible,
            "bounds": [self.bounds.0, self.bounds.1, self.bounds.2, self.bounds.3],
        })
    }
}

/// A record collected during prepaint, before ids are assigned.
struct Record {
    path: String,
    base: String,
    meta: Meta,
    bounds: (f32, f32, f32, f32),
    visible: bool,
    focused: bool,
}

impl Record {
    fn to_node(&self, id: String) -> Node {
        Node {
            id,
            path: self.path.clone(),
            role: self.meta.role,
            label: self.meta.label.clone(),
            value: self.meta.value.clone(),
            enabled: self.meta.enabled,
            focused: self.focused,
            checked: self.meta.checked,
            selected: self.meta.selected,
            visible: self.visible,
            bounds: self.bounds,
        }
    }
}

#[derive(Default)]
struct Registry {
    main_window: Option<WindowId>,
    collecting: Vec<Record>,
    current: Vec<Node>,
    frame: u64,
}

/// Whether the automation layer is active (one relaxed load when it is not).
static ENABLED: AtomicBool = AtomicBool::new(false);

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));

/// Turns the registry on (the app calls it when `OINO_DESKTOP_AUTOMATION` is set).
pub fn activate() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether the registry records elements at all.
#[must_use]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Records elements of `window` only (the mascot window is ignored).
pub fn set_main_window(window: WindowId) {
    if let Ok(mut r) = REGISTRY.lock() {
        r.main_window = Some(window);
    }
}

/// Swaps in the previous frame's records and numbers their ids. Called from the root render
/// (which also publishes the focus handles seen during the last prepaint).
pub fn begin_frame(cx: &mut App) {
    if !enabled() {
        return;
    }
    super::state::publish(cx);
    let Ok(mut r) = REGISTRY.lock() else {
        return;
    };
    let records = std::mem::take(&mut r.collecting);
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut nodes = Vec::with_capacity(records.len());
    for record in &records {
        let n = seen.entry(record.base.as_str()).or_insert(0);
        *n += 1;
        let id = if *n == 1 {
            record.base.clone()
        } else {
            format!("{}#{}", record.base, *n)
        };
        nodes.push(record.to_node(id));
    }
    r.current = nodes;
    r.frame = r.frame.wrapping_add(1);
}

/// The current element snapshot (the last completed frame).
#[must_use]
pub fn snapshot() -> Vec<Node> {
    REGISTRY
        .lock()
        .map(|r| r.current.clone())
        .unwrap_or_default()
}

/// The number of completed frames seen so far (a `wait_for` can require a newer frame).
#[must_use]
pub fn frame_number() -> u64 {
    REGISTRY.lock().map(|r| r.frame).unwrap_or_default()
}

/// Resolves a selector against `nodes`: exact id, else role + label substring, else a unique
/// label substring.
///
/// # Errors
///
/// `Err` is a human-readable message: nothing matched, or several elements did (it lists them
/// with id, role and label so a scenario can be made specific).
pub fn select<'a>(nodes: &'a [Node], sel: &super::protocol::Selector) -> Result<&'a Node, String> {
    let mut hits: Vec<&Node> = nodes
        .iter()
        .filter(|n| sel.matches(n))
        .filter(|n| n.visible || sel.include_hidden)
        .collect();
    if sel.by_id() && hits.is_empty() {
        // Fall back to hidden elements when the id names one exactly (menus render only when
        // open, so a hidden hit is almost always a stale id).
        hits = nodes.iter().filter(|n| sel.matches(n)).collect();
    }
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(format!("no element matches {sel:?}")),
        _ => {
            // Prefer an exactly-named id over label matches.
            if let Some(exact) = hits
                .iter()
                .find(|n| sel.id.as_deref() == Some(n.id.as_str()))
                .copied()
            {
                return Ok(exact);
            }
            let names: Vec<String> = hits
                .iter()
                .take(8)
                .map(|n| format!("{} ({}, {:?})", n.id, n.role.as_str(), n.label))
                .collect();
            Err(format!(
                "{} elements match {sel:?}: {}",
                hits.len(),
                names.join(", ")
            ))
        }
    }
}

/// A probe over `element`: records it during prepaint while automation is on.
#[must_use]
pub fn probe<E: Element>(element: E, meta: Meta) -> Probe<E> {
    Probe {
        inner: element,
        meta,
    }
}

/// Fluent form of [`probe`] for element chains: `div().id("board-fab")…​.automation(meta)`.
///
/// It must be the last call of the chain — after it, element-specific methods (`on_click`,
/// `child`, …) are gone.
pub trait Automatable: Sized {
    /// Wraps this element in a probe with `meta`.
    #[must_use]
    fn automation(self, meta: Meta) -> Probe<Self>
    where
        Self: Element;
}

impl<T: Element> Automatable for T {
    fn automation(self, meta: Meta) -> Probe<Self> {
        Probe { inner: self, meta }
    }
}

/// The wrapper element behind [`probe`]; transparent for layout, paint and hit testing.
pub struct Probe<E: Element> {
    inner: E,
    meta: Meta,
}

impl<E: Element> Element for Probe<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self
            .inner
            .prepaint(id, inspector_id, bounds, request_layout, window, cx);
        if enabled() {
            record(id, bounds, &self.meta, window);
        }
        state
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

impl<E: Element> IntoElement for Probe<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Appends one record (the prepaint hook).
fn record(id: Option<&GlobalElementId>, bounds: Bounds<Pixels>, meta: &Meta, window: &Window) {
    // The focus state is read here, where the window is at hand.
    let focused = meta.focus.as_ref().is_some_and(|f| f.is_focused(window));
    let mut meta = meta.clone();
    let focus = meta.focus.take();
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let viewport = window.viewport_size();
    let visible = width > 0.5
        && height > 0.5
        && left < f32::from(viewport.width)
        && top < f32::from(viewport.height)
        && left + width > 0.0
        && top + height > 0.0;
    let (path, base) = match id {
        Some(id) => {
            let base = id
                .last()
                .map_or_else(|| "anon".to_owned(), ToString::to_string);
            (id.to_string(), base)
        }
        None => ("anon".to_owned(), "anon".to_owned()),
    };
    let Ok(mut r) = REGISTRY.lock() else {
        return;
    };
    if r.main_window
        .is_some_and(|w| w != window.window_handle().window_id())
    {
        return;
    }
    if let Some(focus) = focus {
        super::state::stash_focus(&path, focus);
    }
    r.collecting.push(Record {
        path,
        base,
        meta,
        bounds: (left, top, width, height),
        visible,
        focused,
    });
}
