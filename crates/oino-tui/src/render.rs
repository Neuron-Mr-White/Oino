#![forbid(unsafe_code)]

use crate::{
    app::{ChordState, OverlayKind, TuiFocus, TuiState},
    command::CommandSuggestionsView,
    composer::{byte_index_at_char, char_count, ComposerState, INPUT_PLACEHOLDER},
    message::MessageView,
    settings::{collapse_mode_label, thinking_label, CollapseMode, SettingsPage, SettingsState},
    theme::Theme,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const FOOTER_HEIGHT: u16 = 1;
const MIN_TRANSCRIPT_HEIGHT: u16 = 3;
const MIN_COMPOSER_ROWS: usize = 3;
const MAX_COMPOSER_HEIGHT: u16 = 9;
const INPUT_PROMPT: &str = "› ";
const TINY_MESSAGE: &str = "Oino needs at least 20x8";

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
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(area);

    render_transcript(frame, chunks[0], state, theme);
    render_composer(frame, chunks[1], state, theme);
    render_footer(frame, chunks[2], state, theme);

    if state.overlay.is_none() {
        if let Some(suggestions) = state.command_suggestions_view() {
            render_command_suggestions(frame, area, chunks[1], &suggestions, theme);
        }
    }

    if matches!(state.overlay, Some(OverlayKind::Settings)) {
        render_settings_overlay(frame, area, &state.settings, theme);
    }

    if state.chord != ChordState::None {
        render_chord_hint(frame, area, state.chord, theme);
    }
}

fn render_chord_hint(frame: &mut Frame<'_>, area: Rect, chord: ChordState, theme: &Theme) {
    let title = match chord {
        ChordState::CtrlO => " Ctrl-O chord: s settings • Esc cancel ",
        ChordState::None => return,
    };
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

fn render_transcript(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    let mut lines = transcript_lines(
        &state.messages,
        state.error.as_deref(),
        inner_width,
        state.settings.thinking_collapse_mode,
        state.settings.tool_collapse_mode,
        theme,
    );

    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "No messages yet. Send a task to start.",
            Style::default().fg(theme.muted),
        )]));
    }

    if lines.len() > inner_height {
        let skip = lines.len().saturating_sub(inner_height);
        lines = lines.into_iter().skip(skip).collect();
    }

    let block = Block::default()
        .title(Span::styled(" Oino ", theme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_composer(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let title = if state.working {
        " Task (working) "
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

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &TuiState, theme: &Theme) {
    let style = if state.error.is_some() {
        theme.error
    } else if state.working {
        theme.working
    } else {
        theme.footer
    };
    frame.render_widget(Paragraph::new(state.status.clone()).style(style), area);
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
                Line::from(vec![
                    Span::styled(format!("{marker} {}", item.label), style),
                    Span::styled(
                        format!("  {}", item.summary),
                        Style::default().fg(theme.muted),
                    ),
                ])
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
    if suggestions.title == "Models" {
        5
    } else {
        4
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
        SettingsPage::Thinking => "arrows/jk move • Enter apply • Esc/← back • Ctrl-C quit",
        SettingsPage::Collapse => "arrows/jk move • Enter/→ cycle • Esc/← back",
    };
    let status = format!("{} • {controls}", settings.status);
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

fn transcript_lines(
    messages: &[MessageView],
    error: Option<&str>,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for message in messages {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(bubble_lines(
            message,
            width,
            thinking_mode,
            tool_mode,
            theme,
        ));
    }
    if let Some(error) = error {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        let error_message = MessageView {
            id: oino_types::OinoId::nil(),
            role: "error".into(),
            title: None,
            content: error.into(),
            thinking: None,
            thinking_redacted: false,
            is_error: true,
        };
        lines.extend(bubble_lines(
            &error_message,
            width,
            thinking_mode,
            tool_mode,
            theme,
        ));
    }
    lines
}

fn bubble_lines(
    message: &MessageView,
    available_width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if is_empty_assistant_message(message) {
        return Vec::new();
    }
    if available_width < 16 {
        return vec![Line::styled(
            format!("{}: {}", message.role, message.content),
            theme.bubble_border_for_role(&message.role, message.is_error),
        )];
    }

    let max_bubble_width = available_width.clamp(16, 80).min(available_width);
    let content_width = max_bubble_width.saturating_sub(4).max(1);
    let message_content = display_message_content(message, tool_mode);
    let wrapped = wrap_text(&message_content, content_width);
    let thinking_text = thinking_display_text(message, thinking_mode);
    let thinking_wrapped = thinking_text
        .as_deref()
        .map(|thinking| wrap_text(thinking, content_width.saturating_sub(2).max(1)))
        .unwrap_or_default();
    let role_label = message
        .title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| {
            if message.role.is_empty() {
                "message".to_string()
            } else {
                message.role.clone()
            }
        });
    let content_max = wrapped
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .chain(
            thinking_wrapped
                .iter()
                .map(|line| UnicodeWidthStr::width(line.as_str()).saturating_add(2)),
        )
        .max()
        .unwrap_or(0);
    let thinking_label_width = if thinking_wrapped.is_empty() {
        0
    } else {
        UnicodeWidthStr::width("◌")
    };
    let label_width = UnicodeWidthStr::width(role_label.as_str()).saturating_add(2);
    let inner_width = content_width.min(
        content_max
            .max(thinking_label_width)
            .max(label_width)
            .max(1),
    );
    let bubble_width = inner_width.saturating_add(4);
    let left_pad = if message.is_user() {
        available_width.saturating_sub(bubble_width)
    } else {
        0
    };
    let border_style = theme.bubble_border_for_role(&message.role, message.is_error);

    let mut lines = Vec::new();
    lines.push(Line::styled(
        format!(
            "{}{}",
            " ".repeat(left_pad),
            top_border(&role_label, inner_width)
        ),
        border_style,
    ));
    if !thinking_wrapped.is_empty() {
        lines.push(bubble_content_line(
            left_pad,
            inner_width,
            border_style,
            vec![Span::styled("◌", Style::default().fg(theme.muted))],
        ));
        for line in wrap_text(
            thinking_text.as_deref().unwrap_or_default(),
            inner_width.saturating_sub(2).max(1),
        ) {
            let text = format!("  {line}");
            lines.push(bubble_content_line(
                left_pad,
                inner_width,
                border_style,
                vec![Span::styled(text, Style::default().fg(theme.muted))],
            ));
        }
        if message_content != "<empty>" {
            lines.push(bubble_content_line(
                left_pad,
                inner_width,
                border_style,
                vec![],
            ));
        }
    }
    if message_content != "<empty>" || thinking_wrapped.is_empty() {
        for line in wrapped {
            lines.push(bubble_content_line(
                left_pad,
                inner_width,
                border_style,
                vec![Span::raw(line)],
            ));
        }
    }
    lines.push(Line::styled(
        format!("{}╰{}╯", " ".repeat(left_pad), "─".repeat(inner_width + 2)),
        border_style,
    ));
    lines
}

fn bubble_content_line(
    left_pad: usize,
    inner_width: usize,
    border_style: Style,
    mut content: Vec<Span<'static>>,
) -> Line<'static> {
    let content_width = content
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum::<usize>();
    if content_width < inner_width {
        content.push(Span::raw(" ".repeat(inner_width - content_width)));
    }
    let mut spans = vec![
        Span::raw(" ".repeat(left_pad)),
        Span::styled("│ ", border_style),
    ];
    spans.extend(content);
    spans.push(Span::styled(" │", border_style));
    Line::from(spans)
}

fn is_empty_assistant_message(message: &MessageView) -> bool {
    message.is_assistant()
        && message.content == "<empty>"
        && message
            .thinking
            .as_ref()
            .is_none_or(|thinking| thinking.trim().is_empty())
        && !message.thinking_redacted
}

fn display_message_content(message: &MessageView, tool_mode: CollapseMode) -> String {
    if message.role.starts_with("tool:") {
        match tool_mode {
            CollapseMode::Full => message.content.clone(),
            CollapseMode::Truncate => truncate_display(&message.content),
            CollapseMode::Collapse => "[collapsed]".into(),
        }
    } else {
        message.content.clone()
    }
}

fn thinking_display_text(message: &MessageView, thinking_mode: CollapseMode) -> Option<String> {
    let text = if message.thinking_redacted {
        "[redacted]".to_string()
    } else {
        message
            .thinking
            .as_ref()
            .filter(|thinking| !thinking.trim().is_empty())?
            .clone()
    };
    Some(match thinking_mode {
        CollapseMode::Full => text,
        CollapseMode::Truncate => truncate_display(&text),
        CollapseMode::Collapse => "[collapsed]".into(),
    })
}

fn truncate_display(text: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut truncated = text.chars().take(MAX_CHARS).collect::<String>();
    if text.chars().count() > MAX_CHARS {
        truncated.push('…');
    }
    truncated
}

fn top_border(label: &str, inner_width: usize) -> String {
    let label = truncate_to_width(label, inner_width.saturating_sub(1));
    let used = UnicodeWidthStr::width(label.as_str()).saturating_add(2);
    let rest = inner_width.saturating_add(2).saturating_sub(used);
    format!("╭ {label} {}╮", "─".repeat(rest))
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
    let available_height = total_height
        .saturating_sub(FOOTER_HEIGHT)
        .saturating_sub(MIN_TRANSCRIPT_HEIGHT)
        .max(3);
    let content_width = composer_content_width_for_width(width);
    let line_count = wrap_text(input, content_width).len().max(MIN_COMPOSER_ROWS);
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

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_width = 0usize;
        for grapheme in raw.graphemes(true) {
            let grapheme_width = grapheme.width();
            if current_width + grapheme_width > width && current_width != 0 {
                lines.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push_str(grapheme);
            current_width += grapheme_width;
            if current_width >= width {
                lines.push(current);
                current = String::new();
                current_width = 0;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

#[allow(dead_code)]
fn cursor_byte(input: &str, cursor: usize) -> usize {
    byte_index_at_char(input, cursor.min(char_count(input)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::TuiState, message::MessageView};
    use ratatui::{backend::TestBackend, Terminal};

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
            is_error: false,
        });
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "tool:bash".into(),
            title: None,
            content: "long tool output".into(),
            thinking: None,
            thinking_redacted: false,
            is_error: false,
        });
        let buffer = draw_state(80, 24, &state);
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("[collapsed]"));
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
            is_error: false,
        });
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "tool:write".into(),
            title: None,
            content: "Successfully wrote file".into(),
            thinking: None,
            thinking_redacted: false,
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
    fn render_thinking_as_section_not_inline_plain_text() {
        let mut state = TuiState::new();
        state.messages.push(MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "final answer".into(),
            thinking: Some("internal reasoning".into()),
            thinking_redacted: false,
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
        assert!(composer_height("a\n".repeat(20).as_str(), 80, 10) <= 6);
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
        assert!(text.contains("Ctrl-O chord"));
        assert!(text.contains("s settings"));
        assert!(text.contains("Esc cancel"));
    }

    #[test]
    fn render_command_suggestions_above_composer() {
        let mut state = TuiState::new();
        state.composer.replace_text("/");
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
