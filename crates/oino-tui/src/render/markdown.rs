//! Markdown → styled lines (pi-tui `Markdown` with pi's markdown theme).

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::highlight::highlight;
use super::text::{OLine, Run, wrap};
use crate::theme::Theme;

/// Base text styling for a markdown block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MdStyle {
    /// Default text style (colour, italic for thinking, bg for user messages).
    pub(crate) base: Style,
}

enum Container {
    Quote,
    List { ordered: Option<u64> },
    Item { marker: Option<String> },
}

#[derive(Default)]
struct Inline {
    emphasis: usize,
    strong: usize,
    strike: usize,
    link: Option<String>,
    heading: Option<HeadingLevel>,
}

struct Table {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<Vec<Run>>>,
    current_row: Vec<Vec<Run>>,
    cell: Vec<Run>,
}

struct Renderer<'t> {
    theme: &'t Theme,
    base: Style,
    width: usize,
    out: Vec<OLine>,
    containers: Vec<Container>,
    inline: Inline,
    runs: Vec<Run>,
    code: Option<(Option<String>, String)>,
    table: Option<Table>,
    pending_blank: bool,
}

impl Renderer<'_> {
    fn inline_style(&self) -> Style {
        let mut style = self.base;
        if let Some(level) = self.inline.heading {
            style = style.fg(self.theme.md_heading).add_modifier(Modifier::BOLD);
            if matches!(level, HeadingLevel::H1) {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
        }
        if self.inline.emphasis > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.inline.strong > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.inline.strike > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.inline.link.is_some() {
            style = style
                .fg(self.theme.md_link)
                .add_modifier(Modifier::UNDERLINED);
        }
        style
    }

    fn prefixes(&mut self) -> (Vec<Run>, Vec<Run>) {
        let mut first = Vec::new();
        let mut rest = Vec::new();
        let quote = Style::default().fg(self.theme.md_quote_border);
        let bullet = Style::default().fg(self.theme.md_list_bullet);
        for container in &mut self.containers {
            match container {
                Container::Quote => {
                    first.push(Run::new("│ ", quote));
                    rest.push(Run::new("│ ", quote));
                }
                Container::List { .. } => {}
                Container::Item { marker } => {
                    let width = marker.as_ref().map_or(2, |m| m.width());
                    match marker.take() {
                        Some(m) => first.push(Run::new(m, bullet)),
                        None => first.push(Run::new(" ".repeat(width.max(2)), Style::default())),
                    }
                    rest.push(Run::new(" ".repeat(width.max(2)), Style::default()));
                    // Keep the width for continuation blocks of this item.
                    *marker = None;
                }
            }
        }
        (first, rest)
    }

    fn blank_line(&mut self) {
        if self.pending_blank && !self.out.is_empty() {
            let (_, rest) = self.prefixes_peek();
            let spans: Vec<Span<'static>> = rest
                .into_iter()
                .filter(|r| r.text.trim() == "│")
                .map(|r| Span::styled(r.text, r.style))
                .collect();
            self.out.push(Line::from(spans));
        }
        self.pending_blank = false;
    }

    fn prefixes_peek(&self) -> (Vec<Run>, Vec<Run>) {
        let quote = Style::default().fg(self.theme.md_quote_border);
        let rest = self
            .containers
            .iter()
            .filter(|c| matches!(c, Container::Quote))
            .map(|_| Run::new("│ ", quote))
            .collect::<Vec<_>>();
        (rest.clone(), rest)
    }

    fn flush_paragraph(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        self.blank_line();
        let runs = std::mem::take(&mut self.runs);
        let (first, rest) = self.prefixes();
        let quoted = self
            .containers
            .iter()
            .any(|c| matches!(c, Container::Quote));
        let runs: Vec<Run> = if quoted {
            runs.into_iter()
                .map(|r| {
                    Run::new(
                        r.text,
                        r.style
                            .fg(self.theme.md_quote)
                            .add_modifier(Modifier::ITALIC),
                    )
                })
                .collect()
        } else {
            runs
        };
        self.out.extend(wrap(&runs, self.width, &first, &rest));
        self.pending_blank = true;
    }

    fn flush_code(&mut self, lang: Option<&str>, code: &str) {
        self.blank_line();
        let (first, rest) = self.prefixes();
        let border = Style::default().fg(self.theme.md_code_block_border);
        let prefix_spans = |runs: &[Run]| -> Vec<Span<'static>> {
            runs.iter()
                .map(|r| Span::styled(r.text.clone(), r.style))
                .collect()
        };
        let mut top = prefix_spans(&first);
        top.push(Span::styled(
            format!("```{}", lang.unwrap_or_default()),
            border,
        ));
        self.out.push(Line::from(top));
        let code_style = self.base.fg(self.theme.md_code_block);
        for spans in highlight(code.trim_end_matches('\n'), lang, self.theme, code_style) {
            let mut line = prefix_spans(&rest);
            line.push(Span::raw("  "));
            line.extend(spans);
            self.out.push(Line::from(line));
        }
        let mut bottom = prefix_spans(&rest);
        bottom.push(Span::styled("```", border));
        self.out.push(Line::from(bottom));
        self.pending_blank = true;
    }

    fn flush_table(&mut self, table: &Table) {
        self.blank_line();
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let cell_text = |cell: &[Run]| cell.iter().map(|r| r.text.as_str()).collect::<String>();
        let mut widths = vec![0usize; columns];
        for row in &table.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell_text(cell).width());
            }
        }
        let total: usize = widths.iter().sum::<usize>() + 3 * columns + 1;
        if total > self.width {
            let budget = self.width.saturating_sub(3 * columns + 1).max(columns);
            let each = (budget / columns).max(1);
            for w in &mut widths {
                *w = (*w).min(each);
            }
        }
        let border = Style::default().fg(self.theme.md_code_block_border);
        let rule = |l: &str, m: &str, r: &str| {
            let mut s = l.to_owned();
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                s.push_str(if i + 1 == widths.len() { r } else { m });
            }
            Line::from(Span::styled(s, border))
        };
        self.out.push(rule("┌", "┬", "┐"));
        for (r, row) in table.rows.iter().enumerate() {
            let mut spans = vec![Span::styled("│", border)];
            for (i, width) in widths.iter().enumerate() {
                let text = row.get(i).map(|c| cell_text(c)).unwrap_or_default();
                let text = oino_view::format::truncate(&text, *width, "…");
                let pad = width.saturating_sub(text.width());
                let (left, right) = match table.alignments.get(i) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                let style = row
                    .get(i)
                    .and_then(|c| c.first())
                    .map_or(self.base, |run| run.style);
                let style = if r == 0 {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                };
                spans.push(Span::raw(format!(" {}", " ".repeat(left))));
                spans.push(Span::styled(text, style));
                spans.push(Span::raw(format!("{} ", " ".repeat(right))));
                spans.push(Span::styled("│", border));
            }
            self.out.push(Line::from(spans));
            if r == 0 && table.rows.len() > 1 {
                self.out.push(rule("├", "┼", "┤"));
            }
        }
        self.out.push(rule("└", "┴", "┘"));
        self.pending_blank = true;
    }

    fn push_text(&mut self, text: &str) {
        let style = self.inline_style();
        if let Some(table) = &mut self.table {
            table.cell.push(Run::new(text, style));
        } else {
            self.runs.push(Run::new(text, style));
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per pulldown-cmark event kind"
    )]
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    self.flush_paragraph();
                    self.inline.heading = Some(level);
                    if !matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
                        let hashes = "#".repeat(heading_depth(level));
                        self.push_text(&format!("{hashes} "));
                    }
                }
                Tag::BlockQuote(_) => {
                    self.flush_paragraph();
                    self.blank_line();
                    self.containers.push(Container::Quote);
                }
                Tag::CodeBlock(kind) => {
                    self.flush_paragraph();
                    let lang = match kind {
                        CodeBlockKind::Fenced(lang) => {
                            let lang = lang.split_whitespace().next().unwrap_or("").to_owned();
                            (!lang.is_empty()).then_some(lang)
                        }
                        CodeBlockKind::Indented => None,
                    };
                    self.code = Some((lang, String::new()));
                }
                Tag::List(start) => {
                    self.flush_paragraph();
                    self.blank_line();
                    self.containers.push(Container::List { ordered: start });
                }
                Tag::Item => {
                    self.flush_paragraph();
                    let marker = match self.containers.iter_mut().rev().find_map(|c| match c {
                        Container::List { ordered } => Some(ordered),
                        _ => None,
                    }) {
                        Some(Some(n)) => {
                            let marker = format!("{n}. ");
                            *n += 1;
                            marker
                        }
                        _ => "- ".to_owned(),
                    };
                    self.containers.push(Container::Item {
                        marker: Some(marker),
                    });
                    self.pending_blank = false;
                }
                Tag::Emphasis => self.inline.emphasis += 1,
                Tag::Strong => self.inline.strong += 1,
                Tag::Strikethrough => self.inline.strike += 1,
                Tag::Link { dest_url, .. } => self.inline.link = Some(dest_url.to_string()),
                Tag::Image { dest_url, .. } => {
                    self.push_text(&format!("[image: {dest_url}]"));
                }
                Tag::Table(alignments) => {
                    self.flush_paragraph();
                    self.table = Some(Table {
                        alignments,
                        rows: Vec::new(),
                        current_row: Vec::new(),
                        cell: Vec::new(),
                    });
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => self.flush_paragraph(),
                TagEnd::Heading(_) => {
                    self.flush_paragraph();
                    self.inline.heading = None;
                }
                TagEnd::BlockQuote(_) | TagEnd::List(_) => {
                    self.flush_paragraph();
                    self.containers.pop();
                    self.pending_blank = true;
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, code)) = self.code.take() {
                        self.flush_code(lang.as_deref(), &code);
                    }
                }
                TagEnd::Item => {
                    self.flush_paragraph();
                    self.containers.pop();
                    self.pending_blank = false;
                }
                TagEnd::Emphasis => self.inline.emphasis = self.inline.emphasis.saturating_sub(1),
                TagEnd::Strong => self.inline.strong = self.inline.strong.saturating_sub(1),
                TagEnd::Strikethrough => self.inline.strike = self.inline.strike.saturating_sub(1),
                TagEnd::Link => {
                    if let Some(url) = self.inline.link.take() {
                        let shown: String = self
                            .runs
                            .iter()
                            .rev()
                            .take(1)
                            .map(|r| r.text.clone())
                            .collect();
                        if !url.is_empty() && shown != url && !url.starts_with('#') {
                            let style = self.base.fg(self.theme.md_link_url);
                            self.runs.push(Run::new(format!(" ({url})"), style));
                        }
                    }
                }
                TagEnd::TableCell => {
                    if let Some(table) = &mut self.table {
                        let cell = std::mem::take(&mut table.cell);
                        table.current_row.push(cell);
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    if let Some(table) = &mut self.table {
                        let row = std::mem::take(&mut table.current_row);
                        table.rows.push(row);
                    }
                }
                TagEnd::Table => {
                    if let Some(table) = self.table.take() {
                        self.flush_table(&table);
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some((_, code)) = &mut self.code {
                    code.push_str(&text);
                } else {
                    self.push_text(&text);
                }
            }
            Event::Code(code) => {
                let style = self.base.fg(self.theme.md_code);
                if let Some(table) = &mut self.table {
                    table.cell.push(Run::new(code.to_string(), style));
                } else {
                    self.runs.push(Run::new(code.to_string(), style));
                }
            }
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.push_text("\n"),
            Event::Rule => {
                self.flush_paragraph();
                self.blank_line();
                self.out.push(Line::from(Span::styled(
                    "─".repeat(self.width),
                    Style::default().fg(self.theme.md_hr),
                )));
                self.pending_blank = true;
            }
            Event::TaskListMarker(done) => {
                let style = if done {
                    Style::default().fg(self.theme.success)
                } else {
                    Style::default().fg(self.theme.muted)
                };
                self.runs
                    .push(Run::new(if done { "[x] " } else { "[ ] " }, style));
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push_text(&html),
            Event::FootnoteReference(name) => self.push_text(&format!("[^{name}]")),
            Event::InlineMath(m) | Event::DisplayMath(m) => self.push_text(&m),
        }
    }
}

const fn heading_depth(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Renders markdown to lines of at most `width` cells.
#[must_use]
pub(crate) fn render(text: &str, width: usize, style: MdStyle, theme: &Theme) -> Vec<OLine> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut renderer = Renderer {
        theme,
        base: style.base,
        width: width.max(4),
        out: Vec::new(),
        containers: Vec::new(),
        inline: Inline::default(),
        runs: Vec::new(),
        code: None,
        table: None,
        pending_blank: false,
    };
    for event in Parser::new_ext(text, options) {
        renderer.event(event);
    }
    if let Some((lang, code)) = renderer.code.take() {
        // Unterminated fence while streaming.
        renderer.flush_code(lang.as_deref(), &code);
    }
    renderer.flush_paragraph();
    renderer.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[OLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn renders_blocks() {
        let theme = Theme::default();
        let style = MdStyle {
            base: Style::default(),
        };
        let out = plain(&render(
            "# Title\n\nSome *text* and `code`.\n\n- one\n- two\n\n```rust\nfn x() {}\n```\n\n> quoted",
            40,
            style,
            &theme,
        ));
        assert_eq!(
            out,
            vec![
                "Title",
                "",
                "Some text and code.",
                "",
                "- one",
                "- two",
                "",
                "```rust",
                "  fn x() {}",
                "```",
                "",
                "│ quoted",
            ]
        );
        let table = plain(&render(
            "| a | b |\n|---|--:|\n| 1 | 22 |",
            40,
            style,
            &theme,
        ));
        assert_eq!(table[0], "┌───┬────┐");
        assert_eq!(table[3], "│ 1 │ 22 │");
    }
}
