//! TUI renderer for the ui-protocol v2 node trees (§4, §12.4).
//!
//! Every entry renders into an off-screen ratatui [`Buffer`] of its rect (a definite slot
//! size, or the auto height of [`measure::height`]); [`buffer_to_lines`] then merges
//! same-style cell runs into the line model of the TUI compositor ([`crate::render::text`]).
//! Buffers are cached per `(entryId, entry.rev, rect, theme, focus/local state)`.

pub(crate) mod extras;
pub(crate) mod focus;
pub(crate) mod layout;
pub(crate) mod measure;
pub(crate) mod style;
pub(crate) mod widgets;

use std::collections::HashMap;

use oino_ui::node::Node;
use oino_ui::slot::{UiEntry, UiView};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::Theme;
use focus::UiLocalState;

use crate::render::text::OLine;

/// Immutable inputs of one render pass.
pub(crate) struct Ctx<'a> {
    /// Active theme (§5).
    pub(crate) theme: &'a Theme,
    /// Frame tick (spinner animation).
    pub(crate) tick: u64,
    /// Local state of the entry being rendered (§4.6).
    pub(crate) local: &'a UiLocalState,
}

impl Ctx<'_> {
    /// Base text style of a root: the theme's body text.
    pub(crate) fn base(&self) -> Style {
        Style::default().fg(self.theme.text)
    }
}

/// Mutable bookkeeping of one render pass (focus order, §4.6).
#[derive(Debug, Default)]
pub(crate) struct RenderState {
    focus_counter: usize,
    focused: Option<usize>,
}

impl RenderState {
    fn next_focus(&mut self) -> bool {
        let focused = self.focused == Some(self.focus_counter);
        self.focus_counter += 1;
        focused
    }
}

/// Renders a tree root into a fresh buffer of `area` and returns its lines.
#[must_use]
pub(crate) fn render_root(root: &Node, area: Rect, cx: &Ctx<'_>) -> Vec<OLine> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let mut buffer = Buffer::empty(area);
    let mut state = RenderState {
        focused: cx.local.focused,
        ..RenderState::default()
    };
    render_node(root, "", area, &mut buffer, cx, &mut state, cx.base());
    buffer_to_lines(&buffer)
}

/// Renders one registry entry (tree, reserved fallback, `lines` or `status`) to lines.
#[must_use]
pub(crate) fn render_entry(entry: &UiEntry, area: Rect, cx: &Ctx<'_>) -> Vec<OLine> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    match &entry.view {
        UiView::Tree { root } => render_root(root, area, cx),
        UiView::Reserved { fallback, .. } => fallback
            .as_deref()
            .map_or_else(Vec::new, |root| render_root(root, area, cx)),
        UiView::Lines { lines } => {
            let base = cx.base();
            let text: Vec<Line<'static>> = lines
                .iter()
                .map(|line| Line::from(Span::styled(line.clone(), base)))
                .collect();
            let mut buffer = Buffer::empty(area);
            ratatui::widgets::Paragraph::new(text)
                .style(base)
                .render(area, &mut buffer);
            buffer_to_lines(&buffer)
        }
        UiView::Status { text } => {
            let base = cx.base();
            let mut buffer = Buffer::empty(area);
            ratatui::widgets::Paragraph::new(style::line(text, base, cx.theme))
                .style(base)
                .render(area, &mut buffer);
            buffer_to_lines(&buffer)
        }
        UiView::Renderer { .. } | UiView::Settings { .. } => Vec::new(),
    }
}

/// Merges same-style cell runs into lines (trailing cells included, so backgrounds keep
/// their width).
#[must_use]
pub(crate) fn buffer_to_lines(buffer: &Buffer) -> Vec<OLine> {
    let area = buffer.area;
    let mut out = Vec::with_capacity(usize::from(area.height));
    for y in 0..area.height {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut text = String::new();
        let mut style = buffer[(area.x, area.y + y)].style();
        for x in 0..area.width {
            let cell = &buffer[(area.x + x, area.y + y)];
            if cell.style() != style {
                if !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), style));
                }
                style = cell.style();
            }
            text.push_str(cell.symbol());
        }
        if !text.is_empty() {
            spans.push(Span::styled(text, style));
        }
        out.push(Line::from(spans));
    }
    out
}

/// One node, recursively (§4).
#[allow(
    clippy::too_many_arguments,
    reason = "the recursive renderer threads the buffer, state, path and inherited style"
)]
pub(crate) fn render_node(
    node: &Node,
    path: &str,
    area: Rect,
    buffer: &mut Buffer,
    cx: &Ctx<'_>,
    state: &mut RenderState,
    inherited: Style,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let common = node.common();
    let base = style::patch(common.style, inherited, cx.theme);
    match node {
        Node::Layout(layout) => {
            let collapsed = layout::collapses(layout, area.width);
            if collapsed {
                let heights: Vec<u16> = layout
                    .children
                    .iter()
                    .map(|child| measure::height(child, area.width, cx))
                    .collect();
                let rects =
                    layout::split_vertical_auto(&heights, layout.spacing.unwrap_or(0), area);
                for (index, (child, rect)) in layout.children.iter().zip(rects).enumerate() {
                    let segment = focus::key_or(&child.common().key.map(str::to_owned), index);
                    render_node(
                        child,
                        &focus::join(path, &segment),
                        rect,
                        buffer,
                        cx,
                        state,
                        base,
                    );
                }
            } else {
                let rects = layout::split(layout, area);
                for (index, (child, rect)) in layout.children.iter().zip(rects).enumerate() {
                    let segment = focus::key_or(&child.common().key.map(str::to_owned), index);
                    render_node(
                        child,
                        &focus::join(path, &segment),
                        rect,
                        buffer,
                        cx,
                        state,
                        base,
                    );
                }
            }
        }
        Node::Block(block) => {
            let spec = style::BlockSpec::from(block);
            let collapsible = block.collapsible.unwrap_or(false);
            let collapsed = if collapsible {
                let focused = state.next_focus();
                let _ = focused;
                cx.local.collapsed(path, block.collapsed.unwrap_or(false))
            } else {
                block.collapsed.unwrap_or(false)
            };
            let marker = collapsible.then_some(collapsed);
            let ratatui_block = style::block(&spec, marker, base, cx.theme).style(base);
            let inner = ratatui_block.inner(area);
            ratatui_block.render(area, buffer);
            if !collapsed && let Some(child) = &block.child {
                render_node(
                    child,
                    &focus::join(path, "child"),
                    inner,
                    buffer,
                    cx,
                    state,
                    base,
                );
            }
        }
        Node::Unknown(unknown) => {
            if let Some(fallback) = &unknown.fallback {
                render_node(
                    fallback,
                    &focus::join(path, "fallback"),
                    area,
                    buffer,
                    cx,
                    state,
                    base,
                );
            }
        }
        Node::Paragraph(_)
        | Node::List(_)
        | Node::Table(_)
        | Node::Tabs(_)
        | Node::Gauge(_)
        | Node::LineGauge(_)
        | Node::Sparkline(_)
        | Node::BarChart(_)
        | Node::Chart(_)
        | Node::Scrollbar(_) => {
            widgets::render(node, path, area, buffer, cx, state, base);
        }
        _ => {
            extras::render(node, path, area, buffer, cx, state, base);
        }
    }
}

/// Renders a v2 node tree to lines (golden tests of WP-U4 and the benches; the slots call
/// [`render_entry`] instead). `width` × `height` is the node's area in terminal cells.
#[must_use]
pub fn render_v2_node(node: &Node, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
    let local = UiLocalState::default();
    let cx = Ctx {
        theme,
        tick: 0,
        local: &local,
    };
    render_root(
        node,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        &cx,
    )
}

/// Intrinsic height of a v2 node at `width` columns (§4.4), for auto slots and tests.
#[must_use]
pub fn measure_v2_node(node: &Node, width: u16, theme: &Theme) -> u16 {
    let local = UiLocalState::default();
    let cx = Ctx {
        theme,
        tick: 0,
        local: &local,
    };
    measure::height(node, width, &cx)
}

/// One rendered entry, cached by revision and rect (§12.4 point 3).
#[derive(Clone, Debug)]
struct Cached {
    key: CacheKey,
    lines: Vec<OLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CacheKey {
    rev: u64,
    width: u16,
    height: u16,
    epoch: u64,
    focused: Option<usize>,
    tick: u64,
    theme: &'static str,
}

/// Line cache of the registry slots. A patch invalidates only its entry (`rev`).
#[derive(Debug, Default)]
pub(crate) struct UiCache {
    entries: HashMap<String, Cached>,
}

impl UiCache {
    /// Rendered lines of `entry` in `area`, re-rendering only when the cache key changed.
    pub(crate) fn lines(
        &mut self,
        entry: &UiEntry,
        area: Rect,
        cx: &Ctx<'_>,
        animated: bool,
    ) -> Vec<OLine> {
        let key = CacheKey {
            rev: entry.rev,
            width: area.width,
            height: area.height,
            epoch: cx.local.epoch,
            focused: cx.local.focused,
            tick: if animated { cx.tick } else { 0 },
            theme: cx.theme.name,
        };
        if let Some(cached) = self.entries.get(&entry.id)
            && cached.key == key
        {
            return cached.lines.clone();
        }
        let lines = render_entry(entry, area, cx);
        self.entries.insert(
            entry.id.clone(),
            Cached {
                key,
                lines: lines.clone(),
            },
        );
        lines
    }

    /// Drops the entries that are no longer in the registry (§6.6).
    pub(crate) fn retain(&mut self, entries: &[&UiEntry]) {
        let ids: std::collections::HashSet<&str> =
            entries.iter().map(|entry| entry.id.as_str()).collect();
        self.entries.retain(|id, _| ids.contains(id.as_str()));
    }

    /// A patch for `entry` arrived; its `rev` differs, so the next draw re-renders it.
    pub(crate) fn invalidate(&mut self, id: &str) {
        self.entries.remove(id);
    }
}

/// Whether a tree animates (contains a `Spinner`, §4.5.2).
#[must_use]
pub(crate) fn animated(entry: &UiEntry) -> bool {
    fn walk(node: &Node) -> bool {
        match node {
            Node::Spinner(_) => true,
            Node::Layout(layout) => layout.children.iter().any(walk),
            Node::Block(block) => block.child.as_deref().is_some_and(walk),
            Node::Tabs(tabs) => tabs
                .panes
                .as_ref()
                .is_some_and(|panes| panes.values().any(walk)),
            Node::Empty(empty) => empty.action.as_deref().is_some_and(walk),
            Node::Unknown(unknown) => unknown.fallback.as_deref().is_some_and(walk),
            _ => false,
        }
    }
    match &entry.view {
        UiView::Tree { root } => walk(root),
        UiView::Reserved { fallback, .. } => fallback.as_deref().is_some_and(walk),
        _ => false,
    }
}
