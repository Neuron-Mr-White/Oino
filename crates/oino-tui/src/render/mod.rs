//! Frame layout and drawing (container order: chat, pending messages, status,
//! widgets above editor, editor, widgets below editor, footer).
//!
//! Every function is pure over [`App`] state; the only mutation is the per-row line cache
//! and the remembered transcript height used for paging.

mod chrome;
pub(crate) mod highlight;
pub(crate) mod markdown;
pub(crate) mod ous;
mod overlays;
pub(crate) mod text;
mod tools;
mod transcript;
pub(crate) mod ui;

pub use ui::{measure_v2_node, render_v2_node};

use std::path::PathBuf;
use std::sync::Arc;

use oino_ui::slot::{Slot, UiView};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, CachedRow};
use crate::host::{RenderRequest, TuiHost};
use crate::theme::Theme;
use text::OLine;

/// Shared inputs of row renderers.
pub(crate) struct RenderCtx<'a> {
    pub(crate) theme: &'a Theme,
    pub(crate) width: usize,
    pub(crate) expanded: bool,
    pub(crate) hide_thinking: bool,
    pub(crate) expand_key: String,
    pub(crate) home: Option<String>,
    pub(crate) tick: u64,
    pub(crate) spinner: &'static str,
    pub(crate) host: Arc<dyn TuiHost>,
}

impl RenderCtx<'_> {
    /// Asks the host for an extension render spec (ui-protocol §6.4).
    pub(crate) fn render_spec(&self, request: &RenderRequest) -> Option<Vec<OLine>> {
        let node = self.host.render(request)?;
        if node.is_null() {
            return None;
        }
        Some(transcript::spec_lines(&node, self))
    }
}

/// A block of lines for the bottom stack plus an optional cursor (x, y within the block).
pub(crate) struct Block {
    pub(crate) lines: Vec<OLine>,
    pub(crate) cursor: Option<(u16, u16)>,
}

impl Block {
    pub(crate) fn new(lines: Vec<OLine>) -> Self {
        Self {
            lines,
            cursor: None,
        }
    }
}

fn height(lines: &[OLine]) -> u16 {
    u16::try_from(lines.len()).unwrap_or(u16::MAX)
}

fn ctx(app: &App, width: usize) -> RenderCtx<'_> {
    RenderCtx {
        theme: &app.theme,
        width,
        expanded: app.tools_expanded,
        hide_thinking: app.hide_thinking,
        expand_key: {
            let k = app.keys.key_text("app.tools.expand");
            k.split('/').next().unwrap_or("").to_owned()
        },
        home: app.host.home().map(|h: PathBuf| h.display().to_string()),
        tick: app.tick,
        spinner: app.spinner(),
        host: Arc::clone(&app.host),
    }
}

/// Ids of a registry slot in render order (§6.4).
pub(super) fn slot_ids(app: &App, slot: Slot) -> Vec<String> {
    app.registry
        .entries_for(slot)
        .iter()
        .map(|entry| entry.id.clone())
        .collect()
}

/// The entry a non-stack slot paints (focused, else the last in registry order).
pub(super) fn slot_entry_id(app: &App, slot: Slot) -> Option<String> {
    let entries = app.registry.entries_for(slot);
    app.focused_entry
        .as_deref()
        .and_then(|id| entries.iter().find(|entry| entry.id == id).copied())
        .or_else(|| entries.last().copied())
        .map(|entry| entry.id.clone())
}

/// Intrinsic height of a registry entry (§4.4), 0 for views without content.
pub(super) fn ui_entry_height(app: &App, id: &str, width: u16) -> u16 {
    let Some(entry) = app.registry.entry(id) else {
        return 0;
    };
    let local = app.ui_local.get(id).cloned().unwrap_or_default();
    let cx = ui::Ctx {
        theme: &app.theme,
        tick: app.tick,
        local: &local,
    };
    match &entry.view {
        UiView::Tree { root } => ui::measure::height(root, width, &cx),
        UiView::Reserved { fallback, .. } => fallback
            .as_deref()
            .map_or(0, |root| ui::measure::height(root, width, &cx)),
        UiView::Lines { lines } => u16::try_from(lines.len()).unwrap_or(u16::MAX),
        UiView::Status { .. } => 1,
        UiView::Renderer { .. } | UiView::Settings { .. } => 0,
    }
}

/// Rendered lines of an entry at `area`, cached per `entry.rev` (§12.4 point 3).
pub(super) fn ui_entry_lines(app: &mut App, id: &str, area: Rect) -> Vec<OLine> {
    let Some(entry) = app.registry.entry(id) else {
        return Vec::new();
    };
    let local = app.ui_local.get(id).cloned().unwrap_or_default();
    let animated = ui::animated(entry);
    let cx = ui::Ctx {
        theme: &app.theme,
        tick: app.tick,
        local: &local,
    };
    app.ui_cache.lines(entry, area, &cx, animated)
}

/// Rendered lines of an entry in an auto slot (§4.4), capped at `max` rows.
pub(super) fn ui_entry_auto(app: &mut App, id: &str, width: u16, max: u16) -> Vec<OLine> {
    let height = ui_entry_height(app, id, width).clamp(1, max.max(1));
    ui_entry_lines(
        app,
        id,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    )
}

/// Turn-footer lines of the turns that end at this row (§6.7).
fn turn_footers(
    app: &App,
    cx: &RenderCtx<'_>,
    cache: &mut std::collections::HashMap<(String, String, u64, bool), Vec<OLine>>,
    turn_entry: Option<&str>,
    turn_index: usize,
    running: bool,
) -> Vec<OLine> {
    let width = u16::try_from(cx.width).unwrap_or(u16::MAX);
    let mut out = Vec::new();
    for entry in app.registry.entries_for(Slot::TurnFooter) {
        let lines = match &entry.view {
            UiView::Renderer { key } => {
                // Live (not yet persisted) turns have no entry id: key by turn index so two
                // running turns never share a cached footer.
                let turn = turn_entry.map_or_else(|| format!("#turn-{turn_index}"), str::to_owned);
                let cache_key = (entry.id.clone(), turn, entry.rev, running);
                if let Some(lines) = cache.get(&cache_key) {
                    lines.clone()
                } else {
                    let request = RenderRequest {
                        target: "turnFooter",
                        key: key.clone(),
                        entry_id: Some(entry.id.clone()),
                        turn_entry_id: turn_entry.map(str::to_owned),
                        turn_index: Some(u64::try_from(turn_index).unwrap_or(0)),
                        running,
                        density: RenderRequest::density_for(false),
                        expanded: false,
                        width: RenderRequest::width_for(cx.width),
                        data: serde_json::Value::Null,
                    };
                    let lines = cx
                        .host
                        .render(&request)
                        .and_then(|node| serde_json::from_value::<oino_ui::node::Node>(node).ok())
                        .map(|node| {
                            let local = ui::focus::UiLocalState::default();
                            let measure_ctx = ui::Ctx {
                                theme: cx.theme,
                                tick: cx.tick,
                                local: &local,
                            };
                            let height = ui::measure::height(&node, width, &measure_ctx);
                            render_v2_node(&node, width, height.min(6), cx.theme)
                        })
                        .unwrap_or_default();
                    cache.insert(cache_key, lines.clone());
                    lines
                }
            }
            UiView::Reserved { fallback, .. } => {
                fallback.as_deref().map_or_else(Vec::new, |node| {
                    let local = ui::focus::UiLocalState::default();
                    let measure_ctx = ui::Ctx {
                        theme: cx.theme,
                        tick: cx.tick,
                        local: &local,
                    };
                    let height = ui::measure::height(node, width, &measure_ctx);
                    render_v2_node(node, width, height.min(6), cx.theme)
                })
            }
            _ => Vec::new(),
        };
        out.extend(lines);
    }
    out
}

/// Draws the whole frame.
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let base = Style::default().fg(app.theme.text);
    if area.width < 20 || area.height < 6 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Terminal too small",
                Style::default().fg(app.theme.warning),
            ))),
            area,
        );
        return;
    }
    let width = usize::from(area.width);

    // Bottom stack, top to bottom.
    let status = chrome::status_lines(app, width);
    let above = chrome::widget_lines(app, Slot::Widget, width);
    let editor = chrome::editor_block(app, width, usize::from(area.height) / 2);
    let below = chrome::widget_lines(app, Slot::WidgetBelow, width);
    let footer = chrome::footer_lines(app, width);
    let mut bottom: Vec<OLine> = Vec::new();
    bottom.extend(status);
    bottom.extend(above);
    let editor_top = height(&bottom);
    bottom.extend(editor.lines);
    bottom.extend(below);
    bottom.extend(footer);
    let max_bottom = area.height.saturating_sub(3);
    // Keep the editor and what follows; drop lines from the top of the stack.
    let overflow = height(&bottom).saturating_sub(max_bottom);
    bottom.drain(..usize::from(overflow).min(bottom.len()));
    let bottom_height = height(&bottom);
    let transcript_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height - bottom_height,
    };
    let bottom_area = Rect {
        x: area.x,
        y: area.y + transcript_area.height,
        width: area.width,
        height: bottom_height,
    };
    if let Some(id) = slot_entry_id(app, Slot::Fullscreen) {
        let block = overlays::ui_entry_block(app, &id, width, usize::from(transcript_area.height));
        frame.render_widget(Clear, transcript_area);
        frame.render_widget(Paragraph::new(block.lines).style(base), transcript_area);
    } else {
        draw_transcript(frame, app, transcript_area);
    }
    frame.render_widget(Paragraph::new(bottom).style(base), bottom_area);

    // Composer cursor.
    if let Some((x, y)) = editor.cursor {
        let cy = bottom_area.y + (editor_top + y).saturating_sub(overflow);
        if cy < bottom_area.y + bottom_area.height {
            frame.set_cursor_position(Position::new(area.x + x.min(area.width - 1), cy));
        }
    }

    // Autocomplete popup just above the editor.
    if let Some(popup) = chrome::completion_popup(app, width) {
        let h = height(&popup).min(transcript_area.height);
        let editor_y = (bottom_area.y + editor_top).saturating_sub(overflow);
        let y = editor_y.saturating_sub(h).max(area.y);
        let rect = Rect {
            x: area.x,
            y,
            width: area.width,
            height: h,
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(popup).style(base), rect);
    }

    // v2 overlays / side-pane tabs over the transcript; dialogs sit above them (§3.2).
    if app.ui.active_dialog().is_none()
        && app.overlay.is_none()
        && let Some(block) = overlays::ui_modal(app, width, usize::from(transcript_area.height))
    {
        draw_modal(frame, block, area, transcript_area, base);
    }
    // Modal panels and large custom dialogs over the transcript.
    if let Some(block) = overlays::modal(app, width, usize::from(transcript_area.height)) {
        draw_modal(frame, block, area, transcript_area, base);
    }
}

/// Centres a modal block inside the transcript area.
fn draw_modal(frame: &mut Frame<'_>, block: Block, area: Rect, transcript_area: Rect, base: Style) {
    let h = height(&block.lines).min(transcript_area.height);
    let w = block
        .lines
        .iter()
        .map(text::line_width)
        .max()
        .unwrap_or(0)
        .min(usize::from(area.width));
    let w = u16::try_from(w).unwrap_or(area.width);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (transcript_area.height - h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(block.lines).style(base), rect);
    if let Some((x, y)) = block.cursor {
        frame.set_cursor_position(Position::new(rect.x + x, rect.y + y));
    }
}

/// Header lines at the top of the transcript (built-in header).
fn header_lines(app: &mut App, width: usize) -> Vec<OLine> {
    let quiet = app
        .settings
        .get("quietStartup")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut out = Vec::new();
    if !quiet {
        out.extend(chrome::builtin_header(app, width));
    }
    let ids = slot_ids(app, Slot::Header);
    for id in ids {
        out.push(Line::default());
        out.extend(ui_entry_auto(
            app,
            &id,
            u16::try_from(width).unwrap_or(u16::MAX),
            12,
        ));
    }
    out
}

/// Inline layout: the header and every transcript row not printed yet, once the session
/// is idle (rows of a running turn still change). The runner writes them above the
/// viewport (`Terminal::insert_before`); they are never redrawn.
pub fn take_scrollback(app: &mut App, width: u16) -> Vec<Line<'static>> {
    if !app.inline || app.busy() || app.bash_running.is_some() || !app.loaded {
        return Vec::new();
    }
    let rows = app.transcript.rows().len();
    if app.committed_rows > rows {
        // The transcript was rebuilt (new or switched session): print it again.
        app.committed_rows = 0;
    }
    if app.committed_rows == rows && app.header_committed {
        return Vec::new();
    }
    let lines = transcript_lines(app, width, app.committed_rows, !app.header_committed);
    app.committed_rows = rows;
    app.header_committed = true;
    lines
}

/// Rendered transcript lines from row `first` (with the header when `header`), blank lines
/// between rows; uses and fills the row cache.
fn transcript_lines(app: &mut App, w16: u16, first: usize, header: bool) -> Vec<OLine> {
    let width = usize::from(w16);
    let flags = u8::from(app.tools_expanded) | (u8::from(app.hide_thinking) << 1);
    let mut lines = if header {
        header_lines(app, width)
    } else {
        Vec::new()
    };
    let all_rows = app.transcript.rows();
    let base_turns = all_rows
        .iter()
        .take(first)
        .filter(|row| matches!(row.kind, oino_view::transcript::RowKind::User(_)))
        .count();
    let turn_count = all_rows
        .iter()
        .filter(|row| matches!(row.kind, oino_view::transcript::RowKind::User(_)))
        .count();
    let rows: Vec<_> = all_rows.iter().skip(first).cloned().collect();
    let streaming = app.session.streaming;
    let mut footer_cache = std::mem::take(&mut app.footer_cache);
    let cx = ctx(app, width);
    let mut fresh: Vec<(u64, CachedRow)> = Vec::new();
    let mut turn_entry: Option<String> = None;
    let mut turn_index = base_turns;
    let mut seen_turn = false;
    for (index, row) in rows.iter().enumerate() {
        if let oino_view::transcript::RowKind::User(user) = &row.kind {
            if seen_turn {
                turn_index += 1;
            }
            seen_turn = true;
            turn_entry.clone_from(&user.entry_id);
        }
        let cached = app
            .cache
            .get(&row.id)
            .filter(|c| c.rev == row.rev && c.width == w16 && c.flags == flags);
        let row_lines = if let Some(c) = cached {
            c.lines.clone()
        } else {
            let rendered = transcript::row_lines(row, &cx);
            fresh.push((
                row.id,
                CachedRow {
                    rev: row.rev,
                    width: w16,
                    flags,
                    lines: rendered.clone(),
                },
            ));
            rendered
        };
        if !row_lines.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.extend(row_lines);
        }
        let last_of_turn = rows
            .get(index + 1)
            .is_none_or(|next| matches!(next.kind, oino_view::transcript::RowKind::User(_)));
        if last_of_turn {
            let running = streaming && turn_index + 1 == turn_count;
            let footers = turn_footers(
                app,
                &cx,
                &mut footer_cache,
                turn_entry.as_deref(),
                turn_index,
                running,
            );
            if !footers.is_empty() {
                lines.extend(footers);
            }
        }
    }
    drop(cx);
    app.footer_cache = footer_cache;
    for (id, row) in fresh {
        app.cache.insert(id, row);
    }
    lines
}

fn draw_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let (first, header) = if app.inline {
        (app.committed_rows, !app.header_committed)
    } else {
        (0, true)
    };
    let lines = transcript_lines(app, area.width, first, header);
    let visible = usize::from(area.height);
    app.transcript_height = visible;
    let total = lines.len();
    let max_scroll = total.saturating_sub(visible);
    app.scroll = app.scroll.min(max_scroll);
    let end = total - app.scroll;
    let start = end.saturating_sub(visible);
    let mut shown: Vec<OLine> = lines[start..end].to_vec();
    if app.scroll > 0
        && let Some(last) = shown.last_mut()
    {
        *last = Line::from(Span::styled(
            format!("↓ {} more lines (pageDown / ctrl+end)", app.scroll),
            Style::default().fg(app.theme.dim),
        ));
    }
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(app.theme.text)),
        area,
    );
}
