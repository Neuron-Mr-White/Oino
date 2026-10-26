#![forbid(unsafe_code)]

use crate::{text::truncate_to_width, theme::Theme};
use mmdflux::{render_diagram, OutputFormat, RenderConfig};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use std::{borrow::Cow, sync::OnceLock};
use syntect::{
    easy::ScopeRegionIterator,
    parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet},
};
use syntect_assets::assets::HighlightingAssets;
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
    code_border: Style,
    code_line_number: Style,
    link: Style,
    muted: Style,
    quote: Style,
    list_marker: Style,
    task_done_marker: Style,
    task_pending_marker: Style,
    table_border: Style,
    syntax_comment: Style,
    syntax_keyword: Style,
    syntax_function: Style,
    syntax_variable: Style,
    syntax_string: Style,
    syntax_number: Style,
    syntax_type: Style,
    syntax_operator: Style,
    syntax_punctuation: Style,
}

impl MarkdownStyles {
    fn new(base: Style, theme: &Theme) -> Self {
        Self {
            base,
            heading: Style::default()
                .fg(theme.markdown_heading)
                .add_modifier(Modifier::BOLD),
            heading_secondary: Style::default()
                .fg(theme.markdown_heading_secondary)
                .add_modifier(Modifier::BOLD),
            emphasis: Style::default().add_modifier(Modifier::ITALIC),
            strong: Style::default().add_modifier(Modifier::BOLD),
            strike: Style::default().add_modifier(Modifier::CROSSED_OUT),
            code: base.fg(theme.markdown_fg).bg(theme.markdown_code_bg),
            code_border: Style::default()
                .fg(theme.markdown_code_border)
                .add_modifier(Modifier::BOLD),
            code_line_number: Style::default().fg(theme.markdown_code_line_number),
            link: base
                .fg(theme.markdown_link)
                .add_modifier(Modifier::UNDERLINED),
            muted: Style::default().fg(theme.markdown_muted),
            quote: Style::default()
                .fg(theme.markdown_quote)
                .add_modifier(Modifier::ITALIC),
            list_marker: Style::default().fg(theme.markdown_list_marker),
            task_done_marker: Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
            task_pending_marker: Style::default()
                .fg(theme.warning.fg.unwrap_or(theme.markdown_marker))
                .add_modifier(Modifier::BOLD),
            table_border: Style::default()
                .fg(theme.markdown_table_border)
                .add_modifier(Modifier::BOLD),
            syntax_comment: base.fg(theme.syntax_comment).bg(theme.markdown_code_bg),
            syntax_keyword: base.fg(theme.syntax_keyword).bg(theme.markdown_code_bg),
            syntax_function: base.fg(theme.syntax_function).bg(theme.markdown_code_bg),
            syntax_variable: base.fg(theme.syntax_variable).bg(theme.markdown_code_bg),
            syntax_string: base.fg(theme.syntax_string).bg(theme.markdown_code_bg),
            syntax_number: base.fg(theme.syntax_number).bg(theme.markdown_code_bg),
            syntax_type: base.fg(theme.syntax_type).bg(theme.markdown_code_bg),
            syntax_operator: base.fg(theme.syntax_operator).bg(theme.markdown_code_bg),
            syntax_punctuation: base.fg(theme.syntax_punctuation).bg(theme.markdown_code_bg),
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
    marker_style: Style,
    continuation: String,
    marker_pending: bool,
}

#[derive(Debug)]
struct TableState {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
}

impl TableState {
    fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
        }
    }
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
    heading_level: Option<HeadingLevel>,
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
            heading_level: None,
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

        let normalized = unwrap_markdown_table_fences(markdown);
        for event in Parser::new_ext(normalized.as_ref(), options) {
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
            Event::Start(Tag::Table(alignments)) => {
                self.flush_current_line();
                self.table = Some(TableState::new(alignments));
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
                self.heading_level = Some(level);
                self.style_stack.push(self.styles.heading_for(level));
            }
            Event::End(TagEnd::Heading(level)) => {
                self.render_heading(level);
                self.heading_level = None;
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
                            .push(Span::styled(" ↗ ", self.styles.muted));
                        self.current_spans.push(Span::styled(url, self.styles.link));
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
            Event::TaskListMarker(checked) => self.apply_task_list_marker(checked),
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
            marker_style: self.styles.list_marker,
            continuation,
            marker_pending: true,
        });
    }

    fn apply_task_list_marker(&mut self, checked: bool) {
        let (marker, style) = if checked {
            ("✓ ", self.styles.task_done_marker)
        } else {
            ("○ ", self.styles.task_pending_marker)
        };
        if let Some(item) = self
            .item_stack
            .last_mut()
            .filter(|item| item.marker_pending)
        {
            item.marker = marker.to_string();
            item.marker_style = style;
            item.continuation = " ".repeat(marker.width());
        } else {
            self.push_span(marker, style);
        }
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
                initial.push_span(Span::styled(item.marker.clone(), item.marker_style));
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

    fn render_heading(&mut self, level: HeadingLevel) {
        if self.current_spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.current_spans);
        let title = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
            .trim()
            .to_string();
        if title.is_empty() {
            return;
        }
        match level {
            HeadingLevel::H1 => self.render_h1(&title),
            HeadingLevel::H2 => self.render_h2(&title),
            HeadingLevel::H3 => self.render_h3(&title),
            _ => self.render_h3(&title),
        }
    }

    fn render_h1(&mut self, title: &str) {
        let (initial, subsequent) = self.current_prefixes();
        let available = self.width.saturating_sub(line_width(&initial)).max(1);
        let title_width = available.saturating_sub(4).max(1);
        let visible_title = truncate_to_width(title, title_width);
        let top = heading_border_line('╭', '╮', "", available);
        let middle = format!(
            "│ {} │",
            pad_to_width(&visible_title, available.saturating_sub(4).max(1))
        );
        let bottom = heading_border_line('╰', '╯', "", available);
        self.lines.push(prefixed_line(
            Line::styled(top, self.styles.heading),
            initial,
        ));
        self.lines.push(prefixed_line(
            Line::styled(middle, self.styles.heading),
            subsequent.clone(),
        ));
        self.lines.push(prefixed_line(
            Line::styled(bottom, self.styles.heading),
            subsequent,
        ));
    }

    fn render_h2(&mut self, title: &str) {
        let (mut initial, subsequent) = self.current_prefixes();
        initial.push_span(Span::styled("▌ ", self.styles.heading_secondary));
        push_wrapped_line(
            &mut self.lines,
            Line::styled(title.to_string(), self.styles.heading_secondary),
            self.width,
            initial,
            subsequent.clone(),
        );
        let available = self.width.saturating_sub(line_width(&subsequent)).max(1);
        self.lines.push(prefixed_line(
            Line::styled("─".repeat(available.min(80)), self.styles.muted),
            subsequent,
        ));
    }

    fn render_h3(&mut self, title: &str) {
        let (mut initial, subsequent) = self.current_prefixes();
        initial.push_span(Span::styled("▌ ", self.styles.heading_secondary));
        push_wrapped_line(
            &mut self.lines,
            Line::styled(title.to_string(), self.styles.heading_secondary),
            self.width,
            initial,
            subsequent,
        );
    }

    fn render_code_block(&mut self) {
        let lang = self
            .code_block_lang
            .as_deref()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or("code");

        if lang == "mermaid" {
            self.render_mermaid_block();
            return;
        }

        let code = self.code_block_content.clone();
        let mut parts = code.split('\n').collect::<Vec<_>>();
        if parts.last().is_some_and(|part| part.is_empty()) {
            parts.pop();
        }
        if parts.is_empty() {
            parts.push("");
        }

        let mut consumed_block_prefix = false;
        let label = lang.to_string();
        self.push_code_block_border(Some(&label), true, &mut consumed_block_prefix);
        let mut code_highlighter = CodeHighlighter::new(&label);
        let number_width = parts.len().to_string().width().max(1);
        for (index, part) in parts.into_iter().enumerate() {
            let line_number = (index + 1).to_string();
            let prefix_width = line_width(&self.continuation_prefixes().0);
            let available = self.width.saturating_sub(prefix_width).max(1);
            let code_width = available
                .saturating_sub(number_width.saturating_add(7))
                .max(1);
            let wrapped = wrap_spans_to_width(
                code_highlighter.highlight_line(part, self.styles),
                code_width,
            );
            for (visual_index, segment) in wrapped.into_iter().enumerate() {
                let (mut line, _) = self.block_prefixes(&mut consumed_block_prefix);
                line.push_span(Span::styled("│ ", self.styles.code_border));
                let number = if visual_index == 0 {
                    format!(
                        "{}{}",
                        " ".repeat(number_width.saturating_sub(line_number.width())),
                        line_number
                    )
                } else {
                    " ".repeat(number_width)
                };
                line.push_span(Span::styled(number, self.styles.code_line_number));
                line.push_span(Span::styled(" │ ", self.styles.code_border));
                let segment_width = line_width(&segment);
                line.spans.extend(segment.spans);
                if segment_width < code_width {
                    line.push_span(Span::styled(
                        " ".repeat(code_width - segment_width),
                        self.styles.code,
                    ));
                }
                line.push_span(Span::styled(" │", self.styles.code_border));
                self.lines.push(line);
            }
        }
        self.push_code_block_border(None, false, &mut consumed_block_prefix);
    }

    fn render_mermaid_block(&mut self) {
        let mermaid_src = self.code_block_content.trim().to_string();

        let mut consumed_block_prefix = false;
        let label = "mermaid diagram";
        self.push_code_block_border(Some(label), true, &mut consumed_block_prefix);

        let rendered_text =
            match render_diagram(&mermaid_src, OutputFormat::Text, &RenderConfig::default()) {
                Ok(text) => text,
                Err(err) => {
                    // Fall back to showing the error as raw text inside the code block
                    let prefix_width = line_width(&self.continuation_prefixes().0);
                    let available = self.width.saturating_sub(prefix_width).max(1);
                    let code_width = available.saturating_sub(4).max(1);
                    let error_msg = format!("mermaid render error: {err}");
                    let wrapped = wrap_spans_to_width(
                        vec![Span::styled(error_msg, self.styles.code)],
                        code_width,
                    );
                    for segment in wrapped {
                        let (mut line, _) = self.block_prefixes(&mut consumed_block_prefix);
                        line.push_span(Span::styled("│ ", self.styles.code_border));
                        let segment_width = line_width(&segment);
                        line.spans.extend(segment.spans);
                        if segment_width < code_width {
                            line.push_span(Span::styled(
                                " ".repeat(code_width - segment_width),
                                self.styles.code,
                            ));
                        }
                        line.push_span(Span::styled(" │", self.styles.code_border));
                        self.lines.push(line);
                    }
                    self.push_code_block_border(None, false, &mut consumed_block_prefix);
                    return;
                }
            };

        // Render the mmdflux text output inside the code block frame
        for text_line in rendered_text.lines() {
            let prefix_width = line_width(&self.continuation_prefixes().0);
            let available = self.width.saturating_sub(prefix_width).max(1);
            let code_width = available.saturating_sub(4).max(1);

            // Strip ANSI escape sequences from mmdflux output for TUI rendering
            let stripped = strip_ansi(text_line);
            let wrapped =
                wrap_spans_to_width(vec![Span::styled(stripped, self.styles.code)], code_width);
            for segment in wrapped {
                let (mut line, _) = self.block_prefixes(&mut consumed_block_prefix);
                line.push_span(Span::styled("│ ", self.styles.code_border));
                let segment_width = line_width(&segment);
                line.spans.extend(segment.spans);
                if segment_width < code_width {
                    line.push_span(Span::styled(
                        " ".repeat(code_width - segment_width),
                        self.styles.code,
                    ));
                }
                line.push_span(Span::styled(" │", self.styles.code_border));
                self.lines.push(line);
            }
        }

        self.push_code_block_border(None, false, &mut consumed_block_prefix);
    }

    fn push_code_block_border(
        &mut self,
        label: Option<&str>,
        top: bool,
        consumed_block_prefix: &mut bool,
    ) {
        let (initial, subsequent) = self.block_prefixes(consumed_block_prefix);
        let available = self.width.saturating_sub(line_width(&initial)).max(1);
        let border = code_block_border_text(label, top, available);
        push_wrapped_line(
            &mut self.lines,
            Line::styled(border, self.styles.code_border),
            self.width,
            initial,
            subsequent,
        );
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
        let alignments = normalize_table_alignments(&table.alignments, column_count);
        let widths = table_column_widths(&rows, self.width);
        if widths.is_empty() {
            return;
        }

        let mut consumed_block_prefix = false;
        self.push_table_border(&widths, "┌", "┬", "┐", &mut consumed_block_prefix);
        for (row_index, row) in rows.iter().enumerate() {
            self.push_table_row(
                row,
                &widths,
                &alignments,
                row_index == 0,
                &mut consumed_block_prefix,
            );
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
        alignments: &[Alignment],
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
                let alignment = alignments.get(index).copied().unwrap_or(Alignment::None);
                line.push_span(Span::styled(
                    align_to_width(segment, *width, alignment),
                    cell_style,
                ));
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
            .split([',', ' ', '\t'])
            .next()
            .filter(|lang| !lang.trim().is_empty())
            .map(ToString::to_string),
        CodeBlockKind::Indented => None,
    }
}

fn code_block_border_text(label: Option<&str>, top: bool, width: usize) -> String {
    let (left, right) = if top { ('╭', '╮') } else { ('╰', '╯') };
    if width <= 1 {
        return left.to_string();
    }

    let inner_width = width.saturating_sub(2);
    let mut inner = String::new();
    if top {
        if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
            let max_label_width = inner_width.saturating_sub(4);
            if max_label_width > 0 {
                inner.push('─');
                inner.push(' ');
                inner.push_str(&truncate_to_width(label, max_label_width));
                inner.push(' ');
            }
        }
    }

    let used = inner.width();
    if used < inner_width {
        inner.push_str(&"─".repeat(inner_width - used));
    } else if used > inner_width {
        inner = truncate_to_width(&inner, inner_width);
    }
    format!("{left}{inner}{right}")
}

fn heading_border_line(left: char, right: char, label: &str, width: usize) -> String {
    let width = width.max(2);
    let mut line = String::new();
    line.push(left);
    if !label.is_empty() && width > 4 {
        line.push('─');
        line.push_str(&truncate_to_width(label, width.saturating_sub(4)));
    }
    let used = line.width();
    if used < width.saturating_sub(1) {
        line.push_str(&"─".repeat(width.saturating_sub(1) - used));
    }
    line.push(right);
    truncate_to_width(&line, width)
}

fn prefixed_line(mut line: Line<'static>, prefix: Line<'static>) -> Line<'static> {
    let mut out = prefix;
    apply_line_style_to_spans(&mut line);
    out.spans.append(&mut line.spans);
    out
}

fn apply_line_style_to_spans(line: &mut Line<'static>) {
    for span in &mut line.spans {
        span.style = line.style.patch(span.style);
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

#[derive(Debug)]
struct SyntectAssets {
    syntaxes: SyntaxSet,
}

static SYNTECT_ASSETS: OnceLock<SyntectAssets> = OnceLock::new();

fn syntect_assets() -> &'static SyntectAssets {
    SYNTECT_ASSETS.get_or_init(|| {
        let assets = HighlightingAssets::from_binary();
        let syntaxes = assets
            .get_syntax_set()
            .map_or_else(|_| SyntaxSet::load_defaults_newlines(), Clone::clone);
        SyntectAssets { syntaxes }
    })
}

fn syntect_syntax_set() -> &'static SyntaxSet {
    &syntect_assets().syntaxes
}

struct CodeHighlighter {
    parse_state: ParseState,
    scope_stack: ScopeStack,
}

impl CodeHighlighter {
    fn new(lang: &str) -> Self {
        let syntaxes = syntect_syntax_set();
        let syntax = syntax_for(syntaxes, lang);
        Self {
            parse_state: ParseState::new(syntax),
            scope_stack: ScopeStack::new(),
        }
    }

    fn highlight_line(&mut self, line: &str, styles: MarkdownStyles) -> Vec<Span<'static>> {
        let Ok(ops) = self.parse_state.parse_line(line, syntect_syntax_set()) else {
            return vec![Span::styled(line.to_string(), styles.code)];
        };
        let mut spans = Vec::new();
        for (region, op) in ScopeRegionIterator::new(&ops, line) {
            let _ = self.scope_stack.apply(op);
            if region.is_empty() {
                continue;
            }
            spans.push(Span::styled(
                region.to_string(),
                style_for_scope_stack(&self.scope_stack, styles),
            ));
        }
        if spans.is_empty() {
            vec![Span::styled(String::new(), styles.code)]
        } else {
            spans
        }
    }
}

fn syntax_for<'a>(syntax_set: &'a SyntaxSet, lang: &str) -> &'a SyntaxReference {
    let lang = lang.trim().trim_start_matches('.');
    if lang.is_empty() {
        return syntax_set.find_syntax_plain_text();
    }

    syntax_set
        .find_syntax_by_token(lang)
        .or_else(|| syntax_set.find_syntax_by_extension(lang))
        .or_else(|| syntax_for_common_alias(syntax_set, lang))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text())
}

fn syntax_for_common_alias<'a>(
    syntax_set: &'a SyntaxSet,
    lang: &str,
) -> Option<&'a SyntaxReference> {
    let extension = match lang.to_ascii_lowercase().as_str() {
        "c++" => "cpp",
        "c#" => "cs",
        "dockerfile" => "Dockerfile",
        "golang" => "go",
        "js" | "node" => "js",
        "jsx" | "react" => "jsx",
        "md" | "mdx" => "md",
        "py" => "py",
        "shell" => "sh",
        "typescriptreact" => "tsx",
        "yml" => "yaml",
        _ => return None,
    };
    syntax_set
        .find_syntax_by_extension(extension)
        .or_else(|| syntax_set.find_syntax_by_token(extension))
}

fn style_for_scope_stack(scope_stack: &ScopeStack, styles: MarkdownStyles) -> Style {
    let scopes = scope_stack.to_string();
    if scopes.contains("comment") {
        return styles.syntax_comment.add_modifier(Modifier::ITALIC);
    }
    if scopes.contains("string") || scopes.contains("constant.character") {
        return styles.syntax_string;
    }
    if scopes.contains("constant.numeric") {
        return styles.syntax_number;
    }
    if scopes.contains("keyword.operator") || scopes.contains("punctuation.operator") {
        return styles.syntax_operator;
    }
    if scopes.contains("keyword") || scopes.contains("storage") {
        return styles.syntax_keyword.add_modifier(Modifier::BOLD);
    }
    if scopes.contains("entity.name.function")
        || scopes.contains("support.function")
        || scopes.contains("variable.function")
    {
        return styles.syntax_function;
    }
    if scopes.contains("entity.name.type")
        || scopes.contains("support.type")
        || scopes.contains("storage.type")
    {
        return styles.syntax_type;
    }
    if scopes.contains("variable")
        || scopes.contains("entity.name")
        || scopes.contains("constant.language")
        || scopes.contains("constant.other")
    {
        return styles.syntax_variable;
    }
    if scopes.contains("punctuation") {
        return styles.syntax_punctuation;
    }
    styles.code
}

fn unwrap_markdown_table_fences(markdown: &str) -> Cow<'_, str> {
    if !markdown.contains("```") && !markdown.contains("~~~") {
        return Cow::Borrowed(markdown);
    }

    #[derive(Debug)]
    enum ActiveFence {
        Passthrough(FenceLine),
        MarkdownCandidate {
            fence: FenceLine,
            opening: String,
            body: String,
        },
    }

    let mut out = String::with_capacity(markdown.len());
    let mut active: Option<ActiveFence> = None;

    for line in markdown.split_inclusive('\n') {
        if let Some(current) = active.take() {
            match current {
                ActiveFence::Passthrough(fence) => {
                    out.push_str(line);
                    if !is_closing_fence_line(line, fence) {
                        active = Some(ActiveFence::Passthrough(fence));
                    }
                }
                ActiveFence::MarkdownCandidate {
                    fence,
                    opening,
                    mut body,
                } => {
                    if is_closing_fence_line(line, fence) {
                        if markdown_fence_body_contains_table(&body, fence.blockquoted) {
                            out.push_str(&body);
                        } else {
                            out.push_str(&opening);
                            out.push_str(&body);
                            out.push_str(line);
                        }
                    } else {
                        body.push_str(line);
                        active = Some(ActiveFence::MarkdownCandidate {
                            fence,
                            opening,
                            body,
                        });
                    }
                }
            }
            continue;
        }

        if let Some((fence, is_markdown)) = opening_fence_line(line) {
            if is_markdown {
                active = Some(ActiveFence::MarkdownCandidate {
                    fence,
                    opening: line.to_string(),
                    body: String::new(),
                });
            } else {
                out.push_str(line);
                active = Some(ActiveFence::Passthrough(fence));
            }
        } else {
            out.push_str(line);
        }
    }

    if let Some(current) = active {
        match current {
            ActiveFence::Passthrough(_) => {}
            ActiveFence::MarkdownCandidate { opening, body, .. } => {
                out.push_str(&opening);
                out.push_str(&body);
            }
        }
    }

    Cow::Owned(out)
}

fn opening_fence_line(line: &str) -> Option<(FenceLine, bool)> {
    let scanned = strip_fence_line_prefix(line)?;
    let (marker, len) = parse_fence_marker(scanned)?;
    let info = scanned[len..].trim();
    let is_markdown = info
        .split_whitespace()
        .next()
        .is_some_and(|token| matches!(token, "md" | "markdown"));
    Some((
        FenceLine {
            marker,
            len,
            blockquoted: line_is_blockquoted_fence(line),
        },
        is_markdown,
    ))
}

#[derive(Debug, Clone, Copy)]
struct FenceLine {
    marker: char,
    len: usize,
    blockquoted: bool,
}

fn is_closing_fence_line(line: &str, fence: FenceLine) -> bool {
    let Some(scanned) = strip_fence_line_prefix(line) else {
        return false;
    };
    if fence.blockquoted != line_is_blockquoted_fence(line) {
        return false;
    }
    parse_fence_marker(scanned).is_some_and(|(marker, len)| {
        marker == fence.marker && len >= fence.len && scanned[len..].trim().is_empty()
    })
}

fn strip_fence_line_prefix(line: &str) -> Option<&str> {
    let without_newline = line.strip_suffix('\n').unwrap_or(line);
    let mut byte_index = 0usize;
    let mut columns = 0usize;
    for ch in without_newline.chars() {
        match ch {
            ' ' if columns < 4 => {
                byte_index += ch.len_utf8();
                columns += 1;
            }
            '\t' if columns < 4 => {
                byte_index += ch.len_utf8();
                columns += 4;
            }
            _ => break,
        }
        if columns >= 4 {
            return None;
        }
    }

    let trimmed = &without_newline[byte_index..];
    Some(strip_blockquote_marker(trimmed))
}

fn strip_blockquote_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('>') else {
        return line;
    };
    rest.strip_prefix(' ').unwrap_or(rest)
}

fn line_is_blockquoted_fence(line: &str) -> bool {
    let without_newline = line.strip_suffix('\n').unwrap_or(line);
    without_newline.trim_start().starts_with('>')
}

fn parse_fence_marker(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let len = line.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some((marker, len))
}

fn markdown_fence_body_contains_table(body: &str, blockquoted: bool) -> bool {
    let mut previous: Option<String> = None;
    for line in body.lines() {
        let text = if blockquoted {
            strip_blockquote_marker(line)
        } else {
            line
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            previous = None;
            continue;
        }
        if let Some(header) = previous.as_deref() {
            if is_table_header_line(header) && is_table_delimiter_line(trimmed) {
                return true;
            }
        }
        previous = Some(trimmed.to_string());
    }
    false
}

fn is_table_header_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|') && !is_table_delimiter_line(trimmed)
}

fn is_table_delimiter_line(line: &str) -> bool {
    let trimmed = line.trim().trim_matches('|').trim();
    if trimmed.is_empty() || !trimmed.contains("---") {
        return false;
    }
    trimmed
        .split('|')
        .map(str::trim)
        .all(|cell| !cell.is_empty() && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
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

fn normalize_table_alignments(alignments: &[Alignment], column_count: usize) -> Vec<Alignment> {
    let mut normalized = alignments
        .iter()
        .copied()
        .take(column_count)
        .collect::<Vec<_>>();
    normalized.resize(column_count, Alignment::None);
    normalized
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

fn align_to_width(text: &str, width: usize, alignment: Alignment) -> String {
    let used = text.width();
    if used >= width {
        return text.to_string();
    }
    let padding = width - used;
    match alignment {
        Alignment::Right => format!("{}{text}", " ".repeat(padding)),
        Alignment::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
        }
        Alignment::Left | Alignment::None => format!("{text}{}", " ".repeat(padding)),
    }
}

fn wrap_spans_to_width(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = Line::default();
    let mut current_width = 0usize;
    let mut has_content = false;

    for span in spans {
        let style = span.style;
        for grapheme in span.content.as_ref().graphemes(true) {
            let grapheme_width = grapheme.width();
            if current_width.saturating_add(grapheme_width) > width && has_content {
                out.push(current);
                current = Line::default();
                current_width = 0;
                has_content = false;
            }
            push_coalesced_span(&mut current, grapheme, style);
            current_width = current_width.saturating_add(grapheme_width);
            has_content = has_content || current_width > 0 || grapheme_width == 0;
        }
    }

    if has_content || out.is_empty() {
        out.push(current);
    }
    out
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
    let line_style = line.style;
    for span in line.spans {
        let style = line_style.patch(span.style);
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

fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    // Consume CSI sequence: parameters (0x20-0x3f), intermediates (0x20-0x2f), final (0x40-0x7e)
                    while let Some(&next) = chars.peek() {
                        if (0x40..=0x7e).contains(&(next as u8)) {
                            chars.next();
                            break;
                        }
                        chars.next();
                    }
                }
                Some(']') => {
                    chars.next();
                    // Consume OSC sequence until BEL (0x07) or ST (ESC \)
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    // Other escape sequences (2-char)
                    chars.next();
                }
                None => {}
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

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

        assert!(plain_lines.iter().any(|line| line.contains("Title")));
        assert!(plain_lines.contains(&"• Bold item with code".to_string()));
        assert!(plain_lines
            .iter()
            .any(|line| line.contains("Oino ↗ https://example.invalid")));
        assert!(plain_lines.iter().any(|line| line.contains("fn main() {}")));
        assert!(!plain_lines.iter().any(|line| line.contains("**Bold**")));
        assert!(!plain_lines.iter().any(|line| line.contains("`code`")));
    }

    #[test]
    fn code_blocks_render_closed_right_border() {
        let lines = render_markdown_lines(
            "```rust\nfn main() {}\n```",
            40,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert!(plain_lines[0].ends_with('╮'), "{}", plain_lines[0]);
        assert!(plain_lines
            .iter()
            .any(|line| line.contains("fn main") && line.ends_with('│')));
        assert!(plain_lines.iter().any(|line| line.ends_with('╯')));
        assert!(plain_lines.iter().all(|line| line.width() <= 40));
    }

    #[test]
    fn headings_use_yellow_bold_render_test_style() {
        let theme = Theme::default();
        let styles = MarkdownStyles::new(Style::default(), &theme);

        assert_eq!(styles.heading.fg, Some(theme.markdown_heading));
        assert_eq!(
            styles.heading_secondary.fg,
            Some(theme.markdown_heading_secondary)
        );
        assert!(styles.heading.add_modifier.contains(Modifier::BOLD));
        assert!(styles
            .heading_secondary
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(!styles.heading.add_modifier.contains(Modifier::UNDERLINED));
        assert!(!styles
            .heading_secondary
            .add_modifier
            .contains(Modifier::UNDERLINED));
    }

    #[test]
    fn markdown_component_roles_control_visible_spans() {
        let theme = Theme {
            markdown_heading: Color::Blue,
            markdown_heading_secondary: Color::Magenta,
            markdown_link: Color::Green,
            markdown_list_marker: Color::Yellow,
            markdown_code_border: Color::Red,
            markdown_code_line_number: Color::Cyan,
            ..Theme::default()
        };
        let lines = render_markdown_lines(
            "## Heading\n\n- [Oino](https://example.invalid)\n\n```rust\nfn main() {}\n```",
            80,
            Style::default(),
            &theme,
        );

        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains("Heading") && span.style.fg == Some(Color::Magenta)
            })
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains("Oino") && span.style.fg == Some(Color::Green)
            })
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains('•') && span.style.fg == Some(Color::Yellow)
            })
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains('╭') && span.style.fg == Some(Color::Red)
            })
        }));
    }

    #[test]
    fn syntax_component_roles_control_code_spans() {
        let theme = Theme {
            syntax_keyword: Color::Blue,
            syntax_function: Color::Magenta,
            syntax_string: Color::Green,
            syntax_comment: Color::Yellow,
            syntax_operator: Color::Red,
            syntax_punctuation: Color::Cyan,
            ..Theme::default()
        };
        let lines = render_markdown_lines(
            "```rust\nfn main() { let name = \"oino\"; // hi\n}\n```",
            80,
            Style::default(),
            &theme,
        );

        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains("fn") && span.style.fg == Some(Color::Blue)
            })
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains("main") && span.style.fg == Some(Color::Magenta)
            })
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains("oino") && span.style.fg == Some(Color::Green)
            })
        }));
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref().contains("// hi") && span.style.fg == Some(Color::Yellow)
            })
        }));
    }

    #[test]
    fn h1_banner_does_not_repeat_title_or_show_hash_badge() {
        let lines = render_markdown_lines("# H1 heading", 40, Style::default(), &Theme::default());
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert!(!plain_lines[0].contains('#'));
        assert!(!plain_lines[0].contains("H1 heading"));
        assert_eq!(
            plain_lines
                .iter()
                .filter(|line| line.contains("H1 heading"))
                .count(),
            1
        );
    }

    #[test]
    fn h2_and_h3_use_yellow_left_rail_not_bullets() {
        let theme = Theme::default();
        let lines = render_markdown_lines(
            "## H2 heading\n\n### H3 heading",
            80,
            Style::default(),
            &theme,
        );
        let Some(h2) = lines.iter().find(|line| plain(line).contains("H2 heading")) else {
            panic!("h2 line");
        };
        let Some(h3) = lines.iter().find(|line| plain(line).contains("H3 heading")) else {
            panic!("h3 line");
        };

        for line in [h2, h3] {
            let text = plain(line);
            assert!(text.starts_with("▌ "));
            assert!(!text.starts_with("◆ "));
            assert!(!text.starts_with("• "));
            assert!(line.spans.iter().any(|span| {
                span.content.as_ref().contains('▌')
                    && span.style.fg == Some(theme.markdown_heading_secondary)
                    && span.style.add_modifier.contains(Modifier::BOLD)
            }));
            assert!(line.spans.iter().any(|span| {
                span.content.as_ref().contains("heading")
                    && span.style.fg == Some(theme.markdown_heading_secondary)
                    && span.style.add_modifier.contains(Modifier::BOLD)
            }));
        }
    }

    #[test]
    fn syntect_highlights_common_languages_beyond_old_manual_set() {
        let styles = MarkdownStyles::new(Style::default(), &Theme::default());
        let assets = syntect_assets();
        let samples = [
            ("json", "{ \"name\": true }"),
            ("toml", "name = \"oino\""),
            ("yaml", "name: oino"),
            ("sql", "select * from sessions where id = 1"),
            ("go", "func main() { fmt.Println(\"hi\") }"),
        ];

        for (lang, sample) in samples {
            let syntax = syntax_for(&assets.syntaxes, lang);
            assert_ne!(
                syntax.name, "Plain Text",
                "{lang} should resolve via syntect"
            );

            let mut highlighter = CodeHighlighter::new(lang);
            let spans = highlighter.highlight_line(sample, styles);
            assert_eq!(
                spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>(),
                sample
            );
            assert!(
                spans.iter().any(|span| {
                    !span.content.as_ref().trim().is_empty() && span.style.fg != styles.code.fg
                }),
                "{lang} should produce colored spans: {spans:?}"
            );
        }
    }

    #[test]
    fn renders_task_lists_as_colored_status_markers() {
        let lines = render_markdown_lines(
            "- [x] completed\n- [ ] incomplete",
            80,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert!(plain_lines.contains(&"✓ completed".to_string()));
        assert!(plain_lines.contains(&"○ incomplete".to_string()));
        assert!(!plain_lines.iter().any(|line| line.contains("☑")));
        assert!(!plain_lines.iter().any(|line| line.contains("☐")));
        assert!(!plain_lines.iter().any(|line| line.contains("• [")));
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
    fn renders_code_blocks_with_labelled_border() {
        let lines = render_markdown_lines(
            "```rust,no_run\nfn main() {}\n```",
            32,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert!(plain_lines
            .first()
            .is_some_and(|line| line.starts_with("╭─ rust")));
        assert!(plain_lines
            .iter()
            .any(|line| line.starts_with("│ 1 │ fn main() {}") && line.ends_with('│')));
        assert!(plain_lines
            .iter()
            .any(|line| line.starts_with('╰') && line.ends_with('╯')));
        assert!(plain_lines.iter().all(|line| line.width() <= 32));
    }

    #[test]
    fn unwraps_markdown_fence_tables_before_rendering() {
        let lines = render_markdown_lines(
            "```markdown\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n",
            40,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();
        let joined = plain_lines.join("\n");

        assert!(plain_lines.iter().any(|line| line.starts_with('┌')));
        assert!(joined.contains("│ 1"));
        assert!(!joined.contains("```"));
    }

    #[test]
    fn keeps_non_table_markdown_fences_as_code() {
        let lines = render_markdown_lines(
            "```markdown\n**bold**\n```\n",
            40,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();
        let joined = plain_lines.join("\n");

        assert!(plain_lines
            .first()
            .is_some_and(|line| line.starts_with("╭─ markdown")));
        assert!(joined.contains("│ **bold**"));
        assert!(!plain_lines.iter().any(|line| line.starts_with('┌')));
    }

    #[test]
    fn honors_markdown_table_alignment() {
        assert_eq!(align_to_width("left", 6, Alignment::Left), "left  ");
        assert_eq!(align_to_width("7", 4, Alignment::Right), "   7");
        assert_eq!(align_to_width("ok", 4, Alignment::Center), " ok ");

        let lines = render_markdown_lines(
            "| Item | Qty | Note |\n| :--- | ---: | :---: |\n| A | 7 | ok |",
            80,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();
        let row = plain_lines
            .iter()
            .find(|line| line.contains('A') && line.contains('7') && line.contains("ok"))
            .unwrap_or_else(|| panic!("missing aligned table row: {plain_lines:?}"));

        assert!(row.contains("│ A"));
        assert!(row.contains("│    7 │"));
        assert!(row.contains("│  ok  │"));
    }

    #[test]
    fn renders_mermaid_block_as_unicode_diagram() {
        let lines = render_markdown_lines(
            "```mermaid\ngraph LR\n    A-->B\n```",
            60,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();
        let joined = plain_lines.join("\n");

        // Should have the "mermaid diagram" label in the border
        assert!(plain_lines
            .first()
            .is_some_and(|line| line.contains("mermaid diagram")));

        // Should contain rendered Unicode box-drawing characters (not raw "graph LR")
        assert!(joined.contains('┌') || joined.contains('│'));
        assert!(!joined.contains("graph LR"));
        assert!(!joined.contains("A-->B"));

        // Should contain node labels from the diagram
        assert!(joined.contains('A'));
        assert!(joined.contains('B'));

        // All lines should respect width
        assert!(plain_lines.iter().all(|line| line.width() <= 60));
    }

    #[test]
    fn mermaid_block_with_invalid_syntax_shows_error() {
        let lines = render_markdown_lines(
            "```mermaid\nnot a valid diagram at all xyz!!!\n```",
            60,
            Style::default(),
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();
        let _joined = plain_lines.join("\n");

        // Should still render with a border
        assert!(plain_lines
            .first()
            .is_some_and(|line| line.contains("mermaid diagram")));

        // Should show error or gracefully degrade
        // (mmdflux may render partial or error — either way no panic)
        assert!(plain_lines
            .iter()
            .any(|line| line.ends_with('│') || line.contains("error")));
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
