//! Frame layout and drawing (pi `TUI` container order: chat, pending messages, status,
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

use std::path::PathBuf;
use std::sync::Arc;

use oino_view::ui_state::PanelPlacement;
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
    if let Some(panel) = app.ui.focused_panel().cloned()
        && panel.placement == PanelPlacement::Fullscreen
    {
        let block = overlays::panel_block(app, &panel, width, usize::from(area.height));
        frame.render_widget(Paragraph::new(block.lines).style(base), area);
        return;
    }

    // Bottom stack, top to bottom.
    let status = chrome::status_lines(app, width);
    let above = chrome::widget_lines(app, oino_view::ui_state::Placement::AboveEditor, width);
    let editor = chrome::editor_block(app, width, usize::from(area.height) / 2);
    let below = chrome::widget_lines(app, oino_view::ui_state::Placement::BelowEditor, width);
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
    draw_transcript(frame, app, transcript_area);
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

    // Modal panels and large custom dialogs over the transcript.
    if let Some(block) = overlays::modal(app, width, usize::from(transcript_area.height)) {
        let h = height(&block.lines).min(transcript_area.height);
        let w = block
            .lines
            .iter()
            .map(text::line_width)
            .max()
            .unwrap_or(0)
            .min(width);
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
}

/// Header lines at the top of the transcript (pi built-in header).
fn header_lines(app: &App, width: usize) -> Vec<OLine> {
    let quiet = app
        .settings
        .get("quietStartup")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut out = Vec::new();
    if !quiet {
        out.extend(chrome::builtin_header(app, width));
    }
    let cx = ctx(app, width);
    for spec in app.ui.headers.values() {
        out.push(Line::default());
        out.extend(text::boxed(
            ous::render(
                spec,
                width.saturating_sub(2),
                &app.theme,
                ous::Focus::default(),
                app.tick,
            ),
            width,
            1,
            None,
        ));
    }
    for widget in app.ui.widgets_at(oino_view::ui_state::Placement::Header) {
        out.extend(chrome::widget_body(widget, &cx));
    }
    out
}

fn draw_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let width = usize::from(area.width);
    let w16 = area.width;
    let flags = u8::from(app.tools_expanded) | (u8::from(app.hide_thinking) << 1);
    let mut lines = header_lines(app, width);
    let rows: Vec<_> = app.transcript.rows().to_vec();
    let cx = ctx(app, width);
    let mut fresh: Vec<(u64, CachedRow)> = Vec::new();
    for row in &rows {
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
        if row_lines.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(row_lines);
    }
    drop(cx);
    for (id, row) in fresh {
        app.cache.insert(id, row);
    }
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
