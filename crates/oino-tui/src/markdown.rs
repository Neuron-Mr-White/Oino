#![forbid(unsafe_code)]

use crate::theme::Theme;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy)]
struct MarkdownStyles {
    base: Style,
    heading: Style,
    heading_secondary: Style,
    emphasis: Style,
    strong: Style,
    strike: Style,
    code: Style,
    link: Style,
    muted: Style,
    quote: Style,
    list_marker: Style,
    table_border: Style,
}

impl MarkdownStyles {
    fn new(base: Style, theme: &Theme) -> Self {
        Self {
            base,
            heading: base
                .fg(theme.focused_border)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            heading_secondary: base.fg(theme.focused_border).add_modifier(Modifier::BOLD),
            emphasis: Style::default().add_modifier(Modifier::ITALIC),
            strong: Style::default().add_modifier(Modifier::BOLD),
            strike: Style::default().add_modifier(Modifier::CROSSED_OUT),
            code: base.fg(theme.focused_border),
            link: base
                .fg(theme.focused_border)
                .add_modifier(Modifier::UNDERLINED),
            muted: Style::default().fg(theme.muted),
            quote: Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
            list_marker: Style::default().fg(theme.focused_border),
            table_border: Style::default()
                .fg(theme.focused_border)
                .add_modifier(Modifier::BOLD),
        }
    }

    fn heading_for(self, level: HeadingLevel) -> Style {
        match level {
            HeadingLevel::H1 => self.heading,
            _ => self.heading_secondary,
        }
    }
}

#[derive(Debug, Clone)]
struct ListState {
    ordered: bool,
    next: u64,
}

#[derive(Debug, Clone)]
struct ItemContext {
    marker: String,
    continuation: String,
    marker_pending: bool,
}

#[derive(Debug, Default)]
struct TableState {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
}

struct MarkdownRenderer {
    width: usize,
    styles: MarkdownStyles,
    lines: Vec<Line<'static>>,
    current_spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_stack: Vec<ListState>,
    item_stack: Vec<ItemContext>,
    blockquote_depth: usize,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_content: String,
    table: Option<TableState>,
    link_targets: Vec<String>,
    in_image: bool,
    image_url: Option<String>,
    image_alt: String,
}

pub(crate) fn render_markdown_lines(
    markdown: &str,
    width: usize,
    base_style: Style,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut renderer = MarkdownRenderer::new(width, base_style, theme);
    renderer.render(markdown);
    renderer.finish()
}

pub(crate) fn prefixed_markdown_lines(
    markdown: &str,
    width: usize,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
    base_style: Style,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let prefix_width = line_width(&initial_prefix).max(line_width(&subsequent_prefix));
    let content_width = width.saturating_sub(prefix_width).max(1);
    let rendered = render_markdown_lines(markdown, content_width, base_style, theme);
    prefix_lines(rendered, initial_prefix, subsequent_prefix)
}

fn prefix_lines(
    lines: Vec<Line<'static>>,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        let mut prefixed = if index == 0 {
            initial_prefix.clone()
        } else {
            subsequent_prefix.clone()
        };
        prefixed.spans.extend(line.spans);
        out.push(prefixed);
    }
    out
}

impl MarkdownRenderer {
    fn new(width: usize, base_style: Style, theme: &Theme) -> Self {
        Self {
            width: width.max(1),
            styles: MarkdownStyles::new(base_style, theme),
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: Vec::new(),
            list_stack: Vec::new(),
            item_stack: Vec::new(),
            blockquote_depth: 0,
            in_code_block: false,
            code_block_lang: None,
            code_block_content: String::new(),
            table: None,
            link_targets: Vec::new(),
            in_image: false,
            image_url: None,
            image_alt: String::new(),
        }
    }

    fn render(&mut self, markdown: &str) {
        if markdown.trim().is_empty() {
            return;
        }

        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_SMART_PUNCTUATION);

        for event in Parser::new_ext(markdown, options) {
            self.handle_event(event);
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_current_line();
        while self.lines.last().is_some_and(is_blank_line) {
            self.lines.pop();
        }
        self.lines
    }

    fn handle_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                self.flush_current_line();
                self.in_code_block = true;
                self.code_block_content.clear();
                self.code_block_lang = code_block_language(kind);
            }
            Event::End(TagEnd::CodeBlock) if self.in_code_block => {
                self.render_code_block();
                self.in_code_block = false;
                self.code_block_lang = None;
                self.code_block_content.clear();
                self.push_blank();
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text)
                if self.in_code_block =>
            {
                self.code_block_content.push_str(&text);
            }
            Event::Code(code) if self.in_code_block => {
                self.code_block_content.push_str(&code);
            }
            Event::SoftBreak | Event::HardBreak if self.in_code_block => {
                self.code_block_content.push('\n');
            }
            Event::Start(Tag::Table(_)) => {
                self.flush_current_line();
                self.table = Some(TableState::default());
            }
            Event::End(TagEnd::Table) if self.table.is_some() => {
                self.render_table();
                self.table = None;
                self.push_blank();
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) if self.table.is_some() => {
                if let Some(table) = self.table.as_mut() {
                    table.current_row.clear();
                }
            }
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow)
                if self.table.is_some() =>
            {
                if let Some(table) = self.table.as_mut() {
                    if !table.current_row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.current_row));
                    }
                }
            }
            Event::Start(Tag::TableCell) if self.table.is_some() => {
                if let Some(table) = self.table.as_mut() {
                    table.current_cell.clear();
                }
            }
            Event::End(TagEnd::TableCell) if self.table.is_some() => {
                if let Some(table) = self.table.as_mut() {
                    table
                        .current_row
                        .push(table.current_cell.trim().to_string());
                    table.current_cell.clear();
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text)
                if self.table.is_some() =>
            {
                self.push_table_text(&text);
            }
            Event::Code(code) if self.table.is_some() => {
                self.push_table_text(&code);
            }
            Event::SoftBreak | Event::HardBreak if self.table.is_some() => {
                self.push_table_text(" ");
            }
            Event::TaskListMarker(checked) if self.table.is_some() => {
                self.push_table_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                self.flush_current_line();
                if self.list_stack.is_empty() && self.blockquote_depth == 0 {
                    self.push_blank();
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                self.flush_current_line();
                self.style_stack.push(self.styles.heading_for(level));
            }
            Event::End(TagEnd::Heading(_)) => {
                self.flush_current_line();
                self.pop_style();
                self.push_blank();
            }
            Event::Start(Tag::Strong) => self.style_stack.push(self.styles.strong),
            Event::End(TagEnd::Strong) => self.pop_style(),
            Event::Start(Tag::Emphasis) => self.style_stack.push(self.styles.emphasis),
            Event::End(TagEnd::Emphasis) => self.pop_style(),
            Event::Start(Tag::Strikethrough) => self.style_stack.push(self.styles.strike),
            Event::End(TagEnd::Strikethrough) => self.pop_style(),
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.link_targets.push(dest_url.to_string());
                self.style_stack.push(self.styles.link);
            }
            Event::End(TagEnd::Link) => {
                self.pop_style();
                if let Some(url) = self.link_targets.pop() {
                    if !url.trim().is_empty() {
                        self.current_spans
                            .push(Span::styled(format!(" ({url})"), self.styles.muted));
                    }
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                self.in_image = true;
                self.image_url = Some(dest_url.to_string());
                self.image_alt.clear();
            }
            Event::End(TagEnd::Image) => {
                self.push_image_label();
                self.in_image = false;
                self.image_url = None;
                self.image_alt.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush_current_line();
                self.blockquote_depth = self.blockquote_depth.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush_current_line();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                if self.blockquote_depth == 0 {
                    self.push_blank();
                }
            }
            Event::Start(Tag::List(start)) => {
                let next = start.unwrap_or(1);
                self.list_stack.push(ListState {
                    ordered: start.is_some(),
                    next,
                });
            }
            Event::End(TagEnd::List(_)) => {
                self.flush_current_line();
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.push_blank();
                }
            }
            Event::Start(Tag::Item) => self.start_item(),
            Event::End(TagEnd::Item) => {
                self.flush_current_line();
                self.item_stack.pop();
            }
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_code(&code),
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.flush_current_line(),
            Event::Rule => {
                self.flush_current_line();
                let width = self.width.clamp(1, 80);
                self.lines
                    .push(Line::styled("─".repeat(width), self.styles.muted));
                self.push_blank();
            }
            Event::TaskListMarker(checked) => {
                self.push_span(
                    if checked { "[x] " } else { "[ ] " },
                    self.styles.list_marker,
                );
            }
            Event::FootnoteReference(label) => {
                self.push_span(format!("[{label}]"), self.styles.muted);
            }
            _ => {}
        }
    }

    fn push_table_text(&mut self, text: &str) {
        if let Some(table) = self.table.as_mut() {
            table.current_cell.push_str(text);
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.in_image {
            self.image_alt.push_str(text);
            return;
        }
        self.push_span(text, self.current_style());
    }

    fn push_code(&mut self, code: &str) {
        self.push_span(code, self.current_style().patch(self.styles.code));
    }

    fn push_span(&mut self, text: impl Into<String>, style: Style) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.current_spans.push(Span::styled(text, style));
    }

    fn push_image_label(&mut self) {
        let alt = if self.image_alt.trim().is_empty() {
            "image"
        } else {
            self.image_alt.trim()
        };
        let label = if let Some(url) = self.image_url.as_ref().filter(|url| !url.trim().is_empty())
        {
            format!("[image: {alt}] ({url})")
        } else {
            format!("[image: {alt}]")
        };
        self.current_spans
            .push(Span::styled(label, self.styles.muted));
    }

    fn current_style(&self) -> Style {
        self.style_stack
            .iter()
            .fold(self.styles.base, |style, next| style.patch(*next))
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn start_item(&mut self) {
        let marker = if let Some(state) = self.list_stack.last_mut() {
            if state.ordered {
                let marker = format!("{}. ", state.next);
                state.next = state.next.saturating_add(1);
                marker
            } else {
                "• ".to_string()
            }
        } else {
            "• ".to_string()
        };
        let continuation = " ".repeat(marker.width());
        self.item_stack.push(ItemContext {
            marker,
            continuation,
            marker_pending: true,
        });
    }

    fn flush_current_line(&mut self) {
        if self.current_spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.current_spans);
        let line = Line::from(spans);
        let (initial_prefix, subsequent_prefix) = self.current_prefixes();
        push_wrapped_line(
            &mut self.lines,
            line,
            self.width,
            initial_prefix,
            subsequent_prefix,
        );
    }

    fn current_prefixes(&mut self) -> (Line<'static>, Line<'static>) {
        let mut initial = Line::default();
        let mut subsequent = Line::default();
        for _ in 0..self.blockquote_depth {
            initial.push_span(Span::styled("│ ", self.styles.quote));
            subsequent.push_span(Span::styled("│ ", self.styles.quote));
        }
        for item in &mut self.item_stack {
            if item.marker_pending {
                initial.push_span(Span::styled(item.marker.clone(), self.styles.list_marker));
                subsequent.push_span(Span::raw(item.continuation.clone()));
                item.marker_pending = false;
            } else {
                initial.push_span(Span::raw(item.continuation.clone()));
                subsequent.push_span(Span::raw(item.continuation.clone()));
            }
        }
        (initial, subsequent)
    }

    fn continuation_prefixes(&self) -> (Line<'static>, Line<'static>) {
        let mut initial = Line::default();
        let mut subsequent = Line::default();
        for _ in 0..self.blockquote_depth {
            initial.push_span(Span::styled("│ ", self.styles.quote));
            subsequent.push_span(Span::styled("│ ", self.styles.quote));
        }
        for item in &self.item_stack {
            initial.push_span(Span::raw(item.continuation.clone()));
            subsequent.push_span(Span::raw(item.continuation.clone()));
        }
        (initial, subsequent)
    }

    fn render_code_block(&mut self) {
        let code = self.code_block_content.clone();
        let mut parts = code.split('\n').collect::<Vec<_>>();
        if parts.last().is_some_and(|part| part.is_empty()) {
            parts.pop();
        }
        if parts.is_empty() {
            parts.push("");
        }
        let mut consumed_block_prefix = false;
        if let Some(lang) = self
            .code_block_lang
            .as_ref()
            .filter(|lang| !lang.trim().is_empty())
            .cloned()
        {
            let (mut initial, mut subsequent) = self.block_prefixes(&mut consumed_block_prefix);
            initial.push_span(Span::styled(format!("{lang} "), self.styles.muted));
            subsequent.push_span(Span::styled("  ", self.styles.muted));
            push_wrapped_line(
                &mut self.lines,
                Line::styled("", self.styles.muted),
                self.width,
                initial,
                subsequent,
            );
        }
        for part in parts {
            let (mut initial, mut subsequent) = self.block_prefixes(&mut consumed_block_prefix);
            initial.push_span(Span::styled("    ", self.styles.muted));
            subsequent.push_span(Span::styled("    ", self.styles.muted));
            push_wrapped_line(
                &mut self.lines,
                Line::styled(part.to_string(), self.styles.code),
                self.width,
                initial,
                subsequent,
            );
        }
    }

    fn block_prefixes(
        &mut self,
        consumed_block_prefix: &mut bool,
    ) -> (Line<'static>, Line<'static>) {
        if *consumed_block_prefix {
            self.continuation_prefixes()
        } else {
            *consumed_block_prefix = true;
            self.current_prefixes()
        }
    }

    fn render_table(&mut self) {
        let Some(table) = self.table.as_ref() else {
            return;
        };
        let rows = table.rows.clone();
        if rows.is_empty() {
            return;
        }
        let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
        if column_count == 0 {
            return;
        }

        let rows = normalize_table_rows(&rows, column_count);
        let widths = table_column_widths(&rows, self.width);
        if widths.is_empty() {
            return;
        }

        let mut consumed_block_prefix = false;
        self.push_table_border(&widths, "┌", "┬", "┐", &mut consumed_block_prefix);
        for (row_index, row) in rows.iter().enumerate() {
            self.push_table_row(row, &widths, row_index == 0, &mut consumed_block_prefix);
            if row_index + 1 < rows.len() {
                self.push_table_border(&widths, "├", "┼", "┤", &mut consumed_block_prefix);
            }
        }
        self.push_table_border(&widths, "└", "┴", "┘", &mut consumed_block_prefix);
    }

    fn push_table_border(
        &mut self,
        widths: &[usize],
        left: &str,
        mid: &str,
        right: &str,
        consumed_block_prefix: &mut bool,
    ) {
        let mut border = String::from(left);
        for (index, width) in widths.iter().enumerate() {
            border.push_str(&"─".repeat(width.saturating_add(2)));
            if index + 1 < widths.len() {
                border.push_str(mid);
            }
        }
        border.push_str(right);
        let (initial, subsequent) = self.block_prefixes(consumed_block_prefix);
        push_wrapped_line(
            &mut self.lines,
            Line::from(Span::styled(border, self.styles.table_border)),
            self.width,
            initial,
            subsequent,
        );
    }

    fn push_table_row(
        &mut self,
        row: &[String],
        widths: &[usize],
        is_header: bool,
        consumed_block_prefix: &mut bool,
    ) {
        let wrapped = row
            .iter()
            .zip(widths.iter().copied())
            .map(|(cell, width)| wrap_table_cell(cell, width))
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let cell_style = if is_header {
            self.styles.base.add_modifier(Modifier::BOLD)
        } else {
            self.styles.base
        };

        for visual_row in 0..height {
            let mut line = Line::default();
            line.push_span(Span::styled("│ ", self.styles.table_border));
            for (index, width) in widths.iter().enumerate() {
                if index > 0 {
                    line.push_span(Span::styled(" │ ", self.styles.table_border));
                }
                let segment = wrapped
                    .get(index)
                    .and_then(|segments| segments.get(visual_row))
                    .map(String::as_str)
                    .unwrap_or("");
                line.push_span(Span::styled(pad_to_width(segment, *width), cell_style));
            }
            line.push_span(Span::styled(" │", self.styles.table_border));
            let (initial, subsequent) = self.block_prefixes(consumed_block_prefix);
            push_wrapped_line(&mut self.lines, line, self.width, initial, subsequent);
        }
    }

    fn push_blank(&mut self) {
        if self.lines.last().is_none_or(|line| !is_blank_line(line)) {
            self.lines.push(Line::from(""));
        }
    }
}

fn code_block_language(kind: CodeBlockKind<'_>) -> Option<String> {
    match kind {
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .filter(|lang| !lang.trim().is_empty())
            .map(ToString::to_string),
        CodeBlockKind::Indented => None,
    }
}

fn normalize_table_rows(rows: &[Vec<String>], column_count: usize) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| {
            let mut normalized = row.iter().take(column_count).cloned().collect::<Vec<_>>();
            normalized.resize(column_count, String::new());
            normalized
        })
        .collect()
}

fn table_column_widths(rows: &[Vec<String>], max_width: usize) -> Vec<usize> {
    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return Vec::new();
    }

    let chrome = column_count.saturating_mul(3).saturating_add(1);
    let available = max_width.saturating_sub(chrome).max(column_count);
    let min_width = if available >= column_count.saturating_mul(4) {
        4
    } else {
        1
    };
    let mut widths = vec![min_width; column_count];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            let desired = cell.width().max(longest_word_width(cell)).max(min_width);
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(desired);
            }
        }
    }

    while widths.iter().sum::<usize>() > available {
        let Some((index, width)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > min_width)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[index] = width.saturating_sub(1);
    }

    if widths.iter().sum::<usize>() > available {
        let base = (available / column_count).max(1);
        let mut remainder = available.saturating_sub(base.saturating_mul(column_count));
        for width in &mut widths {
            *width = base;
            if remainder > 0 {
                *width = width.saturating_add(1);
                remainder = remainder.saturating_sub(1);
            }
        }
    }

    widths
}

fn longest_word_width(text: &str) -> usize {
    text.split_whitespace()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
}

fn wrap_table_cell(cell: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if cell.trim().is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in cell.split_whitespace() {
        let word_width = word.width();
        if current_width == 0 {
            if word_width > width {
                push_wrapped_table_word(word, width, &mut current, &mut current_width, &mut lines);
            } else {
                current.push_str(word);
                current_width = word_width;
            }
        } else if current_width.saturating_add(1).saturating_add(word_width) <= width {
            current.push(' ');
            current.push_str(word);
            current_width = current_width.saturating_add(1).saturating_add(word_width);
        } else {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
            if word_width > width {
                push_wrapped_table_word(word, width, &mut current, &mut current_width, &mut lines);
            } else {
                current.push_str(word);
                current_width = word_width;
            }
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn push_wrapped_table_word(
    word: &str,
    width: usize,
    current: &mut String,
    current_width: &mut usize,
    lines: &mut Vec<String>,
) {
    for grapheme in word.graphemes(true) {
        let grapheme_width = grapheme.width();
        if current_width.saturating_add(grapheme_width) > width && !current.is_empty() {
            lines.push(std::mem::take(current));
            *current_width = 0;
        }
        current.push_str(grapheme);
        *current_width = current_width.saturating_add(grapheme_width);
        if *current_width >= width {
            lines.push(std::mem::take(current));
            *current_width = 0;
        }
    }
}

fn pad_to_width(text: &str, width: usize) -> String {
    let used = text.width();
    if used >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
}

fn push_wrapped_line(
    out: &mut Vec<Line<'static>>,
    line: Line<'static>,
    width: usize,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
) {
    let width = width.max(1);
    let mut current = initial_prefix;
    let mut current_width = line_width(&current);
    let mut has_content = false;
    let mut prefix_width = current_width;
    for span in line.spans {
        let style = span.style;
        for grapheme in span.content.as_ref().graphemes(true) {
            let grapheme_width = grapheme.width();
            if current_width.saturating_add(grapheme_width) > width && has_content {
                out.push(current);
                current = subsequent_prefix.clone();
                current_width = line_width(&current);
                prefix_width = current_width;
                has_content = false;
            }
            push_coalesced_span(&mut current, grapheme, style);
            current_width = current_width.saturating_add(grapheme_width);
            has_content = has_content || current_width > prefix_width || grapheme_width == 0;
        }
    }
    out.push(current);
}

fn push_coalesced_span(line: &mut Line<'static>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = line.spans.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(text);
            return;
        }
    }
    line.push_span(Span::styled(text.to_string(), style));
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn is_blank_line(line: &Line<'_>) -> bool {
    line.spans
        .iter()
        .all(|span| span.content.as_ref().trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn renders_common_markdown_blocks_without_raw_markers() {
        let lines = render_markdown_lines(
            "## Title\n\n- **Bold** item with `code`\n- Link to [Oino](https://example.invalid)\n\n```rust\nfn main() {}\n```",
            80,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert!(plain_lines.contains(&"Title".to_string()));
        assert!(plain_lines.contains(&"• Bold item with code".to_string()));
        assert!(plain_lines
            .iter()
            .any(|line| line.contains("Oino (https://example.invalid)")));
        assert!(plain_lines.iter().any(|line| line.contains("fn main() {}")));
        assert!(!plain_lines.iter().any(|line| line.contains("**Bold**")));
        assert!(!plain_lines.iter().any(|line| line.contains("`code`")));
    }

    #[test]
    fn wraps_prefixed_markdown_inside_available_width() {
        let lines = prefixed_markdown_lines(
            "This is a long **markdown** sentence that should wrap under the assistant bullet.",
            26,
            Line::from("• "),
            Line::from("  "),
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert!(plain_lines.len() > 1);
        assert!(plain_lines[0].starts_with("• "));
        assert!(plain_lines[1].starts_with("  "));
        assert!(plain_lines.iter().all(|line| line.width() <= 26));
    }

    #[test]
    fn renders_tables_as_wrapped_box_grid() {
        let lines = render_markdown_lines(
            "| Name | Notes |\n| --- | --- |\n| Alpha | This is a longer table cell that should wrap instead of being truncated. |\n| Beta | short |",
            54,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();
        let joined = plain_lines.join("\n");

        assert!(plain_lines
            .first()
            .is_some_and(|line| line.starts_with('┌')));
        assert!(
            plain_lines
                .iter()
                .filter(|line| line.starts_with('├'))
                .count()
                >= 2
        );
        assert!(plain_lines.last().is_some_and(|line| line.starts_with('└')));
        assert!(joined.contains("Alpha"));
        assert!(joined.contains("instead of being"));
        assert!(joined.contains("truncated"));
        assert!(!joined.contains('…'));
        assert!(plain_lines.iter().all(|line| line.width() <= 54));

        let border_span = lines
            .first()
            .and_then(|line| line.spans.first())
            .unwrap_or_else(|| panic!("missing table border span"));
        assert_eq!(border_span.style.fg, Some(Theme::default().focused_border));
        assert!(border_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn preserves_inline_styles_for_emphasis_and_strong() {
        let lines = render_markdown_lines(
            "A **bold** and *soft* word",
            80,
            Style::default(),
            &Theme::default(),
        );
        let bold_span = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref().contains("bold"));
        let italic_span = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.as_ref().contains("soft"));

        assert!(bold_span.is_some_and(|span| span.style.add_modifier.contains(Modifier::BOLD)));
        assert!(italic_span.is_some_and(|span| span.style.add_modifier.contains(Modifier::ITALIC)));
    }
}
