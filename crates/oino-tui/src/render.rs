#![forbid(unsafe_code)]

use crate::{
    app::{
        ChordState, OverlayKind, SendPanelItem, SendPanelSection, SessionListItem, TuiFocus,
        TuiState,
    },
    command::{CommandSuggestionCategory, CommandSuggestionsView},
    composer::{byte_index_at_char, char_count, ComposerState, INPUT_PLACEHOLDER},
    help::{help_entries, HelpEntry},
    keymap::{key_action_rows, KeymapPreset, ShortcutKind},
    settings::{
        chat_style_label, chat_style_value, collapse_mode_label, thinking_label, ChatStyle,
        KeymapsMode, SettingsPage, SettingsState,
    },
    text::{truncate_to_width, truncate_with_ellipsis, wrap_text, wrapped_line_count},
    theme::{theme_cache_hash, Theme},
    transcript::transcript_line_blocks,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const MIN_TRANSCRIPT_HEIGHT: u16 = 3;
const MIN_COMPOSER_ROWS: usize = 3;
const MAX_COMPOSER_HEIGHT: u16 = 9;
const INPUT_PROMPT: &str = "› ";
const TINY_MESSAGE: &str = "Oino needs at least 20x8";
const TRANSCRIPT_LEFT_PADDING: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalClickTargetKind {
    Url,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalClickTarget {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub text: String,
    pub target: String,
    pub kind: TerminalClickTargetKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUrlOverlay {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub url: String,
}

#[derive(Debug, Clone)]
struct PreparedTranscriptBlock {
    start: usize,
    lines: Arc<Vec<Line<'static>>>,
}

#[derive(Debug, Clone)]
struct PreparedTranscript {
    blocks: Vec<PreparedTranscriptBlock>,
    total_lines: usize,
}

impl PreparedTranscript {
    fn from_blocks(blocks: Vec<Arc<Vec<Line<'static>>>>) -> Self {
        let mut prepared_blocks = Vec::with_capacity(blocks.len());
        let mut total_lines = 0usize;
        for lines in blocks {
            if lines.is_empty() {
                continue;
            }
            let len = lines.len();
            prepared_blocks.push(PreparedTranscriptBlock {
                start: total_lines,
                lines,
            });
            total_lines = total_lines.saturating_add(len);
        }
        Self {
            blocks: prepared_blocks,
            total_lines,
        }
    }

    const fn total_lines(&self) -> usize {
        self.total_lines
    }

    fn materialize_line_slice(&self, start: usize, end: usize) -> Vec<Line<'static>> {
        let end = end.min(self.total_lines);
        if start >= end {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(end - start);
        for block in &self.blocks[self.first_overlapping_block_index(start)..] {
            let block_start = block.start;
            if block_start >= end {
                break;
            }
            let block_end = block_start.saturating_add(block.lines.len());
            let overlap_start = start.max(block_start);
            let overlap_end = end.min(block_end);
            if overlap_start >= overlap_end {
                continue;
            }
            let local_start = overlap_start - block_start;
            let local_end = overlap_end - block_start;
            out.extend_from_slice(&block.lines[local_start..local_end]);
        }
        out
    }

    fn first_overlapping_block_index(&self, line: usize) -> usize {
        self.blocks
            .partition_point(|block| block.start.saturating_add(block.lines.len()) <= line)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TranscriptCacheKey {
    width: usize,
    transcript_version: u64,
    message_count: usize,
    thinking_mode: u8,
    tool_mode: u8,
    chat_style: u8,
    status_kind: u8,
    status_text: Option<String>,
    error: Option<String>,
    theme_hash: u64,
}

#[derive(Default)]
struct TranscriptCacheState {
    entries: HashMap<TranscriptCacheKey, Arc<PreparedTranscript>>,
    order: VecDeque<TranscriptCacheKey>,
}

impl TranscriptCacheState {
    fn get(&mut self, key: &TranscriptCacheKey) -> Option<Arc<PreparedTranscript>> {
        let prepared = self.entries.get(key)?.clone();
        if let Some(position) = self.order.iter().position(|entry| entry == key) {
            if let Some(entry) = self.order.remove(position) {
                self.order.push_back(entry);
            }
        }
        Some(prepared)
    }

    fn insert(&mut self, key: TranscriptCacheKey, prepared: Arc<PreparedTranscript>) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key.clone(), prepared);
            if let Some(position) = self.order.iter().position(|entry| entry == &key) {
                let _ = self.order.remove(position);
            }
            self.order.push_back(key);
            return;
        }
        self.entries.insert(key.clone(), prepared);
        self.order.push_back(key);
        while self.order.len() > TRANSCRIPT_CACHE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

const TRANSCRIPT_CACHE_LIMIT: usize = 12;
static TRANSCRIPT_CACHE: OnceLock<Mutex<TranscriptCacheState>> = OnceLock::new();

fn transcript_cache() -> &'static Mutex<TranscriptCacheState> {
    TRANSCRIPT_CACHE.get_or_init(|| Mutex::new(TranscriptCacheState::default()))
}

pub fn render(frame: &mut Frame<'_>, state: &TuiState) {
    render_with_theme(frame, state, &Theme::default());
}

pub fn render_with_theme(frame: &mut Frame<'_>, state: &TuiState, theme: &Theme) {
    let area = frame.area();
    if area.width < 20 || area.height < 8 {
        render_tiny(frame, area, theme);
        return;
    }

    let composer_height = composer_height(state.composer.text(), area.width, area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(MIN_TRANSCRIPT_HEIGHT),
            Constraint::Length(composer_height),
        ])
        .split(area);

    render_transcript(frame, chunks[0], state, theme);
    render_composer(frame, chunks[1], state, theme);

    if state.overlay.is_none() {
        if let Some(suggestions) = state.command_suggestions_view() {
            render_command_suggestions(frame, area, chunks[1], &suggestions, theme);
        }
    }

    match state.overlay {
        Some(OverlayKind::Help) => render_help_overlay(frame, area, state, theme),
        Some(OverlayKind::Settings) => render_settings_overlay(frame, area, &state.settings, theme),
        Some(OverlayKind::SendPanel) => render_send_panel_overlay(frame, area, state, theme),
        Some(OverlayKind::Sessions) => render_sessions_overlay(frame, area, state, theme),
        Some(OverlayKind::Prompts) => render_prompts_overlay(frame, area, state, theme),
        Some(OverlayKind::Skills) => render_skills_overlay(frame, area, state, theme),
        Some(OverlayKind::Inspect) => render_inspect_overlay(frame, area, state, theme),
        None => {}
    }

    if state.chord != ChordState::None {
        render_chord_hint(frame, area, state, theme);
    }
}

fn render_chord_hint(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    if state.chord == ChordState::None {
        return;
    }
    let keymap = &state.settings.keymap;
    let title = format!(
        " {}: Enter queue • / draft • s settings • q send panel • Esc cancel ",
        keymap.chord_key
    );
    frame.render_widget(
        Block::default()
            .title(Span::styled(title, theme.error))
            .borders(Borders::ALL)
            .border_style(theme.error),
        area,
    );
}

fn render_tiny(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let paragraph = Paragraph::new(TINY_MESSAGE).style(theme.warning);
    frame.render_widget(paragraph, area);
}

pub fn transcript_visible_lines(state: &TuiState, width: u16, height: u16) -> usize {
    if width < 20 || height < 8 {
        return 1;
    }
    let composer_height = composer_height(state.composer.text(), width, height);
    height
        .saturating_sub(composer_height)
        .saturating_sub(2)
        .max(1) as usize
}

pub fn terminal_cursor_position(state: &TuiState, width: u16, height: u16) -> Option<(u16, u16)> {
    if width < 20 || height < 8 || state.focus != TuiFocus::Composer || !state.composer.is_enabled()
    {
        return None;
    }
    let composer_height = composer_height(state.composer.text(), width, height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(MIN_TRANSCRIPT_HEIGHT),
            Constraint::Length(composer_height),
        ])
        .split(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
    let position = composer_cursor_position(chunks[1], &state.composer);
    Some((position.x, position.y))
}

pub fn transcript_url_overlays(
    state: &TuiState,
    width: u16,
    height: u16,
) -> Vec<TerminalUrlOverlay> {
    transcript_click_targets(state, width, height)
        .into_iter()
        .filter(|target| target.kind == TerminalClickTargetKind::Url)
        .map(|target| TerminalUrlOverlay {
            x: target.x,
            y: target.y,
            text: target.text,
            url: target.target,
        })
        .collect()
}

pub fn transcript_click_targets(
    state: &TuiState,
    width: u16,
    height: u16,
) -> Vec<TerminalClickTarget> {
    if width < 20
        || height < 8
        || state.overlay.is_some()
        || state.chord != ChordState::None
        || state.command_suggestions_view().is_some()
    {
        return Vec::new();
    }

    let composer_height = composer_height(state.composer.text(), width, height);
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height: height.saturating_sub(composer_height),
    };
    let inner_height = area.height.saturating_sub(2) as usize;
    if inner_height == 0 {
        return Vec::new();
    }

    let theme = Theme::default();
    let full_inner_width = transcript_full_content_width(area.width);
    let mut transcript = prepared_transcript_for_width(state, full_inner_width, &theme);
    let mut has_scrollbar =
        transcript.total_lines() > inner_height && area.width > 4 && inner_height > 1;
    if has_scrollbar {
        transcript =
            prepared_transcript_for_width(state, full_inner_width.saturating_sub(1).max(1), &theme);
        has_scrollbar = transcript.total_lines() > inner_height;
    }
    let content_width = full_inner_width.saturating_sub(usize::from(has_scrollbar));
    let start = state
        .transcript_scroll
        .visible_start(transcript.total_lines(), inner_height);
    let visible_lines =
        transcript.materialize_line_slice(start, start.saturating_add(inner_height));

    let mut targets = Vec::new();
    for (visible_index, line) in visible_lines.iter().enumerate() {
        let plain = plain_line(line);
        for line_target in line_click_targets(&plain) {
            if line_target.column.saturating_add(line_target.width) > content_width {
                continue;
            }
            targets.push(TerminalClickTarget {
                x: transcript_content_x(area).saturating_add(line_target.column as u16),
                y: area
                    .y
                    .saturating_add(1)
                    .saturating_add(visible_index as u16),
                width: u16::try_from(line_target.width).unwrap_or(u16::MAX),
                text: line_target.text,
                target: line_target.target,
                kind: line_target.kind,
            });
        }
    }
    targets
}

fn transcript_full_content_width(area_width: u16) -> usize {
    area_width
        .saturating_sub(2)
        .saturating_sub(TRANSCRIPT_LEFT_PADDING) as usize
}

fn transcript_content_x(area: Rect) -> u16 {
    area.x
        .saturating_add(1)
        .saturating_add(TRANSCRIPT_LEFT_PADDING)
}

fn render_transcript(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let full_inner_width = transcript_full_content_width(area.width);
    let mut transcript = prepared_transcript_for_width(state, full_inner_width, theme);
    let mut has_scrollbar =
        transcript.total_lines() > inner_height && area.width > 4 && inner_height > 1;
    if has_scrollbar {
        transcript =
            prepared_transcript_for_width(state, full_inner_width.saturating_sub(1).max(1), theme);
        has_scrollbar = transcript.total_lines() > inner_height;
    }

    let total_lines = transcript.total_lines();
    let start = state
        .transcript_scroll
        .visible_start(total_lines, inner_height);
    let scrolled_offset = state
        .transcript_scroll
        .resolved_offset_from_bottom(total_lines, inner_height);
    let lines = transcript.materialize_line_slice(start, start.saturating_add(inner_height));

    let title = match (state.working, scrolled_offset) {
        (true, 0) => transcript_title_with_status(state, "Generating…", None),
        (true, offset) => transcript_title_with_status(state, "Generating…", Some(offset)),
        (false, 0) => transcript_title(state, None),
        (false, offset) => transcript_title(state, Some(offset)),
    };
    let border_style = if state.focus == TuiFocus::Transcript {
        Style::default().fg(theme.focused_border)
    } else {
        Style::default().fg(theme.panel_border)
    };
    let title_style = if state.working {
        theme.working.add_modifier(Modifier::BOLD)
    } else {
        theme.title
    };
    let block = Block::default()
        .title(Span::styled(title, title_style))
        .borders(Borders::ALL)
        .border_style(border_style)
        .padding(Padding::left(TRANSCRIPT_LEFT_PADDING));
    frame.render_widget(Paragraph::new(lines).block(block), area);

    if has_scrollbar {
        render_transcript_scrollbar(frame, area, start, inner_height, total_lines, theme);
    }
}

fn transcript_title(state: &TuiState, offset: Option<usize>) -> String {
    let title = state.session_title.trim();
    let base = if title.is_empty() {
        "Oino".to_string()
    } else {
        format!("Oino • {title}")
    };
    match offset {
        Some(offset) => format!(" {base} ↑{offset} "),
        None => format!(" {base} "),
    }
}

fn transcript_title_with_status(state: &TuiState, status: &str, offset: Option<usize>) -> String {
    let title = state.session_title.trim();
    let base = if title.is_empty() {
        format!("Oino • {status}")
    } else {
        format!("Oino • {title} • {status}")
    };
    match offset {
        Some(offset) => format!(" {base} ↑{offset} "),
        None => format!(" {base} "),
    }
}

fn prepared_transcript_for_width(
    state: &TuiState,
    width: usize,
    theme: &Theme,
) -> Arc<PreparedTranscript> {
    let (status_kind, status_text) = transcript_status(state);
    let key = TranscriptCacheKey {
        width,
        transcript_version: state.transcript_version(),
        message_count: state.messages.len(),
        thinking_mode: collapse_mode_key(state.settings.thinking_collapse_mode),
        tool_mode: collapse_mode_key(state.settings.tool_collapse_mode),
        chat_style: chat_style_key(state.settings.chat_style),
        status_kind,
        status_text: status_text.clone(),
        error: state.error.clone(),
        theme_hash: theme_cache_hash(theme),
    };

    if !cfg!(test) {
        let mut cache = match transcript_cache().lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(prepared) = cache.get(&key) {
            return prepared;
        }
        drop(cache);
    }

    let prepared = Arc::new(build_prepared_transcript(
        state,
        width,
        theme,
        status_kind,
        status_text,
    ));

    if !cfg!(test) {
        if let Ok(mut cache) = transcript_cache().lock() {
            cache.insert(key, prepared.clone());
        }
    }

    prepared
}

fn build_prepared_transcript(
    state: &TuiState,
    width: usize,
    theme: &Theme,
    status_kind: u8,
    status_text: Option<String>,
) -> PreparedTranscript {
    let mut blocks = transcript_line_blocks(
        &state.messages,
        state.error.as_deref(),
        width,
        state.settings.thinking_collapse_mode,
        state.settings.tool_collapse_mode,
        state.settings.chat_style,
        theme,
    );

    if let Some(status) = status_text {
        if blocks.iter().any(|block| !block.is_empty()) {
            blocks.push(Arc::new(vec![Line::from("")]));
        }
        blocks.push(status_line_block(status_kind, status, theme));
    }

    if !blocks.iter().any(|block| !block.is_empty()) {
        blocks.push(Arc::new(vec![Line::from(vec![Span::styled(
            "No messages yet. Send a task to start.",
            Style::default().fg(theme.muted),
        )])]));
    }

    PreparedTranscript::from_blocks(blocks)
}

fn status_line_block(status_kind: u8, status: String, theme: &Theme) -> Arc<Vec<Line<'static>>> {
    let line = if status_kind == TRANSCRIPT_STATUS_ACTIVITY {
        Line::from(vec![
            Span::styled("● ", theme.working.add_modifier(Modifier::BOLD)),
            Span::styled(status, theme.working),
        ])
    } else {
        Line::from(vec![
            Span::styled("• ", Style::default().fg(theme.muted)),
            Span::styled(status, theme.footer),
        ])
    };
    Arc::new(vec![line])
}

const TRANSCRIPT_STATUS_NONE: u8 = 0;
const TRANSCRIPT_STATUS_ACTIVITY: u8 = 1;
const TRANSCRIPT_STATUS_NOTICE: u8 = 2;

fn transcript_status(state: &TuiState) -> (u8, Option<String>) {
    if let Some(status) = state.activity_status() {
        (TRANSCRIPT_STATUS_ACTIVITY, Some(status))
    } else if let Some(status) = state.notice_status() {
        (TRANSCRIPT_STATUS_NOTICE, Some(status))
    } else {
        (TRANSCRIPT_STATUS_NONE, None)
    }
}

const fn collapse_mode_key(mode: crate::settings::CollapseMode) -> u8 {
    match mode {
        crate::settings::CollapseMode::Full => 0,
        crate::settings::CollapseMode::Truncate => 1,
        crate::settings::CollapseMode::Collapse => 2,
    }
}

const fn chat_style_key(style: ChatStyle) -> u8 {
    match style {
        ChatStyle::Chat => 0,
        ChatStyle::Agentic => 1,
        ChatStyle::Minimal => 2,
    }
}

fn plain_line(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineClickTarget {
    column: usize,
    width: usize,
    text: String,
    target: String,
    kind: TerminalClickTargetKind,
}

fn line_click_targets(text: &str) -> Vec<LineClickTarget> {
    let mut targets = url_ranges(text)
        .into_iter()
        .map(|(column, width, text, url)| LineClickTarget {
            column,
            width,
            text,
            target: url,
            kind: TerminalClickTargetKind::Url,
        })
        .collect::<Vec<_>>();
    targets.extend(
        image_ranges(text)
            .into_iter()
            .map(|(column, width, target)| LineClickTarget {
                column,
                width,
                text: target.clone(),
                target,
                kind: TerminalClickTargetKind::Image,
            }),
    );
    targets
}

fn url_ranges(text: &str) -> Vec<(usize, usize, String, String)> {
    let mut ranges = Vec::new();
    let mut search_start = 0usize;
    while search_start < text.len() {
        let rest = &text[search_start..];
        let Some(relative_start) = next_url_start(rest) else {
            break;
        };
        let start = search_start + relative_start;
        let mut end = text.len();
        for (offset, ch) in text[start..].char_indices() {
            if ch.is_whitespace() || matches!(ch, '<' | '>' | '"') {
                end = start + offset;
                break;
            }
        }
        let mut url = text[start..end].to_string();
        trim_url_trailing_punctuation(&mut url);
        if !url.is_empty() {
            let visible_end = start + url.len();
            let visible_start = rendered_link_start(text, start).unwrap_or(start);
            let visible = text[visible_start..visible_end].to_string();
            ranges.push((text[..visible_start].width(), visible.width(), visible, url));
        }
        search_start = end.max(start.saturating_add(1));
    }
    ranges
}

fn rendered_link_start(text: &str, url_start: usize) -> Option<usize> {
    let arrow = " ↗ ";
    let prefix = text.get(..url_start)?;
    if !prefix.ends_with(arrow) {
        return None;
    }
    let arrow_start = url_start.saturating_sub(arrow.len());
    let before_arrow = text.get(..arrow_start)?;
    let delimiters = ["• ", "✓ ", "○ ", "☑ ", "☐ ", ": ", "│ ", "| "];
    let start = delimiters
        .iter()
        .filter_map(|delimiter| {
            before_arrow
                .rfind(delimiter)
                .map(|index| index + delimiter.len())
        })
        .max()
        .or_else(|| {
            before_arrow
                .char_indices()
                .rev()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(index, ch)| index + ch.len_utf8())
        })
        .unwrap_or(0);
    Some(start)
}

fn image_ranges(text: &str) -> Vec<(usize, usize, String)> {
    let mut ranges = Vec::new();
    let mut search_start = 0usize;
    while search_start < text.len() {
        let Some(relative_start) = text[search_start..].find("[image:") else {
            break;
        };
        let start = search_start + relative_start;
        let Some(label_end_relative) = text[start..].find("] (") else {
            break;
        };
        let target_start = start + label_end_relative + "] (".len();
        let Some(target_end_relative) = text[target_start..].find(')') else {
            break;
        };
        let target_end = target_start + target_end_relative;
        let full_end = target_end + 1;
        let target = text[target_start..target_end].trim().to_string();
        if !target.is_empty() {
            ranges.push((text[..start].width(), text[start..full_end].width(), target));
        }
        search_start = full_end;
    }
    ranges
}

fn next_url_start(text: &str) -> Option<usize> {
    [text.find("https://"), text.find("http://")]
        .into_iter()
        .flatten()
        .min()
}

fn trim_url_trailing_punctuation(url: &mut String) {
    while url
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':' | ')' | ']' | '}'))
    {
        url.pop();
    }
}

fn render_transcript_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    start: usize,
    visible_lines: usize,
    total_lines: usize,
    theme: &Theme,
) {
    let viewport = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    if viewport.width == 0 || viewport.height == 0 || total_lines <= visible_lines {
        return;
    }
    let scrollable_range = total_lines.saturating_sub(visible_lines);
    let mut scrollbar_state = ScrollbarState::new(scrollable_range)
        .position(start.min(scrollable_range))
        .viewport_content_length(visible_lines);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(Style::default().fg(theme.panel_border))
            .thumb_symbol("┃")
            .thumb_style(
                Style::default()
                    .fg(theme.focused_border)
                    .add_modifier(Modifier::BOLD),
            ),
        viewport,
        &mut scrollbar_state,
    );
}

fn render_composer(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let title = if state.working {
        " Task • steer while streaming "
    } else {
        " Task "
    };
    let border_style = if state.focus == TuiFocus::Composer && state.composer.is_enabled() {
        Style::default().fg(theme.focused_border)
    } else {
        Style::default().fg(theme.panel_border)
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);
    let lines = composer_lines(area, &state.composer, theme);
    frame.render_widget(Paragraph::new(lines).block(block), area);

    if state.focus == TuiFocus::Composer && state.composer.is_enabled() {
        frame.set_cursor_position(composer_cursor_position(area, &state.composer));
    }
}

fn render_command_suggestions(
    frame: &mut Frame<'_>,
    full_area: Rect,
    composer_area: Rect,
    suggestions: &CommandSuggestionsView,
    theme: &Theme,
) {
    let available_height = composer_area.y.saturating_sub(full_area.y).max(1);
    let max_content_rows = command_suggestion_max_rows(suggestions);
    let desired_content_rows = suggestions.items.len().max(1).min(max_content_rows);
    let height = u16::try_from(desired_content_rows.saturating_add(2))
        .unwrap_or(u16::MAX)
        .min(available_height);
    if height < 3 {
        return;
    }
    let content_capacity = height.saturating_sub(2).max(1) as usize;
    let width = full_area.width.saturating_sub(4).clamp(24, 72);
    let x = full_area.x + full_area.width.saturating_sub(width) / 2;
    let y = composer_area.y.saturating_sub(height);
    let area = Rect {
        x,
        y,
        width,
        height,
    };
    frame.render_widget(Clear, area);

    let lines = if suggestions.items.is_empty() {
        vec![Line::styled(
            format!("No suggestion matches `{}`", suggestions.query),
            Style::default().fg(theme.muted),
        )]
    } else {
        let range = visible_range(
            suggestions.selected,
            suggestions.items.len(),
            content_capacity,
        );
        let start = range.start;
        suggestions.items[range]
            .iter()
            .enumerate()
            .map(|(offset, item)| {
                let index = start + offset;
                let active = index == suggestions.selected;
                let marker = arrow_marker(active);
                let style = item_style(active, false, theme);
                let mut spans = vec![Span::styled(marker.to_string(), style)];
                if let Some(label) = item.category.label() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        label,
                        command_category_style(item.category, theme),
                    ));
                }
                spans.push(Span::styled(format!(" {}", item.label), style));
                spans.push(Span::styled(
                    format!("  {}", item.summary),
                    Style::default().fg(theme.muted),
                ));
                Line::from(spans)
            })
            .collect()
    };

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(
                    " {} ",
                    command_suggestion_title(suggestions, content_capacity)
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.focused_border)),
        ),
        area,
    );
}

fn command_suggestion_max_rows(suggestions: &CommandSuggestionsView) -> usize {
    match suggestions.title.as_str() {
        "Files" => 10,
        "Models" => 5,
        _ => 4,
    }
}

fn command_category_style(category: CommandSuggestionCategory, theme: &Theme) -> Style {
    match category {
        CommandSuggestionCategory::System => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        CommandSuggestionCategory::Prompt => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        CommandSuggestionCategory::Skill => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        CommandSuggestionCategory::Model
        | CommandSuggestionCategory::File
        | CommandSuggestionCategory::Value => Style::default().fg(theme.muted),
    }
}

fn command_suggestion_title(
    suggestions: &CommandSuggestionsView,
    content_capacity: usize,
) -> String {
    if suggestions.items.is_empty() {
        return suggestions.title.clone();
    }
    if suggestions.title == "Models" || suggestions.items.len() > content_capacity {
        format!(
            "{} {}/{}",
            suggestions.title,
            suggestions
                .selected
                .saturating_add(1)
                .min(suggestions.items.len()),
            suggestions.items.len()
        )
    } else {
        suggestions.title.clone()
    }
}

fn render_inspect_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 88, 82);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Inspect ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let option = if state.inspect.loading {
        Line::from(vec![
            Span::styled("› ", Style::default().fg(theme.focused_border)),
            Span::styled(
                "Full prompt",
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" • loading…", Style::default().fg(theme.muted)),
        ])
    } else {
        Line::from(vec![
            Span::styled("› ", Style::default().fg(theme.focused_border)),
            Span::styled(
                "Full prompt",
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" • {} tokens", state.inspect.token_count),
                Style::default().fg(theme.muted),
            ),
        ])
    };
    let export_hint = state.inspect.export_message.as_deref().map_or_else(
        || {
            Line::styled(
                "Press e to export chat as HTML",
                Style::default().fg(theme.muted),
            )
        },
        |message| Line::styled(message.to_string(), Style::default().fg(theme.muted)),
    );
    frame.render_widget(Paragraph::new(vec![option, export_hint]), sections[0]);

    let content_width = sections[1].width as usize;
    let content_lines = if state.inspect.loading {
        vec![Line::styled(
            "Loading inspect snapshot…",
            Style::default().fg(theme.muted),
        )]
    } else if state.inspect.full_prompt.trim().is_empty() {
        vec![Line::styled(
            "No prompt snapshot available.",
            Style::default().fg(theme.muted),
        )]
    } else {
        state
            .inspect
            .full_prompt
            .lines()
            .flat_map(|line| {
                wrap_text(line, content_width.max(1))
                    .into_iter()
                    .map(Line::from)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let visible = sections[1].height as usize;
    let max_start = content_lines.len().saturating_sub(visible);
    let start = state.inspect.scroll.min(max_start);
    let end = start.saturating_add(visible).min(content_lines.len());
    frame.render_widget(
        Paragraph::new(content_lines[start..end].to_vec()),
        sections[1],
    );

    let footer = if content_lines.len() > visible {
        format!(
            "↑/↓ scroll • PgUp/PgDn page • e export • q/Esc close • {}/{}",
            start.saturating_add(1).min(content_lines.len()),
            content_lines.len()
        )
    } else {
        "e export • q/Esc close".into()
    };
    frame.render_widget(
        Paragraph::new(Line::styled(footer, Style::default().fg(theme.muted))),
        sections[2],
    );
}

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 78);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Help ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(1)])
        .split(inner);

    let entries = help_entries(&state.settings.keymap);
    let content_height = list_content_height(sections[0]);
    let content_width = sections[0].width.saturating_sub(2) as usize;
    let mut lines = vec![help_search_line(state, theme), Line::from("")];
    let entries_height = content_height.saturating_sub(lines.len()).max(1);
    let filtered_indices = state.filtered_help_indices();
    let max_scroll = filtered_indices.len().saturating_sub(entries_height);
    let start = state.help_scroll.min(max_scroll);
    let end = start
        .saturating_add(entries_height)
        .min(filtered_indices.len());
    if filtered_indices.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("No help topics match `{}`", state.help_search),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
    } else {
        lines.extend(
            filtered_indices[start..end]
                .iter()
                .filter_map(|entry_index| {
                    entries
                        .get(*entry_index)
                        .map(|entry| help_entry_line(entry, content_width, theme))
                }),
        );
    }
    let title = if state.help_search.trim().is_empty() {
        if max_scroll == 0 {
            " Oino Help ".to_string()
        } else {
            format!(
                " Oino Help {}-{} / {} ",
                start.saturating_add(1),
                end,
                filtered_indices.len()
            )
        }
    } else {
        format!(
            " Oino Help {} match{} for `{}` ",
            filtered_indices.len(),
            if filtered_indices.len() == 1 {
                ""
            } else {
                "es"
            },
            state.help_search
        )
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        sections[0],
    );

    let controls = if state.help_search_active {
        "type to fuzzy search • ↑/↓ scroll • Enter keep results • Esc clear search"
    } else if max_scroll == 0 {
        "Press / to search • Esc/q close"
    } else {
        "↑/↓ or j/k scroll • / search • PgUp/PgDn page • Home/End jump • Esc/q close"
    };
    frame.render_widget(
        Paragraph::new(truncate_to_width(controls, sections[1].width as usize)).style(theme.footer),
        sections[1],
    );
}

fn help_search_line(state: &TuiState, theme: &Theme) -> Line<'static> {
    if state.help_search_active {
        return Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.focused_border)),
            Span::raw(state.help_search.clone()),
            Span::styled("█", Style::default().fg(theme.focused_border)),
        ]);
    }
    if state.help_search.is_empty() {
        Line::styled("Press / to search help", Style::default().fg(theme.muted))
    } else {
        Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.muted)),
            Span::raw(state.help_search.clone()),
        ])
    }
}

fn help_entry_line(entry: &HelpEntry, width: usize, theme: &Theme) -> Line<'static> {
    match entry {
        HelpEntry::Heading(text) => Line::styled(
            truncate_with_ellipsis(text, width),
            theme.title.add_modifier(Modifier::BOLD),
        ),
        HelpEntry::Item(key, description) => {
            let separator = " — ";
            let prefix = format!("{key}{separator}");
            let prefix_width = UnicodeWidthStr::width(prefix.as_str());
            let description =
                truncate_with_ellipsis(description, width.saturating_sub(prefix_width));
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(theme.focused_border)),
                Span::styled(description, Style::default().fg(theme.fg)),
            ])
        }
        HelpEntry::Text(text) => Line::styled(
            truncate_with_ellipsis(text, width),
            Style::default().fg(theme.muted),
        ),
        HelpEntry::Blank => Line::from(""),
    }
}

fn render_send_panel_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 70);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Send Panel ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);

    let content_width = sections[0].width.saturating_sub(2) as usize;
    let content_height = sections[0].height.saturating_sub(2).max(1) as usize;
    let lines = send_panel_lines(state, content_width, content_height, theme);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Steer / Queue / Draft ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        sections[0],
    );

    let controls = if state.send_panel.confirm_delete {
        "Press y to confirm deletion • n/Esc cancel"
    } else {
        "↑/↓ select • Enter load • q queue input • d draft input • x delete • Esc close"
    };
    let status = if state.send_panel.confirm_delete || state.status.trim().is_empty() {
        controls.to_string()
    } else {
        format!("{} • {controls}", state.status)
    };
    frame.render_widget(
        Paragraph::new(truncate_to_width(&status, sections[1].width as usize)).style(theme.footer),
        sections[1],
    );
}

fn send_panel_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let items = state.send_panel_items();
    let mut lines = vec![Line::from(vec![
        Span::styled("Input: ", Style::default().fg(theme.muted)),
        if state.input().trim().is_empty() {
            Span::styled("empty", theme.placeholder)
        } else {
            Span::styled(
                panel_preview(state.input(), content_width.saturating_sub(7)),
                Style::default().fg(theme.fg),
            )
        },
    ])];
    lines.push(Line::from(""));

    for section in [
        SendPanelSection::Steer,
        SendPanelSection::Queue,
        SendPanelSection::Draft,
    ] {
        let count = items.iter().filter(|item| item.section == section).count();
        let heading = match section {
            SendPanelSection::Steer => format!("Steer ({count}) — Enter while streaming"),
            SendPanelSection::Queue => format!("Queue ({count}) — q sends current input here"),
            SendPanelSection::Draft => format!("Draft ({count}) — d parks current input"),
        };
        lines.push(Line::styled(
            heading,
            Style::default()
                .fg(theme.tool_border)
                .add_modifier(Modifier::BOLD),
        ));

        let mut section_has_items = false;
        for (flat_index, item) in items.iter().enumerate() {
            if item.section != section {
                continue;
            }
            section_has_items = true;
            let active = flat_index == state.send_panel.cursor;
            lines.push(send_panel_item_line(item, active, content_width, theme));
        }
        if !section_has_items {
            lines.push(Line::styled("  (empty)", Style::default().fg(theme.muted)));
        }
        lines.push(Line::from(""));
    }

    if lines.last().is_some_and(|line| plain_line(line).is_empty()) {
        lines.pop();
    }

    if lines.len() > content_height {
        let selected_line = send_panel_selected_line(&items, state.send_panel.cursor).unwrap_or(0);
        let range = visible_range(selected_line, lines.len(), content_height);
        return lines[range].to_vec();
    }

    lines
}

fn send_panel_item_line(
    item: &SendPanelItem,
    active: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = arrow_marker(active);
    let label = format!("{marker} {}. ", item.index.saturating_add(1));
    let preview_width = width.saturating_sub(label.width());
    Line::from(vec![
        Span::styled(label, item_style(active, false, theme)),
        Span::styled(
            panel_preview(&item.text, preview_width),
            item_style(active, false, theme),
        ),
    ])
}

fn send_panel_selected_line(items: &[SendPanelItem], cursor: usize) -> Option<usize> {
    let selected = items.get(cursor)?;
    let mut line = 2usize;
    for section in [
        SendPanelSection::Steer,
        SendPanelSection::Queue,
        SendPanelSection::Draft,
    ] {
        line = line.saturating_add(1);
        let mut section_has_items = false;
        for (flat_index, item) in items.iter().enumerate() {
            if item.section != section {
                continue;
            }
            section_has_items = true;
            if item.section == selected.section && flat_index == cursor {
                return Some(line);
            }
            line = line.saturating_add(1);
        }
        if !section_has_items {
            line = line.saturating_add(1);
        }
        line = line.saturating_add(1);
    }
    None
}

fn panel_preview(text: &str, width: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_to_width(&compact, width.max(1))
}

fn render_sessions_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 72);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Sessions ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);

    let content_height = list_content_height(sections[0]);
    let content_width = sections[0].width.saturating_sub(2) as usize;
    let lines = sessions_lines(state, content_width, content_height, theme);
    let filtered_indices = state.filtered_session_indices();
    let title = if state.sessions.loading || state.sessions.items.is_empty() {
        " Saved Sessions ".to_string()
    } else if filtered_indices.is_empty() {
        format!(" Saved Sessions 0/{} ", state.sessions.items.len())
    } else if state.sessions.search.trim().is_empty() {
        format!(
            " Saved Sessions {}/{} ",
            state
                .sessions
                .cursor
                .saturating_add(1)
                .min(state.sessions.items.len()),
            state.sessions.items.len()
        )
    } else {
        format!(
            " Saved Sessions {}/{} ({} total) ",
            state
                .session_cursor_filtered_position()
                .saturating_add(1)
                .min(filtered_indices.len()),
            filtered_indices.len(),
            state.sessions.items.len()
        )
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        sections[0],
    );

    let controls = if state.sessions.search_active {
        "type to fuzzy search • ↑/↓ move • Enter continue • Esc clear search"
    } else {
        "↑/↓ select • / search • Enter continue • r reload • Esc close"
    };
    let status = format!("{} • {controls}", state.status);
    frame.render_widget(
        Paragraph::new(truncate_to_width(&status, sections[1].width as usize)).style(theme.footer),
        sections[1],
    );
}

fn sessions_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![sessions_search_line(state, theme), Line::from("")];
    let remaining_height = content_height.saturating_sub(lines.len()).max(1);

    if state.sessions.loading {
        lines.push(Line::styled(
            "Loading saved sessions…",
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if state.sessions.items.is_empty() {
        lines.push(Line::styled(
            "No saved sessions yet.",
            Style::default().fg(theme.muted),
        ));
        lines.push(Line::styled(
            truncate_with_ellipsis(
                "Send a prompt to create one, or use /new when you explicitly want a fresh session.",
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }

    let filtered_indices = state.filtered_session_indices();
    if filtered_indices.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("No sessions match `{}`", state.sessions.search),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }

    let filtered_position = state.session_cursor_filtered_position();
    let range = visible_range(filtered_position, filtered_indices.len(), remaining_height);
    lines.extend(
        filtered_indices[range.clone()]
            .iter()
            .enumerate()
            .filter_map(|(offset, item_index)| {
                let item = state.sessions.items.get(*item_index)?;
                let display_index = range.start + offset;
                let active = *item_index == state.sessions.cursor;
                Some(sessions_item_line(
                    display_index,
                    item,
                    active,
                    content_width,
                    theme,
                ))
            }),
    );
    lines
}

fn sessions_search_line(state: &TuiState, theme: &Theme) -> Line<'static> {
    if state.sessions.search_active {
        return Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.focused_border)),
            Span::raw(state.sessions.search.clone()),
            Span::styled("█", Style::default().fg(theme.focused_border)),
        ]);
    }
    if state.sessions.search.is_empty() {
        Line::styled(
            "Press / to search sessions",
            Style::default().fg(theme.muted),
        )
    } else {
        Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.muted)),
            Span::raw(state.sessions.search.clone()),
        ])
    }
}

fn sessions_item_line(
    index: usize,
    item: &SessionListItem,
    active: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = arrow_marker(active);
    let current = if item.current { "●" } else { " " };
    let prefix = format!("{marker} {current} {}. ", index.saturating_add(1));
    let description = if item.preview.trim().is_empty() {
        item.cwd.clone()
    } else {
        item.preview.clone()
    };
    let separator = " - ";
    let reserved = prefix.width().saturating_add(separator.width());
    let available = width.saturating_sub(reserved).max(1);
    let title_width = if available < 8 {
        available
    } else {
        (available / 3).clamp(8, available)
    };
    let title = truncate_with_ellipsis(&item.name, title_width);
    let used = reserved.saturating_add(title.width());
    let description_width = width.saturating_sub(used).max(1);
    let description = truncate_with_ellipsis(&description, description_width);
    let row_style = item_style(active, item.current, theme);
    let title_style = if active {
        row_style.add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.focused_border)
            .add_modifier(Modifier::BOLD)
    };
    Line::from(vec![
        Span::styled(prefix, row_style),
        Span::styled(title, title_style),
        Span::styled(separator, Style::default().fg(theme.muted)),
        Span::styled(description, Style::default().fg(theme.muted)),
    ])
}

fn render_prompts_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 72);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .title(Span::styled(" Prompts ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);
    let inner = overlay.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);
    let content_height = list_content_height(sections[0]);
    let content_width = sections[0].width.saturating_sub(2) as usize;
    let lines = prompts_lines(state, content_width, content_height, theme);
    let filtered_indices = state.filtered_prompt_indices();
    let title = resource_title(
        "Prompt Templates",
        state.prompts.loading,
        state.prompt_resources.len(),
        filtered_indices.len(),
        state.prompt_cursor_filtered_position(),
        state.prompts.search.trim().is_empty(),
    );
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        sections[0],
    );
    let controls = if state.prompts.search_active {
        "type to fuzzy search • ↑/↓ move • Enter expand • Tab complete • Esc clear search"
    } else {
        "↑/↓ select • / search • Enter expand • Tab complete • r reload • Esc close"
    };
    let status = format!("{} • {controls}", state.status);
    frame.render_widget(
        Paragraph::new(truncate_to_width(&status, sections[1].width as usize)).style(theme.footer),
        sections[1],
    );
}

fn render_skills_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 72);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .title(Span::styled(" Skills ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);
    let inner = overlay.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);
    let content_height = list_content_height(sections[0]);
    let content_width = sections[0].width.saturating_sub(2) as usize;
    let lines = skills_lines(state, content_width, content_height, theme);
    let filtered_indices = state.filtered_skill_indices();
    let title = resource_title(
        "Skills",
        state.skills.loading,
        state.skill_resources.len(),
        filtered_indices.len(),
        state.skill_cursor_filtered_position(),
        state.skills.search.trim().is_empty(),
    );
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        sections[0],
    );
    let controls = if state.skills.search_active {
        "type to fuzzy search • ↑/↓ move • Enter run • Tab complete • Esc clear search"
    } else {
        "↑/↓ select • / search • Enter run • Tab complete • r reload • Esc close"
    };
    let status = format!("{} • {controls}", state.status);
    frame.render_widget(
        Paragraph::new(truncate_to_width(&status, sections[1].width as usize)).style(theme.footer),
        sections[1],
    );
}

fn resource_title(
    label: &str,
    loading: bool,
    total: usize,
    filtered: usize,
    filtered_position: usize,
    search_empty: bool,
) -> String {
    if loading || total == 0 {
        format!(" {label} ")
    } else if filtered == 0 {
        format!(" {label} 0/{total} ")
    } else if search_empty {
        format!(
            " {label} {}/{} ",
            filtered_position.saturating_add(1).min(total),
            total
        )
    } else {
        format!(
            " {label} {}/{} ({} total) ",
            filtered_position.saturating_add(1).min(filtered),
            filtered,
            total
        )
    }
}

fn prompts_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        resource_search_line(
            state.prompts.search_active,
            &state.prompts.search,
            "Press / to search prompts",
            theme,
        ),
        Line::from(""),
    ];
    let remaining_height = content_height.saturating_sub(lines.len()).max(1);
    if state.prompts.loading {
        lines.push(Line::styled(
            "Reloading resources…",
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if state.prompt_resources.is_empty() {
        lines.push(Line::styled(
            "No prompt templates found.",
            Style::default().fg(theme.muted),
        ));
        lines.push(Line::styled(
            truncate_with_ellipsis(
                "Add Markdown files under <project>/.oino/prompts/.",
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered_indices = state.filtered_prompt_indices();
    if filtered_indices.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("No prompts match `{}`", state.prompts.search),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered_position = state.prompt_cursor_filtered_position();
    let range = visible_range(filtered_position, filtered_indices.len(), remaining_height);
    lines.extend(
        filtered_indices[range.clone()]
            .iter()
            .enumerate()
            .filter_map(|(offset, item_index)| {
                let item = state.prompt_resources.get(*item_index)?;
                let display_index = range.start + offset;
                let active = *item_index == state.prompts.cursor;
                Some(resource_item_line(
                    display_index,
                    ResourceLineItem {
                        command: item.display_name(),
                        description: &item.description,
                        scope: &item.scope,
                        source: &item.source,
                    },
                    active,
                    content_width,
                    theme,
                ))
            }),
    );
    lines
}

fn skills_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        resource_search_line(
            state.skills.search_active,
            &state.skills.search,
            "Press / to search skills",
            theme,
        ),
        Line::from(""),
    ];
    let remaining_height = content_height.saturating_sub(lines.len()).max(1);
    if state.skills.loading {
        lines.push(Line::styled(
            "Reloading resources…",
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if state.skill_resources.is_empty() {
        lines.push(Line::styled(
            "No skills found.",
            Style::default().fg(theme.muted),
        ));
        lines.push(Line::styled(
            truncate_with_ellipsis(
                "Add skills under ~/.oino/skills/ or <project>/.oino/skills/.",
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered_indices = state.filtered_skill_indices();
    if filtered_indices.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("No skills match `{}`", state.skills.search),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered_position = state.skill_cursor_filtered_position();
    let range = visible_range(filtered_position, filtered_indices.len(), remaining_height);
    lines.extend(
        filtered_indices[range.clone()]
            .iter()
            .enumerate()
            .filter_map(|(offset, item_index)| {
                let item = state.skill_resources.get(*item_index)?;
                let display_index = range.start + offset;
                let active = *item_index == state.skills.cursor;
                Some(resource_item_line(
                    display_index,
                    ResourceLineItem {
                        command: item.command(),
                        description: &item.description,
                        scope: &item.scope,
                        source: &item.source,
                    },
                    active,
                    content_width,
                    theme,
                ))
            }),
    );
    lines
}

fn resource_search_line(
    active: bool,
    search: &str,
    empty_hint: &str,
    theme: &Theme,
) -> Line<'static> {
    if active {
        return Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.focused_border)),
            Span::raw(search.to_string()),
            Span::styled("█", Style::default().fg(theme.focused_border)),
        ]);
    }
    if search.is_empty() {
        Line::styled(empty_hint.to_string(), Style::default().fg(theme.muted))
    } else {
        Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.muted)),
            Span::raw(search.to_string()),
        ])
    }
}

struct ResourceLineItem<'a> {
    command: String,
    description: &'a str,
    scope: &'a str,
    source: &'a str,
}

fn resource_item_line(
    index: usize,
    item: ResourceLineItem<'_>,
    active: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = arrow_marker(active);
    let prefix = format!(
        "{marker} {}. {} [{}] — ",
        index.saturating_add(1),
        item.command,
        item.scope
    );
    let detail = format!("{} • {}", item.description, item.source);
    let text = truncate_with_ellipsis(&format!("{prefix}{detail}"), width.max(1));
    Line::styled(text, item_style(active, false, theme))
}

fn render_settings_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let overlay = centered_rect(area, 82, 72);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Settings ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);

    match settings.page {
        SettingsPage::Menu => render_settings_menu(frame, sections[0], settings, theme),
        SettingsPage::Models => render_model_settings(frame, sections[0], settings, theme),
        SettingsPage::Thinking => render_thinking_settings(frame, sections[0], settings, theme),
        SettingsPage::Collapse => render_collapse_settings(frame, sections[0], settings, theme),
        SettingsPage::ChatStyle => render_chat_style_settings(frame, sections[0], settings, theme),
        SettingsPage::Tools => render_tools_settings(frame, sections[0], settings, theme),
        SettingsPage::Keymaps => render_keymap_settings(frame, sections[0], settings, theme),
    }
    render_settings_footer(frame, sections[1], settings, theme);
}

fn render_settings_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let items = settings.menu_items();
    let mut lines = vec![Line::styled(
        "Choose a settings page:",
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(items.iter().enumerate().map(|(index, item)| {
        let active = index == settings.menu_cursor;
        let marker = arrow_marker(active);
        let detail = match item.page() {
            SettingsPage::Models => format!("current: {}", settings.selected_model_label()),
            SettingsPage::Thinking => format!(
                "current: {}",
                thinking_label(settings.selected_thinking_level)
            ),
            SettingsPage::Collapse => format!(
                "thinking: {}, tool: {}",
                collapse_mode_label(settings.thinking_collapse_mode),
                collapse_mode_label(settings.tool_collapse_mode)
            ),
            SettingsPage::ChatStyle => {
                format!("current: {}", chat_style_label(settings.chat_style))
            }
            SettingsPage::Tools => format!("{} registered", settings.tools.len()),
            SettingsPage::Keymaps => format!("preset: {}", settings.keymap.preset.label()),
            SettingsPage::Menu => String::new(),
        };
        let text = format!("{marker} {}  {}", item.label(), detail);
        Line::styled(text, item_style(active, false, theme))
    }));

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Settings Pages ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_model_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let filtered_indices = settings.filtered_model_indices();
    let filtered_position = settings.model_cursor_filtered_position();
    let title = if settings.models.is_empty() {
        " Model Selection ".to_string()
    } else if settings.refreshing {
        format!(
            " Model Selection {}/{} ({} total, refreshing) ",
            filtered_position
                .saturating_add(1)
                .min(filtered_indices.len()),
            filtered_indices.len(),
            settings.models.len()
        )
    } else {
        format!(
            " Model Selection {}/{} ({} total) ",
            filtered_position
                .saturating_add(1)
                .min(filtered_indices.len()),
            filtered_indices.len(),
            settings.models.len()
        )
    };
    let mut lines = vec![model_search_line(settings, theme), Line::from("")];
    if settings.models.is_empty() {
        lines.push(Line::styled(
            "Loading model catalog…",
            Style::default().fg(theme.muted),
        ));
    } else if filtered_indices.is_empty() {
        lines.push(Line::styled(
            format!("No models match `{}`", settings.model_search),
            Style::default().fg(theme.muted),
        ));
    } else {
        let visible_height = list_content_height(area).saturating_sub(2).max(1);
        let range = visible_range(filtered_position, filtered_indices.len(), visible_height);
        lines.extend(
            filtered_indices
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end.saturating_sub(range.start))
                .filter_map(|(_, model_index)| {
                    let model = settings.models.get(*model_index)?;
                    let active = *model_index == settings.model_cursor;
                    let selected = model.id == settings.selected_model;
                    let marker = selection_marker(active, selected);
                    let style = item_style(active, selected, theme);
                    Some(Line::styled(
                        format!("{marker} {}", model.display_name),
                        style,
                    ))
                }),
        );
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        area,
    );
}

fn model_search_line(settings: &SettingsState, theme: &Theme) -> Line<'static> {
    if settings.model_search_active {
        return Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.focused_border)),
            Span::raw(settings.model_search.clone()),
            Span::styled("█", Style::default().fg(theme.focused_border)),
        ]);
    }
    if settings.model_search.is_empty() {
        Line::styled("Press / to search models", Style::default().fg(theme.muted))
    } else {
        Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.muted)),
            Span::raw(settings.model_search.clone()),
        ])
    }
}

fn render_thinking_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let levels = settings.thinking_levels();
    let mut lines = vec![Line::styled(
        format!("Model: {}", settings.selected_model_label()),
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(levels.iter().enumerate().map(|(index, level)| {
        let active = index == settings.thinking_cursor;
        let selected = *level == settings.selected_thinking_level;
        let marker = selection_marker(active, selected);
        let style = item_style(active, selected, theme);
        Line::styled(format!("{marker} {}", thinking_label(*level)), style)
    }));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Thinking Level ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_collapse_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let items = [
        ("Thinking", settings.thinking_collapse_mode),
        ("Tool", settings.tool_collapse_mode),
    ];
    let mut lines = vec![Line::styled(
        "Enter cycles: Full → Truncate → Collapse",
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(items.iter().enumerate().map(|(index, (label, mode))| {
        let active = index == settings.collapse_cursor;
        let marker = arrow_marker(active);
        Line::styled(
            format!("{marker} {label}: {}", collapse_mode_label(*mode)),
            item_style(active, false, theme),
        )
    }));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Collapse Mode ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_chat_style_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let descriptions = [
        (ChatStyle::Chat, "current rounded chat bubbles"),
        (ChatStyle::Agentic, "Codex-like activity transcript"),
        (ChatStyle::Minimal, "jcode-like compact transcript"),
    ];
    let mut lines = vec![Line::styled(
        "Changing style re-renders the current transcript immediately.",
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(
        descriptions
            .iter()
            .enumerate()
            .map(|(index, (style, description))| {
                let active = index == settings.chat_style_cursor;
                let selected = *style == settings.chat_style;
                let marker = selection_marker(active, selected);
                Line::styled(
                    format!(
                        "{marker} {} ({}) — {description}",
                        chat_style_label(*style),
                        chat_style_value(*style)
                    ),
                    item_style(active, selected, theme),
                )
            }),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Chat Style ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_tools_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let title = if settings.tools.is_empty() {
        " Tools ".to_string()
    } else {
        format!(
            " Tools {}/{} ",
            settings
                .tool_cursor
                .saturating_add(1)
                .min(settings.tools.len()),
            settings.tools.len()
        )
    };
    let mut lines = vec![Line::styled(
        "Project controls this workspace. Global is the default copied into new projects.",
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    if settings.tools.is_empty() {
        lines.push(Line::styled(
            "No tools registered.",
            Style::default().fg(theme.muted),
        ));
    } else {
        let visible_height = list_content_height(area).saturating_sub(2).max(1);
        let range = visible_range(settings.tool_cursor, settings.tools.len(), visible_height);
        lines.extend(
            settings
                .tools
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end.saturating_sub(range.start))
                .map(|(index, tool)| {
                    let active = index == settings.tool_cursor;
                    let marker = arrow_marker(active);
                    Line::styled(
                        format!("{marker} {}", tool.label()),
                        item_style(active, tool.global_enabled || tool.project_enabled, theme),
                    )
                }),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_keymap_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    match &settings.keymaps_mode {
        KeymapsMode::List => render_keymap_list(frame, area, settings, theme),
        KeymapsMode::Detail => render_keymap_detail(frame, area, settings, theme),
        KeymapsMode::ShortcutType { .. } => {
            render_keymap_shortcut_type(frame, area, settings, theme)
        }
        KeymapsMode::Capture { kind, strokes, .. } => {
            render_keymap_capture(frame, area, settings, *kind, strokes, theme);
        }
        KeymapsMode::ChordKeyCapture => render_chord_key_capture(frame, area, settings, theme),
        KeymapsMode::PresetSelect => render_keymap_preset_select(frame, area, settings, theme),
        KeymapsMode::PresetConfirm { preset } => {
            render_keymap_preset_confirm(frame, area, *preset, theme);
        }
    }
}

fn render_keymap_list(frame: &mut Frame<'_>, area: Rect, settings: &SettingsState, theme: &Theme) {
    let rows = key_action_rows();
    let visible_height = list_content_height(area).saturating_sub(2).max(1);
    let range = visible_range(settings.keymap_cursor, rows.len(), visible_height);
    let mut lines = vec![Line::styled(
        format!(
            "Preset: {} • Chord key: {} • Enter action • g edit chord key • p preset",
            settings.keymap.preset.label(),
            settings.keymap.chord_key
        ),
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(
        rows.iter()
            .enumerate()
            .skip(range.start)
            .take(range.end.saturating_sub(range.start))
            .map(|(index, info)| {
                let active = index == settings.keymap_cursor;
                let marker = arrow_marker(active);
                let shortcut = settings.keymap.label_for(info.action);
                let text = format!(
                    "{marker} [{}] {}  —  {}",
                    info.context.label(),
                    info.label,
                    shortcut
                );
                Line::styled(text, item_style(active, false, theme))
            }),
    );
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(format!(
                    " Keymaps {}/{} ",
                    settings.keymap_cursor.saturating_add(1).min(rows.len()),
                    rows.len()
                ))
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        area,
    );
}

fn render_keymap_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let action = settings.current_keymap_action();
    let info = action.info();
    let bindings = settings.current_keymap_bindings();
    let mut lines = vec![
        Line::styled(
            format!("{} ({})", info.label, action.id()),
            Style::default().fg(theme.focused_border),
        ),
        Line::styled(info.description, Style::default().fg(theme.muted)),
        Line::from(""),
    ];
    if bindings.is_empty() {
        lines.push(Line::styled("› Unassigned", theme.warning));
    } else {
        lines.extend(bindings.iter().enumerate().map(|(index, binding)| {
            let active = index == settings.keymap_binding_cursor;
            let marker = selection_marker(active, false);
            Line::styled(
                format!("{marker} {binding}"),
                item_style(active, false, theme),
            )
        }));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Keymap Action ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_keymap_shortcut_type(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let action = settings.current_keymap_action();
    let mut lines = vec![
        Line::styled(
            format!("Choose shortcut type for {}", action.info().label),
            Style::default().fg(theme.muted),
        ),
        Line::from(""),
    ];
    lines.extend(ShortcutKind::all().iter().enumerate().map(|(index, kind)| {
        let active = index == settings.keymap_shortcut_kind_cursor;
        let description = match kind {
            ShortcutKind::Chord => "global chord key plus one suffix key",
            ShortcutKind::Combination => "one key event, e.g. F2 or Ctrl-S",
        };
        Line::styled(
            format!(
                "{} {} — {}",
                arrow_marker(active),
                kind.label(),
                description
            ),
            item_style(active, false, theme),
        )
    }));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Shortcut Type ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_keymap_capture(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    kind: ShortcutKind,
    strokes: &[crate::keymap::KeyStroke],
    theme: &Theme,
) {
    let action = settings.current_keymap_action();
    let captured = if strokes.is_empty() {
        "(none yet)".to_string()
    } else {
        strokes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let prompt = match kind {
        ShortcutKind::Combination => "Press the key combination to assign. Esc cancels.",
        ShortcutKind::Chord if strokes.is_empty() => {
            "Press the suffix key. The global chord key is prepended. Esc cancels."
        }
        ShortcutKind::Chord => {
            "Press the suffix key. The global chord key is prepended. Esc cancels."
        }
    };
    let lines = vec![
        Line::styled(
            format!("Assigning {}", action.info().label),
            Style::default().fg(theme.focused_border),
        ),
        Line::styled(
            format!("Type: {}", kind.label()),
            Style::default().fg(theme.muted),
        ),
        Line::styled(
            format!("Captured: {captured}"),
            Style::default().fg(theme.fg),
        ),
        Line::from(""),
        Line::styled(prompt, theme.warning),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Listening for Shortcut ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_chord_key_capture(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let lines = vec![
        Line::styled(
            "Set the global chord key",
            Style::default().fg(theme.focused_border),
        ),
        Line::styled(
            format!("Current: {}", settings.keymap.chord_key),
            Style::default().fg(theme.muted),
        ),
        Line::from(""),
        Line::styled(
            "Press one key event such as Ctrl-X, Alt-Space, or F12.",
            Style::default().fg(theme.fg),
        ),
        Line::styled(
            "Plain text keys are disallowed so normal typing still works. Esc cancels.",
            theme.warning,
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Global Chord Key ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_keymap_preset_select(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let mut lines = vec![
        Line::styled(
            "Select a preset. Applying it resets every keybind after confirmation.",
            theme.warning,
        ),
        Line::from(""),
    ];
    lines.extend(
        KeymapPreset::all()
            .iter()
            .enumerate()
            .map(|(index, preset)| {
                let active = index == settings.keymap_preset_cursor;
                let selected = *preset == settings.keymap.preset;
                Line::styled(
                    format!("{} {}", selection_marker(active, selected), preset.label()),
                    item_style(active, selected, theme),
                )
            }),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Keymap Preset ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_keymap_preset_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    preset: KeymapPreset,
    theme: &Theme,
) {
    let lines = vec![
        Line::styled(
            format!("Reset every keybind to the {} preset?", preset.label()),
            theme.warning.add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::styled(
            "Y confirms • N/Esc cancels",
            Style::default().fg(theme.muted),
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Confirm Preset Reset ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Center),
        area,
    );
}

fn render_settings_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let controls = match settings.page {
        SettingsPage::Menu => "arrows/jk move • Enter/→ open • Esc close",
        SettingsPage::Models if settings.model_search_active => {
            "type to search • arrows move matches • Enter keep search • Esc clear search"
        }
        SettingsPage::Models => "arrows/jk move • / search • Enter apply • Esc/← back",
        SettingsPage::Thinking => "arrows/jk move • Enter apply • Esc/← back • Ctrl-C twice quit",
        SettingsPage::Collapse => "arrows/jk move • Enter/→ cycle • Esc/← back",
        SettingsPage::ChatStyle => "arrows/jk move • Enter apply • Esc/← back",
        SettingsPage::Tools => {
            "arrows/jk move • g toggle global • p/Space/Enter toggle project • Esc/← back"
        }
        SettingsPage::Keymaps => match settings.keymaps_mode {
            KeymapsMode::List => {
                "arrows/jk move • Enter detail • g chord key • p preset • Esc/← back"
            }
            KeymapsMode::Detail => {
                "arrows/jk move • Enter edit • a add • x remove • c clear • r reset • Esc back"
            }
            KeymapsMode::ShortcutType { .. } => "arrows/jk choose type • Enter listen • Esc back",
            KeymapsMode::Capture { .. } => "press shortcut input • Esc cancel",
            KeymapsMode::ChordKeyCapture => "press global chord key • Esc cancel",
            KeymapsMode::PresetSelect => "arrows/jk choose preset • Enter confirm • Esc back",
            KeymapsMode::PresetConfirm { .. } => "Y reset all keybinds • N/Esc cancel",
        },
    };
    let status = if settings.page == SettingsPage::Tools {
        format!("Project controls this workspace; Global seeds new projects • {controls}")
    } else {
        format!("{} • {controls}", settings.status)
    };
    frame.render_widget(
        Paragraph::new(truncate_to_width(&status, area.width as usize)).style(theme.footer),
        area,
    );
}

fn list_content_height(area: Rect) -> usize {
    area.height.saturating_sub(2).max(1) as usize
}

fn visible_range(cursor: usize, len: usize, capacity: usize) -> std::ops::Range<usize> {
    if len == 0 {
        return 0..0;
    }
    let capacity = capacity.max(1).min(len);
    let cursor = cursor.min(len.saturating_sub(1));
    let half = capacity / 2;
    let mut start = cursor.saturating_sub(half);
    if start + capacity > len {
        start = len.saturating_sub(capacity);
    }
    start..start + capacity
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = area.width.saturating_mul(percent_x).saturating_div(100);
    let height = area.height.saturating_mul(percent_y).saturating_div(100);
    let width = width.clamp(24, area.width);
    let height = height.clamp(8, area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn section_border_style(active: bool, theme: &Theme) -> Style {
    let color = if active {
        theme.focused_border
    } else {
        theme.panel_border
    };
    Style::default().fg(color)
}

fn arrow_marker(active: bool) -> &'static str {
    if active {
        "›"
    } else {
        " "
    }
}

fn selection_marker(active: bool, selected: bool) -> &'static str {
    match (active, selected) {
        (true, true) => "● ›",
        (true, false) => "  ›",
        (false, true) => "●  ",
        (false, false) => "   ",
    }
}

fn item_style(active: bool, selected: bool, theme: &Theme) -> Style {
    let style = if selected {
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg)
    };
    if active {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    }
}

fn composer_lines(area: Rect, composer: &ComposerState, theme: &Theme) -> Vec<Line<'static>> {
    if composer.text().is_empty() {
        return vec![Line::from(vec![
            Span::raw(INPUT_PROMPT),
            Span::styled(INPUT_PLACEHOLDER, theme.placeholder),
        ])];
    }

    let content_width = composer_content_width(area);
    let content_height = composer_content_height(area);
    let (visible_lines, _, _) = layout_textarea(
        composer.text(),
        composer.cursor(),
        content_width,
        content_height,
    );

    visible_lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix = if index == 0 {
                INPUT_PROMPT.to_string()
            } else {
                " ".repeat(INPUT_PROMPT.chars().count())
            };
            Line::from(vec![Span::raw(prefix), Span::raw(line)])
        })
        .collect()
}

fn composer_cursor_position(area: Rect, composer: &ComposerState) -> Position {
    let content_width = composer_content_width(area);
    let content_height = composer_content_height(area);
    let (_, cursor_row, cursor_col) = layout_textarea(
        composer.text(),
        composer.cursor(),
        content_width,
        content_height,
    );
    Position::new(
        area.x
            .saturating_add(1)
            .saturating_add(INPUT_PROMPT.chars().count() as u16)
            .saturating_add(u16::try_from(cursor_col).unwrap_or(u16::MAX)),
        area.y
            .saturating_add(1)
            .saturating_add(u16::try_from(cursor_row).unwrap_or(u16::MAX)),
    )
}

fn composer_height(input: &str, width: u16, total_height: u16) -> u16 {
    let available_height = total_height.saturating_sub(MIN_TRANSCRIPT_HEIGHT).max(3);
    let content_width = composer_content_width_for_width(width);
    let line_count = wrapped_line_count(input, content_width).max(MIN_COMPOSER_ROWS);
    let desired = line_count.saturating_add(2);
    let cap = available_height.clamp(3, MAX_COMPOSER_HEIGHT) as usize;
    desired.clamp(3, cap) as u16
}

fn composer_content_width(area: Rect) -> usize {
    composer_content_width_for_width(area.width)
}

fn composer_content_width_for_width(width: u16) -> usize {
    let inner_width = width.saturating_sub(2) as usize;
    inner_width
        .saturating_sub(INPUT_PROMPT.chars().count())
        .max(1)
}

fn composer_content_height(area: Rect) -> usize {
    area.height.saturating_sub(2).max(1) as usize
}

fn layout_textarea(
    input: &str,
    cursor: usize,
    width: usize,
    max_height: usize,
) -> (Vec<String>, usize, usize) {
    let mut lines = wrap_text(input, width.max(1));
    if lines.is_empty() {
        lines.push(String::new());
    }
    let (cursor_row, cursor_col) = cursor_row_col(input, cursor, width.max(1));
    let max_height = max_height.max(1);
    let mut start = 0usize;
    if cursor_row >= max_height {
        start = cursor_row + 1 - max_height;
    }
    if start + max_height > lines.len() {
        start = lines.len().saturating_sub(max_height);
    }
    let visible = lines
        .into_iter()
        .skip(start)
        .take(max_height)
        .collect::<Vec<_>>();
    let visible_cursor_row = cursor_row.saturating_sub(start);
    (
        visible,
        visible_cursor_row,
        cursor_col.min(width.saturating_sub(1)),
    )
}

fn cursor_row_col(input: &str, cursor: usize, width: usize) -> (usize, usize) {
    let mut row = 0usize;
    let mut col = 0usize;
    let mut char_idx = 0usize;

    for grapheme in input.graphemes(true) {
        if char_idx >= cursor {
            break;
        }
        let grapheme_chars = grapheme.chars().count();
        let next_char_idx = char_idx.saturating_add(grapheme_chars);
        let cursor_inside = cursor < next_char_idx;

        if grapheme == "\n" {
            row += 1;
            col = 0;
            char_idx = next_char_idx;
            if cursor_inside {
                break;
            }
            continue;
        }

        let grapheme_width = grapheme.width();
        if col + grapheme_width > width && col != 0 {
            row += 1;
            col = 0;
        }
        col += grapheme_width;
        if col >= width {
            row += 1;
            col = 0;
        }
        if cursor_inside {
            break;
        }
        char_idx = next_char_idx;
    }

    (row, col)
}

#[allow(dead_code)]
fn cursor_byte(input: &str, cursor: usize) -> usize {
    byte_index_at_char(input, cursor.min(char_count(input)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{OverlayKind, SessionListItem, TuiState},
        message::{MessageView, ToolCallView},
        settings::CollapseMode,
        TuiAction,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::json;

    fn draw_state(width: u16, height: u16, state: &TuiState) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(err) => panic!("terminal init failed: {err}"),
        };
        if let Err(err) = terminal.draw(|frame| render(frame, state)) {
            panic!("draw failed: {err}");
        }
        terminal.backend().buffer().clone()
    }

    fn line_texts(lines: Vec<Line<'static>>) -> Vec<String> {
        lines.iter().map(plain_line).collect()
    }

    #[test]
    fn prepared_transcript_materializes_requested_slice() {
        let prepared = PreparedTranscript::from_blocks(vec![
            std::sync::Arc::new(vec![Line::from("a0"), Line::from("a1")]),
            std::sync::Arc::new(Vec::new()),
            std::sync::Arc::new(vec![Line::from("b0"), Line::from("b1"), Line::from("b2")]),
            std::sync::Arc::new(vec![Line::from("c0")]),
        ]);

        assert_eq!(prepared.total_lines(), 6);
        assert_eq!(
            line_texts(prepared.materialize_line_slice(1, 5)),
            vec!["a1", "b0", "b1", "b2"]
        );
        assert_eq!(
            line_texts(prepared.materialize_line_slice(2, 3)),
            vec!["b0"]
        );
        assert!(prepared.materialize_line_slice(6, 9).is_empty());
    }

    #[test]
    fn tiny_terminal_uses_fallback_message() {
        let state = TuiState::new();
        let buffer = draw_state(18, 6, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Oino needs"));
    }

    #[test]
    fn render_includes_chat_and_composer_placeholder() {
        let mut state = TuiState::new();
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "hello".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        let buffer = draw_state(80, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("test/model"));
        assert!(text.contains("hello"));
        assert!(text.contains(INPUT_PLACEHOLDER));
        assert!(!text.contains(crate::app::HELP_STATUS));
    }

    #[test]
    fn transcript_container_has_left_padding() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Minimal;
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "user".into(),
            title: None,
            content: "hello".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });

        let width = 40usize;
        let buffer = draw_state(width as u16, 12, &state);
        let symbol = |x: usize, y: usize| buffer.content()[y * width + x].symbol();

        assert_eq!(symbol(1, 1), " ");
        assert_eq!(symbol(2, 1), "1");
    }

    #[test]
    fn render_working_state_shows_generating_indicator() {
        let mut state = TuiState::new();
        state.set_working(true);
        let buffer = draw_state(80, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Calling"));
        assert!(text.contains("steer"));
    }

    #[test]
    fn render_transcript_respects_scroll_offset() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Minimal;
        for index in 0..20 {
            state.messages.push(MessageView {
                id: oino_types::OinoId::from_u128(index),
                role: "assistant".into(),
                title: Some("test/model".into()),
                content: format!("message {index:02}"),
                thinking: None,
                thinking_redacted: false,
                tool_call_id: None,
                tool_calls: Vec::new(),
                is_error: false,
            });
        }

        let tail = draw_state(80, 18, &state)
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(tail.contains("message 19"));
        assert!(!tail.contains("message 00"));
        assert!(
            tail.contains("┃"),
            "long transcript should show scrollbar thumb"
        );

        state.scroll_transcript_to_top();
        let top = draw_state(80, 18, &state)
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(top.contains("message 00"));
        assert!(!top.contains("message 19"));
        assert!(top.contains("Oino ↑"));
        assert!(
            top.contains("┃"),
            "scrolled transcript should keep scrollbar visible"
        );
    }

    #[test]
    fn transcript_url_overlays_target_visible_urls() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Minimal;
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "See [Oino](https://example.invalid/docs).".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });

        let overlays = transcript_url_overlays(&state, 80, 20);

        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].text, "Oino ↗ https://example.invalid/docs");
        assert_eq!(overlays[0].url, "https://example.invalid/docs");
        assert!(overlays[0].x > 0);
        assert!(overlays[0].y > 0);
    }

    #[test]
    fn transcript_click_targets_include_image_placeholders() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Minimal;
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "![diagram](assets/diagram.png)".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });

        let targets = transcript_click_targets(&state, 80, 20);

        assert!(targets.iter().any(|target| {
            target.kind == TerminalClickTargetKind::Image && target.target == "assets/diagram.png"
        }));
    }

    #[test]
    fn render_collapse_modes_hide_thinking_and_tool_content() {
        let mut state = TuiState::new();
        state.settings.thinking_collapse_mode = CollapseMode::Collapse;
        state.settings.tool_collapse_mode = CollapseMode::Collapse;
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "final answer".into(),
            thinking: Some("secret internal reasoning".into()),
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "tool:bash".into(),
            title: None,
            content: "long tool output".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        let buffer = draw_state(80, 24, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("[collapsed]"));
        assert!(text.contains("✓ Bash"));
        assert!(!text.contains("secret internal reasoning"));
        assert!(!text.contains("long tool output"));
        assert!(text.contains("final answer"));
    }

    #[test]
    fn render_skips_assistant_tool_call_only_bubble() {
        let mut state = TuiState::new();
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("openrouter:test/model".into()),
            content: "<empty>".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "tool:write".into(),
            title: None,
            content: "Successfully wrote file".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        let buffer = draw_state(100, 24, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("<tool-call:write>"));
        assert!(!text.contains("openrouter:test/model"));
        assert!(text.contains("tool:write"));
        assert!(text.contains("Successfully wrote file"));
    }

    #[test]
    fn agentic_style_renders_unresolved_tool_call_as_running_activity() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Agentic;
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "<empty>".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: vec![ToolCallView {
                id: oino_types::OinoId::nil(),
                name: "bash".into(),
                arguments: json!({ "command": "cargo test" }),
            }],
            is_error: false,
        });
        let buffer = draw_state(90, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Running Bash cargo test"));
        assert!(!text.contains("test/model"));
    }

    #[test]
    fn minimal_style_renders_numbered_prompt_and_compact_tool_row() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Minimal;
        let call_id = oino_types::OinoId::nil();
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "user".into(),
            title: None,
            content: "run tests".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "<empty>".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: vec![ToolCallView {
                id: call_id,
                name: "bash".into(),
                arguments: json!({ "command": "cargo test" }),
            }],
            is_error: false,
        });
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "tool:bash".into(),
            title: None,
            content: "ok".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: Some(call_id),
            tool_calls: Vec::new(),
            is_error: false,
        });
        let buffer = draw_state(90, 24, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("1› run tests"));
        assert!(text.contains("✓ Bash cargo test"));
        assert!(!text.contains("running"));
    }

    #[test]
    fn render_thinking_as_section_not_inline_plain_text() {
        let mut state = TuiState::new();
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "final answer".into(),
            thinking: Some("internal reasoning".into()),
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        let buffer = draw_state(80, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("◌"));
        assert!(!text.contains("◌ thinking"));
        assert!(text.contains("internal reasoning"));
        assert!(text.contains("final answer"));
        assert!(!text.contains("<thinking:"));
    }

    #[test]
    fn composer_height_grows_but_keeps_transcript_space() {
        assert_eq!(composer_height("", 80, 24), 5);
        assert!(composer_height("a\nb\nc\nd\ne\nf", 80, 24) > 5);
        assert!(composer_height("a\n".repeat(20).as_str(), 80, 10) <= 7);
    }

    #[test]
    fn render_chord_mode_full_screen_hint() {
        let mut state = TuiState::new();
        state.chord = ChordState::CtrlO;
        let buffer = draw_state(80, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Ctrl-O:"));
        assert!(text.contains("s settings"));
        assert!(text.contains("q send panel"));
        assert!(text.contains("Esc cancel"));
    }

    #[test]
    fn command_skill_label_uses_distinct_color() {
        let theme = Theme::default();
        let skill = command_category_style(CommandSuggestionCategory::Skill, &theme);

        assert_eq!(
            skill,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        );
        assert_ne!(
            skill,
            command_category_style(CommandSuggestionCategory::System, &theme)
        );
        assert_ne!(
            skill,
            command_category_style(CommandSuggestionCategory::Prompt, &theme)
        );
    }

    #[test]
    fn render_command_suggestions_above_composer() {
        let mut state = TuiState::new();
        state.composer.replace_text("/");
        state.refresh_command_suggestions();
        let buffer = draw_state(80, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Commands"));
        assert!(text.contains("/settings"));
        assert!(text.contains("Open or change settings"));
    }

    #[test]
    fn render_model_command_suggestions_scroll_to_selected_item() {
        let mut state = TuiState::new();
        state.composer.replace_text("/settings model ");
        state.set_model_catalog(
            (0..60)
                .map(|index| crate::settings::ModelOption::new(format!("openrouter:model-{index}")))
                .collect::<Vec<_>>(),
            "loaded",
        );
        state.command_suggestions.selected = 39;
        let buffer = draw_state(100, 30, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Models 40/60"));
        assert!(text.contains("› openrouter:model-39"));
        assert!(!text.contains("openrouter:model-0"));
    }

    #[test]
    fn render_help_overlay_shows_command_and_file_attach_guidance() {
        let mut state = TuiState::new();
        state.open_help();
        let buffer = draw_state(90, 28, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Oino Help"));
        assert!(text.contains("/help"));
        assert!(text.contains("@"));
        assert!(text.contains("file paths"));
        assert!(text.contains("Press / to search help"));
        assert!(text.contains("/ search"));
        assert!(text.contains("Esc/q close"));
    }

    #[test]
    fn render_help_overlay_filters_search_results() {
        let mut state = TuiState::new();
        state.open_help();
        assert_eq!(
            state.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('/'),
                crossterm::event::KeyModifiers::NONE,
            )),
            TuiAction::None
        );
        for ch in "queue".chars() {
            assert_eq!(
                state.handle_key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char(ch),
                    crossterm::event::KeyModifiers::NONE,
                )),
                TuiAction::None
            );
        }
        let buffer = draw_state(90, 28, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Search: queue"));
        assert!(text.contains("match"));
        assert!(text.contains("Send panel q"));
        assert!(text.contains("type to fuzzy search"));
    }

    #[test]
    fn render_sessions_overlay_ellipsizes_long_rows() {
        let mut state = TuiState::new();
        state.overlay = Some(OverlayKind::Sessions);
        state.set_sessions(vec![SessionListItem {
            id: "12345678-1234-1234-1234-123456789abc".into(),
            name: "very long saved session title that would otherwise run off the edge".into(),
            cwd: "/tmp/a/very/deep/project/path/that/keeps/going".into(),
            message_count: 42,
            preview: "this is a long preview with enough text to exceed the panel width".into(),
            current: false,
        }]);

        let buffer = draw_state(56, 18, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("Press / to search sessions"));
        assert!(text.contains("…"));
    }

    #[test]
    fn render_transcript_title_includes_session_title() {
        let mut state = TuiState::new();
        state.set_session_title("Design Review");
        let buffer = draw_state(80, 20, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Oino • Design Review"));
    }

    #[test]
    fn render_settings_tools_page_lists_scope_statuses() {
        let mut state = TuiState::new();
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.open_tools();
        state.set_tool_settings(vec![
            crate::settings::ToolSettingsItem::global("bash"),
            crate::settings::ToolSettingsItem::global("set_session_title")
                .with_scopes(false, false),
        ]);
        let buffer = draw_state(100, 30, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Tools 1/2"));
        assert!(text.contains("Bash - [Global - ON] [Project - OFF]"));
        assert!(text.contains("Set Session Title - [Global - OFF] [Project - OFF]"));
    }

    #[test]
    fn render_settings_overlay_on_top_of_chat() {
        let mut state = TuiState::with_settings("a", oino_types::ThinkingLevel::Off);
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new("a")],
            "cached models loaded",
        );
        state.overlay = Some(crate::app::OverlayKind::Settings);
        let buffer = draw_state(100, 30, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Settings"));
        assert!(text.contains("Model Selection"));
        assert!(text.contains("Thinking Level"));
        assert!(text.contains("›"));
    }

    #[test]
    fn render_model_selection_as_dedicated_child_page() {
        let mut state = TuiState::with_settings("a", oino_types::ThinkingLevel::Off);
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new("a")],
            "cached models loaded",
        );
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.page = crate::settings::SettingsPage::Models;
        let buffer = draw_state(100, 30, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Model Selection"));
        assert!(text.contains("a"));
        assert!(!text.contains("Settings Pages"));
    }

    #[test]
    fn model_selection_scrolls_to_keep_cursor_visible() {
        let mut state = TuiState::with_settings("model-39", oino_types::ThinkingLevel::Off);
        let models = (0..60)
            .map(|index| crate::settings::ModelOption::new(format!("model-{index}")))
            .collect::<Vec<_>>();
        state.set_model_catalog(models, "cached models loaded");
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.page = crate::settings::SettingsPage::Models;
        state.settings.model_cursor = 39;
        let buffer = draw_state(80, 16, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Model Selection 40/60"));
        assert!(text.contains("model-39"));
        assert!(text.contains("● › model-39"));
        assert!(!text.contains("model-0"));
    }

    #[test]
    fn visible_range_centers_cursor_without_overflow() {
        assert_eq!(visible_range(0, 100, 10), 0..10);
        assert_eq!(visible_range(50, 100, 10), 45..55);
        assert_eq!(visible_range(99, 100, 10), 90..100);
        assert_eq!(visible_range(0, 0, 10), 0..0);
    }
}
