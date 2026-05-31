#![forbid(unsafe_code)]

use crate::{
    app::{
        extension_surface_slot_key, ChordState, ExtensionManagementView, OverlayKind,
        SendPanelItem, SendPanelSection, SessionListItem, TuiFocus, TuiState,
    },
    command::{CommandSuggestionCategory, CommandSuggestionsView},
    composer::{byte_index_at_char, char_count, ComposerState, INPUT_PLACEHOLDER},
    help::{help_entries, HelpEntry},
    keymap::{key_action_rows, KeymapPreset, ShortcutKind},
    message::MessageView,
    settings::{
        chat_style_label, chat_style_value, collapse_mode_label, thinking_label, ChatStyle,
        KeymapsMode, SettingsMenuItem, SettingsPage, SettingsState,
    },
    text::{truncate_to_width, truncate_with_ellipsis, wrap_text, wrapped_line_count},
    theme::{parse_theme_color, theme_cache_hash, Theme},
    transcript::transcript_line_blocks,
};
use oino_extension_core::{
    ui_surface_layout_decision, ActiveContribution, UiSurfaceContribution, UiSurfaceKind,
    UiSurfaceLayoutDecision,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Wrap,
    },
    Frame,
};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
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
const FOOTER_STATUS_TOP_ID: &str = "footer_status_top";
const FOOTER_STATUS_BOTTOM_ID: &str = "footer_status_bottom";
const FOOTER_STATUS_PACKAGE_ID: &str = "oino.footer_status";
const COMPOSER_DIRECT_TOP_SLOT: &str = "composer:direct-top";
const COMPOSER_DIRECT_BOTTOM_SLOT: &str = "composer:direct-bottom";

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
    let theme =
        theme_with_extension_tokens(state, Theme::from_resolved_theme(state.active_theme()));
    render_with_theme(frame, state, &theme);
}

fn theme_with_extension_tokens(state: &TuiState, mut theme: Theme) -> Theme {
    for (token, value) in &state.extension_ui.theme.tokens {
        let Some(color) = parse_theme_color(value) else {
            continue;
        };
        match normalize_theme_token(token).as_str() {
            "accent" => {
                theme.accent = color;
                theme.focused_border = color;
                theme.title = theme.title.fg(color);
            }
            "success" => {
                theme.success = color;
                theme.working = theme.working.fg(color);
            }
            "text" | "fg" => theme.fg = color,
            "muted" => {
                theme.muted = color;
                theme.footer = theme.footer.fg(color);
            }
            "dim" => {
                theme.dim = color;
                theme.placeholder = theme.placeholder.fg(color);
            }
            "focused_border" | "border_accent" => theme.focused_border = color,
            "panel_border" | "border" | "border_muted" => theme.panel_border = color,
            "user_border" | "user_message_text" => theme.user_border = color,
            "assistant_border" | "assistant_message_text" => theme.assistant_border = color,
            "tool_border" | "tool_title" => theme.tool_border = color,
            "title" => theme.title = theme.title.fg(color),
            "warning" => theme.warning = theme.warning.fg(color),
            "error" => theme.error = theme.error.fg(color),
            "footer" | "status" | "inline_status" => theme.footer = theme.footer.fg(color),
            "working" | "working_indicator" => theme.working = theme.working.fg(color),
            _ => {}
        }
    }
    theme
}

fn normalize_theme_token(token: &str) -> String {
    let mut normalized = String::new();
    for (index, ch) in token.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '.' | ' ') {
            normalized.push('_');
        } else {
            normalized.push(ch);
        }
    }
    normalized
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppLayout {
    main_panel: Option<Rect>,
    transcript: Rect,
    extension_footer: Option<Rect>,
    composer_top: Option<Rect>,
    composer: Rect,
    composer_bottom: Option<Rect>,
}

fn app_layout(state: &TuiState, area: Rect) -> AppLayout {
    let composer_height = composer_height(state.composer.text(), area.width, area.height);
    let main_panel_count = extension_main_panel_surfaces(state, area)
        .into_iter()
        .take(1)
        .count();
    let footer_count = extension_status_footer_surfaces(state, area)
        .into_iter()
        .take(3)
        .count();
    let composer_top_count = extension_composer_top_surfaces(state, area)
        .into_iter()
        .take(1)
        .count();
    let composer_bottom_count = extension_composer_bottom_surfaces(state, area)
        .into_iter()
        .take(1)
        .count();
    let main_height = if main_panel_count == 0 { 0 } else { 4 };
    let footer_height = if footer_count == 0 {
        0
    } else {
        footer_count as u16 + 2
    };
    let composer_top_height = composer_top_count as u16;
    let composer_bottom_height = composer_bottom_count as u16;

    let mut constraints = Vec::new();
    if main_height > 0 {
        constraints.push(Constraint::Length(main_height));
    }
    constraints.push(Constraint::Min(MIN_TRANSCRIPT_HEIGHT));
    if footer_height > 0 {
        constraints.push(Constraint::Length(footer_height));
    }
    if composer_top_height > 0 {
        constraints.push(Constraint::Length(composer_top_height));
    }
    constraints.push(Constraint::Length(composer_height));
    if composer_bottom_height > 0 {
        constraints.push(Constraint::Length(composer_bottom_height));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut index = 0;
    let main_panel = (main_height > 0).then(|| {
        let rect = chunks[index];
        index += 1;
        rect
    });
    let transcript = chunks[index];
    index += 1;
    let extension_footer = (footer_height > 0).then(|| {
        let rect = chunks[index];
        index += 1;
        rect
    });
    let composer_top = (composer_top_height > 0).then(|| {
        let rect = chunks[index];
        index += 1;
        rect
    });
    let composer = chunks[index];
    index += 1;
    let composer_bottom = (composer_bottom_height > 0).then(|| chunks[index]);

    AppLayout {
        main_panel,
        transcript,
        extension_footer,
        composer_top,
        composer,
        composer_bottom,
    }
}

pub fn render_with_theme(frame: &mut Frame<'_>, state: &TuiState, theme: &Theme) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(theme.fg).bg(theme.bg)),
        area,
    );
    if area.width < 20 || area.height < 8 {
        render_tiny(frame, area, state, theme);
        return;
    }

    let layout = app_layout(state, area);
    if let Some(main_panel_area) = layout.main_panel {
        render_extension_main_panel(frame, main_panel_area, state, theme);
    }

    let transcript_area = layout.transcript;
    let sidebar_surfaces = extension_surfaces(state, UiSurfaceKind::Sidebar, transcript_area);
    if sidebar_surfaces.is_empty() || transcript_area.width < 64 {
        render_transcript(frame, transcript_area, state, theme);
    } else {
        let sidebar_width = extension_sidebar_width(&sidebar_surfaces, transcript_area.width);
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(30), Constraint::Length(sidebar_width)])
            .split(transcript_area);
        render_transcript(frame, panes[0], state, theme);
        render_extension_sidebar(frame, panes[1], state, theme, &sidebar_surfaces);
    }

    if let Some(footer_area) = layout.extension_footer {
        render_extension_footer(frame, footer_area, state, theme);
    }

    if let Some(composer_top_area) = layout.composer_top {
        render_extension_composer_top(frame, composer_top_area, state, theme);
    }

    let composer_area = layout.composer;
    render_composer(frame, composer_area, state, theme);

    if let Some(composer_bottom_area) = layout.composer_bottom {
        render_extension_composer_bottom(frame, composer_bottom_area, state, theme);
    }

    if state.overlay.is_none() {
        if let Some(suggestions) = state.command_suggestions_view() {
            render_command_suggestions(frame, area, composer_area, &suggestions, theme);
        }
        render_extension_floating_panels(frame, area, state, theme);
        render_extension_autosuggest_badges(frame, area, composer_area, state, theme);
    }

    match state.overlay {
        Some(OverlayKind::Help) => render_help_overlay(frame, area, state, theme),
        Some(OverlayKind::Settings) => {
            render_settings_overlay(frame, area, &state.settings, theme);
            render_extension_settings_badges(frame, area, state, theme);
        }
        Some(OverlayKind::SendPanel) => render_send_panel_overlay(frame, area, state, theme),
        Some(OverlayKind::Sessions) => render_sessions_overlay(frame, area, state, theme),
        Some(OverlayKind::Prompts) => render_prompts_overlay(frame, area, state, theme),
        Some(OverlayKind::Skills) => render_skills_overlay(frame, area, state, theme),
        Some(OverlayKind::Extensions) => render_extensions_overlay(frame, area, state, theme),
        Some(OverlayKind::Inspect) => render_inspect_overlay(frame, area, state, theme),
        Some(OverlayKind::Usage) => render_usage_overlay(frame, area, state, theme),
        Some(OverlayKind::AskUser) => render_ask_user_overlay(frame, area, state, theme),
        Some(OverlayKind::Btw) => render_btw_overlay(frame, area, state, theme),
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
        " {}: Enter queue • / draft • s settings • q send • b btw • Esc cancel ",
        keymap.chord_key
    );
    frame.render_widget(
        Block::default()
            .title(Span::styled(
                title,
                diagnostic_style(theme.diagnostic_error, theme).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(diagnostic_style(theme.diagnostic_error, theme)),
        area,
    );
}

fn render_tiny(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let mut lines = vec![Line::from(Span::styled(
        TINY_MESSAGE,
        diagnostic_style(theme.diagnostic_warning, theme),
    ))];
    let badges = extension_tiny_fallback_labels(state, area);
    if !badges.is_empty() && area.height > 1 {
        lines.push(Line::from(Span::styled(
            format!("Ext: {}", badges.join(", ")),
            badge_style(theme.badge_muted, theme),
        )));
    }
    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn extension_surfaces(
    state: &TuiState,
    surface_kind: UiSurfaceKind,
    area: Rect,
) -> Vec<&ActiveContribution<UiSurfaceContribution>> {
    let mut seen_effective_id = std::collections::BTreeSet::new();
    let mut seen_contribution_id = std::collections::BTreeSet::new();
    state
        .extension_ui
        .surfaces
        .iter()
        .filter(|surface| surface.entry.contribution.surface == surface_kind)
        .filter(|surface| seen_effective_id.insert(surface.effective_id.as_str().to_string()))
        .filter(|surface| {
            seen_contribution_id.insert(surface.entry.contribution.id.as_str().to_string())
        })
        .filter(|surface| {
            !state
                .extension_ui
                .surface_controller
                .is_slot_hidden(&extension_surface_slot_key(&surface.entry.contribution))
        })
        .filter(|surface| {
            ui_surface_layout_decision(&surface.entry.contribution, area.width, area.height.max(8))
                == UiSurfaceLayoutDecision::Render
        })
        .collect()
}

fn extension_main_panel_surfaces(
    state: &TuiState,
    area: Rect,
) -> Vec<&ActiveContribution<UiSurfaceContribution>> {
    [
        UiSurfaceKind::Header,
        UiSurfaceKind::MainPanel,
        UiSurfaceKind::WidgetAboveComposer,
        UiSurfaceKind::WidgetBelowComposer,
    ]
    .into_iter()
    .flat_map(|kind| extension_surfaces(state, kind, area))
    .collect()
}

fn extension_composer_top_surfaces(
    state: &TuiState,
    area: Rect,
) -> Vec<&ActiveContribution<UiSurfaceContribution>> {
    extension_surfaces(state, UiSurfaceKind::FooterTop, area)
        .into_iter()
        .filter(|surface| surface_slot(&surface.entry.contribution) == COMPOSER_DIRECT_TOP_SLOT)
        .collect()
}

fn extension_composer_bottom_surfaces(
    state: &TuiState,
    area: Rect,
) -> Vec<&ActiveContribution<UiSurfaceContribution>> {
    extension_surfaces(state, UiSurfaceKind::FooterBottom, area)
        .into_iter()
        .filter(|surface| surface_slot(&surface.entry.contribution) == COMPOSER_DIRECT_BOTTOM_SLOT)
        .collect()
}

fn is_composer_direct_surface(surface: &UiSurfaceContribution) -> bool {
    matches!(
        (surface.surface, surface_slot(surface)),
        (UiSurfaceKind::FooterTop, COMPOSER_DIRECT_TOP_SLOT)
            | (UiSurfaceKind::FooterBottom, COMPOSER_DIRECT_BOTTOM_SLOT)
    )
}

fn extension_status_footer_surfaces(
    state: &TuiState,
    area: Rect,
) -> Vec<&ActiveContribution<UiSurfaceContribution>> {
    [
        UiSurfaceKind::FooterTop,
        UiSurfaceKind::Footer,
        UiSurfaceKind::FooterBottom,
        UiSurfaceKind::InlineStatus,
        UiSurfaceKind::Status,
        UiSurfaceKind::WorkingIndicator,
        UiSurfaceKind::Notification,
        UiSurfaceKind::Health,
        UiSurfaceKind::Theme,
    ]
    .into_iter()
    .flat_map(|kind| extension_surfaces(state, kind, area))
    .filter(|surface| !is_composer_direct_surface(&surface.entry.contribution))
    .collect()
}

fn surface_slot(surface: &UiSurfaceContribution) -> &str {
    if surface.layout.slot == "primary" {
        surface.surface.default_slot()
    } else {
        surface.layout.slot.as_str()
    }
}

fn extension_tiny_fallback_labels(state: &TuiState, area: Rect) -> Vec<String> {
    state
        .extension_ui
        .surfaces
        .iter()
        .filter(|surface| {
            !state
                .extension_ui
                .surface_controller
                .is_slot_hidden(&extension_surface_slot_key(&surface.entry.contribution))
        })
        .filter_map(|surface| {
            match ui_surface_layout_decision(&surface.entry.contribution, area.width, area.height) {
                UiSurfaceLayoutDecision::CompactBadge | UiSurfaceLayoutDecision::StatusLine => {
                    Some(extension_surface_label(state, surface))
                }
                UiSurfaceLayoutDecision::Render | UiSurfaceLayoutDecision::Hide => None,
            }
        })
        .take(3)
        .collect()
}

fn extension_surface_label(
    state: &TuiState,
    surface: &ActiveContribution<UiSurfaceContribution>,
) -> String {
    if let Some(label) = builtin_footer_status_label(state, surface) {
        return label;
    }
    let title = surface.entry.contribution.title.trim();
    let mut label = if title.is_empty() {
        surface.effective_id.as_str().to_string()
    } else {
        title.to_string()
    };
    if let Some(summary) = state
        .extension_ui
        .state_summaries
        .get(&surface.effective_id)
    {
        if !summary.trim().is_empty() {
            label.push_str(": ");
            label.push_str(summary.trim());
        }
    }
    if surface_has_conflict(state, &surface.effective_id) {
        label.push_str(" ⚠ conflict");
    }
    label
}

fn builtin_footer_status_label(
    state: &TuiState,
    surface: &ActiveContribution<UiSurfaceContribution>,
) -> Option<String> {
    let id = surface.entry.contribution.id.as_str();
    let owner_is_builtin_footer = surface
        .entry
        .metadata
        .package_id
        .as_ref()
        .is_some_and(|package_id| package_id.as_str() == FOOTER_STATUS_PACKAGE_ID)
        || surface
            .entry
            .metadata
            .extension_id
            .as_ref()
            .is_some_and(|extension_id| extension_id.as_str() == FOOTER_STATUS_PACKAGE_ID);
    if owner_is_builtin_footer
        && (id == FOOTER_STATUS_TOP_ID || surface.effective_id.as_str() == FOOTER_STATUS_TOP_ID)
    {
        return Some(format!(
            "model: {} • thinking: {} • {}",
            state.settings.selected_model_label(),
            thinking_label(state.settings.selected_thinking_level),
            usage_footer_label(state)
        ));
    }
    if owner_is_builtin_footer
        && (id == FOOTER_STATUS_BOTTOM_ID
            || surface.effective_id.as_str() == FOOTER_STATUS_BOTTOM_ID)
    {
        let cwd = state.runtime_status.working_directory.trim();
        let cwd = if cwd.is_empty() { "." } else { cwd };
        return Some(format!(
            "cwd: {cwd} • branch: {} • {}",
            branch_footer_label(state),
            context_status_label(state)
        ));
    }
    None
}

fn context_status_label(state: &TuiState) -> String {
    match (
        state.runtime_status.context_tokens,
        state.settings.selected_model_context_length(),
    ) {
        (Some(used), Some(limit)) if limit > 0 => {
            let percent = (used as f64 / limit as f64 * 100.0).clamp(0.0, 999.0);
            format!("context: {:.0}%/{}", percent, compact_count(limit))
        }
        (Some(used), _) => format!("context: {}/unknown", compact_count(used)),
        (None, Some(limit)) if limit > 0 => format!("context: unknown/{}", compact_count(limit)),
        _ => "context: unknown".into(),
    }
}

fn compact_count(value: usize) -> String {
    if value >= 1_000_000 {
        compact_scaled_count(value, 1_000_000, "m")
    } else if value >= 1_000 {
        compact_scaled_count(value, 1_000, "k")
    } else {
        value.to_string()
    }
}

fn compact_scaled_count(value: usize, unit: usize, suffix: &str) -> String {
    let mut scaled = format!("{:.1}", value as f64 / unit as f64);
    if scaled.ends_with(".0") {
        scaled.truncate(scaled.len() - 2);
    }
    format!("{scaled}{suffix}")
}

fn surface_has_conflict(
    state: &TuiState,
    surface_id: &oino_extension_core::ContributionId,
) -> bool {
    state
        .extension_ui
        .conflicts
        .iter()
        .any(|conflict| conflict.owners.iter().any(|owner| owner == surface_id))
}

fn extension_sidebar_width(
    surfaces: &[&ActiveContribution<UiSurfaceContribution>],
    available_width: u16,
) -> u16 {
    let requested = surfaces
        .iter()
        .filter_map(|surface| surface.entry.contribution.layout.max_width)
        .max()
        .unwrap_or(32)
        .max(
            surfaces
                .iter()
                .map(|surface| surface.entry.contribution.layout.min_width)
                .max()
                .unwrap_or(24),
        );
    requested.min(available_width.saturating_sub(32)).max(20)
}

fn render_extension_sidebar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    theme: &Theme,
    surfaces: &[&ActiveContribution<UiSurfaceContribution>],
) {
    let lines = extension_surface_lines(state, surfaces, area.width.saturating_sub(2) as usize);
    let title = extension_group_title(
        "Extensions",
        surfaces.len(),
        state.extension_ui.conflicts.len(),
        extension_surfaces_have_focus(state, surfaces),
    );
    let paragraph = Paragraph::new(lines).style(panel_style(theme)).block(
        Block::default()
            .title(Span::styled(title, theme.title))
            .borders(Borders::ALL)
            .border_style(theme.panel_border)
            .style(panel_style(theme)),
    );
    frame.render_widget(paragraph, area);
}

fn render_extension_main_panel(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let surfaces = extension_main_panel_surfaces(state, area);
    if surfaces.is_empty() {
        return;
    }
    let lines = extension_surface_lines(state, &surfaces, area.width.saturating_sub(2) as usize);
    let paragraph = Paragraph::new(lines).style(panel_style(theme)).block(
        Block::default()
            .title(Span::styled(
                extension_group_title(
                    "Extension Main",
                    surfaces.len(),
                    0,
                    extension_surfaces_have_focus(state, &surfaces),
                ),
                theme.title,
            ))
            .borders(Borders::ALL)
            .border_style(theme.panel_border)
            .style(panel_style(theme)),
    );
    frame.render_widget(paragraph, area);
}

fn branch_footer_label(state: &TuiState) -> String {
    state
        .runtime_status
        .git_branch
        .as_deref()
        .filter(|branch| !branch.trim().is_empty())
        .unwrap_or("-")
        .to_string()
}

fn usage_footer_label(state: &TuiState) -> String {
    let Some(report) = state.usage.report.as_ref() else {
        return "cost: pending".into();
    };
    if report.session.reported_turns == 0 {
        return "cost: no usage".into();
    }
    let tokens = compact_count(report.session.total_tokens as usize);
    let cache_hit = cache_hit_rate_footer_label(
        report.session.input_tokens,
        report.session.cache_read_tokens,
    );
    if report.session.costs.is_empty() {
        format!("usage: {tokens} tok • cost: n/a • cache hit: {cache_hit}")
    } else {
        format!(
            "usage: {tokens} tok • cost: {} • cache hit: {cache_hit}",
            report.session.costs.join(", ")
        )
    }
}

fn cache_hit_rate_footer_label(input_tokens: u64, cache_read_tokens: u64) -> String {
    if input_tokens == 0 {
        return "n/a".into();
    }
    format!(
        "{:.1}%",
        (cache_read_tokens as f64 / input_tokens as f64) * 100.0
    )
}

fn render_extension_footer(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let surfaces = extension_status_footer_surfaces(state, area);
    if surfaces.is_empty() {
        return;
    }
    let lines = extension_surface_lines(state, &surfaces, area.width.saturating_sub(2) as usize);
    let title = extension_group_title(
        "Extension Status",
        surfaces.len(),
        state.extension_ui.conflicts.len(),
        extension_surfaces_have_focus(state, &surfaces),
    );
    let paragraph = Paragraph::new(lines).style(panel_style(theme)).block(
        Block::default()
            .title(Span::styled(title, theme.title))
            .borders(Borders::ALL)
            .border_style(theme.panel_border)
            .style(panel_style(theme)),
    );
    frame.render_widget(paragraph, area);
}

fn render_extension_composer_top(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    theme: &Theme,
) {
    let surfaces = extension_composer_top_surfaces(state, area);
    render_extension_composer_status_line(frame, area, state, theme, &surfaces);
}

fn render_extension_composer_bottom(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    theme: &Theme,
) {
    let surfaces = extension_composer_bottom_surfaces(state, area);
    render_extension_composer_status_line(frame, area, state, theme, &surfaces);
}

fn render_extension_composer_status_line(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    theme: &Theme,
    surfaces: &[&ActiveContribution<UiSurfaceContribution>],
) {
    if surfaces.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let text = surfaces
        .iter()
        .map(|surface| extension_surface_label(state, surface))
        .collect::<Vec<_>>()
        .join(" • ");
    let text = truncate_with_ellipsis(&text, area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme.footer)))
            .style(Style::default().fg(theme.muted).bg(theme.bg)),
        area,
    );
}

fn render_extension_floating_panels(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    theme: &Theme,
) {
    let mut surfaces = extension_surfaces(state, UiSurfaceKind::FloatingPanel, area);
    surfaces.extend(extension_surfaces(state, UiSurfaceKind::Overlay, area));
    if surfaces.is_empty() {
        return;
    }
    let width = if area.width < 24 {
        area.width
    } else {
        area.width.min(54)
    };
    let height = (surfaces.len() as u16)
        .saturating_add(3)
        .min(area.height.saturating_sub(2));
    if height < 3 {
        return;
    }
    let has_top_right_surface = surfaces.iter().any(|surface| {
        surface.entry.contribution.layout.slot == "floating:top-right"
            || surface.entry.contribution.layout.slot == "top-right"
    });
    let panel_area = if has_top_right_surface {
        Rect {
            x: area.x + area.width.saturating_sub(width),
            y: area.y,
            width,
            height,
        }
    } else {
        Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        }
    };
    frame.render_widget(Clear, panel_area);
    let lines = extension_surface_lines(
        state,
        &surfaces,
        panel_area.width.saturating_sub(2) as usize,
    );
    let paragraph = Paragraph::new(lines).style(panel_style(theme)).block(
        Block::default()
            .title(Span::styled(
                extension_group_title(
                    "Extension Panel",
                    surfaces.len(),
                    0,
                    extension_surfaces_have_focus(state, &surfaces),
                ),
                theme.title,
            ))
            .borders(Borders::ALL)
            .border_style(theme.focused_border)
            .style(panel_style(theme)),
    );
    frame.render_widget(paragraph, panel_area);
}

fn render_extension_autosuggest_badges(
    frame: &mut Frame<'_>,
    area: Rect,
    composer_area: Rect,
    state: &TuiState,
    theme: &Theme,
) {
    let mut surfaces = extension_surfaces(state, UiSurfaceKind::Autosuggest, area);
    surfaces.extend(extension_surfaces(
        state,
        UiSurfaceKind::TranscriptRenderer,
        area,
    ));
    surfaces.extend(extension_surfaces(
        state,
        UiSurfaceKind::MessageRenderer,
        area,
    ));
    surfaces.extend(extension_surfaces(
        state,
        UiSurfaceKind::ToolCallRenderer,
        area,
    ));
    surfaces.extend(extension_surfaces(
        state,
        UiSurfaceKind::ToolResultRenderer,
        area,
    ));
    surfaces.extend(extension_surfaces(state, UiSurfaceKind::ToolRenderer, area));
    surfaces.extend(extension_surfaces(state, UiSurfaceKind::Editor, area));
    if surfaces.is_empty() || composer_area.y == 0 {
        return;
    }
    let labels = surfaces
        .iter()
        .take(4)
        .map(|surface| extension_surface_label(state, surface))
        .collect::<Vec<_>>()
        .join(" • ");
    if labels.is_empty() {
        return;
    }
    let y = composer_area.y.saturating_sub(1);
    let badge_area = Rect {
        x: 1,
        y,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(truncate_to_width(
            &format!("Ext: {labels}"),
            badge_area.width as usize,
        ))
        .style(badge_style(theme.badge_accent, theme)),
        badge_area,
    );
}

fn render_extension_settings_badges(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
    theme: &Theme,
) {
    let surfaces = extension_surfaces(state, UiSurfaceKind::SettingsPage, area);
    if surfaces.is_empty() {
        return;
    }
    let height = (surfaces.len() as u16).saturating_add(2).min(6);
    let panel_area = Rect {
        x: area.x.saturating_add(area.width.saturating_sub(34)),
        y: area.y.saturating_add(1),
        width: area.width.min(33),
        height,
    };
    frame.render_widget(Clear, panel_area);
    let lines = extension_surface_lines(
        state,
        &surfaces,
        panel_area.width.saturating_sub(2) as usize,
    );
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(" Extension Settings ", theme.title))
            .borders(Borders::ALL)
            .border_style(theme.panel_border),
    );
    frame.render_widget(paragraph, panel_area);
}

fn extension_surface_lines(
    state: &TuiState,
    surfaces: &[&ActiveContribution<UiSurfaceContribution>],
    width: usize,
) -> Vec<Line<'static>> {
    let mut by_slot: BTreeMap<String, Vec<&ActiveContribution<UiSurfaceContribution>>> =
        BTreeMap::new();
    for surface in surfaces {
        by_slot
            .entry(extension_surface_slot_key(&surface.entry.contribution))
            .or_default()
            .push(*surface);
    }

    let mut lines = Vec::new();
    for (slot, group) in by_slot {
        let active_index = state
            .extension_ui
            .surface_controller
            .active_tab(&slot, group.len());
        if group.len() > 1 {
            lines.push(extension_surface_tab_line(
                state,
                &slot,
                &group,
                active_index,
                width,
            ));
        }
        if let Some(surface) = group.get(active_index) {
            let label = extension_surface_label(state, surface);
            let focused = state
                .extension_ui
                .surface_controller
                .focused_slot
                .as_deref()
                == Some(slot.as_str())
                || state.extension_ui.focused_surface.as_ref() == Some(&surface.effective_id);
            let focus = if focused { "▶ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(focus.to_string(), Style::default().fg(Color::Cyan)),
                Span::raw(truncate_to_width(&label, width.saturating_sub(focus.len()))),
            ]));
        }
    }
    lines
}

fn extension_surface_tab_line(
    state: &TuiState,
    slot: &str,
    surfaces: &[&ActiveContribution<UiSurfaceContribution>],
    active_index: usize,
    width: usize,
) -> Line<'static> {
    let mut text = format!("tabs {slot}: ");
    for (index, surface) in surfaces.iter().enumerate() {
        if index > 0 {
            text.push_str(" | ");
        }
        let label = surface.entry.contribution.title.trim();
        let label = if label.is_empty() {
            surface.effective_id.as_str()
        } else {
            label
        };
        if index == active_index {
            text.push('[');
            text.push_str(label);
            text.push(']');
        } else {
            text.push_str(label);
        }
    }
    if state
        .extension_ui
        .surface_controller
        .focused_slot
        .as_deref()
        == Some(slot)
    {
        text.insert_str(0, "FOCUS • ");
    }
    Line::styled(
        truncate_with_ellipsis(&text, width),
        Style::default().fg(Color::DarkGray),
    )
}

fn extension_group_title(label: &str, count: usize, conflicts: usize, focused: bool) -> String {
    let focus = if focused { " • FOCUSED" } else { "" };
    if conflicts == 0 {
        format!(" {label} {count}{focus} ")
    } else {
        format!(
            " {label} {count} • {conflicts} conflict{}{focus} ",
            if conflicts == 1 { "" } else { "s" }
        )
    }
}

fn extension_surfaces_have_focus(
    state: &TuiState,
    surfaces: &[&ActiveContribution<UiSurfaceContribution>],
) -> bool {
    surfaces.iter().any(|surface| {
        let slot = extension_surface_slot_key(&surface.entry.contribution);
        state
            .extension_ui
            .surface_controller
            .focused_slot
            .as_deref()
            == Some(slot.as_str())
            || state.extension_ui.focused_surface.as_ref() == Some(&surface.effective_id)
    })
}

pub fn transcript_visible_lines(state: &TuiState, width: u16, height: u16) -> usize {
    if width < 20 || height < 8 {
        return 1;
    }
    let layout = app_layout(
        state,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    );
    layout.transcript.height.saturating_sub(2).max(1) as usize
}

pub fn terminal_cursor_position(state: &TuiState, width: u16, height: u16) -> Option<(u16, u16)> {
    if width < 20 || height < 8 {
        return None;
    }
    if state.overlay == Some(OverlayKind::Btw) {
        let overlay = centered_rect(
            Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            86,
            72,
        );
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(6),
                Constraint::Length(5),
                Constraint::Length(2),
            ])
            .split(inner);
        let input_inner = sections[1].inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let last_line = state.btw.input.rsplit('\n').next().unwrap_or("");
        let line_count = state.btw.input.matches('\n').count() as u16;
        let x = input_inner
            .x
            .saturating_add(last_line.width() as u16)
            .min(input_inner.right().saturating_sub(1));
        let y = input_inner
            .y
            .saturating_add(line_count)
            .min(input_inner.bottom().saturating_sub(1));
        return Some((x, y));
    }
    if state.focus != TuiFocus::Composer || !state.composer.is_enabled() {
        return None;
    }
    let layout = app_layout(
        state,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    );
    let position = composer_cursor_position(layout.composer, &state.composer);
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

    let layout = app_layout(
        state,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    );
    let area = layout.transcript;
    let inner_height = area.height.saturating_sub(2) as usize;
    if inner_height == 0 {
        return Vec::new();
    }

    let theme = Theme::default();
    let full_inner_width = transcript_full_content_width(area.width);
    let (transcript, has_scrollbar) =
        prepared_transcript_for_viewport(state, full_inner_width, area.width, inner_height, &theme);
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
    let (transcript, has_scrollbar) =
        prepared_transcript_for_viewport(state, full_inner_width, area.width, inner_height, theme);

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
        .style(Style::default().fg(theme.fg).bg(theme.panel_bg))
        .padding(Padding::left(TRANSCRIPT_LEFT_PADDING));
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.fg).bg(theme.panel_bg))
            .block(block),
        area,
    );

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

fn prepared_transcript_for_viewport(
    state: &TuiState,
    full_width: usize,
    area_width: u16,
    inner_height: usize,
    theme: &Theme,
) -> (Arc<PreparedTranscript>, bool) {
    let can_show_scrollbar = area_width > 4 && inner_height > 1;
    if can_show_scrollbar && transcript_line_lower_bound(state) > inner_height {
        let narrowed =
            prepared_transcript_for_width(state, full_width.saturating_sub(1).max(1), theme);
        if narrowed.total_lines() > inner_height {
            return (narrowed, true);
        }
    }

    let full = prepared_transcript_for_width(state, full_width, theme);
    let needs_scrollbar = can_show_scrollbar && full.total_lines() > inner_height;
    if !needs_scrollbar {
        return (full, false);
    }

    let narrowed = prepared_transcript_for_width(state, full_width.saturating_sub(1).max(1), theme);
    let has_scrollbar = narrowed.total_lines() > inner_height;
    (narrowed, has_scrollbar)
}

fn transcript_line_lower_bound(state: &TuiState) -> usize {
    let mut blocks = 0usize;
    let mut lines = 0usize;
    for message in &state.messages {
        let block_lines = message_line_lower_bound(message, state.settings.thinking_collapse_mode);
        if block_lines == 0 {
            continue;
        }
        if state.settings.chat_style != ChatStyle::Minimal && blocks > 0 {
            lines = lines.saturating_add(1);
        }
        blocks = blocks.saturating_add(1);
        lines = lines.saturating_add(block_lines);
    }

    if state.error.is_some() {
        if state.settings.chat_style != ChatStyle::Minimal && blocks > 0 {
            lines = lines.saturating_add(1);
        }
        blocks = blocks.saturating_add(1);
        lines = lines.saturating_add(1);
    }

    let (_, status_text) = transcript_status(state);
    if status_text.is_some() {
        if blocks > 0 {
            lines = lines.saturating_add(1);
        }
        blocks = blocks.saturating_add(1);
        lines = lines.saturating_add(1);
    }

    if blocks == 0 {
        1
    } else {
        lines
    }
}

fn message_line_lower_bound(
    message: &MessageView,
    thinking_mode: crate::settings::CollapseMode,
) -> usize {
    if message.is_assistant() {
        return usize::from(
            message.content != "<empty>"
                || has_displayable_thinking_for_scroll(message, thinking_mode),
        );
    }
    if message.is_user() || message.is_error || message.role.starts_with("tool:") {
        return 1;
    }
    1
}

fn has_displayable_thinking_for_scroll(
    message: &MessageView,
    thinking_mode: crate::settings::CollapseMode,
) -> bool {
    thinking_mode != crate::settings::CollapseMode::Collapse
        && (message.thinking_redacted
            || message
                .thinking
                .as_ref()
                .is_some_and(|thinking| !thinking.trim().is_empty()))
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
        format!(
            " Task • {} mode • steer while streaming ",
            state.agent_mode.label()
        )
    } else {
        format!(" Task • {} mode ", state.agent_mode.label())
    };
    let border_style = if state.focus == TuiFocus::Composer && state.composer.is_enabled() {
        Style::default().fg(theme.focused_border)
    } else {
        Style::default().fg(theme.panel_border)
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().fg(theme.fg).bg(theme.composer_bg));
    let lines = composer_lines(area, &state.composer, theme);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.fg).bg(theme.composer_bg))
            .block(block),
        area,
    );

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
    let width = full_area.width.saturating_sub(4).min(72);
    if width < 12 {
        return;
    }
    let x = full_area.x + full_area.width.saturating_sub(width) / 2;
    let y = composer_area.y.saturating_sub(height);
    let area = Rect {
        x,
        y,
        width,
        height,
    };
    frame.render_widget(Clear, area);

    let content_width = area.width.saturating_sub(2) as usize;
    let lines = command_suggestion_lines(suggestions, content_capacity, content_width, theme);
    let title = truncate_with_ellipsis(
        &format!(
            " {} ",
            command_suggestion_title(suggestions, content_capacity)
        ),
        area.width.saturating_sub(2) as usize,
    );

    frame.render_widget(
        Paragraph::new(lines)
            .style(suggestion_panel_style(theme))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.suggestion_border))
                    .style(suggestion_panel_style(theme)),
            ),
        area,
    );
}

fn command_suggestion_lines(
    suggestions: &CommandSuggestionsView,
    content_capacity: usize,
    content_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if suggestions.items.is_empty() {
        return vec![Line::styled(
            truncate_with_ellipsis(
                &format!("No suggestion matches `{}`", suggestions.query),
                content_width,
            ),
            suggestion_muted_style(theme),
        )];
    }

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
            let style = suggestion_item_style(active, theme);
            let mut spans = vec![Span::styled(marker.to_string(), style)];
            if let Some(label) = item.category.label() {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    label,
                    command_category_style(item.category, theme),
                ));
            }
            let label_style = if item.category == CommandSuggestionCategory::Hint {
                command_category_style(item.category, theme)
            } else {
                suggestion_label_style(active, theme)
            };
            spans.push(Span::styled(format!(" {}", item.label), label_style));
            spans.push(Span::styled(
                format!("  {}", item.summary),
                suggestion_muted_style(theme),
            ));
            Line::from(truncate_spans_to_width(spans, content_width))
        })
        .collect()
}

fn command_suggestion_max_rows(suggestions: &CommandSuggestionsView) -> usize {
    match suggestions.title.as_str() {
        "Files" => 10,
        "Models" => 5,
        _ => 4,
    }
}

fn command_category_style(category: CommandSuggestionCategory, theme: &Theme) -> Style {
    let color = match category {
        CommandSuggestionCategory::System => theme.badge_warning,
        CommandSuggestionCategory::Prompt => theme.badge_accent,
        CommandSuggestionCategory::Skill => theme.badge_error,
        CommandSuggestionCategory::Extension => theme.badge_success,
        CommandSuggestionCategory::Model
        | CommandSuggestionCategory::File
        | CommandSuggestionCategory::Value => theme.badge_muted,
        CommandSuggestionCategory::Hint => theme.badge_accent,
    };
    badge_style(color, theme).add_modifier(Modifier::BOLD)
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
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
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
            Constraint::Length(2),
        ])
        .split(inner);

    let header_width = sections[0].width as usize;
    let option_status = if state.inspect.loading {
        " • loading…".to_string()
    } else {
        format!(" • {} tokens", state.inspect.token_count)
    };
    let option = Line::from(truncate_spans_to_width(
        vec![
            Span::styled("› ", Style::default().fg(theme.focused_border)),
            Span::styled(
                "Full prompt",
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(option_status, Style::default().fg(theme.muted)),
        ],
        header_width,
    ));
    let export_hint_text = state
        .inspect
        .export_message
        .as_deref()
        .unwrap_or("Press e to export chat as HTML");
    let export_hint = Line::styled(
        truncate_with_ellipsis(export_hint_text, header_width),
        Style::default().fg(theme.muted),
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
    render_overlay_footer(
        frame,
        sections[2],
        &footer,
        Style::default().fg(theme.muted),
    );
}

fn render_usage_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 74);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Usage ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(usage_summary_lines(
            state,
            sections[0].width as usize,
            theme,
        )),
        sections[0],
    );

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(sections[1]);
    render_usage_provider_list(frame, body_chunks[0], state, theme);
    render_usage_detail(frame, body_chunks[1], state, theme);

    let controls = if state.usage.search_active {
        "type to filter • ↑/↓ move • Enter accept • Esc clear"
    } else {
        "↑/↓ providers • PgUp/PgDn page • / filter • r refresh • Esc/q close"
    };
    render_overlay_footer(
        frame,
        sections[2],
        &format!("{} • {controls}", state.status),
        theme.footer,
    );
}

fn usage_summary_lines(state: &TuiState, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let Some(report) = &state.usage.report else {
        let message = if state.usage.loading {
            "Loading usage report…"
        } else if let Some(error) = &state.usage.error {
            error.as_str()
        } else {
            "No usage report loaded yet. Press r or run /usage to refresh."
        };
        return vec![Line::styled(
            truncate_with_ellipsis(message, width),
            Style::default().fg(theme.muted),
        )];
    };
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(&report.status_line, width),
        theme.title,
    )];
    let token_line = format!(
        "Session: {} assistant turn(s), {} reported • {} total tokens ({} in / {} out / {} cache read / {} cache write)",
        report.session.assistant_turns,
        report.session.reported_turns,
        report.session.total_tokens,
        report.session.input_tokens,
        report.session.output_tokens,
        report.session.cache_read_tokens,
        report.session.cache_write_tokens
    );
    lines.push(Line::styled(
        truncate_with_ellipsis(&token_line, width),
        Style::default().fg(theme.fg),
    ));
    let cost_line = if report.session.costs.is_empty() {
        "Cost: no provider cost data reported".into()
    } else {
        format!("Cost: {}", report.session.costs.join(", "))
    };
    lines.push(Line::styled(
        truncate_with_ellipsis(&cost_line, width),
        Style::default().fg(theme.muted),
    ));
    lines
}

fn render_usage_provider_list(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let content_width = area.width.saturating_sub(2) as usize;
    let content_height = list_content_height(area).max(1);
    let title = usage_provider_list_title(state);
    frame.render_widget(
        Paragraph::new(usage_provider_lines(
            state,
            content_width,
            content_height,
            theme,
        ))
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        area,
    );
}

fn usage_provider_list_title(state: &TuiState) -> String {
    let Some(report) = &state.usage.report else {
        return " Providers ".into();
    };
    let filtered = state.usage.filtered_provider_indices();
    if report.providers.is_empty() {
        " Providers 0/0 ".into()
    } else if filtered.is_empty() {
        format!(" Providers 0/{} ", report.providers.len())
    } else {
        let position = filtered
            .iter()
            .position(|index| *index == state.usage.cursor)
            .unwrap_or(0);
        if state.usage.search.trim().is_empty() {
            format!(" Providers {}/{} ", position + 1, report.providers.len())
        } else {
            format!(
                " Providers {}/{} ({} total) ",
                position + 1,
                filtered.len(),
                report.providers.len()
            )
        }
    }
}

fn usage_provider_lines(
    state: &TuiState,
    width: usize,
    height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![usage_search_line(state, width, theme), Line::from("")];
    let remaining_height = height.saturating_sub(lines.len()).max(1);
    if state.usage.loading {
        lines.push(Line::styled(
            truncate_with_ellipsis("Refreshing provider usage…", width),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if let Some(error) = &state.usage.error {
        lines.push(Line::styled(
            truncate_with_ellipsis(&format!("Error: {error}"), width),
            diagnostic_style(theme.diagnostic_error, theme),
        ));
        return lines;
    }
    let Some(report) = &state.usage.report else {
        lines.push(Line::styled(
            truncate_with_ellipsis("No usage report loaded.", width),
            Style::default().fg(theme.muted),
        ));
        return lines;
    };
    if report.providers.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                "No provider rows yet. Send a prompt or refresh usage after selecting a model.",
                width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered = state.usage.filtered_provider_indices();
    if filtered.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("No providers match `{}`", state.usage.search),
                width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered_position = filtered
        .iter()
        .position(|index| *index == state.usage.cursor)
        .unwrap_or(0);
    let range = visible_range(filtered_position, filtered.len(), remaining_height);
    for (offset, provider_index) in filtered[range.clone()].iter().enumerate() {
        if let Some(provider) = report.providers.get(*provider_index) {
            let display_index = range.start + offset;
            let active = *provider_index == state.usage.cursor;
            lines.push(usage_provider_line(
                display_index,
                provider,
                active,
                width,
                theme,
            ));
        }
    }
    lines
}

fn usage_search_line(state: &TuiState, width: usize, theme: &Theme) -> Line<'static> {
    if state.usage.search_active {
        return Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Filter: ", Style::default().fg(theme.focused_border)),
                Span::raw(state.usage.search.clone()),
                Span::styled("█", Style::default().fg(theme.focused_border)),
            ],
            width,
        ));
    }
    if state.usage.search.is_empty() {
        Line::styled(
            truncate_with_ellipsis("Press / to filter providers", width),
            Style::default().fg(theme.muted),
        )
    } else {
        Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Filter: ", Style::default().fg(theme.muted)),
                Span::raw(state.usage.search.clone()),
            ],
            width,
        ))
    }
}

fn usage_provider_line(
    index: usize,
    provider: &crate::app::UsagePanelProvider,
    active: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = arrow_marker(active);
    let prefix = format!("{marker} {}. ", index.saturating_add(1));
    let tokens = if provider.reported_turns == 0 {
        "no token report".into()
    } else {
        format!("{} tokens", provider.total_tokens)
    };
    let summary = format!(
        "{} ({}) • {} • {}",
        provider.display_name, provider.provider_id, provider.status, tokens
    );
    let available = width.saturating_sub(prefix.width()).max(1);
    let row_style = item_style(active, provider.status == "available", theme);
    Line::from(vec![
        Span::styled(prefix, row_style),
        Span::styled(truncate_with_ellipsis(&summary, available), row_style),
    ])
}

fn render_usage_detail(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = Vec::new();
    if let Some(error) = &state.usage.error {
        lines.push(Line::styled(
            truncate_with_ellipsis(&format!("Error: {error}"), content_width),
            diagnostic_style(theme.diagnostic_error, theme),
        ));
    } else if state.usage.loading {
        lines.push(Line::styled(
            "Refreshing usage report…",
            Style::default().fg(theme.muted),
        ));
    } else if let Some(provider) = state.usage.selected_provider() {
        lines.push(Line::styled(
            truncate_with_ellipsis(&provider.display_name, content_width),
            theme.title,
        ));
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("{} • {}", provider.provider_id, provider.status),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        lines.push(Line::from(""));
        for line in wrap_text(&provider.message, content_width.max(1))
            .into_iter()
            .take(6)
        {
            lines.push(Line::styled(line, Style::default().fg(theme.fg)));
        }
        lines.push(Line::from(""));
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!(
                    "Session: {} assistant turn(s), {} reported turn(s), {} tokens",
                    provider.assistant_turns, provider.reported_turns, provider.total_tokens
                ),
                content_width,
            ),
            Style::default().fg(theme.fg),
        ));
        if provider.costs.is_empty() {
            lines.push(Line::styled(
                truncate_with_ellipsis("Cost: no provider cost data", content_width),
                Style::default().fg(theme.muted),
            ));
        } else {
            lines.push(Line::styled(
                truncate_with_ellipsis(
                    &format!("Cost: {}", provider.costs.join(", ")),
                    content_width,
                ),
                Style::default().fg(theme.muted),
            ));
        }
        lines.push(Line::from(""));
        lines.push(Line::styled("Account usage", theme.title));
        if let Some(source) = &provider.account_source {
            lines.push(Line::styled(
                truncate_with_ellipsis(source, content_width),
                Style::default().fg(theme.muted),
            ));
            if let Some(balance) = &provider.account_balance {
                lines.push(Line::styled(
                    truncate_with_ellipsis(&format!("Balance: {balance}"), content_width),
                    Style::default().fg(theme.fg),
                ));
            }
            if provider.account_limits.is_empty() {
                lines.push(Line::styled(
                    truncate_with_ellipsis("No account limits reported.", content_width),
                    Style::default().fg(theme.muted),
                ));
            } else {
                for limit in provider.account_limits.iter().take(5) {
                    lines.push(Line::styled(
                        truncate_with_ellipsis(limit, content_width),
                        Style::default().fg(theme.fg),
                    ));
                }
            }
        } else {
            lines.push(Line::styled(
                truncate_with_ellipsis(
                    "No live account usage fetched yet; provider readiness is shown above.",
                    content_width,
                ),
                Style::default().fg(theme.muted),
            ));
        }
    } else {
        lines.push(Line::styled(
            truncate_with_ellipsis("No provider usage row selected.", content_width),
            Style::default().fg(theme.muted),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(" Details ")
                .borders(Borders::ALL)
                .border_style(section_border_style(false, theme)),
        ),
        area,
    );
}

fn render_ask_user_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 82, 72);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .title(Span::styled(" Ask User ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .padding(Padding::new(1, 1, 0, 0));
    frame.render_widget(block, overlay);
    let inner = overlay.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    let mut lines = Vec::new();
    if let Some(ask) = &state.ask_user {
        if let Some(question) = ask.question() {
            let header = if question.header.trim().is_empty() {
                format!(
                    "Question {}/{}",
                    ask.current + 1,
                    ask.request.questions.len()
                )
            } else {
                format!(
                    "{} · {}/{}",
                    question.header.trim(),
                    ask.current + 1,
                    ask.request.questions.len()
                )
            };
            lines.push(Line::styled(header, theme.title));
            lines.push(Line::from(question.question.clone()));
            lines.push(Line::from(""));
            for (index, option) in question.options.iter().enumerate() {
                let cursor = if index == ask.cursor { "›" } else { " " };
                let checked = if ask.selected[ask.current].contains(&index) {
                    if question.multi_select {
                        "[x]"
                    } else {
                        "(*)"
                    }
                } else if question.multi_select {
                    "[ ]"
                } else {
                    "( )"
                };
                let style = if index == ask.cursor {
                    Style::default()
                        .fg(theme.selected_fg)
                        .bg(theme.selection_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                lines.push(Line::styled(
                    format!("{cursor} {checked} {}", option.label),
                    style,
                ));
                if !option.description.trim().is_empty() {
                    lines.push(Line::styled(
                        format!(
                            "      {}",
                            truncate_to_width(
                                &option.description,
                                inner.width.saturating_sub(8) as usize
                            )
                        ),
                        Style::default().fg(theme.muted),
                    ));
                }
            }
            if let Some(preview) = ask
                .selected_option()
                .and_then(|option| option.preview.as_deref())
            {
                lines.push(Line::from(""));
                lines.push(Line::styled("Preview", theme.title));
                for line in preview.lines().take(8) {
                    lines.push(Line::styled(
                        truncate_to_width(line, inner.width.saturating_sub(2) as usize),
                        Style::default().fg(theme.muted),
                    ));
                }
            }
            if ask.custom_active {
                lines.push(Line::from(""));
                lines.push(Line::styled(
                    format!("Custom: {}", ask.custom_input),
                    Style::default()
                        .fg(theme.selected_fg)
                        .bg(theme.selection_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
    } else {
        lines.push(Line::from("Waiting for question data…"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.fg))
            .wrap(Wrap { trim: false }),
        sections[0],
    );
    render_overlay_footer(
        frame,
        sections[1],
        "↑/↓ move • Space toggle • Enter select/next • c custom • t chat • Esc cancel",
        theme.footer,
    );
}

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 78);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Help ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
    frame.render_widget(block, overlay);

    let inner = overlay.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);

    let entries = help_entries(&state.settings.keymap);
    let content_height = list_content_height(sections[0]);
    let content_width = sections[0].width.saturating_sub(2) as usize;
    let mut lines = vec![
        help_search_line(state, content_width, theme),
        Line::from(""),
    ];
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
            settings_muted_style(theme),
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
        truncate_with_ellipsis(
            &format!(
                " Oino Help {} match{} for `{}` ",
                filtered_indices.len(),
                if filtered_indices.len() == 1 {
                    ""
                } else {
                    "es"
                },
                state.help_search
            ),
            content_width,
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
    render_overlay_footer(frame, sections[1], controls, theme.footer);
}

fn help_search_line(state: &TuiState, width: usize, theme: &Theme) -> Line<'static> {
    if state.help_search_active {
        return Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.focused_border)),
                Span::raw(state.help_search.clone()),
                Span::styled("█", Style::default().fg(theme.focused_border)),
            ],
            width,
        ));
    }
    if state.help_search.is_empty() {
        Line::styled(
            truncate_with_ellipsis("Press / to search help", width),
            Style::default().fg(theme.muted),
        )
    } else {
        Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", settings_muted_style(theme)),
                Span::raw(state.help_search.clone()),
            ],
            width,
        ))
    }
}

fn help_entry_line(entry: &HelpEntry, width: usize, theme: &Theme) -> Line<'static> {
    match entry {
        HelpEntry::Heading(text) => Line::styled(
            truncate_with_ellipsis(text, width),
            theme.title.add_modifier(Modifier::BOLD),
        ),
        HelpEntry::Item(key, description) => {
            let prefix = format!("{key} — ");
            Line::from(truncate_spans_to_width(
                vec![
                    Span::styled(prefix, Style::default().fg(theme.focused_border)),
                    Span::styled(description.clone(), Style::default().fg(theme.fg)),
                ],
                width,
            ))
        }
        HelpEntry::Text(text) => Line::styled(
            truncate_with_ellipsis(text, width),
            Style::default().fg(theme.muted),
        ),
        HelpEntry::Blank => Line::from(""),
    }
}

fn render_btw_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 72);
    frame.render_widget(Clear, overlay);
    let model_label = if state.btw.inherited {
        "inherit"
    } else {
        "configured"
    };
    let title = format!(
        " BTW • plan • {model_label}: {} ",
        state.btw.effective_model
    );
    let block = Block::default()
        .title(Span::styled(title, theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
    frame.render_widget(block, overlay);
    let inner = overlay.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(5),
            Constraint::Length(2),
        ])
        .split(inner);

    let mut lines = Vec::new();
    if state.btw.messages.is_empty() {
        lines.push(Line::styled("No BTW messages yet.", theme.placeholder));
    } else {
        for message in &state.btw.messages {
            let title = message.title.as_deref().unwrap_or(&message.role);
            lines.push(Line::styled(format!("{title}:"), theme.title));
            for line in wrap_text(
                &message.content,
                sections[0].width.saturating_sub(2) as usize,
            ) {
                lines.push(Line::from(line));
            }
            lines.push(Line::from(""));
        }
    }
    if let Some(error) = &state.btw.error {
        lines.push(Line::styled(
            format!("Error: {error}"),
            diagnostic_style(theme.diagnostic_error, theme),
        ));
    } else if state.btw.working {
        lines.push(Line::styled(
            "Running…",
            diagnostic_style(theme.diagnostic_warning, theme),
        ));
    }
    let height = sections[0].height.saturating_sub(2) as usize;
    let start = lines.len().saturating_sub(height.max(1));
    frame.render_widget(
        Paragraph::new(lines[start..].to_vec())
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Side conversation ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            ),
        sections[0],
    );

    let input = if state.btw.input.is_empty() {
        "Type a BTW prompt…"
    } else {
        state.btw.input.as_str()
    };
    let input_style = if state.btw.input.is_empty() {
        theme.placeholder
    } else {
        Style::default().fg(theme.fg)
    };
    frame.render_widget(
        Paragraph::new(input.to_string())
            .style(input_style)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Input ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            ),
        sections[1],
    );
    render_overlay_footer(frame, sections[2], "Enter send • Ctrl-Enter newline • type /new to reset here • /btw new opens a fresh BTW panel • /model btw inherit|<model> • Esc close", theme.footer);
}

fn render_send_panel_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 70);
    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(" Send Panel ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
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
    render_overlay_footer(frame, sections[1], &status, theme.footer);
}

fn send_panel_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let items = state.send_panel_items();
    let input_label = "Input: ";
    let input_value = if state.input().trim().is_empty() {
        Span::styled("empty", theme.placeholder)
    } else {
        Span::styled(
            panel_preview(
                state.input(),
                content_width.saturating_sub(input_label.width()),
            ),
            Style::default().fg(theme.fg),
        )
    };
    let mut lines = vec![Line::from(truncate_spans_to_width(
        vec![
            Span::styled(input_label, Style::default().fg(theme.muted)),
            input_value,
        ],
        content_width,
    ))];
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
            truncate_with_ellipsis(&heading, content_width),
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
            lines.push(Line::styled(
                truncate_with_ellipsis("  (empty)", content_width),
                Style::default().fg(theme.muted),
            ));
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
    Line::from(truncate_spans_to_width(
        vec![
            Span::styled(label, item_style(active, false, theme)),
            Span::styled(
                panel_preview(&item.text, preview_width),
                item_style(active, false, theme),
            ),
        ],
        width,
    ))
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
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
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
    render_overlay_footer(frame, sections[1], &status, theme.footer);
}

fn sessions_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        sessions_search_line(state, content_width, theme),
        Line::from(""),
    ];
    let remaining_height = content_height.saturating_sub(lines.len()).max(1);

    if state.sessions.loading {
        lines.push(Line::styled(
            truncate_with_ellipsis("Loading saved sessions…", content_width),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if state.sessions.items.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis("No saved sessions yet.", content_width),
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

fn sessions_search_line(state: &TuiState, width: usize, theme: &Theme) -> Line<'static> {
    if state.sessions.search_active {
        return Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.focused_border)),
                Span::raw(state.sessions.search.clone()),
                Span::styled("█", Style::default().fg(theme.focused_border)),
            ],
            width,
        ));
    }
    if state.sessions.search.is_empty() {
        Line::styled(
            truncate_with_ellipsis("Press / to search sessions", width),
            Style::default().fg(theme.muted),
        )
    } else {
        Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.muted)),
                Span::raw(state.sessions.search.clone()),
            ],
            width,
        ))
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

fn render_extensions_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 88, 78);
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Block::default()
            .title(Span::styled(" Extensions ", theme.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.focused_border))
            .style(panel_style(theme)),
        overlay,
    );
    let inner = overlay.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(4)])
        .split(inner);
    let content_height = list_content_height(sections[0]);
    let content_width = sections[0].width.saturating_sub(2) as usize;
    let lines = extension_management_lines(state, content_width, content_height, theme);
    let filtered = &state.extension_management.filtered_indices;
    let title = extension_management_title(state, filtered.len());
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(section_border_style(true, theme)),
        ),
        sections[0],
    );
    let controls = if state.extension_management.install_active {
        "type path, Git URL, or owner/repo • Enter install • Esc cancel"
    } else if state.extension_management.remove_confirm.is_some() {
        "Enter/Y uninstall • N/Esc cancel"
    } else if state.extension_management.search_active {
        "type to fuzzy search • ↑/↓ move • Enter toggle project • Esc clear search"
    } else {
        "Tab switch tab • 1 Manage • 2 Registered • ↑/↓ select • / search • i/I install • u/x uninstall • g/p toggles • o/O prefer winner • c/C clear override • Esc close"
    };
    let status = format!("{} • {controls}", state.status);
    render_overlay_footer(frame, sections[1], &status, theme.footer);
}

fn extension_management_title(state: &TuiState, filtered_len: usize) -> String {
    let current = if filtered_len == 0 {
        0
    } else {
        state
            .extension_management
            .cursor
            .saturating_add(1)
            .min(filtered_len)
    };
    let total = state
        .extension_management
        .count_for_view(state.extension_management.view);
    let suffix = if state.extension_management.search.trim().is_empty() {
        format!("{current}/{total}")
    } else {
        format!("{current}/{filtered_len} ({total} in tab)")
    };
    format!(
        " Extensions • {} tab • {suffix} ",
        state.extension_management.view.label()
    )
}

fn extension_management_lines(
    state: &TuiState,
    content_width: usize,
    content_height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(extension_management_tabs_line(state, content_width, theme));
    let search = if state.extension_management.install_active {
        let input = if state.extension_management.install_input.is_empty() {
            "<package path, Git URL, or owner/repo>"
        } else {
            &state.extension_management.install_input
        };
        format!(
            "Install {} package: {input}",
            state.extension_management.install_scope.label()
        )
    } else if let Some(confirm) = &state.extension_management.remove_confirm {
        format!(
            "Confirm uninstall {} package `{}`",
            confirm.scope.label(),
            confirm.package_id
        )
    } else if state.extension_management.search_active {
        format!("Search: {}", state.extension_management.search)
    } else if state.extension_management.search.is_empty() {
        "Press / to search • i/I install • u/x uninstall • o/O prefer conflict winner • c/C clear override".into()
    } else {
        format!("Filter: {}", state.extension_management.search)
    };
    lines.push(Line::styled(
        truncate_with_ellipsis(&search, content_width),
        Style::default().fg(theme.muted),
    ));
    if let Some(item) = state.extension_management.selected_item() {
        lines.push(extension_management_selected_line(
            item,
            content_width,
            theme,
        ));
    }
    lines.push(Line::from(""));
    if state.extension_management.items.is_empty() {
        lines.push(Line::styled(
            "No extensions discovered yet.",
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let filtered = &state.extension_management.filtered_indices;
    if filtered.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!(
                    "No extension items match `{}`",
                    state.extension_management.search
                ),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    let remaining = content_height.saturating_sub(lines.len()).max(1);
    let range = visible_range(state.extension_management.cursor, filtered.len(), remaining);
    lines.extend(
        filtered[range.clone()]
            .iter()
            .enumerate()
            .filter_map(|(offset, item_index)| {
                let item = state.extension_management.items.get(*item_index)?;
                let active = range.start + offset == state.extension_management.cursor;
                Some(extension_management_item_line(
                    item,
                    active,
                    content_width,
                    theme,
                ))
            }),
    );
    lines
}

fn extension_management_item_line(
    item: &crate::app::ExtensionManagementItem,
    active: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = if active { "›" } else { " " };
    let diagnostics = if item.diagnostics.is_empty() {
        String::new()
    } else {
        format!(" • {} diag", item.diagnostics.len())
    };
    let conflicts = if item.conflicts.is_empty() {
        String::new()
    } else {
        format!(" • {} conflict", item.conflicts.len())
    };
    let overrides = extension_override_badges(item);
    let mut spans = vec![
        Span::styled(
            format!("{marker} "),
            extension_role_style(theme.settings_active, active, theme),
        ),
        Span::styled(
            "P:",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            extension_on_off(item.project_enabled),
            extension_role_style(
                extension_toggle_color(item.project_enabled, theme),
                active,
                theme,
            ),
        ),
        Span::styled(
            " G:",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            extension_on_off(item.global_enabled),
            extension_role_style(
                extension_toggle_color(item.global_enabled, theme),
                active,
                theme,
            ),
        ),
    ];
    if !overrides.is_empty() {
        spans.push(Span::styled(
            overrides.to_string(),
            extension_role_style(theme.extension_override, active, theme),
        ));
    }
    spans.extend([
        Span::styled(
            " [",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            extension_management_kind_label(item),
            extension_role_style(extension_kind_color(item, theme), active, theme),
        ),
        Span::styled(
            "] ",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            item.id.clone(),
            extension_role_style(theme.settings_fg, active, theme),
        ),
        Span::styled(
            " — ",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            item.title.clone(),
            extension_role_style(theme.settings_fg, active, theme),
        ),
        Span::styled(
            " • ",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            item.health.clone(),
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            " • ",
            extension_role_style(theme.settings_muted, active, theme),
        ),
        Span::styled(
            item.permission.clone(),
            extension_role_style(theme.settings_muted, active, theme),
        ),
    ]);
    if !diagnostics.is_empty() {
        spans.push(Span::styled(
            diagnostics.to_string(),
            extension_role_style(theme.extension_diagnostic, active, theme),
        ));
    }
    if !conflicts.is_empty() {
        spans.push(Span::styled(
            conflicts.to_string(),
            extension_role_style(theme.extension_conflict, active, theme),
        ));
    }
    Line::from(truncate_spans_to_width(spans, width))
}

fn extension_toggle_color(value: bool, theme: &Theme) -> Color {
    if value {
        theme.extension_enabled
    } else {
        theme.extension_disabled
    }
}

fn extension_kind_color(item: &crate::app::ExtensionManagementItem, theme: &Theme) -> Color {
    match item.target {
        crate::app::ExtensionManagementTarget::Package => theme.extension_package,
        crate::app::ExtensionManagementTarget::Extension => theme.extension_runtime,
        crate::app::ExtensionManagementTarget::Contribution => theme.extension_contribution,
    }
}

fn extension_management_tabs_line(state: &TuiState, width: usize, theme: &Theme) -> Line<'static> {
    let management = &state.extension_management;
    let segments = ExtensionManagementView::ALL
        .iter()
        .map(|view| {
            let label = format!("{} ({})", view.label(), management.count_for_view(*view));
            if *view == management.view {
                format!("[{label}]")
            } else {
                label
            }
        })
        .collect::<Vec<_>>()
        .join("  ");
    Line::styled(
        truncate_with_ellipsis(&format!("Tabs: {segments}"), width),
        Style::default().fg(theme.accent),
    )
}

fn extension_management_selected_line(
    item: &crate::app::ExtensionManagementItem,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let diagnostics = if item.diagnostics.is_empty() {
        String::new()
    } else {
        format!(" • {} diag", item.diagnostics.len())
    };
    let conflicts = if item.conflicts.is_empty() {
        String::new()
    } else {
        format!(" • {} conflict", item.conflicts.len())
    };
    let overrides = extension_override_badges(item);
    let mut spans = vec![
        Span::styled("Selected: ", settings_muted_style(theme)),
        Span::styled("P:", settings_muted_style(theme)),
        Span::styled(
            extension_on_off(item.project_enabled),
            Style::default().fg(extension_toggle_color(item.project_enabled, theme)),
        ),
        Span::styled(" G:", settings_muted_style(theme)),
        Span::styled(
            extension_on_off(item.global_enabled),
            Style::default().fg(extension_toggle_color(item.global_enabled, theme)),
        ),
    ];
    if !overrides.is_empty() {
        spans.push(Span::styled(
            overrides,
            Style::default().fg(theme.extension_override),
        ));
    }
    spans.extend([
        Span::styled(" • [", settings_muted_style(theme)),
        Span::styled(
            extension_management_kind_label(item),
            Style::default().fg(extension_kind_color(item, theme)),
        ),
        Span::styled("] ", settings_muted_style(theme)),
        Span::styled(item.id.clone(), settings_text_style(theme)),
        Span::styled(" — ", settings_muted_style(theme)),
        Span::styled(item.title.clone(), settings_text_style(theme)),
        Span::styled(" • ", settings_muted_style(theme)),
        Span::styled(item.health.clone(), settings_muted_style(theme)),
        Span::styled(" • ", settings_muted_style(theme)),
        Span::styled(item.permission.clone(), settings_muted_style(theme)),
    ]);
    if !diagnostics.is_empty() {
        spans.push(Span::styled(
            diagnostics,
            Style::default().fg(theme.extension_diagnostic),
        ));
    }
    if !conflicts.is_empty() {
        spans.push(Span::styled(
            conflicts,
            Style::default().fg(theme.extension_conflict),
        ));
    }
    Line::from(truncate_spans_to_width(spans, width))
}

fn extension_management_kind_label(item: &crate::app::ExtensionManagementItem) -> String {
    let kind = match item.target {
        crate::app::ExtensionManagementTarget::Package => "package",
        crate::app::ExtensionManagementTarget::Extension => "extension",
        crate::app::ExtensionManagementTarget::Contribution => item.family.as_str(),
    };
    format!("{} {kind}", item.scope)
}

fn extension_on_off(value: bool) -> &'static str {
    if value {
        "ON"
    } else {
        "OFF"
    }
}

fn extension_override_badges(item: &crate::app::ExtensionManagementItem) -> String {
    match (item.global_override, item.project_override) {
        (false, false) => String::new(),
        (true, false) => " OVR:G".into(),
        (false, true) => " OVR:P".into(),
        (true, true) => " OVR:G/P".into(),
    }
}

fn render_prompts_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 72);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .title(Span::styled(" Prompts ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
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
    render_overlay_footer(frame, sections[1], &status, theme.footer);
}

fn render_skills_overlay(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let overlay = centered_rect(area, 86, 72);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .title(Span::styled(" Skills ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
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
    render_overlay_footer(frame, sections[1], &status, theme.footer);
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
            content_width,
            theme,
        ),
        Line::from(""),
    ];
    let remaining_height = content_height.saturating_sub(lines.len()).max(1);
    if state.prompts.loading {
        lines.push(Line::styled(
            truncate_with_ellipsis("Reloading resources…", content_width),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if state.prompt_resources.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis("No prompt templates found.", content_width),
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
            content_width,
            theme,
        ),
        Line::from(""),
    ];
    let remaining_height = content_height.saturating_sub(lines.len()).max(1);
    if state.skills.loading {
        lines.push(Line::styled(
            truncate_with_ellipsis("Reloading resources…", content_width),
            Style::default().fg(theme.muted),
        ));
        return lines;
    }
    if state.skill_resources.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis("No skills found.", content_width),
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
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    if active {
        return Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.focused_border)),
                Span::raw(search.to_string()),
                Span::styled("█", Style::default().fg(theme.focused_border)),
            ],
            width,
        ));
    }
    if search.is_empty() {
        Line::styled(
            truncate_with_ellipsis(empty_hint, width),
            Style::default().fg(theme.muted),
        )
    } else {
        Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.muted)),
                Span::raw(search.to_string()),
            ],
            width,
        ))
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
        .title(Span::styled(" Settings ", settings_title_style(theme)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focused_border))
        .style(panel_style(theme));
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
        SettingsPage::Auth => render_auth_settings(frame, sections[0], settings, theme),
        SettingsPage::Keymaps => render_keymap_settings(frame, sections[0], settings, theme),
        SettingsPage::Theme => render_theme_settings(frame, sections[0], settings, theme),
        SettingsPage::Notify => render_notify_settings(frame, sections[0], settings, theme),
        SettingsPage::Compaction => render_compaction_settings(frame, sections[0], settings, theme),
        SettingsPage::NotifyModelPicker => render_sub_model_panel(
            frame,
            sections[0],
            &settings.sub_model_picker,
            " Summary Model ",
            theme,
        ),
        SettingsPage::CompactionModelPicker => render_sub_model_panel(
            frame,
            sections[0],
            &settings.sub_model_picker,
            " Compaction Model ",
            theme,
        ),
        SettingsPage::Extensions => render_settings_extensions_page(frame, sections[0], theme),
    }
    render_settings_footer(frame, sections[1], settings, theme);
}

fn render_settings_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let items = settings.menu_items();
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis("Choose a settings page:", content_width),
        settings_muted_style(theme),
    )];
    lines.push(Line::from(""));
    lines.extend(items.iter().enumerate().map(|(index, item)| {
        let active = index == settings.menu_cursor;
        let marker = arrow_marker(active);
        let detail = match item {
            SettingsMenuItem::ModelSelection => {
                format!("current: {}", settings.selected_model_label())
            }
            SettingsMenuItem::ThinkingLevel => format!(
                "current: {}",
                thinking_label(settings.selected_thinking_level)
            ),
            SettingsMenuItem::CollapseMode => format!(
                "thinking: {}, tool: {}",
                collapse_mode_label(settings.thinking_collapse_mode),
                collapse_mode_label(settings.tool_collapse_mode)
            ),
            SettingsMenuItem::ChatStyle => {
                format!("current: {}", chat_style_label(settings.chat_style))
            }
            SettingsMenuItem::Tools => format!("{} registered", settings.tools.len()),
            SettingsMenuItem::Auth => format!("{} provider(s)", settings.auth_items.len()),
            SettingsMenuItem::Keymaps => format!("preset: {}", settings.keymap.preset.label()),
            SettingsMenuItem::Theme => settings.effective_theme.as_ref().map_or_else(
                || "current: system".into(),
                |theme| {
                    format!(
                        "current: {} ({})",
                        theme.display_name,
                        theme.selected_scope.label()
                    )
                },
            ),
            SettingsMenuItem::Notify => {
                let status = if settings.notify.effective_enabled() {
                    "on"
                } else {
                    "off"
                };
                let topic = settings
                    .notify
                    .effective_text(crate::settings::NotifyField::Topic)
                    .unwrap_or_else(|| "missing topic".into());
                format!("{status}, topic: {topic}")
            }
            SettingsMenuItem::Extensions => "open extension manager".into(),
            SettingsMenuItem::Compaction => "configure session compaction".into(),
        };
        let text = truncate_with_ellipsis(
            &format!("{marker} {}  {}", item.label(), detail),
            content_width,
        );
        Line::styled(text, settings_item_style(active, false, theme))
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

fn render_compaction_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let content_width = area.width.saturating_sub(2) as usize;
    let compact = &settings.compact;
    let method_label = if compact.method_is_llm { "LLM" } else { "VCC" };
    let model_display = compact.model.as_deref().unwrap_or("inherit");
    let prompt_display = compact
        .prompt
        .as_deref()
        .unwrap_or("default (.oino/prompts/compact.md)");
    let threshold_display = compact
        .threshold_pct
        .map(|p| format!("{p}%"))
        .unwrap_or_else(|| "disabled".to_string());

    let rows: [(String, String); 5] = [
        (
            "Method".into(),
            format!(
                "{method_label} — use Left/Right to toggle. VCC is deterministic, LLM summarizes with an AI model."
            ),
        ),
        (
            "Auto-compact".into(),
            format!(
                "{} — use Left/Right to toggle. Triggers when context exceeds threshold.",
                if compact.auto_enabled { "on" } else { "off" }
            ),
        ),
        (
            "Threshold".into(),
            format!(
                "{threshold_display} — use /compact threshold <pct> to change"
            ),
        ),
        (
            "LLM Model".into(),
            format!(
                "{model_display} — use /compact model <provider:model> to change"
            ),
        ),
        (
            "LLM Prompt".into(),
            format!(
                "{prompt_display} — use /compact prompt <path> to change"
            ),
        ),
    ];

    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            "Configure session compaction behaviour. Use /compact commands to change values.",
            content_width,
        ),
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));

    for (index, (label, description)) in rows.iter().enumerate() {
        let active = index == compact.cursor;
        let marker = if active { "\u{25b6} " } else { "  " };
        lines.push(Line::styled(
            truncate_with_ellipsis(&format!("{marker}{label}: {description}"), content_width),
            item_style(active, false, theme),
        ));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Compaction ")
                    .borders(Borders::ALL)
                    .border_style(section_border_style(true, theme)),
            )
            .alignment(Alignment::Left),
        area,
    );
}

fn render_settings_extensions_page(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = vec![
        Line::styled(truncate_with_ellipsis("Extension Manager", content_width), theme.title),
        Line::from(""),
        Line::styled(
            truncate_with_ellipsis("Press Enter to open the extension manager.", content_width),
            settings_text_style(theme),
        ),
        Line::styled(
            truncate_with_ellipsis(
                "Install packages, toggle project/global policy, and review contribution diagnostics there.",
                content_width,
            ),
            settings_muted_style(theme),
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Extensions ")
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
    let sel = &settings.model_selector;
    render_model_panel(frame, area, sel, " Model Selection ", theme);
}

fn render_sub_model_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    selector: &crate::model_selector::ModelSelector,
    title: &str,
    theme: &Theme,
) {
    render_model_panel(frame, area, selector, title, theme);
}

/// Reusable model selector panel renderer. Used for all model picker contexts.
pub(crate) fn render_model_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    selector: &crate::model_selector::ModelSelector,
    default_title: &str,
    theme: &Theme,
) {
    let filtered_indices = selector.filtered_indices();
    let filtered_position = selector.cursor_filtered_position();
    let title = if selector.models.is_empty() {
        default_title.to_string()
    } else if selector.refreshing {
        format!(
            "{} {}/{} ({} total, refreshing) ",
            default_title.trim_end(),
            filtered_position
                .saturating_add(1)
                .min(filtered_indices.len()),
            filtered_indices.len(),
            selector.models.len()
        )
    } else {
        format!(
            "{} {}/{} ({} total) ",
            default_title.trim_end(),
            filtered_position
                .saturating_add(1)
                .min(filtered_indices.len()),
            filtered_indices.len(),
            selector.models.len()
        )
    };
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![
        model_search_line(selector, content_width, theme),
        Line::from(""),
    ];
    if selector.models.is_empty() {
        lines.push(Line::styled(
            "Loading model catalog…",
            settings_muted_style(theme),
        ));
    } else if filtered_indices.is_empty() {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!("No models match `{}`", selector.search),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ));
    } else {
        let visible_height = list_content_height(area).saturating_sub(2).max(1);
        let range = visible_range(filtered_position, filtered_indices.len(), visible_height);
        let mut last_provider = "";
        lines.extend(
            filtered_indices
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end.saturating_sub(range.start))
                .filter_map(|(_, model_index)| {
                    let model = selector.models.get(*model_index)?;
                    let active = *model_index == selector.cursor;
                    let selected = model.id == selector.initial_model;
                    let marker = selection_marker(active, selected);
                    let style = settings_item_style(active, selected, theme);
                    let show_group = !selector.search_active
                        && !model.provider.is_empty()
                        && model.provider != "unknown"
                        && model.provider != last_provider;
                    if show_group {
                        last_provider = &model.provider;
                    }
                    let provider_label = if model.provider_label.trim().is_empty() {
                        model.provider.as_str()
                    } else {
                        model.provider_label.as_str()
                    };
                    let availability_label = match model.availability {
                        crate::settings::ModelAvailability::Unknown => None,
                        availability => Some(availability.label()),
                    };
                    let mut label = if model.provider == model.id || model.provider == "unknown" {
                        availability_label.map_or_else(
                            || model.display_name.clone(),
                            |availability| format!("{} ({availability})", model.display_name),
                        )
                    } else if let Some(availability) = availability_label {
                        format!(
                            "[{} • {}] {}",
                            provider_label, availability, model.display_name
                        )
                    } else {
                        format!("[{}] {}", provider_label, model.display_name)
                    };
                    if selector.show_pricing
                        && !selector.search_active
                        && selector.search.is_empty()
                    {
                        label.push_str(&model_pricing_suffix(model));
                    }
                    let mut result = Vec::new();
                    if show_group && !active {
                        result.push(Line::styled(
                            truncate_with_ellipsis(&format!(" {}", provider_label), content_width),
                            Style::default().fg(theme.muted),
                        ));
                    }
                    result.push(Line::styled(
                        truncate_with_ellipsis(&format!("{marker} {label}"), content_width),
                        style,
                    ));
                    Some(result)
                })
                .flatten(),
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

fn model_pricing_suffix(model: &crate::settings::ModelOption) -> String {
    let Some(pricing) = model.pricing.as_ref() else {
        return "  in n/a out n/a cache n/a".into();
    };
    let source = if pricing.source.trim().is_empty() {
        ""
    } else {
        match pricing.source.as_str() {
            "provider" => " src:provider",
            source => {
                return format!(
                    "  in {} out {} cache {} src:{}",
                    format_model_price_per_million(pricing.input_per_token.as_deref()),
                    format_model_price_per_million(pricing.output_per_token.as_deref()),
                    format_model_price_per_million(pricing.cache_hit_per_token.as_deref()),
                    source
                )
            }
        }
    };
    format!(
        "  in {} out {} cache {}{}",
        format_model_price_per_million(pricing.input_per_token.as_deref()),
        format_model_price_per_million(pricing.output_per_token.as_deref()),
        format_model_price_per_million(pricing.cache_hit_per_token.as_deref()),
        source
    )
}

fn format_model_price_per_million(value: Option<&str>) -> String {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return "n/a".into();
    };
    let Ok(per_token) = value.parse::<f64>() else {
        return "n/a".into();
    };
    if per_token == 0.0 {
        "$0/M".into()
    } else {
        format!("${:.2}/M", per_token * 1_000_000.0)
    }
}

fn model_search_line(
    selector: &crate::model_selector::ModelSelector,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    if selector.search_active {
        return Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.focused_border)),
                Span::raw(selector.search.clone()),
                Span::styled("█", Style::default().fg(theme.focused_border)),
            ],
            width,
        ));
    }
    if selector.search.is_empty() {
        Line::styled(
            truncate_with_ellipsis("Press / to search models • p toggle prices", width),
            settings_muted_style(theme),
        )
    } else {
        Line::from(truncate_spans_to_width(
            vec![
                Span::styled("Search: ", Style::default().fg(theme.muted)),
                Span::raw(selector.search.clone()),
            ],
            width,
        ))
    }
}

fn render_thinking_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let levels = settings.thinking_levels();
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            &format!("Model: {}", settings.selected_model_label()),
            content_width,
        ),
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(levels.iter().enumerate().map(|(index, level)| {
        let active = index == settings.thinking_cursor;
        let selected = *level == settings.selected_thinking_level;
        let marker = selection_marker(active, selected);
        let style = item_style(active, selected, theme);
        Line::styled(
            truncate_with_ellipsis(
                &format!("{marker} {}", thinking_label(*level)),
                content_width,
            ),
            style,
        )
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
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis("Enter cycles: Full → Truncate → Collapse", content_width),
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    lines.extend(items.iter().enumerate().map(|(index, (label, mode))| {
        let active = index == settings.collapse_cursor;
        let marker = arrow_marker(active);
        Line::styled(
            truncate_with_ellipsis(
                &format!("{marker} {label}: {}", collapse_mode_label(*mode)),
                content_width,
            ),
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
        (ChatStyle::Agentic, "Activity-focused transcript"),
        (ChatStyle::Minimal, "Compact transcript for small terminals"),
    ];
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            "Changing style re-renders the current transcript immediately.",
            content_width,
        ),
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
                    truncate_with_ellipsis(
                        &format!(
                            "{marker} {} ({}) — {description}",
                            chat_style_label(*style),
                            chat_style_value(*style)
                        ),
                        content_width,
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
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            "Project controls this workspace. Global is the default copied into new projects.",
            content_width,
        ),
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
                        truncate_with_ellipsis(
                            &format!("{marker} {}", tool.label()),
                            content_width,
                        ),
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

fn render_auth_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let title = if settings.auth_items.is_empty() {
        " Auth & Providers ".to_string()
    } else {
        format!(
            " Auth & Providers {}/{} ",
            settings
                .auth_cursor
                .saturating_add(1)
                .min(settings.auth_items.len()),
            settings.auth_items.len()
        )
    };
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            "Extension auth/runtime readiness. Recommended: /router setup. Built-in provider auth commands have been removed.",
            content_width,
        ),
        Style::default().fg(theme.muted),
    )];
    lines.push(Line::from(""));
    if settings.auth_items.is_empty() {
        lines.push(Line::styled(
            "No provider auth status loaded.",
            Style::default().fg(theme.muted),
        ));
    } else {
        let visible_height = list_content_height(area).saturating_sub(2).max(1);
        let range = visible_range(
            settings.auth_cursor,
            settings.auth_items.len(),
            visible_height,
        );
        lines.extend(
            settings
                .auth_items
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end.saturating_sub(range.start))
                .map(|(index, item)| {
                    let active = index == settings.auth_cursor;
                    let marker = arrow_marker(active);
                    let setup = item
                        .setup_url
                        .as_ref()
                        .map_or(String::new(), |url| format!(" • setup: {url}"));
                    Line::styled(
                        truncate_with_ellipsis(
                            &format!("{marker} {}{}", item.label(), setup),
                            content_width,
                        ),
                        item_style(active, item.current, theme),
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

fn render_notify_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    use crate::settings::{NotifyField, NotifySettingsState};

    let content_width = area.width.saturating_sub(2) as usize;
    let active_scope = settings.notify.scope;
    let scope = settings.notify.scope_settings(active_scope);
    let title = format!(" Notify • editing {} ", active_scope.label());
    let effective_server = settings
        .notify
        .effective_text(NotifyField::Server)
        .unwrap_or_else(|| "https://ntfy.sh".into());
    let effective_topic = settings
        .notify
        .effective_text(NotifyField::Topic)
        .unwrap_or_else(|| "<topic required>".into());
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            &format!(
                "Effective: {} • {}/{}",
                if settings.notify.effective_enabled() {
                    "ON"
                } else {
                    "OFF"
                },
                effective_server,
                effective_topic
            ),
            content_width,
        ),
        theme.title,
    )];
    if !settings.notify.available {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                "Install builtin:notify from /extensions to activate host notification hooks.",
                content_width,
            ),
            settings_muted_style(theme),
        ));
    } else {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                "Enter edits/toggles selected row • p project • g global • x clears the scoped value",
                content_width,
            ),
            settings_muted_style(theme),
        ));
    }
    if let Some(edit) = &settings.notify.edit {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!(
                    "Editing {} {}: {}█",
                    edit.scope.label(),
                    edit.field.label(),
                    edit.input
                ),
                content_width,
            ),
            Style::default().fg(theme.focused_border),
        ));
    } else {
        lines.push(Line::styled(
            truncate_with_ellipsis(
                &format!(
                    "Scoped {} values below; blank project values inherit global.",
                    active_scope.label()
                ),
                content_width,
            ),
            settings_muted_style(theme),
        ));
    }
    lines.push(Line::from(""));

    let visible_height = list_content_height(area).saturating_sub(lines.len()).max(1);
    let range = visible_range(
        settings.notify.cursor,
        NotifySettingsState::ROWS.len(),
        visible_height,
    );
    lines.extend(
        NotifySettingsState::ROWS
            .iter()
            .enumerate()
            .skip(range.start)
            .take(range.end.saturating_sub(range.start))
            .map(|(index, row)| {
                let active = index == settings.notify.cursor;
                notify_settings_row_line(*row, scope, active, content_width, theme)
            }),
    );

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

fn notify_settings_row_line(
    row: crate::settings::NotifyRow,
    scope: &crate::settings::NotifyScopeSettings,
    active: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    use crate::settings::{NotifyEventKind, NotifyField, NotifyRow};

    let value = match row {
        NotifyRow::Enabled => scope
            .enabled
            .map(|enabled| if enabled { "ON" } else { "OFF" }.to_string())
            .unwrap_or_else(|| "inherit".into()),
        NotifyRow::Server => notify_row_text(scope.server.as_deref(), "inherit https://ntfy.sh"),
        NotifyRow::Topic => notify_row_text(scope.topic.as_deref(), "inherit / missing"),
        NotifyRow::Token => scope
            .token
            .as_ref()
            .filter(|token| !token.trim().is_empty())
            .map(|_| "••••••".into())
            .unwrap_or_else(|| "inherit / none".into()),
        NotifyRow::Priority => notify_row_text(scope.priority.as_deref(), "inherit / default"),
        NotifyRow::Tags => scope
            .tags
            .as_ref()
            .filter(|tags| !tags.is_empty())
            .map(|tags| tags.join(","))
            .unwrap_or_else(|| "inherit / none".into()),
        NotifyRow::AgentEnd => notify_event_row_value(scope, NotifyEventKind::AgentEnd),
        NotifyRow::ToolError => notify_event_row_value(scope, NotifyEventKind::ToolError),
        NotifyRow::SummaryEnabled => scope
            .summary_enabled
            .map(|enabled| if enabled { "ON" } else { "OFF" }.to_string())
            .unwrap_or_else(|| "inherit / ON".into()),
        NotifyRow::SummaryModel => {
            notify_row_text(scope.summary_model.as_deref(), "inherit / heuristic")
        }
        NotifyRow::SummaryPrompt => {
            notify_row_text(scope.summary_prompt.as_deref(), "inherit default prompt")
        }
        NotifyRow::SummaryMaxChars => scope
            .summary_max_chars
            .map(|value| value.to_string())
            .unwrap_or_else(|| "inherit / 280".into()),
    };
    let marker = arrow_marker(active);
    let text = truncate_with_ellipsis(&format!("{marker} {}: {value}", row.label()), width);
    let selected = match row {
        NotifyRow::Enabled => scope.enabled.unwrap_or(false),
        NotifyRow::AgentEnd => scope
            .events
            .as_ref()
            .is_some_and(|events| events.contains(&NotifyEventKind::AgentEnd)),
        NotifyRow::ToolError => scope
            .events
            .as_ref()
            .is_some_and(|events| events.contains(&NotifyEventKind::ToolError)),
        NotifyRow::SummaryEnabled => scope.summary_enabled.unwrap_or(true),
        _ => row
            .field()
            .and_then(|field| match field {
                NotifyField::Server => scope.server.as_ref(),
                NotifyField::Topic => scope.topic.as_ref(),
                NotifyField::Token => scope.token.as_ref(),
                NotifyField::Priority => scope.priority.as_ref(),
                NotifyField::Tags => None,
                NotifyField::SummaryModel => scope.summary_model.as_ref(),
                NotifyField::SummaryPrompt => scope.summary_prompt.as_ref(),
                NotifyField::SummaryMaxChars => None,
            })
            .is_some(),
    };
    Line::styled(text, item_style(active, selected, theme))
}

fn notify_row_text(value: Option<&str>, fallback: &str) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| fallback.into())
}

fn notify_event_row_value(
    scope: &crate::settings::NotifyScopeSettings,
    event: crate::settings::NotifyEventKind,
) -> String {
    scope.events.as_ref().map_or_else(
        || "inherit / ON".into(),
        |events| {
            if events.contains(&event) {
                "ON".into()
            } else {
                "OFF".into()
            }
        },
    )
}

fn render_theme_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    settings: &SettingsState,
    theme: &Theme,
) {
    let title = if settings.theme_options.is_empty() {
        " Theme ".to_string()
    } else {
        format!(
            " Theme {}/{} ",
            settings
                .theme_cursor
                .saturating_add(1)
                .min(settings.theme_options.len()),
            settings.theme_options.len()
        )
    };
    let content_width = area.width.saturating_sub(2) as usize;
    let effective = settings.active_or_preview_theme().map_or_else(
        || "Effective: system".to_string(),
        |theme| {
            let label = if settings.preview_theme.is_some() {
                "Preview"
            } else {
                "Effective"
            };
            format!(
                "{label}: {} ({}, {})",
                theme.display_name,
                theme.selected_scope.label(),
                theme.source.label()
            )
        },
    );
    let global = settings
        .global_theme
        .active_id()
        .unwrap_or_else(|| "system".into());
    let project = settings
        .project_theme
        .active_id()
        .unwrap_or_else(|| "inherits global".into());
    let mut lines = Vec::new();
    lines.push(Line::styled(
        truncate_with_ellipsis(&effective, content_width),
        theme.title,
    ));
    lines.push(Line::styled(
        truncate_with_ellipsis(
            &format!("Global: {global} • Project: {project}"),
            content_width,
        ),
        Style::default().fg(theme.muted),
    ));
    lines.push(Line::from(truncate_spans_to_width(
        vec![
            Span::styled("Preview: ", theme.title),
            Span::styled("user", Style::default().fg(theme.user_border)),
            Span::styled(" • ", Style::default().fg(theme.muted)),
            Span::styled("assistant", Style::default().fg(theme.assistant_border)),
            Span::styled(" • ", Style::default().fg(theme.muted)),
            Span::styled("tool", Style::default().fg(theme.tool_border)),
            Span::styled(" • ", Style::default().fg(theme.muted)),
            Span::styled("working", theme.working),
            Span::styled(" • ", Style::default().fg(theme.muted)),
            Span::styled("error", theme.error),
        ],
        content_width,
    )));
    lines.push(Line::from(truncate_spans_to_width(
        vec![
            Span::styled("Selected row ", item_style(true, false, theme)),
            Span::styled("normal text ", Style::default().fg(theme.fg)),
            Span::styled("muted ", Style::default().fg(theme.muted)),
            Span::styled("focused border", Style::default().fg(theme.focused_border)),
        ],
        content_width,
    )));
    lines.push(Line::styled(
        truncate_with_ellipsis(
            "Enter preview • p set project • g set global • r reset project • R reset global",
            content_width,
        ),
        theme.footer,
    ));
    lines.push(Line::from(""));
    if settings.theme_options.is_empty() {
        lines.push(Line::styled(
            "No themes registered.",
            Style::default().fg(theme.muted),
        ));
    } else {
        let visible_height = list_content_height(area).saturating_sub(6).max(1);
        let range = visible_range(
            settings.theme_cursor,
            settings.theme_options.len(),
            visible_height,
        );
        lines.extend(
            settings
                .theme_options
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end.saturating_sub(range.start))
                .map(|(index, option)| {
                    let active = index == settings.theme_cursor;
                    let preview = settings.preview_theme_id() == Some(option.id.as_str());
                    let selected = option.effective || preview;
                    let marker = selection_marker(active, selected);
                    let mut badges = Vec::new();
                    if option.project_active {
                        badges.push("PROJECT");
                    }
                    if option.global_active {
                        badges.push("GLOBAL");
                    }
                    if option.effective {
                        badges.push("EFFECTIVE");
                    }
                    if preview {
                        badges.push("PREVIEW");
                    }
                    let badges = if badges.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", badges.join(" "))
                    };
                    let description = if option.description.trim().is_empty() {
                        option.source.label().to_string()
                    } else {
                        format!("{} • {}", option.source.label(), option.description)
                    };
                    Line::styled(
                        truncate_with_ellipsis(
                            &format!(
                                "{marker} {} ({}){badges} — {description}",
                                option.display_name,
                                option.mode.label()
                            ),
                            content_width,
                        ),
                        item_style(active, selected, theme),
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
    let content_width = area.width.saturating_sub(2) as usize;
    let visible_height = list_content_height(area).saturating_sub(2).max(1);
    let range = visible_range(settings.keymap_cursor, rows.len(), visible_height);
    let mut lines = vec![Line::styled(
        truncate_with_ellipsis(
            &format!(
                "Preset: {} • Chord key: {} • Enter action • g edit chord key • p preset",
                settings.keymap.preset.label(),
                settings.keymap.chord_key
            ),
            content_width,
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
                let text = truncate_with_ellipsis(
                    &format!(
                        "{marker} [{}] {}  —  {}",
                        info.context.label(),
                        info.label,
                        shortcut
                    ),
                    content_width,
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
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::styled(
            truncate_with_ellipsis(&format!("{} ({})", info.label, action.id()), content_width),
            Style::default().fg(theme.focused_border),
        ),
        Line::styled(
            truncate_with_ellipsis(info.description, content_width),
            Style::default().fg(theme.muted),
        ),
        Line::from(""),
    ];
    if bindings.is_empty() {
        lines.push(Line::styled("› Unassigned", theme.warning));
    } else {
        lines.extend(bindings.iter().enumerate().map(|(index, binding)| {
            let active = index == settings.keymap_binding_cursor;
            let marker = selection_marker(active, false);
            Line::styled(
                truncate_with_ellipsis(&format!("{marker} {binding}"), content_width),
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
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::styled(
            truncate_with_ellipsis(
                &format!("Choose shortcut type for {}", action.info().label),
                content_width,
            ),
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
            truncate_with_ellipsis(
                &format!(
                    "{} {} — {}",
                    arrow_marker(active),
                    kind.label(),
                    description
                ),
                content_width,
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
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = vec![
        Line::styled(
            truncate_with_ellipsis(&format!("Assigning {}", action.info().label), content_width),
            Style::default().fg(theme.focused_border),
        ),
        Line::styled(
            truncate_with_ellipsis(&format!("Type: {}", kind.label()), content_width),
            Style::default().fg(theme.muted),
        ),
        Line::styled(
            truncate_with_ellipsis(&format!("Captured: {captured}"), content_width),
            Style::default().fg(theme.fg),
        ),
        Line::from(""),
        Line::styled(truncate_with_ellipsis(prompt, content_width), theme.warning),
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
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = vec![
        Line::styled(
            truncate_with_ellipsis("Set the global chord key", content_width),
            Style::default().fg(theme.focused_border),
        ),
        Line::styled(
            truncate_with_ellipsis(
                &format!("Current: {}", settings.keymap.chord_key),
                content_width,
            ),
            Style::default().fg(theme.muted),
        ),
        Line::from(""),
        Line::styled(
            truncate_with_ellipsis(
                "Press one key event such as Ctrl-X, Alt-Space, or F12.",
                content_width,
            ),
            Style::default().fg(theme.fg),
        ),
        Line::styled(
            truncate_with_ellipsis(
                "Plain text keys are disallowed so normal typing still works. Esc cancels.",
                content_width,
            ),
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
    let content_width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::styled(
            truncate_with_ellipsis(
                "Select a preset. Applying it resets every keybind after confirmation.",
                content_width,
            ),
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
                    truncate_with_ellipsis(
                        &format!("{} {}", selection_marker(active, selected), preset.label()),
                        content_width,
                    ),
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
    let content_width = area.width.saturating_sub(2) as usize;
    let lines = vec![
        Line::styled(
            truncate_with_ellipsis(
                &format!("Reset every keybind to the {} preset?", preset.label()),
                content_width,
            ),
            theme.warning.add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::styled(
            truncate_with_ellipsis("Y confirms • N/Esc cancels", content_width),
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
        SettingsPage::Models if settings.model_selector.search_active => {
            "type to search • arrows move matches • Enter keep search • Esc clear search"
        }
        SettingsPage::Models => "arrows/jk move • / search • Enter apply • Esc/← back",
        SettingsPage::NotifyModelPicker | SettingsPage::CompactionModelPicker => "arrows/jk move • / search • Enter select • Esc/← back",
        SettingsPage::Thinking => "arrows/jk move • Enter apply • Esc/← back • Ctrl-C twice quit",
        SettingsPage::Collapse => "arrows/jk move • Enter/→ cycle • Esc/← back",
        SettingsPage::ChatStyle => "arrows/jk move • Enter apply • Esc/← back",
        SettingsPage::Tools => {
            "arrows/jk move • g toggle global • p/Space/Enter toggle project • Esc/← back"
        }
        SettingsPage::Auth => "arrows/jk move • recommended /router setup • extension readiness only • Esc/← back",
        SettingsPage::Theme => "arrows/jk move • Enter preview • p project • g global • r reset project • R reset global • Esc/← back",
        SettingsPage::Notify if settings.notify.edit.is_some() => "type value • Enter save • Esc cancel edit",
        SettingsPage::Notify => "arrows/jk move • Enter edit/toggle • p project • g global • x clear • Esc/← back",
        SettingsPage::Extensions => "Enter open extension manager • Esc/← back",
        SettingsPage::Compaction => "arrows/jk move • Enter/←→ toggle • Enter on LLM Model row opens model picker • Esc/← back • use /compact commands for threshold/prompt",
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
    } else if settings.page == SettingsPage::Theme {
        if let Some(preview) = &settings.preview_theme {
            format!("Preview: {} • {controls}", preview.display_name)
        } else {
            let effective = settings.effective_theme.as_ref().map_or_else(
                || "system".into(),
                |theme| format!("{} ({})", theme.display_name, theme.selected_scope.label()),
            );
            format!("Theme: {effective} • {controls}")
        }
    } else if settings.page == SettingsPage::Notify {
        format!("Notify ({}) • {controls}", settings.notify.scope.label())
    } else if settings.page == SettingsPage::Extensions {
        format!("Extensions • {controls}")
    } else {
        format!("{} • {controls}", settings.status)
    };
    render_overlay_footer(frame, area, &status, theme.footer);
}

fn render_overlay_footer(frame: &mut Frame<'_>, area: Rect, text: &str, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(style)
            .wrap(Wrap { trim: false }),
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

fn panel_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.panel_bg)
}

fn suggestion_panel_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.suggestion_fg)
        .bg(theme.suggestion_bg)
}

fn suggestion_item_style(active: bool, theme: &Theme) -> Style {
    if active {
        Style::default()
            .fg(theme.suggestion_selected_fg)
            .bg(theme.suggestion_selected_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        suggestion_panel_style(theme)
    }
}

fn suggestion_label_style(active: bool, theme: &Theme) -> Style {
    if active {
        suggestion_item_style(true, theme)
    } else {
        Style::default()
            .fg(theme.suggestion_match)
            .bg(theme.suggestion_bg)
            .add_modifier(Modifier::BOLD)
    }
}

fn suggestion_muted_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.badge_muted)
        .bg(theme.suggestion_bg)
}

fn badge_style(color: Color, theme: &Theme) -> Style {
    Style::default().fg(color).bg(theme.badge_bg)
}

fn diagnostic_style(color: Color, theme: &Theme) -> Style {
    Style::default().fg(color).bg(theme.diagnostic_danger_bg)
}

fn settings_title_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.settings_title)
        .add_modifier(Modifier::BOLD)
}

fn settings_text_style(theme: &Theme) -> Style {
    Style::default().fg(theme.settings_fg).bg(theme.panel_bg)
}

fn settings_muted_style(theme: &Theme) -> Style {
    Style::default().fg(theme.settings_muted).bg(theme.panel_bg)
}

fn settings_active_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.settings_active)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD)
}

fn extension_role_style(color: Color, active: bool, theme: &Theme) -> Style {
    let style = Style::default().fg(color);
    if active {
        style.bg(theme.selection_bg).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn truncate_spans_to_width(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut remaining = width;
    for span in spans {
        if remaining == 0 {
            break;
        }
        let text = span.content.as_ref();
        let text_width = text.width();
        if text_width <= remaining {
            remaining = remaining.saturating_sub(text_width);
            out.push(span);
        } else {
            out.push(Span::styled(
                truncate_with_ellipsis(text, remaining),
                span.style,
            ));
            break;
        }
    }
    out
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
    let mut style = if selected {
        Style::default()
            .fg(theme.selected_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg)
    };
    if active {
        style = style.bg(theme.selection_bg).add_modifier(Modifier::BOLD);
    }
    style
}

fn settings_item_style(active: bool, selected: bool, theme: &Theme) -> Style {
    let mut style = if selected {
        Style::default()
            .fg(theme.settings_changed)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        settings_text_style(theme)
    };
    if active {
        style = settings_active_style(theme);
    }
    style
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
        app::{
            ExtensionManagementItem, ExtensionManagementTarget, OverlayKind, SessionListItem,
            TuiState, UsagePanelProvider, UsagePanelReport, UsagePanelSession,
        },
        message::{MessageView, ToolCallView},
        settings::CollapseMode,
        TuiAction,
    };
    use oino_extension_core::{
        ActiveContribution, ContributionId, ContributionMetadata, ExtensionId, RegistryEntry,
        RegistryEntryKey, SourceDescriptor, SourceKind, SourceScope, UiFocusPolicy,
        UiKeyDispatchPolicy, UiLayoutPolicy, UiSurfaceAction, UiSurfaceStateUpdate,
        UiTinyTerminalFallback, UiVisibilityPolicy,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::json;
    use std::{
        collections::{BTreeMap, BTreeSet},
        error::Error,
        path::PathBuf,
    };

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

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn extension_surface(
        id: &str,
        owner: &str,
        surface: UiSurfaceKind,
        title: &str,
        slot: &str,
        priority: i32,
    ) -> Result<ActiveContribution<UiSurfaceContribution>, Box<dyn Error>> {
        let id = ContributionId::new(id)?;
        let owner = ExtensionId::new(owner)?;
        let mut scopes = BTreeSet::new();
        scopes.insert("extension.surface".into());
        Ok(ActiveContribution {
            effective_id: id.clone(),
            entry: RegistryEntry::new(
                RegistryEntryKey::new(format!("test-{id}")),
                ContributionMetadata::new(
                    id.clone(),
                    SourceDescriptor {
                        scope: SourceScope::Project,
                        kind: SourceKind::LocalPackage,
                        path: Some(PathBuf::from(format!(".oino/extensions/{id}"))),
                        registry: None,
                    },
                )
                .with_extension_id(owner),
                UiSurfaceContribution {
                    id,
                    surface,
                    title: title.into(),
                    state_schema: Some("object".into()),
                    layout: UiLayoutPolicy {
                        slot: slot.into(),
                        priority,
                        min_width: 24,
                        min_height: 8,
                        max_width: Some(32),
                        tiny_terminal: UiTinyTerminalFallback::CompactBadge,
                    },
                    visibility: UiVisibilityPolicy::Visible,
                    focus: UiFocusPolicy::Focusable,
                    key_dispatch: UiKeyDispatchPolicy {
                        scopes,
                        pass_through: false,
                    },
                    conflict: Default::default(),
                },
            ),
        })
    }

    fn extension_surface_with_effective_id(
        effective_id: &str,
        contribution_id: &str,
        owner: &str,
        surface: UiSurfaceKind,
        title: &str,
        slot: &str,
        priority: i32,
    ) -> Result<ActiveContribution<UiSurfaceContribution>, Box<dyn Error>> {
        let mut s = extension_surface(contribution_id, owner, surface, title, slot, priority)?;
        s.effective_id = ContributionId::new(effective_id)?;
        Ok(s)
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
    fn transcript_line_lower_bound_is_conservative() {
        let mut state = TuiState::new();
        state.settings.chat_style = ChatStyle::Chat;
        state.messages = vec![
            MessageView {
                id: oino_types::OinoId::nil(),
                role: "user".into(),
                title: None,
                content: "hello".into(),
                thinking: None,
                thinking_redacted: false,
                tool_call_id: None,
                tool_calls: Vec::new(),
                is_error: false,
            },
            MessageView {
                id: oino_types::OinoId::nil(),
                role: "assistant".into(),
                title: None,
                content: "world".into(),
                thinking: None,
                thinking_redacted: false,
                tool_call_id: None,
                tool_calls: Vec::new(),
                is_error: false,
            },
        ];

        let lower_bound = transcript_line_lower_bound(&state);
        let prepared = prepared_transcript_for_width(&state, 40, &Theme::default());
        assert!(lower_bound <= prepared.total_lines());
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
    fn extension_theme_tokens_apply_through_theme_boundary() {
        let mut state = TuiState::new();
        state.set_extension_theme(crate::app::ExtensionThemeState {
            label: Some("theme.process".into()),
            tokens: BTreeMap::from([
                ("title".into(), "red".into()),
                ("focused_border".into(), "green".into()),
                ("success".into(), "#86efac".into()),
                ("muted".into(), "242".into()),
            ]),
            warnings: Vec::new(),
        });
        let themed = theme_with_extension_tokens(&state, Theme::default());
        assert_eq!(themed.title, Theme::default().title.fg(Color::Red));
        assert_eq!(themed.focused_border, Color::Green);
        assert_eq!(themed.success, Color::Rgb(0x86, 0xef, 0xac));
        assert_eq!(themed.muted, Color::Indexed(242));
    }

    #[test]
    fn extensions_overlay_renders_diagnostics_conflicts_and_enablement() {
        let mut state = TuiState::new();
        state.set_extension_management_items(vec![ExtensionManagementItem {
            target: ExtensionManagementTarget::Contribution,
            id: "ui.processes".into(),
            title: "Proc".into(),
            family: "ui".into(),
            scope: "project".into(),
            health: "Bad".into(),
            state: "Active".into(),
            permission: "ok".into(),
            provenance: "pkg ext".into(),
            diagnostics: vec!["invalid state shape".into()],
            conflicts: vec!["slot conflict".into()],
            entry_key: Some("ui:pkg:ui.processes:/tmp".into()),
            canonical_id: Some("ui.processes".into()),
            global_override: true,
            project_override: false,
            global_enabled: true,
            project_enabled: false,
        }]);
        state
            .extension_management
            .set_view(ExtensionManagementView::Registry);
        state.overlay = Some(OverlayKind::Extensions);
        let buffer = draw_state(120, 30, &state);
        let text = buffer_text(&buffer);
        assert!(text.contains("Extensions"));
        assert!(text.contains("ui.processes"));
        assert!(text.contains("P:OFF G:ON OVR:G"));
        assert!(text.contains("diag"));
        assert!(text.contains("conflict"));
    }

    #[test]
    fn extensions_overlay_ellipsizes_install_input_when_narrow() {
        let mut state = TuiState::new();
        state.overlay = Some(OverlayKind::Extensions);
        state
            .extension_management
            .begin_install(crate::settings::ToolSettingsScope::Project);
        state.extension_management.install_input =
            "https://github.com/example/very-long-extension-package-name-with-extra-tail".into();

        let buffer = draw_state(44, 18, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Install Project package"));
        assert!(text.contains("…"));
        assert!(!text.contains("extra-tail"));
    }

    #[test]
    fn extension_management_rows_stay_width_bounded_when_very_narrow() {
        let mut state = TuiState::new();
        state.set_extension_management_items(vec![ExtensionManagementItem {
            target: ExtensionManagementTarget::Contribution,
            id: "ui.very.long.contribution.identifier.with.tail".into(),
            title: "Very Long Contribution Title That Should Be Ellipsized".into(),
            family: "ui-with-a-long-family-name".into(),
            scope: "project".into(),
            health: "Healthy but verbose".into(),
            state: "Active".into(),
            permission: "many permissions with a long label".into(),
            provenance: "pkg ext".into(),
            diagnostics: vec!["diagnostic with long details".into()],
            conflicts: vec!["conflict with long details".into()],
            entry_key: Some("ui:pkg:ui.very.long.contribution.identifier.with.tail:/tmp".into()),
            canonical_id: Some("ui.very.long.contribution.identifier.with.tail".into()),
            global_override: true,
            project_override: true,
            global_enabled: false,
            project_enabled: true,
        }]);
        state
            .extension_management
            .set_view(ExtensionManagementView::Registry);

        let width = 14;
        let lines = extension_management_lines(&state, width, 8, &Theme::default());
        let plain = line_texts(lines);

        assert!(plain.iter().all(|line| line.width() <= width), "{plain:?}");
        assert!(plain.iter().any(|line| line.contains('…')), "{plain:?}");
        assert!(!plain.join("\n").contains("identifier.with.tail"));
    }

    #[test]
    fn extension_management_lines_use_theme_roles() {
        let mut state = TuiState::new();
        state.set_extension_management_items(vec![ExtensionManagementItem {
            target: ExtensionManagementTarget::Contribution,
            id: "ui.processes".into(),
            title: "Proc".into(),
            family: "ui".into(),
            scope: "project".into(),
            health: "Healthy".into(),
            state: "Active".into(),
            permission: "ok".into(),
            provenance: "pkg ext".into(),
            diagnostics: vec!["invalid state shape".into()],
            conflicts: vec!["slot conflict".into()],
            entry_key: Some("ui:pkg:ui.processes:/tmp".into()),
            canonical_id: Some("ui.processes".into()),
            global_override: true,
            project_override: false,
            global_enabled: true,
            project_enabled: false,
        }]);
        state
            .extension_management
            .set_view(ExtensionManagementView::Registry);
        let theme = Theme {
            extension_enabled: Color::Blue,
            extension_disabled: Color::Red,
            extension_contribution: Color::Green,
            extension_diagnostic: Color::Yellow,
            extension_conflict: Color::Magenta,
            extension_override: Color::Cyan,
            settings_active: Color::White,
            ..Theme::default()
        };

        let lines = extension_management_lines(&state, 120, 10, &theme);
        let row = lines
            .iter()
            .find(|line| plain_line(line).contains("ui.processes"))
            .unwrap_or_else(|| panic!("missing extension row"));

        assert!(row
            .spans
            .iter()
            .any(|span| span.content.as_ref() == "OFF" && span.style.fg == Some(Color::Red)));
        assert!(row
            .spans
            .iter()
            .any(|span| span.content.as_ref() == "ON" && span.style.fg == Some(Color::Blue)));
        assert!(row.spans.iter().any(|span| {
            span.content.as_ref().contains("project ui") && span.style.fg == Some(Color::Green)
        }));
        assert!(row.spans.iter().any(|span| {
            span.content.as_ref().contains("OVR:G") && span.style.fg == Some(Color::Cyan)
        }));
        assert!(row.spans.iter().any(|span| {
            span.content.as_ref().contains("diag") && span.style.fg == Some(Color::Yellow)
        }));
        assert!(row.spans.iter().any(|span| {
            span.content.as_ref().contains("conflict") && span.style.fg == Some(Color::Magenta)
        }));
    }

    #[test]
    fn settings_role_helpers_use_settings_theme_fields() {
        let theme = Theme {
            settings_title: Color::Blue,
            settings_fg: Color::Green,
            settings_muted: Color::Yellow,
            settings_active: Color::Magenta,
            settings_changed: Color::Cyan,
            ..Theme::default()
        };

        assert_eq!(settings_title_style(&theme).fg, Some(Color::Blue));
        assert_eq!(settings_text_style(&theme).fg, Some(Color::Green));
        assert_eq!(settings_muted_style(&theme).fg, Some(Color::Yellow));
        assert_eq!(
            settings_item_style(true, false, &theme).fg,
            Some(Color::Magenta)
        );
        assert_eq!(
            settings_item_style(false, true, &theme).fg,
            Some(Color::Cyan)
        );
    }

    #[test]
    fn registry_backed_extension_surfaces_render_from_state() -> Result<(), Box<dyn Error>> {
        let mut state = TuiState::new();
        state.set_extension_ui_surfaces(vec![
            extension_surface(
                "ui.processes",
                "process-manager",
                UiSurfaceKind::Sidebar,
                "Process Manager",
                "sidebar:right",
                30,
            )?,
            extension_surface(
                "ui.status",
                "process-manager",
                UiSurfaceKind::Footer,
                "Background Jobs",
                "footer:status",
                20,
            )?,
            extension_surface(
                "ui.float",
                "process-manager",
                UiSurfaceKind::FloatingPanel,
                "Process Details",
                "floating:center",
                10,
            )?,
            extension_surface(
                "ui.suggest",
                "process-manager",
                UiSurfaceKind::Autosuggest,
                "Process Suggestions",
                "autosuggest:provider",
                5,
            )?,
        ]);
        state.apply_extension_ui_update(UiSurfaceStateUpdate {
            surface_id: ContributionId::new("ui.processes")?,
            owner_extension_id: ExtensionId::new("process-manager")?,
            state: json!({ "summary": "2 running" }),
            actions: vec![UiSurfaceAction {
                id: "stop".into(),
                label: "Stop selected".into(),
                key_scope: Some("extension.surface".into()),
            }],
        })?;
        assert!(state.focus_extension_surface(&ContributionId::new("ui.processes")?));
        assert_eq!(
            state.extension_key_action_for_scope("extension.surface"),
            Some(TuiAction::RunExtensionUiAction {
                surface_id: "ui.processes".into(),
                action_id: "stop".into(),
            })
        );
        state.apply_extension_ui_update(UiSurfaceStateUpdate {
            surface_id: ContributionId::new("ui.status")?,
            owner_extension_id: ExtensionId::new("process-manager")?,
            state: json!({ "summary": "jobs healthy" }),
            actions: Vec::new(),
        })?;

        let buffer = draw_state(100, 30, &state);
        let text = buffer_text(&buffer);
        assert!(text.contains("Process Manager"));
        assert!(text.contains("2 running"));
        assert!(text.contains("FOCUS"));
        assert!(text.contains("Extension Status"));
        assert!(text.contains("jobs healthy"));
        assert!(text.contains("Extension Panel"));
        assert!(text.contains("Process Details"));
        assert!(text.contains("Process Suggestions"));
        Ok(())
    }

    #[test]
    fn floating_panel_top_right_slot_anchors_to_top_right() -> Result<(), Box<dyn Error>> {
        let mut state = TuiState::new();
        state.set_extension_ui_surfaces(vec![extension_surface(
            "ui.float",
            "process-manager",
            UiSurfaceKind::FloatingPanel,
            "Process Details",
            "floating:top-right",
            10,
        )?]);

        let buffer = draw_state(80, 24, &state);
        let width = buffer.area.width as usize;
        let row = buffer
            .content()
            .iter()
            .take(width)
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(row.trim_start().starts_with('┌'));
        assert!(row.contains("Extension Panel"));
        assert!(row.trim_end().ends_with('┐'));
        Ok(())
    }

    #[test]
    fn registry_backed_extension_surfaces_show_tiny_fallbacks() -> Result<(), Box<dyn Error>> {
        let mut state = TuiState::new();
        state.set_extension_ui_surfaces(vec![extension_surface(
            "ui.tiny",
            "tiny-extension",
            UiSurfaceKind::Status,
            "Tiny Status",
            "status:footer",
            0,
        )?]);
        let buffer = draw_state(18, 6, &state);
        let text = buffer_text(&buffer);
        assert!(text.contains("Oino needs"));
        assert!(text.contains("Ext:"));
        assert!(text.contains("Tiny"));
        Ok(())
    }

    #[test]
    fn model_selector_browse_rows_show_pricing_and_search_hides_pricing() {
        let mut state =
            TuiState::with_settings("router:openai/gpt-4o", oino_types::ThinkingLevel::Off);
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.open_model_selection();
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new("router:openai/gpt-4o")
                .with_display_name("GPT-4o")
                .with_provider_label("OmniRoute/openai")
                .with_pricing(Some(crate::settings::ModelPricing {
                    input_per_token: Some("0.0000025".into()),
                    output_per_token: Some("0.00001".into()),
                    cache_hit_per_token: Some("0.00000125".into()),
                    cache_write_per_token: None,
                    source: "openrouter".into(),
                }))],
            "loaded",
        );

        let text = buffer_text(&draw_state(140, 24, &state));
        assert!(text.contains("$2.50/M"));
        assert!(text.contains("$10.00/M"));
        assert!(text.contains("$1.25/M"));
        assert!(text.contains("src:openrouter"));

        state.settings.model_selector.search_active = true;
        let text = buffer_text(&draw_state(140, 24, &state));
        assert!(text.contains("[OmniRoute/openai] GPT-4o"));
        assert!(!text.contains("$2.50/M"));

        state.settings.model_selector.search_active = false;
        state.settings.model_selector.show_pricing = false;
        let text = buffer_text(&draw_state(140, 24, &state));
        assert!(text.contains("[OmniRoute/openai] GPT-4o"));
        assert!(!text.contains("$2.50/M"));
    }

    #[test]
    fn footer_status_surfaces_render_directly_around_composer() -> Result<(), Box<dyn Error>> {
        let mut state =
            TuiState::with_settings("openrouter:test/model", oino_types::ThinkingLevel::High);
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new("openrouter:test/model")
                .with_display_name("Test Model")
                .with_thinking_levels(crate::settings::all_thinking_levels())
                .with_context_length(Some(100_000))],
            "loaded",
        );
        state.set_working_directory("/repo/oino");
        state.set_git_branch(Some("main".into()));
        state.set_context_tokens(Some(10_000));
        state.set_usage_report(crate::app::UsagePanelReport {
            generated_at_unix: 1,
            status_line: "Usage: 1 reported turn(s), 3 tokens, 0.0004 USD".into(),
            session: crate::app::UsagePanelSession {
                reported_turns: 1,
                input_tokens: 10,
                cache_read_tokens: 5,
                total_tokens: 15,
                costs: vec!["0.0004 USD".into()],
                ..Default::default()
            },
            providers: Vec::new(),
        });
        state.set_extension_ui_surfaces(vec![
            extension_surface(
                FOOTER_STATUS_TOP_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterTop,
                "Footer Status Top",
                COMPOSER_DIRECT_TOP_SLOT,
                100,
            )?,
            extension_surface(
                FOOTER_STATUS_BOTTOM_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterBottom,
                "Footer Status Bottom",
                COMPOSER_DIRECT_BOTTOM_SLOT,
                100,
            )?,
        ]);

        let layout = app_layout(
            &state,
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 24,
            },
        );
        let Some(composer_top) = layout.composer_top else {
            panic!("footer-status top line should reserve a direct composer-top row");
        };
        let Some(composer_bottom) = layout.composer_bottom else {
            panic!("footer-status bottom line should reserve a direct composer-bottom row");
        };
        assert_eq!(composer_top.y + 1, layout.composer.y);
        assert_eq!(
            layout.composer.y + layout.composer.height,
            composer_bottom.y
        );

        let buffer = draw_state(100, 24, &state);
        let text = buffer_text(&buffer);
        assert!(text.contains("model: Test Model"));
        assert!(text.contains("thinking: High"));
        assert!(text.contains("cost: 0.0004 USD"));
        assert!(text.contains("cache hit: 50.0%"));
        assert!(text.contains("cwd: /repo/oino"));
        assert!(text.contains("branch: main"));
        assert!(text.contains("context: 10%/100k"));
        assert!(!text.contains("Extension Status"));
        assert_eq!(transcript_visible_lines(&state, 100, 24), 15);
        Ok(())
    }

    #[test]
    fn footer_status_without_surfaces_keeps_normal_composer_layout() {
        let mut state =
            TuiState::with_settings("openrouter:test/model", oino_types::ThinkingLevel::High);
        state.set_working_directory("/repo/oino");
        state.set_context_tokens(Some(10_000));

        let layout = app_layout(
            &state,
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 20,
            },
        );
        assert!(layout.composer_top.is_none());
        assert!(layout.composer_bottom.is_none());
        assert_eq!(layout.composer.y + layout.composer.height, 20);

        let text = buffer_text(&draw_state(80, 20, &state));
        assert!(!text.contains("model: openrouter:test/model"));
        assert!(!text.contains("cwd: /repo/oino"));
    }

    #[test]
    fn footer_status_deduplicates_same_contribution_from_global_and_project(
    ) -> Result<(), Box<dyn Error>> {
        // When the same footer-status package is installed in both global and project
        // locations, the namespaced conflict strategy gives them different effective_ids
        // but the same contribution id. The renderer must deduplicate by contribution id
        // to avoid showing the footer twice.
        let mut state =
            TuiState::with_settings("openrouter:test/model", oino_types::ThinkingLevel::Off);
        state.set_working_directory("/repo/oino");
        state.set_context_tokens(Some(1_200));
        state.set_extension_ui_surfaces(vec![
            extension_surface(
                FOOTER_STATUS_TOP_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterTop,
                "Footer Status Top",
                COMPOSER_DIRECT_TOP_SLOT,
                100,
            )?,
            // Simulate the namespaced duplicate (different effective_id, same contribution id)
            extension_surface_with_effective_id(
                "oino.footer_status.footer_status_top",
                FOOTER_STATUS_TOP_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterTop,
                "Footer Status Top",
                COMPOSER_DIRECT_TOP_SLOT,
                100,
            )?,
            extension_surface(
                FOOTER_STATUS_BOTTOM_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterBottom,
                "Footer Status Bottom",
                COMPOSER_DIRECT_BOTTOM_SLOT,
                100,
            )?,
        ]);

        let layout = app_layout(
            &state,
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
        );
        // Only one composer-top row despite two entries for footer_status_top
        assert!(layout.composer_top.is_some());
        assert!(layout.composer_bottom.is_some());

        let buffer = draw_state(80, 24, &state);
        let text = buffer_text(&buffer);
        // The model line should appear exactly once, not duplicated
        let model_count = text.matches("model: openrouter:test/model").count();
        assert_eq!(
            model_count, 1,
            "footer status line should appear exactly once, but found {model_count} occurrences"
        );
        assert!(!text.contains("Extension Status"));
        Ok(())
    }

    #[test]
    fn footer_status_lines_ellipsize_in_narrow_terminals() -> Result<(), Box<dyn Error>> {
        let mut state =
            TuiState::with_settings("openrouter:test/model", oino_types::ThinkingLevel::High);
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new("openrouter:test/model")
                .with_display_name("A Very Long Model Display Name")
                .with_thinking_levels(crate::settings::all_thinking_levels())
                .with_context_length(Some(1_000_000))],
            "loaded",
        );
        state.set_working_directory("/very/long/path/to/the/current/oino/project");
        state.set_context_tokens(Some(543_210));
        state.set_extension_ui_surfaces(vec![
            extension_surface(
                FOOTER_STATUS_TOP_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterTop,
                "Footer Status Top",
                COMPOSER_DIRECT_TOP_SLOT,
                100,
            )?,
            extension_surface(
                FOOTER_STATUS_BOTTOM_ID,
                "oino.footer_status",
                UiSurfaceKind::FooterBottom,
                "Footer Status Bottom",
                COMPOSER_DIRECT_BOTTOM_SLOT,
                100,
            )?,
        ]);

        let text = buffer_text(&draw_state(30, 12, &state));
        assert!(text.contains("model: A Very Long"));
        assert!(text.contains("cwd: /very/long"));
        assert!(text.contains('…'));
        assert_eq!(transcript_visible_lines(&state, 30, 12), 3);
        Ok(())
    }

    #[test]
    fn registry_backed_extension_surfaces_show_conflict_badges() -> Result<(), Box<dyn Error>> {
        let mut state = TuiState::new();
        state.set_extension_ui_surfaces(vec![
            extension_surface(
                "ui.first",
                "first-extension",
                UiSurfaceKind::Footer,
                "First Footer",
                "footer:status",
                1,
            )?,
            extension_surface(
                "ui.second",
                "second-extension",
                UiSurfaceKind::Footer,
                "Second Footer",
                "footer:status",
                1,
            )?,
        ]);
        let buffer = draw_state(80, 24, &state);
        let text = buffer_text(&buffer);
        assert!(text.contains("conflict"));
        assert!(text.contains("First Footer"));
        assert!(text.contains("Second Footer"));
        Ok(())
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
        assert!(!text.contains("Writing"));
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
        let buffer = draw_state(90, 32, &state);
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
        assert!(text.contains("q send"));
        assert!(text.contains("Esc cancel"));
    }

    #[test]
    fn command_skill_label_uses_distinct_color() {
        let theme = Theme::default();
        let skill = command_category_style(CommandSuggestionCategory::Skill, &theme);

        assert_eq!(
            skill,
            Style::default()
                .fg(theme.badge_error)
                .bg(theme.badge_bg)
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
    fn suggestion_badge_and_diagnostic_roles_use_theme_fields() {
        let theme = Theme {
            suggestion_fg: Color::Blue,
            suggestion_bg: Color::Red,
            suggestion_match: Color::Green,
            suggestion_selected_fg: Color::Yellow,
            suggestion_selected_bg: Color::Magenta,
            badge_error: Color::Cyan,
            badge_bg: Color::White,
            diagnostic_error: Color::LightRed,
            diagnostic_danger_bg: Color::DarkGray,
            ..Theme::default()
        };

        assert_eq!(suggestion_panel_style(&theme).fg, Some(Color::Blue));
        assert_eq!(suggestion_panel_style(&theme).bg, Some(Color::Red));
        assert_eq!(suggestion_item_style(true, &theme).fg, Some(Color::Yellow));
        assert_eq!(suggestion_item_style(true, &theme).bg, Some(Color::Magenta));
        assert_eq!(suggestion_label_style(false, &theme).fg, Some(Color::Green));
        assert_eq!(
            command_category_style(CommandSuggestionCategory::Skill, &theme).fg,
            Some(Color::Cyan)
        );
        assert_eq!(
            command_category_style(CommandSuggestionCategory::Skill, &theme).bg,
            Some(Color::White)
        );
        assert_eq!(
            diagnostic_style(theme.diagnostic_error, &theme).fg,
            Some(Color::LightRed)
        );
        assert_eq!(
            diagnostic_style(theme.diagnostic_error, &theme).bg,
            Some(Color::DarkGray)
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
    fn command_suggestion_lines_stay_width_bounded_when_narrow() {
        let view = CommandSuggestionsView {
            query: "longmodel".into(),
            title: "Models".into(),
            items: vec![crate::command::CommandSuggestionItem {
                label: "openrouter:example/longmodel-free-with-a-long-tail".into(),
                summary: "Display name with another long tail".into(),
                replacement: "openrouter:example/longmodel-free-with-a-long-tail".into(),
                replace_start: 7,
                replace_end: 15,
                complete_on_enter: true,
                category: CommandSuggestionCategory::Model,
            }],
            selected: 0,
        };
        let width = 14;
        let lines = line_texts(command_suggestion_lines(&view, 4, width, &Theme::default()));

        assert!(lines.iter().all(|line| line.width() <= width), "{lines:?}");
        assert!(lines[0].contains('…'), "{}", lines[0]);
        assert!(!lines[0].contains("long-tail"), "{}", lines[0]);
    }

    #[test]
    fn render_command_suggestions_ellipsizes_long_rows_when_narrow() {
        let mut state = TuiState::new();
        state.composer.replace_text("/model ");
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new(
                "openrouter:example/longmodel-free-with-a-long-tail",
            )],
            "loaded",
        );

        let buffer = draw_state(20, 12, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Models"));
        assert!(text.contains("…"));
        assert!(!text.contains("long-tail"));
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
    fn render_usage_overlay_shows_session_and_provider_details() {
        let mut state = TuiState::new();
        state.overlay = Some(OverlayKind::Usage);
        state.set_usage_report(UsagePanelReport {
            generated_at_unix: 123,
            status_line: "Usage: 1 reported turn, 42 tokens".into(),
            session: UsagePanelSession {
                assistant_turns: 1,
                reported_turns: 1,
                input_tokens: 20,
                output_tokens: 22,
                total_tokens: 42,
                costs: vec!["0.0042 USD".into()],
                ..UsagePanelSession::default()
            },
            providers: vec![UsagePanelProvider {
                provider_id: "openrouter".into(),
                display_name: "OpenRouter".into(),
                status: "available".into(),
                message: "available: 1 reported turn, 42 tokens".into(),
                assistant_turns: 1,
                reported_turns: 1,
                total_tokens: 42,
                costs: vec!["0.0042 USD".into()],
                account_source: Some("fixture • refreshed at 123".into()),
                account_balance: Some("1.5000 USD".into()),
                account_limits: vec!["daily tokens: 25.00/100.00 tokens".into()],
            }],
        });

        let buffer = draw_state(90, 32, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Usage"));
        assert!(text.contains("OpenRouter"));
        assert!(text.contains("42 tokens"));
        assert!(text.contains("0.0042 USD"));
        assert!(text.contains("Account usage"));
        assert!(text.contains("1.5000 USD"));
        assert!(text.contains("daily tokens"));
        assert!(text.contains("r refresh"));
    }

    #[test]
    fn render_usage_overlay_ellipsizes_rows_when_narrow() {
        let mut state = TuiState::new();
        state.overlay = Some(OverlayKind::Usage);
        state.set_usage_report(UsagePanelReport {
            generated_at_unix: 123,
            status_line: "Usage: one extremely long status line that should be clipped".into(),
            session: UsagePanelSession::default(),
            providers: vec![UsagePanelProvider {
                provider_id: "provider-with-a-very-long-tail".into(),
                display_name: "Provider Name With A Very Long Tail".into(),
                status: "not configured".into(),
                message: "message with details that should not run through the border".into(),
                ..UsagePanelProvider::default()
            }],
        });

        let buffer = draw_state(44, 16, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Usage"));
        assert!(text.contains("…"));
        assert!(!text.contains("very-long-tail"));
    }

    #[test]
    fn render_inspect_overlay_ellipsizes_export_message_when_narrow() {
        let mut state = TuiState::new();
        state.overlay = Some(OverlayKind::Inspect);
        state.set_inspect_full_prompt("short prompt", 12_345);
        state.set_inspect_export_message(
            "Exported to /tmp/very/deep/export/path/chat-with-a-long-file-name-extra-tail.html",
        );

        let buffer = draw_state(44, 18, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Full prompt"));
        assert!(text.contains("Exported to"));
        assert!(text.contains("…"));
        assert!(!text.contains("extra-tail"));
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
    fn render_help_overlay_footer_wraps_when_narrow() {
        let mut state = TuiState::new();
        state.open_help();

        let buffer = draw_state(44, 16, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("PgUp/PgDn page"));
        assert!(text.contains("Home/End jump"));
    }

    #[test]
    fn render_help_overlay_ellipsizes_long_search_when_narrow() {
        let mut state = TuiState::new();
        state.open_help();
        state.help_search_active = true;
        state.help_search = "help-search-query-with-a-very-long-tail".into();

        let buffer = draw_state(44, 18, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Search:"));
        assert!(text.contains("…"));
        assert!(!text.contains("very-long-tail"));
    }

    #[test]
    fn help_entry_line_bounds_long_key_hint_when_narrow() {
        let line = help_entry_line(
            &HelpEntry::Item(
                "Ctrl-Shift-Alt-Super-Enter-Or-A-Very-Long-Shortcut".into(),
                "description with a long tail that should not leak".into(),
            ),
            12,
            &Theme::default(),
        );
        let plain = plain_line(&line);

        assert!(plain.width() <= 12, "{plain}");
        assert!(plain.contains('…'), "{plain}");
        assert!(!plain.contains("Shortcut"), "{plain}");
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
    fn send_panel_lines_stay_width_bounded_when_narrow() {
        let mut state = TuiState::new();
        state
            .composer
            .replace_text("current composer input with a very long tail that should disappear");
        state
            .steer_items
            .push("steer request with a very long tail that should disappear".into());
        state
            .queued_items
            .push("queued request with a very long tail that should disappear".into());
        state
            .draft_items
            .push("draft request with a very long tail that should disappear".into());

        let width = 10;
        let lines = line_texts(send_panel_lines(&state, width, 20, &Theme::default()));
        let joined = lines.join("\n");

        assert!(lines.iter().all(|line| line.width() <= width), "{lines:?}");
        assert!(lines.iter().any(|line| line.contains('…')), "{lines:?}");
        assert!(!joined.contains("disappear"));
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
    fn browser_search_rows_stay_width_bounded_when_narrow() {
        let theme = Theme::default();
        let width = 14;

        let mut state = TuiState::new();
        state.sessions.search_active = true;
        state.sessions.search = "session-search-with-a-very-long-tail".into();
        let session_lines = line_texts(sessions_lines(&state, width, 5, &theme));
        assert!(
            session_lines.iter().all(|line| line.width() <= width),
            "{session_lines:?}"
        );
        assert!(session_lines.iter().any(|line| line.contains('…')));

        state.prompts.search_active = true;
        state.prompts.search = "prompt-search-with-a-very-long-tail".into();
        let prompt_lines = line_texts(prompts_lines(&state, width, 5, &theme));
        assert!(
            prompt_lines.iter().all(|line| line.width() <= width),
            "{prompt_lines:?}"
        );
        assert!(prompt_lines.iter().any(|line| line.contains('…')));

        state.skills.search = "skill-filter-with-a-very-long-tail".into();
        let skill_lines = line_texts(skills_lines(&state, width, 5, &theme));
        assert!(
            skill_lines.iter().all(|line| line.width() <= width),
            "{skill_lines:?}"
        );
        assert!(skill_lines.iter().any(|line| line.contains('…')));
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
    fn render_theme_settings_ellipsizes_long_rows_when_narrow() {
        let mut state = TuiState::new();
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.page = crate::settings::SettingsPage::Theme;
        state
            .settings
            .global_theme
            .set_active("global-theme-with-a-name-that-would-run-through-the-settings-panel");
        state
            .settings
            .project_theme
            .set_active("project-theme-with-a-name-that-would-run-through-the-settings-panel");
        state.settings.theme_options = vec![crate::settings::ThemeOption {
            id: "very-long-theme".into(),
            display_name: "Long Theme Name With Far Too Many Words".into(),
            description: "description with a lot of extra detail that should not fit in a narrow settings overlay".into(),
            mode: crate::theme::ThemeMode::Dark,
            source: crate::theme::ThemeSource {
                kind: crate::theme::ThemeSourceKind::File,
                scope: crate::theme::ThemeSourceScope::Project,
            },
            global_active: true,
            project_active: true,
            effective: true,
        }];

        let buffer = draw_state(44, 22, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Theme 1/1"));
        assert!(text.contains("Long Theme Name"));
        assert!(text.contains("…"));
        assert!(!text.contains("description with a lot of extra detail"));
    }

    #[test]
    fn render_keymap_settings_ellipsizes_long_rows_when_narrow() {
        let mut state = TuiState::new();
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.open_keymaps();

        let width: u16 = 44;
        let height: u16 = 18;
        let buffer = draw_state(width, height, &state);
        let width = usize::from(width);
        let height = usize::from(height);
        let lines = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer.content()[y * width + x].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let preset_line = lines
            .iter()
            .find(|line| line.contains("Preset:"))
            .unwrap_or_else(|| panic!("missing preset line: {lines:?}"));
        assert!(preset_line.contains('…'), "{preset_line}");
        let action_line = lines
            .iter()
            .find(|line| line.contains("[Common]"))
            .unwrap_or_else(|| panic!("missing keymap action line: {lines:?}"));
        assert!(action_line.contains('…'), "{action_line}");
    }

    #[test]
    fn render_remaining_settings_pages_ellipsize_when_narrow() {
        let mut state = TuiState::with_settings("long-model", oino_types::ThinkingLevel::Off);
        state.set_model_catalog(
            vec![
                crate::settings::ModelOption::new("long-model").with_display_name(
                    "openrouter:provider-with-a-very-long-name/model-with-a-tail-that-should-hide",
                ),
            ],
            "cached models loaded",
        );
        state.overlay = Some(crate::app::OverlayKind::Settings);

        state.settings.page = crate::settings::SettingsPage::Thinking;
        let thinking = buffer_text(&draw_state(44, 18, &state));
        assert!(thinking.contains("Model:"));
        assert!(thinking.contains("…"));
        assert!(!thinking.contains("tail-that-should-hide"));

        state.settings.page = crate::settings::SettingsPage::Collapse;
        let collapse = buffer_text(&draw_state(44, 18, &state));
        assert!(collapse.contains("Enter cycles"));
        assert!(collapse.contains("…"));

        state.settings.page = crate::settings::SettingsPage::ChatStyle;
        let chat_style = buffer_text(&draw_state(44, 18, &state));
        assert!(chat_style.contains("Changing style"));
        assert!(chat_style.contains("…"));

        state.settings.page = crate::settings::SettingsPage::Extensions;
        let extensions = buffer_text(&draw_state(44, 18, &state));
        assert!(extensions.contains("Install packages"));
        assert!(extensions.contains("…"));
    }

    #[test]
    fn render_settings_tools_page_ellipsizes_long_rows_when_narrow() {
        let mut state = TuiState::new();
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.open_tools();
        state.set_tool_settings(vec![crate::settings::ToolSettingsItem::global("long_tool")
            .with_display_name(
                "Very Long Tool Display Name With A Suffix That Would Overflow The Panel",
            )]);

        let buffer = draw_state(44, 18, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Tools 1/1"));
        assert!(text.contains("Very Long Tool"));
        assert!(text.contains("…"));
        assert!(!text.contains("Overflow The Panel"));
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
    fn render_settings_menu_ellipsizes_long_current_values_when_narrow() {
        let mut state = TuiState::with_settings("long-model", oino_types::ThinkingLevel::Off);
        state.set_model_catalog(
            vec![
                crate::settings::ModelOption::new("long-model").with_display_name(
                    "openrouter:provider-with-a-very-long-name/model-with-an-unbounded-long-suffix",
                ),
            ],
            "cached models loaded",
        );
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.page = crate::settings::SettingsPage::Menu;

        let buffer = draw_state(44, 18, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Settings Pages"));
        assert!(text.contains("Model Selection"));
        assert!(text.contains("current:"));
        assert!(text.contains("…"));
        assert!(!text.contains("unbounded-long-suffix"));
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
        assert!(text.contains("Extensions"));
        assert!(text.contains("open extension manager"));
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
    fn model_selection_ellipsizes_long_rows_when_narrow() {
        let mut state = TuiState::with_settings("long-model", oino_types::ThinkingLevel::Off);
        state.set_model_catalog(
            vec![crate::settings::ModelOption::new("long-model").with_display_name(
                "openrouter:provider-with-a-very-long-name/model-with-a-very-long-name-and-suffix",
            )],
            "cached models loaded",
        );
        state.overlay = Some(crate::app::OverlayKind::Settings);
        state.settings.page = crate::settings::SettingsPage::Models;

        let buffer = draw_state(44, 18, &state);
        let text = buffer_text(&buffer);

        assert!(text.contains("Model Selection"));
        assert!(text.contains("openrouter:provider"));
        assert!(text.contains("…"));
        assert!(!text.contains("very-long-name-and-suffix"));
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
        state.settings.model_selector.cursor = 39;
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
