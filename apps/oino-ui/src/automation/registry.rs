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

/// What one `request_layout` pass reports: the element exists and means `meta`.
struct Seen {
    path: String,
    base: String,
    meta: Meta,
    focused: bool,
}

/// Geometry of the last prepaint of one element (GPUI reuses a cached view's prepaint while
/// its bounds, content mask and text style stay the same, so this can be a frame or a
/// hundred frames old — it is still the element's current geometry).
#[derive(Clone, Copy)]
struct Geometry {
    bounds: (f32, f32, f32, f32),
    visible: bool,
}

#[derive(Default)]
struct Registry {
    main_window: Option<WindowId>,
    /// Elements whose `request_layout` ran in the frame being collected.
    seen: Vec<Seen>,
    /// Geometry from `prepaint`, by element path.
    geometry: std::collections::HashMap<String, Geometry>,
    /// The published snapshot and its geometry (carried forward for reused prepaints).
    current: Vec<Node>,
    live: std::collections::HashMap<String, Geometry>,
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

/// Start of the root render: publishes the focus handles seen during the last frame's
/// `request_layout` pass (the snapshot itself is published by [`end_frame`]).
pub fn begin_frame(cx: &mut App) {
    if !enabled() {
        return;
    }
    super::state::publish(cx);
}

/// End of a frame (a paint callback of the root view): numbers the ids of everything that
/// `request_layout` reported this frame and publishes the snapshot `tree` answers from.
///
/// It must run after prepaint — painting starts only once every element is prepainted — and
/// it must run on *every* frame, because a frame that changes the UI is often the last one
/// before the app goes idle.
pub fn end_frame() {
    if !enabled() {
        return;
    }
    let Ok(mut r) = REGISTRY.lock() else {
        return;
    };
    let seen = std::mem::take(&mut r.seen);
    let geometry = std::mem::take(&mut r.geometry);
    let previous = std::mem::take(&mut r.live);
    let (nodes, live) = publish(&seen, &geometry, &previous);
    r.current = nodes;
    r.live = live;
    r.frame = r.frame.wrapping_add(1);
}

/// Turns one frame's `request_layout` reports into the snapshot: drops everything not seen
/// this frame, numbers duplicate ids (`send`, `send#2`, …) and keeps the geometry of elements
/// whose prepaint GPUI reused.
fn publish(
    seen: &[Seen],
    geometry: &std::collections::HashMap<String, Geometry>,
    previous: &std::collections::HashMap<String, Geometry>,
) -> (Vec<Node>, std::collections::HashMap<String, Geometry>) {
    let empty = Geometry {
        bounds: (0.0, 0.0, 0.0, 0.0),
        visible: false,
    };
    let mut live = std::collections::HashMap::with_capacity(seen.len());
    let mut nodes = Vec::with_capacity(seen.len());
    let mut ids: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for entry in seen {
        if live.contains_key(&entry.path) {
            continue;
        }
        // Fresh geometry when the element was prepainted this frame; otherwise the last
        // prepaint still describes it (GPUI only reuses prepaint when the bounds match).
        let geo = geometry
            .get(&entry.path)
            .copied()
            .or_else(|| previous.get(&entry.path).copied())
            .unwrap_or(empty);
        live.insert(entry.path.clone(), geo);
        let n = ids.entry(entry.base.as_str()).or_insert(0);
        *n += 1;
        let id = if *n == 1 {
            entry.base.clone()
        } else {
            format!("{}#{}", entry.base, *n)
        };
        nodes.push(Node {
            id,
            path: entry.path.clone(),
            role: entry.meta.role,
            label: entry.meta.label.clone(),
            value: entry.meta.value.clone(),
            enabled: entry.meta.enabled,
            focused: entry.focused,
            checked: entry.meta.checked,
            selected: entry.meta.selected,
            visible: geo.visible,
            bounds: geo.bounds,
        });
    }
    (nodes, live)
}

/// A zero-size element for the root view's child list: its paint callback (after every
/// prepaint of the frame) publishes the snapshot. Costs nothing while automation is off.
#[must_use]
pub fn end_marker() -> gpui_kit::AnyElement {
    use gpui_kit::{ParentElement as _, Styled as _};

    gpui_kit::div()
        .absolute()
        .size_0()
        .child(gpui_kit::canvas(
            |_, _, _| (),
            |_, (), _, _| {
                if enabled() {
                    end_frame();
                }
            },
        ))
        .into_any_element()
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
        name: None,
    }
}

/// A probe that gives the element its own id (`name`) in the tree — for elements that are
/// only readable text (an OUS node, a panel line) and would otherwise show up as `anon`.
#[must_use]
pub fn probe_named<E: Element>(element: E, meta: Meta, name: impl Into<String>) -> Probe<E> {
    Probe {
        inner: element,
        meta,
        name: Some(name.into()),
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
        Probe {
            inner: self,
            meta,
            name: None,
        }
    }
}

/// The wrapper element behind [`probe`]; transparent for layout, paint and hit testing.
pub struct Probe<E: Element> {
    inner: E,
    meta: Meta,
    /// Id override (`probe_named`); `None` keeps the inner element's id.
    name: Option<String>,
}

impl<E: Element> Element for Probe<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.name
            .as_ref()
            .map(|name| ElementId::Name(name.clone().into()))
            .or_else(|| self.inner.id())
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
        if enabled() {
            record_seen(id, &self.meta, window);
        }
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
            record_geometry(id, bounds, window);
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

/// The element's path and its own id, from GPUI's global element id.
fn path_and_base(id: Option<&GlobalElementId>) -> (String, String) {
    match id {
        Some(id) => {
            let base = id
                .last()
                .map_or_else(|| "anon".to_owned(), ToString::to_string);
            (id.to_string(), base)
        }
        None => ("anon".to_owned(), "anon".to_owned()),
    }
}

/// Whether this element belongs to the window the driver speaks to (the mascot window is not).
fn ours(window: &Window) -> bool {
    REGISTRY.lock().is_ok_and(|r| {
        r.main_window
            .is_none_or(|w| w == window.window_handle().window_id())
    })
}

/// `request_layout` hook: the element exists this frame, with this meaning
/// (GPUI runs `request_layout` for cached views too, so this never goes stale).
fn record_seen(id: Option<&GlobalElementId>, meta: &Meta, window: &Window) {
    if !ours(window) {
        return;
    }
    let (path, base) = path_and_base(id);
    let focused = meta.focus.as_ref().is_some_and(|f| f.is_focused(window));
    let mut meta = meta.clone();
    if let Some(focus) = meta.focus.take() {
        super::state::stash_focus(&path, focus);
    }
    let Ok(mut r) = REGISTRY.lock() else {
        return;
    };
    r.seen.push(Seen {
        path,
        base,
        meta,
        focused,
    });
}

/// `prepaint` hook: where the element is. Skipped by GPUI when it reuses the prepaint of a
/// cached view, in which case the previous geometry still applies (the cache key includes the
/// bounds).
fn record_geometry(id: Option<&GlobalElementId>, bounds: Bounds<Pixels>, window: &Window) {
    if !ours(window) {
        return;
    }
    let (path, _) = path_and_base(id);
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
    let Ok(mut r) = REGISTRY.lock() else {
        return;
    };
    r.geometry.insert(
        path,
        Geometry {
            bounds: (left, top, width, height),
            visible,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(path: &str, base: &str, label: &str) -> Seen {
        Seen {
            path: path.to_owned(),
            base: base.to_owned(),
            meta: Meta::button(label),
            focused: false,
        }
    }

    fn geo(bounds: (f32, f32, f32, f32)) -> Geometry {
        Geometry {
            bounds,
            visible: true,
        }
    }

    #[test]
    fn numbers_duplicate_ids_in_tree_order() {
        let entries = [
            seen("root.send", "send", "Send"),
            seen("root.row-0", "row-0", "first"),
            seen("root.row-1", "row-0", "second"),
            seen("root.row-2", "row-0", "third"),
        ];
        let (nodes, live) = publish(
            &entries,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::default(),
        );
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["send", "row-0", "row-0#2", "row-0#3"]);
        assert_eq!(live.len(), 4);
        assert!(nodes.iter().all(|n| !n.visible), "no geometry yet");
    }

    #[test]
    fn keeps_geometry_when_prepaint_was_reused() {
        let entries = [seen("root.set-mascot", "set-mascot", "mascot")];
        let previous: std::collections::HashMap<String, Geometry> = [(
            "root.set-mascot".to_owned(),
            geo((1152.0, 167.0, 44.0, 26.0)),
        )]
        .into_iter()
        .collect();
        let (nodes, live) = publish(&entries, &std::collections::HashMap::new(), &previous);
        assert_eq!(nodes[0].bounds, (1152.0, 167.0, 44.0, 26.0));
        assert!(nodes[0].visible);
        assert_eq!(live["root.set-mascot"].bounds, nodes[0].bounds);
    }

    #[test]
    fn drops_elements_that_are_gone_and_dedupes_paths() {
        let entries = [
            seen("root.send", "send", "Send"),
            seen("root.send", "send", "Send"), // the same path twice in one frame
        ];
        let geometry: std::collections::HashMap<String, Geometry> =
            [("root.send".to_owned(), geo((0.0, 0.0, 10.0, 10.0)))]
                .into_iter()
                .collect();
        let (nodes, _) = publish(&entries, &geometry, &std::collections::HashMap::default());
        assert_eq!(nodes.len(), 1, "a repeated path is one element");
        // A frame that no longer reports the element publishes nothing.
        let (nodes, live) = publish(
            &[],
            &std::collections::HashMap::new(),
            &std::collections::HashMap::default(),
        );
        assert!(nodes.is_empty() && live.is_empty());
    }
}
