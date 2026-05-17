#![forbid(unsafe_code)]

use std::{
    io::{self, Stdout, Write},
    time::Duration,
};

use crossterm::{
    cursor::MoveTo,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute, queue,
    style::{
        Attribute as CAttribute, Color as CColor, Print, ResetColor, SetAttribute,
        SetForegroundColor,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const TICK_RATE: Duration = Duration::from_millis(120);
const MIN_WIDTH: u16 = 32;
const MIN_HEIGHT: u16 = 10;
const MAX_READING_WIDTH: u16 = 112;

fn main() -> io::Result<()> {
    let _terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_app(&mut terminal);
    let _ = terminal.show_cursor();
    result
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

#[derive(Debug, Default)]
struct DemoApp {
    scroll: usize,
    total_lines: usize,
    visible_lines: usize,
    link_overlays: Vec<VisibleLink>,
}

#[derive(Debug, Clone)]
struct VisibleLink {
    x: u16,
    y: u16,
    label: &'static str,
    url: &'static str,
}

impl DemoApp {
    fn max_scroll(&self) -> usize {
        self.total_lines.saturating_sub(self.visible_lines)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn scroll_by(&mut self, delta: isize) {
        if delta.is_negative() {
            self.scroll = self.scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll = self
                .scroll
                .saturating_add(delta as usize)
                .min(self.max_scroll());
        }
    }

    fn page(&mut self, down: bool) {
        let amount = self.visible_lines.saturating_sub(2).max(1) as isize;
        self.scroll_by(if down { amount } else { -amount });
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut app = DemoApp::default();
    loop {
        terminal.draw(|frame| render(frame, &mut app))?;
        paint_link_overlays(terminal.backend_mut(), &app)?;
        if event::poll(TICK_RATE)? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match (key.code, key.modifiers) {
                (KeyCode::Char('q') | KeyCode::Esc, _) => break,
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                (KeyCode::Char('j') | KeyCode::Down, _) => app.scroll_by(1),
                (KeyCode::Char('k') | KeyCode::Up, _) => app.scroll_by(-1),
                (KeyCode::PageDown, _) => app.page(true),
                (KeyCode::PageUp, _) => app.page(false),
                (KeyCode::Home, _) => app.scroll = 0,
                (KeyCode::End, _) => app.scroll = app.max_scroll(),
                _ => {}
            }
        }
    }
    Ok(())
}

fn paint_link_overlays(backend: &mut CrosstermBackend<Stdout>, app: &DemoApp) -> io::Result<()> {
    for link in &app.link_overlays {
        queue!(
            backend,
            MoveTo(link.x, link.y),
            SetForegroundColor(CColor::Blue),
            SetAttribute(CAttribute::Underlined),
            Print(osc8_link(link.label, link.url)),
            SetAttribute(CAttribute::NoUnderline),
            ResetColor
        )?;
    }
    backend.flush()
}

fn render(frame: &mut Frame<'_>, app: &mut DemoApp) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    "Oino markdown render-test",
                    Style::default().fg(Color::Cyan),
                ),
                Line::from("Terminal is too small."),
                Line::from("Resize to at least 32x10."),
                Line::styled("q/Esc exits", Style::default().fg(Color::DarkGray)),
            ]),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(area);

    render_header(frame, chunks[0]);
    render_showcase(frame, chunks[1], app);
    render_footer(frame, chunks[2], app);
}

fn render_header(frame: &mut Frame<'_>, area: Rect) {
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let subtitle_style = Style::default().fg(Color::DarkGray);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" Oino ", title_style),
                Span::styled("Ratatui markdown render-test", title_style),
                Span::styled("  proof-of-concept renderer", subtitle_style),
            ]),
            Line::from(vec![
                Span::styled(" Shows stronger headings, OSC8 links, syntax code blocks, tables, quotes, lists, media placeholders ", subtitle_style),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

fn render_showcase(frame: &mut Frame<'_>, area: Rect, app: &mut DemoApp) {
    let block = Block::default()
        .title(Span::styled(
            " Rendered Markdown PoC ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(block, area);

    let viewport = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    if viewport.width == 0 || viewport.height == 0 {
        return;
    }

    let scrollbar_width = 1;
    let reading_width = viewport
        .width
        .saturating_sub(scrollbar_width)
        .clamp(1, MAX_READING_WIDTH);
    let x = viewport.x
        + viewport
            .width
            .saturating_sub(reading_width.saturating_add(scrollbar_width))
            / 2;
    let text_area = Rect {
        x,
        y: viewport.y,
        width: reading_width,
        height: viewport.height,
    };

    let showcase = showcase_lines(reading_width as usize);
    app.total_lines = showcase.lines.len();
    app.visible_lines = text_area.height as usize;
    app.clamp_scroll();
    app.link_overlays = visible_links(&showcase.links, text_area, app.scroll, app.visible_lines);

    let visible = showcase
        .lines
        .into_iter()
        .skip(app.scroll)
        .take(app.visible_lines)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(visible), text_area);

    if app.total_lines > app.visible_lines {
        let scrollbar_area = Rect {
            x: viewport.x + viewport.width.saturating_sub(1),
            y: viewport.y,
            width: 1,
            height: viewport.height,
        };
        let max_scroll = app.max_scroll();
        let mut state = ScrollbarState::new(max_scroll)
            .position(app.scroll.min(max_scroll))
            .viewport_content_length(app.visible_lines);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(Style::default().fg(Color::DarkGray))
                .thumb_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            scrollbar_area,
            &mut state,
        );
    }
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &DemoApp) {
    let percent = if app.max_scroll() == 0 {
        100
    } else {
        app.scroll.saturating_mul(100) / app.max_scroll().max(1)
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" q/Esc ", Style::default().fg(Color::Red)),
                Span::raw("quit  "),
                Span::styled(
                    " j/k ↑/↓ PgUp/PgDn Home/End ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("scroll  "),
                Span::styled(
                    "OSC8 links are clickable in supporting terminals",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::styled(
                format!(
                    " line {}-{} / {}  ·  {}% ",
                    app.scroll.saturating_add(1),
                    app.scroll
                        .saturating_add(app.visible_lines)
                        .min(app.total_lines),
                    app.total_lines,
                    percent,
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ])
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

#[derive(Debug)]
struct Showcase {
    lines: Vec<Line<'static>>,
    links: Vec<LinkTarget>,
}

#[derive(Debug, Clone)]
struct LinkTarget {
    line: usize,
    column: usize,
    label: &'static str,
    url: &'static str,
}

fn visible_links(
    links: &[LinkTarget],
    text_area: Rect,
    scroll: usize,
    visible_lines: usize,
) -> Vec<VisibleLink> {
    links
        .iter()
        .filter_map(|link| {
            if link.line < scroll || link.line >= scroll.saturating_add(visible_lines) {
                return None;
            }
            let label_width = link.label.width();
            if link.column.saturating_add(label_width) > text_area.width as usize {
                return None;
            }
            Some(VisibleLink {
                x: text_area.x.saturating_add(link.column as u16),
                y: text_area.y.saturating_add((link.line - scroll) as u16),
                label: link.label,
                url: link.url,
            })
        })
        .collect()
}

fn showcase_lines(width: usize) -> Showcase {
    let width = width.max(24);
    let theme = DemoTheme::default();
    let mut lines = Vec::new();
    let mut links = Vec::new();

    push_h1(
        &mut lines,
        "H1: Markdown should look like a document",
        width,
        &theme,
    );
    push_wrapped(
        &mut lines,
        "This proof-of-concept is intentionally separate from the production chat renderer. It explores Ratatui markdown blocks that are visually distinct even in a terminal: headings with structure, clickable OSC8 links, syntax-like code blocks, grid tables, block quotes, task lists, media placeholders, and scroll state.",
        width,
        theme.fg,
    );
    blank(&mut lines);

    push_h2(&mut lines, "H2: inline formatting", width, &theme);
    lines.push(Line::from(vec![
        Span::styled("Strong ", theme.fg),
        Span::styled("bold", theme.fg.add_modifier(Modifier::BOLD)),
        Span::styled(", ", theme.fg),
        Span::styled("italic", theme.fg.add_modifier(Modifier::ITALIC)),
        Span::styled(", ", theme.fg),
        Span::styled(
            "bold italic",
            theme
                .fg
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(", ", theme.fg),
        Span::styled("struck", theme.fg.add_modifier(Modifier::CROSSED_OUT)),
        Span::styled(", inline ", theme.fg),
        Span::styled(" code ", theme.inline_code),
        Span::styled(".", theme.fg),
    ]));
    let link_line = lines.len();
    let link_prefix = "Link: ";
    links.push(LinkTarget {
        line: link_line,
        column: link_prefix.width(),
        label: "Oino repository",
        url: "https://github.com",
    });
    lines.push(Line::from(vec![
        Span::styled(link_prefix, theme.muted),
        Span::styled("Oino repository", theme.link),
        Span::styled("  ← OSC8 clickable overlay", theme.muted),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Fallback URL: ", theme.muted),
        Span::styled("https://github.com", theme.link),
    ]));
    blank(&mut lines);

    push_h3(&mut lines, "H3: lists and tasks", width, &theme);
    push_list_item(
        &mut lines,
        "•",
        "Bullets use a visible marker and align wrapped continuations.",
        width,
        &theme,
    );
    push_list_item(
        &mut lines,
        "1.",
        "Ordered lists keep the number gutter stable.",
        width,
        &theme,
    );
    push_list_item(&mut lines, "✓", "Completed task item", width, &theme);
    push_list_item(&mut lines, "○", "Incomplete task item", width, &theme);
    blank(&mut lines);

    push_h2(&mut lines, "Block quote, note, and rule", width, &theme);
    push_quote(
        &mut lines,
        "Block quotes should be visibly quoted, not just normal text. Nested content can share the same left rail and muted/italic treatment.",
        width,
        &theme,
    );
    push_note(
        &mut lines,
        "Design note",
        "Links can be made clickable with OSC8 escape sequences, but the production renderer should expose this as a deliberate terminal-capability feature with a visible URL fallback.",
        width,
        &theme,
    );
    push_rule(&mut lines, width, &theme);

    push_h2(
        &mut lines,
        "Code block with language badge, line numbers, and syntax color",
        width,
        &theme,
    );
    push_code_block(
        &mut lines,
        "rust",
        r#"use ratatui::text::{Line, Span};

fn render_link(label: &str, url: &str) -> Line<'static> {
    // OSC8 makes capable terminals expose a clickable span.
    Line::from(vec![Span::raw(format!("{label} -> {url}"))])
}"#,
        width,
        &theme,
    );
    blank(&mut lines);

    push_h2(
        &mut lines,
        "Table with alignment and wrapped prose",
        width,
        &theme,
    );
    push_table(
        &mut lines,
        &[
            ["Feature", "Markdown", "Ratatui rendering idea"],
            ["Heading", "# / ## / ###", "Use banners, rails, and underline rules so hierarchy is visible without font sizes."],
            ["Link", "[text](url)", "Styled text plus OSC8 hyperlink escape, with fallback URL nearby."],
            ["Code", "```rust", "Boxed block, language badge, line numbers, and syntax-colored spans."],
            ["Table", "pipes", "Grid borders, row dividers, alignment, and wrapped long cells."],
        ],
        width,
        &theme,
    );
    blank(&mut lines);

    push_h2(
        &mut lines,
        "Media, footnotes, HTML, and Unicode width",
        width,
        &theme,
    );
    push_image_placeholder(
        &mut lines,
        "architecture-diagram.png",
        "Oino message rendering pipeline",
        width,
        &theme,
    );
    lines.push(Line::from(vec![
        Span::styled("Footnote", theme.marker),
        Span::styled("¹ ", theme.marker),
        Span::styled(
            "Rendered as a dim reference with its definition grouped near the bottom.",
            theme.fg,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("HTML placeholder: ", theme.marker),
        Span::styled("<kbd>Ctrl-O</kbd>", theme.inline_code),
        Span::styled(
            " can render as a keyboard badge instead of raw HTML.",
            theme.fg,
        ),
    ]));
    push_wrapped(
        &mut lines,
        "Unicode width sample: English, 中文宽字符, emoji ✅🚀, box drawing ┌─┐. A real renderer must measure display width, not byte length.",
        width,
        theme.fg,
    );
    blank(&mut lines);

    push_h2(&mut lines, "Coverage checklist", width, &theme);
    for item in [
        "✓ H1/H2/H3 hierarchy",
        "✓ paragraphs and wrapping",
        "✓ bold/italic/strikethrough/inline code",
        "✓ clickable OSC8 link span + fallback URL",
        "✓ bullet, ordered, and task lists",
        "✓ block quote and admonition card",
        "✓ fenced code with language, line numbers, and highlighting",
        "✓ table borders, row dividers, alignment, and wrapped cells",
        "✓ horizontal rule, image placeholder, footnote, HTML badge, Unicode",
    ] {
        push_list_item(&mut lines, "·", item, width, &theme);
    }

    Showcase { lines, links }
}

#[derive(Debug, Clone, Copy)]
struct DemoTheme {
    fg: Style,
    muted: Style,
    h1: Style,
    h2: Style,
    h3: Style,
    marker: Style,
    link: Style,
    inline_code: Style,
    quote: Style,
    border: Style,
    code: Style,
    code_keyword: Style,
    code_string: Style,
    code_comment: Style,
    code_number: Style,
    table_header: Style,
}

impl Default for DemoTheme {
    fn default() -> Self {
        Self {
            fg: Style::default().fg(Color::Reset),
            muted: Style::default().fg(Color::DarkGray),
            h1: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            h2: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            marker: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            link: Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::UNDERLINED),
            inline_code: Style::default().fg(Color::Yellow).bg(Color::Black),
            quote: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            border: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            code: Style::default().fg(Color::Gray),
            code_keyword: Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            code_string: Style::default().fg(Color::Green),
            code_comment: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            code_number: Style::default().fg(Color::LightRed),
            table_header: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        }
    }
}

fn blank(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from(""));
}

fn push_h1(lines: &mut Vec<Line<'static>>, title: &str, width: usize, theme: &DemoTheme) {
    lines.push(Line::styled(border_line('╭', '╮', "", width), theme.h1));
    lines.push(Line::from(vec![
        Span::styled("│ ", theme.h1),
        Span::styled(pad_to_width(title, width.saturating_sub(4)), theme.h1),
        Span::styled(" │", theme.h1),
    ]));
    lines.push(Line::styled(border_line('╰', '╯', "", width), theme.h1));
}

fn push_h2(lines: &mut Vec<Line<'static>>, title: &str, width: usize, theme: &DemoTheme) {
    lines.push(Line::from(vec![
        Span::styled("▌ ", theme.h2),
        Span::styled(title.to_string(), theme.h2),
    ]));
    lines.push(Line::styled("─".repeat(width.min(80)), theme.muted));
}

fn push_h3(lines: &mut Vec<Line<'static>>, title: &str, _width: usize, theme: &DemoTheme) {
    lines.push(Line::from(vec![
        Span::styled("▌ ", theme.h3),
        Span::styled(title.to_string(), theme.h3),
    ]));
}

fn push_wrapped(lines: &mut Vec<Line<'static>>, text: &str, width: usize, style: Style) {
    for segment in wrap_words(text, width) {
        lines.push(Line::styled(segment, style));
    }
}

fn push_list_item(
    lines: &mut Vec<Line<'static>>,
    marker: &str,
    text: &str,
    width: usize,
    theme: &DemoTheme,
) {
    let marker_width = marker.width().saturating_add(1);
    let content_width = width.saturating_sub(marker_width).max(1);
    for (index, segment) in wrap_words(text, content_width).into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled(format!("{marker} "), theme.marker),
                Span::styled(segment, theme.fg),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(marker_width)),
                Span::styled(segment, theme.fg),
            ]));
        }
    }
}

fn push_quote(lines: &mut Vec<Line<'static>>, text: &str, width: usize, theme: &DemoTheme) {
    let content_width = width.saturating_sub(3).max(1);
    for segment in wrap_words(text, content_width) {
        lines.push(Line::from(vec![
            Span::styled("│ ", theme.quote),
            Span::styled(segment, theme.quote),
        ]));
    }
}

fn push_note(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    body: &str,
    width: usize,
    theme: &DemoTheme,
) {
    let body_width = width.saturating_sub(4).max(1);
    lines.push(Line::styled(
        border_line('╭', '╮', &format!(" ⚑ {title} "), width),
        theme.border,
    ));
    for segment in wrap_words(body, body_width) {
        lines.push(Line::from(vec![
            Span::styled("│ ", theme.border),
            Span::styled(pad_to_width(&segment, body_width), theme.fg),
            Span::styled(" │", theme.border),
        ]));
    }
    lines.push(Line::styled(border_line('╰', '╯', "", width), theme.border));
}

fn push_rule(lines: &mut Vec<Line<'static>>, width: usize, theme: &DemoTheme) {
    lines.push(Line::styled("─".repeat(width), theme.muted));
}

fn push_code_block(
    lines: &mut Vec<Line<'static>>,
    language: &str,
    code: &str,
    width: usize,
    theme: &DemoTheme,
) {
    let line_count = code.lines().count().max(1);
    let number_width = line_count.to_string().width().max(1);
    let prefix_width = number_width.saturating_add(5);
    let suffix_width = 2;
    let code_width = width.saturating_sub(prefix_width + suffix_width).max(8);
    let label = format!(" {language} · syntax · line numbers ");
    lines.push(Line::styled(
        border_line('╭', '╮', &label, width),
        theme.border,
    ));
    for (index, source) in code.lines().enumerate() {
        let number = (index + 1).to_string();
        let mut spans = vec![
            Span::styled("│ ", theme.border),
            Span::styled(
                format!(
                    "{}{}",
                    " ".repeat(number_width.saturating_sub(number.width())),
                    number
                ),
                theme.muted,
            ),
            Span::styled(" │ ", theme.border),
        ];
        let mut code_spans = highlight_rust_line(source, theme);
        let code_span_width = spans_width(&code_spans);
        if code_span_width > code_width {
            code_spans = vec![Span::styled(
                truncate_to_width(source, code_width),
                theme.code,
            )];
        }
        let visible_code_width = spans_width(&code_spans);
        spans.extend(code_spans);
        spans.push(Span::raw(
            " ".repeat(code_width.saturating_sub(visible_code_width)),
        ));
        spans.push(Span::styled(" │", theme.border));
        lines.push(Line::from(spans));
    }
    lines.push(Line::styled(border_line('╰', '╯', "", width), theme.border));
}

fn push_table(lines: &mut Vec<Line<'static>>, rows: &[[&str; 3]], width: usize, theme: &DemoTheme) {
    if rows.is_empty() {
        return;
    }
    let column_widths = table_widths(width);
    push_table_border(lines, '┌', '┬', '┐', &column_widths, theme);
    for (row_index, row) in rows.iter().enumerate() {
        push_table_row(lines, row, &column_widths, row_index == 0, theme);
        if row_index + 1 < rows.len() {
            push_table_border(lines, '├', '┼', '┤', &column_widths, theme);
        }
    }
    push_table_border(lines, '└', '┴', '┘', &column_widths, theme);
}

fn table_widths(width: usize) -> [usize; 3] {
    let chrome = 10usize;
    let available = width.saturating_sub(chrome).max(18);
    let mut widths = [16usize, 16usize, available.saturating_sub(32).max(18)];
    while widths.iter().sum::<usize>() > available {
        let mut max_index = 0usize;
        for index in 1..widths.len() {
            if widths[index] > widths[max_index] {
                max_index = index;
            }
        }
        if widths[max_index] <= 6 {
            break;
        }
        widths[max_index] = widths[max_index].saturating_sub(1);
    }
    widths
}

fn push_table_border(
    lines: &mut Vec<Line<'static>>,
    left: char,
    sep: char,
    right: char,
    widths: &[usize; 3],
    theme: &DemoTheme,
) {
    let mut text = String::new();
    text.push(left);
    for (index, width) in widths.iter().enumerate() {
        text.push_str(&"─".repeat(width.saturating_add(2)));
        if index + 1 < widths.len() {
            text.push(sep);
        }
    }
    text.push(right);
    lines.push(Line::styled(text, theme.border));
}

fn push_table_row(
    lines: &mut Vec<Line<'static>>,
    row: &[&str; 3],
    widths: &[usize; 3],
    header: bool,
    theme: &DemoTheme,
) {
    let wrapped = row
        .iter()
        .zip(widths)
        .map(|(cell, width)| wrap_words(cell, *width))
        .collect::<Vec<_>>();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
    for visual_row in 0..height {
        let mut spans = vec![Span::styled("│ ", theme.border)];
        for (column, width) in widths.iter().enumerate() {
            if column > 0 {
                spans.push(Span::styled(" │ ", theme.border));
            }
            let segment = wrapped
                .get(column)
                .and_then(|segments| segments.get(visual_row))
                .map(String::as_str)
                .unwrap_or("");
            let style = if header { theme.table_header } else { theme.fg };
            let text = match column {
                1 => align_right(segment, *width),
                2 if header => align_center(segment, *width),
                _ => pad_to_width(segment, *width),
            };
            spans.push(Span::styled(text, style));
        }
        spans.push(Span::styled(" │", theme.border));
        lines.push(Line::from(spans));
    }
}

fn push_image_placeholder(
    lines: &mut Vec<Line<'static>>,
    src: &str,
    alt: &str,
    width: usize,
    theme: &DemoTheme,
) {
    let body_width = width.saturating_sub(4).max(1);
    lines.push(Line::styled(
        border_line('╭', '╮', " image placeholder ", width),
        theme.muted,
    ));
    for text in [
        format!("alt: {alt}"),
        format!("src: {src}"),
        "terminal image protocols can replace this block later".to_string(),
    ] {
        lines.push(Line::from(vec![
            Span::styled("│ ", theme.muted),
            Span::styled(pad_to_width(&text, body_width), theme.muted),
            Span::styled(" │", theme.muted),
        ]));
    }
    lines.push(Line::styled(border_line('╰', '╯', "", width), theme.muted));
}

fn highlight_rust_line(line: &str, theme: &DemoTheme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(comment_start) = rest.find("//") {
            let (before, comment) = rest.split_at(comment_start);
            if !before.is_empty() {
                spans.extend(highlight_rust_code_part(before, theme));
            }
            spans.push(Span::styled(comment.to_string(), theme.code_comment));
            return spans;
        }
        spans.extend(highlight_rust_code_part(rest, theme));
        rest = "";
    }
    spans
}

fn highlight_rust_code_part(part: &str, theme: &DemoTheme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = part.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch == '"' {
            let mut end = start + ch.len_utf8();
            let mut escaped = false;
            for (idx, next) in chars.by_ref() {
                end = idx + next.len_utf8();
                if next == '"' && !escaped {
                    break;
                }
                escaped = next == '\\' && !escaped;
                if next != '\\' {
                    escaped = false;
                }
            }
            spans.push(Span::styled(
                part[start..end].to_string(),
                theme.code_string,
            ));
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let mut end = start + ch.len_utf8();
            while let Some((idx, next)) = chars.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    end = idx + next.len_utf8();
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            let token = &part[start..end];
            let style = if is_rust_keyword(token) {
                theme.code_keyword
            } else {
                theme.code
            };
            spans.push(Span::styled(token.to_string(), style));
        } else if ch.is_ascii_digit() {
            let mut end = start + ch.len_utf8();
            while let Some((idx, next)) = chars.peek().copied() {
                if next.is_ascii_digit() {
                    end = idx + next.len_utf8();
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            spans.push(Span::styled(
                part[start..end].to_string(),
                theme.code_number,
            ));
        } else {
            spans.push(Span::styled(ch.to_string(), theme.code));
        }
    }
    spans
}

fn is_rust_keyword(token: &str) -> bool {
    matches!(
        token,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "match"
            | "mod"
            | "mut"
            | "pub"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "trait"
            | "type"
            | "use"
            | "where"
            | "while"
    )
}

fn osc8_link(label: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
}

fn border_line(left: char, right: char, label: &str, width: usize) -> String {
    let width = width.max(2);
    let mut line = String::new();
    line.push(left);
    if !label.is_empty() && width > 4 {
        line.push('─');
        let label = truncate_to_width(label, width.saturating_sub(4));
        line.push_str(&label);
    }
    let used = line.width();
    if used < width.saturating_sub(1) {
        line.push_str(&"─".repeat(width.saturating_sub(1) - used));
    }
    line.push(right);
    truncate_to_width(&line, width)
}

fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = word.width();
        if current_width == 0 {
            if word_width > width {
                push_wrapped_word(word, width, &mut current, &mut current_width, &mut out);
            } else {
                current.push_str(word);
                current_width = word_width;
            }
        } else if current_width.saturating_add(1).saturating_add(word_width) <= width {
            current.push(' ');
            current.push_str(word);
            current_width = current_width.saturating_add(1).saturating_add(word_width);
        } else {
            out.push(std::mem::take(&mut current));
            current_width = 0;
            if word_width > width {
                push_wrapped_word(word, width, &mut current, &mut current_width, &mut out);
            } else {
                current.push_str(word);
                current_width = word_width;
            }
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

fn push_wrapped_word(
    word: &str,
    width: usize,
    current: &mut String,
    current_width: &mut usize,
    out: &mut Vec<String>,
) {
    for grapheme in word.graphemes(true) {
        let grapheme_width = grapheme.width();
        if current_width.saturating_add(grapheme_width) > width && !current.is_empty() {
            out.push(std::mem::take(current));
            *current_width = 0;
        }
        current.push_str(grapheme);
        *current_width = current_width.saturating_add(grapheme_width);
        if *current_width >= width {
            out.push(std::mem::take(current));
            *current_width = 0;
        }
    }
}

fn truncate_to_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used.saturating_add(grapheme_width) > width {
            break;
        }
        out.push_str(grapheme);
        used = used.saturating_add(grapheme_width);
    }
    out
}

fn pad_to_width(text: &str, width: usize) -> String {
    let used = text.width();
    if used >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
}

fn align_right(text: &str, width: usize) -> String {
    let used = text.width();
    if used >= width {
        text.to_string()
    } else {
        format!("{}{text}", " ".repeat(width - used))
    }
}

fn align_center(text: &str, width: usize) -> String {
    let used = text.width();
    if used >= width {
        text.to_string()
    } else {
        let left = (width - used) / 2;
        let right = width - used - left;
        format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
    }
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.as_ref().width()).sum()
}
