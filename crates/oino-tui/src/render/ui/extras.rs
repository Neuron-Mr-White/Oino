//! GUI-extra nodes of the v2 tree (§4.5.2): `Markdown`, `Code`, `Diff`, `Image`, `Log`,
//! `Tree`, `KeyHints`, `Input`, `Toggle`, `Select`, `Button`, `Spinner`, `Empty`.
//!
//! The TUI reuses the existing markdown/syntax-highlight renderers; images render as the
//! documented `[image: alt]` dim box (no kitty/sixel probing in this renderer).

use oino_ui::extras::button::ButtonVariant;
use oino_ui::extras::icon::Icon;
use oino_ui::node::Node;
use oino_ui::widgets::tabs::Tabs as TokenTabs;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget};

use super::focus;
use super::style;
use super::{Ctx, RenderState, measure};

use crate::render::highlight;
use crate::render::markdown::{self, MdStyle};
use crate::render::text::{self, OLine};

/// Unicode glyph of an icon (§4.5.2).
#[must_use]
pub(crate) fn glyph(icon: Icon) -> &'static str {
    match icon {
        Icon::Check => "✓",
        Icon::X => "✗",
        Icon::Dot => "•",
        Icon::Warn => "⚠",
        Icon::Info => "ℹ",
        Icon::Clock => "⏱",
        Icon::Gear => "⚙",
        Icon::Branch => "⑂",
        Icon::Bolt => "⚡",
        Icon::Bot => "🤖",
        Icon::Folder => "▤",
        Icon::File => "·",
        Icon::Search => "⌕",
        Icon::Cloud => "☁",
        Icon::Lock => "🔒",
        Icon::Board => "▦",
        Icon::Terminal => "▮",
        Icon::Globe => "🌐",
        Icon::Diff => "±",
        Icon::List => "≡",
        Icon::Chart => "▁",
        Icon::Play => "▶",
        Icon::Pause => "‖",
        Icon::Stop => "■",
        Icon::Refresh => "↻",
        Icon::Plus => "+",
        Icon::Trash => "🗑",
    }
}

/// Selected tab key of a `Tabs` node (entry-declared, else the first).
#[must_use]
pub(crate) fn selected_tab(tabs: &TokenTabs) -> Option<&str> {
    tabs.selected
        .as_deref()
        .or_else(|| tabs.titles.first().map(|t| t.key.as_str()))
}

/// Rendered markdown lines (§4.5.2).
#[must_use]
pub(crate) fn markdown_lines(
    markdown: &oino_ui::extras::markdown::Markdown,
    width: u16,
    cx: &Ctx<'_>,
) -> Vec<OLine> {
    markdown::render(
        &markdown.source,
        usize::from(width),
        MdStyle { base: cx.base() },
        cx.theme,
    )
}

/// Highlighted code lines (§4.5.2).
#[must_use]
pub(crate) fn code_lines(code: &oino_ui::extras::code::Code, cx: &Ctx<'_>) -> Vec<OLine> {
    let base = Style::default().fg(cx.theme.md_code_block);
    let mut out: Vec<OLine> =
        highlight::highlight(&code.source, code.language.as_deref(), cx.theme, base)
            .into_iter()
            .map(Line::from)
            .collect();
    if code.line_numbers.unwrap_or(false) {
        let start = code.start_line.unwrap_or(1);
        let digits = (start as usize + out.len()).to_string().len();
        let number = Style::default().fg(cx.theme.dim);
        for (index, line) in out.iter_mut().enumerate() {
            let mut spans = vec![Span::styled(
                format!("{:>digits$} ", start as usize + index),
                number,
            )];
            spans.extend(line.spans.clone());
            *line = Line::from(spans);
        }
    }
    if !code.highlight_lines.is_empty() {
        let start = code.start_line.unwrap_or(1);
        for number in &code.highlight_lines {
            let index = usize::try_from(number.saturating_sub(start)).unwrap_or(usize::MAX);
            if let Some(line) = out.get_mut(index) {
                line.style = line.style.bg(cx.theme.selected_bg);
            }
        }
    }
    out
}

/// Unified diff lines (§4.5.2).
#[must_use]
pub(crate) fn diff_lines(diff: &oino_ui::extras::diff::Diff, cx: &Ctx<'_>) -> Vec<OLine> {
    let patch = diff.patch.clone().unwrap_or_else(|| {
        let path = diff.path.as_deref().unwrap_or("a");
        let mut out = format!("--- {path}\n+++ {path}\n");
        for line in diff.old.as_deref().unwrap_or_default().lines() {
            out.push('-');
            out.push_str(line);
            out.push('\n');
        }
        for line in diff.new.as_deref().unwrap_or_default().lines() {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
        out
    });
    patch
        .lines()
        .map(|line| {
            let color = match line.chars().next() {
                Some('+') => cx.theme.diff_added,
                Some('-') => cx.theme.diff_removed,
                Some('@') => cx.theme.md_heading,
                _ => cx.theme.diff_context,
            };
            Line::from(Span::styled(line.to_owned(), Style::default().fg(color)))
        })
        .collect()
}

/// Draws already-rendered lines into `area` (clipped, no wrap).
fn draw(lines: Vec<OLine>, area: Rect, buffer: &mut Buffer, base: Style) {
    Paragraph::new(Text::from(lines))
        .style(base)
        .render(area, buffer);
}

/// Renders the block chrome and returns its inner area.
fn frame(node: &Node, area: Rect, buffer: &mut Buffer, cx: &Ctx<'_>, base: Style) -> Rect {
    let spec = measure::common_block(node);
    buffer.set_style(area, base);
    match spec.map(|spec| style::block(&spec, None, base, cx.theme).style(base)) {
        Some(block) => {
            let inner = block.inner(area);
            block.render(area, buffer);
            inner
        }
        None => area,
    }
}

/// Rendering of every GUI-extra node kind.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per GUI-extra node kind (ui-protocol §4.5.2)"
)]
pub(crate) fn render(
    node: &Node,
    path: &str,
    area: Rect,
    buffer: &mut Buffer,
    cx: &Ctx<'_>,
    state: &mut RenderState,
    base: Style,
) {
    let inner = frame(node, area, buffer, cx, base);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    match node {
        Node::Markdown(markdown) => {
            draw(
                markdown_lines(markdown, inner.width, cx),
                inner,
                buffer,
                base,
            );
        }
        Node::Code(code) => {
            let lines = code_lines(code, cx);
            let lines = if code.wrap.unwrap_or(false) {
                wrap_lines(lines, usize::from(inner.width))
            } else {
                lines
            };
            draw(lines, inner, buffer, base);
        }
        Node::Diff(diff) => draw(diff_lines(diff, cx), inner, buffer, base),
        Node::Image(image) => {
            let max = image.max_height.unwrap_or(3).max(1);
            let label = Style::default().fg(cx.theme.dim);
            let mut lines = Vec::new();
            lines.push(Line::default());
            lines.push(
                Line::from(Span::styled(format!("[image: {}]", image.alt), label))
                    .alignment(ratatui::layout::Alignment::Center),
            );
            lines.truncate(usize::from(max));
            draw(lines, inner, buffer, base);
        }
        Node::Log(log) => {
            let cap = usize::try_from(log.max_lines.unwrap_or(2_000).min(5_000)).unwrap_or(2_000);
            let mut lines: Vec<OLine> = log
                .lines
                .iter()
                .map(|line| style::line(line, base, cx.theme))
                .collect();
            if lines.len() > cap {
                lines = lines.split_off(lines.len() - cap);
            }
            if log.line_numbers.unwrap_or(false) {
                let digits = lines.len().to_string().len();
                let number = Style::default().fg(cx.theme.dim);
                for (index, line) in lines.iter_mut().enumerate() {
                    let mut spans = vec![Span::styled(format!("{:>digits$} ", index + 1), number)];
                    spans.extend(line.spans.clone());
                    *line = Line::from(spans);
                }
            }
            if !log.follow.unwrap_or(true) && lines.len() > usize::from(inner.height) {
                lines.truncate(usize::from(inner.height));
            }
            if log.wrap.unwrap_or(false) {
                lines = wrap_lines(lines, usize::from(inner.width));
            } else {
                lines = lines
                    .into_iter()
                    .map(|line| text::truncate_line(line, usize::from(inner.width)))
                    .collect();
            }
            draw(lines, inner, buffer, base);
        }
        Node::Tree(tree) => {
            let rows = focus::tree_rows(tree, path, cx.local);
            let declared = tree
                .state
                .as_ref()
                .and_then(|state| state.selected.as_ref())
                .and_then(|key| rows.iter().position(|row| &row.key == key))
                .unwrap_or(0);
            let selected = cx
                .local
                .selected(path, declared)
                .min(rows.len().saturating_sub(1));
            let mut lines = Vec::with_capacity(rows.len());
            let selected_style = Style::default()
                .bg(cx.theme.selected_bg)
                .add_modifier(Modifier::BOLD);
            for (index, row) in rows.iter().enumerate() {
                let Some(tree_node) = find_node(&tree.nodes, &row.path, path) else {
                    continue;
                };
                let marker = if row.has_children {
                    if row.expanded { "▾ " } else { "▸ " }
                } else {
                    "  "
                };
                let mut spans = vec![Span::styled(
                    format!("{}{marker}", "  ".repeat(row.depth)),
                    Style::default().fg(cx.theme.muted),
                )];
                if let Some(icon) = tree_node.icon {
                    spans.push(Span::styled(
                        format!("{} ", glyph(icon)),
                        Style::default().fg(cx.theme.accent),
                    ));
                }
                spans.extend(style::line(&tree_node.label, base, cx.theme).spans);
                let line = text::spread(
                    spans,
                    tree_node
                        .badge
                        .as_ref()
                        .map_or_else(Vec::new, |badge| style::line(badge, base, cx.theme).spans),
                    usize::from(inner.width),
                );
                let line = if index == selected {
                    let mut line = line;
                    line.style = selected_style;
                    line
                } else {
                    line
                };
                lines.push(line);
            }
            draw(lines, inner, buffer, base);
        }
        Node::KeyHints(hints) => {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (index, hint) in hints.hints.iter().enumerate() {
                let focused = hint.action.is_some() && state.next_focus();
                if index > 0 {
                    spans.push(Span::styled(" · ", Style::default().fg(cx.theme.dim)));
                }
                let mut hint_style = Style::default().fg(cx.theme.dim);
                if focused {
                    hint_style = hint_style
                        .bg(cx.theme.selected_bg)
                        .add_modifier(Modifier::BOLD);
                }
                spans.push(Span::styled(hint.keys.clone(), hint_style));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    hint.label.clone(),
                    Style::default().fg(cx.theme.muted),
                ));
            }
            draw(vec![Line::from(spans)], inner, buffer, base);
        }
        Node::Input(input) => {
            let focused = state.next_focus();
            let typed = cx
                .local
                .input(path)
                .map_or_else(|| input.value.clone().unwrap_or_default(), str::to_owned);
            let mut style_ = base;
            if input.disabled.unwrap_or(false) {
                style_ = style_.add_modifier(Modifier::DIM);
            } else if focused {
                style_ = style_.bg(cx.theme.selected_bg).add_modifier(Modifier::BOLD);
            }
            let mut spans = Vec::new();
            if let Some(label) = &input.label {
                spans.extend(style::line(label, style_, cx.theme).spans);
                spans.push(Span::raw(" "));
            }
            let shown = if input.secret.unwrap_or(false) {
                "•".repeat(typed.chars().count())
            } else {
                typed.clone()
            };
            if shown.is_empty() {
                spans.push(Span::styled(
                    input.placeholder.clone().unwrap_or_default(),
                    Style::default().fg(cx.theme.dim),
                ));
            } else {
                let mut shown = shown;
                if let Some(max) = input.max_length {
                    let max = usize::try_from(max).unwrap_or(usize::MAX);
                    shown = shown.chars().take(max).collect();
                }
                spans.push(Span::styled(shown, style_));
            }
            if focused && !input.disabled.unwrap_or(false) {
                spans.push(Span::styled("▏", Style::default().fg(cx.theme.accent)));
            }
            let lines: Vec<OLine> = if input.multiline.unwrap_or(false) {
                spans_to_lines(spans, usize::from(inner.width))
                    .into_iter()
                    .take(usize::from(input.rows.unwrap_or(3)))
                    .collect()
            } else {
                vec![Line::from(spans)]
            };
            draw(lines, inner, buffer, base);
        }
        Node::Toggle(toggle) => {
            let focused = state.next_focus();
            let value = cx.local.toggle(path, toggle.value);
            let mut style_ = base;
            if toggle.disabled.unwrap_or(false) {
                style_ = style_.add_modifier(Modifier::DIM);
            } else if focused {
                style_ = style_.bg(cx.theme.selected_bg).add_modifier(Modifier::BOLD);
            }
            let mut spans = vec![Span::styled(if value { "[x] " } else { "[ ] " }, style_)];
            spans.extend(style::line(&toggle.label, style_, cx.theme).spans);
            let line = Line::from(spans);
            draw(vec![line], inner, buffer, base);
        }
        Node::Select(select) => {
            let focused = state.next_focus();
            let value = cx.local.select(path, select.value.as_deref());
            let label = value
                .and_then(|value| {
                    select
                        .options
                        .iter()
                        .find(|option| option.value == value)
                        .map(|option| style::line(&option.label, base, cx.theme))
                })
                .or_else(|| {
                    select.placeholder.as_ref().map(|placeholder| {
                        Line::from(Span::styled(
                            placeholder.clone(),
                            Style::default().fg(cx.theme.dim),
                        ))
                    })
                })
                .unwrap_or_default();
            let mut style_ = base;
            if select.disabled.unwrap_or(false) {
                style_ = style_.add_modifier(Modifier::DIM);
            } else if focused {
                style_ = style_.bg(cx.theme.selected_bg).add_modifier(Modifier::BOLD);
            }
            let mut spans = vec![Span::styled("‹ ", style_)];
            spans.extend(label.spans);
            spans.push(Span::styled(" ›", style_));
            draw(vec![Line::from(spans)], inner, buffer, base);
        }
        Node::Button(button) => {
            let focused = state.next_focus();
            let mut style_ = match button.variant.unwrap_or_default() {
                ButtonVariant::Primary => Style::default().fg(cx.theme.accent),
                ButtonVariant::Danger => Style::default().fg(cx.theme.error),
                ButtonVariant::Ghost => Style::default().fg(cx.theme.muted),
                ButtonVariant::Secondary => base,
            };
            if button.disabled.unwrap_or(false) {
                style_ = style_.add_modifier(Modifier::DIM);
            } else if focused {
                style_ = style_
                    .bg(cx.theme.selected_bg)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED);
            }
            let mut spans = vec![Span::styled("[ ", style_)];
            spans.extend(style::line(&button.label, style_, cx.theme).spans);
            spans.push(Span::styled(" ]", style_));
            if button.confirm.is_some() && cx.local.confirm.get(path).copied().unwrap_or(false) {
                spans.push(Span::styled(
                    " (y/n)",
                    Style::default().fg(cx.theme.warning),
                ));
            }
            draw(vec![Line::from(spans)], inner, buffer, base);
        }
        Node::Spinner(spinner) => {
            const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = FRAMES[usize::try_from(cx.tick % 10).unwrap_or(0)];
            let mut spans = vec![Span::styled(
                format!("{frame} "),
                Style::default().fg(cx.theme.accent),
            )];
            if let Some(label) = &spinner.label {
                spans.extend(style::line(label, base, cx.theme).spans);
            }
            draw(vec![Line::from(spans)], inner, buffer, base);
        }
        Node::Empty(empty) => {
            let centered = ratatui::layout::Alignment::Center;
            let mut lines = Vec::new();
            if let Some(title) = &empty.title {
                lines.push(
                    Line::from(Span::styled(
                        title.clone(),
                        Style::default()
                            .fg(cx.theme.text)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .alignment(centered),
                );
            }
            if let Some(message) = &empty.message {
                lines.push(
                    Line::from(Span::styled(
                        message.clone(),
                        Style::default().fg(cx.theme.muted),
                    ))
                    .alignment(centered),
                );
            }
            let action_row = u16::try_from(lines.len()).unwrap_or(0).min(inner.height);
            draw(lines, inner, buffer, base);
            if let Some(action) = &empty.action {
                let area = Rect {
                    x: inner.x,
                    y: inner.y + action_row,
                    width: inner.width,
                    height: inner.height.saturating_sub(action_row),
                };
                super::render_node(
                    action,
                    &focus::join(path, "action"),
                    area,
                    buffer,
                    cx,
                    state,
                    base,
                );
            }
        }
        _ => {}
    }
}

/// Splits spans into lines at `width` cells (input multiline).
fn spans_to_lines(spans: Vec<Span<'static>>, width: usize) -> Vec<OLine> {
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in spans {
        let mut piece = String::new();
        for c in span.content.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + cw > width && used > 0 {
                if !piece.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut piece), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut current)));
                used = 0;
            }
            piece.push(c);
            used += cw;
        }
        if !piece.is_empty() {
            current.push(Span::styled(piece, span.style));
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

/// Wraps rendered lines to `width` (Log/Code `wrap`).
fn wrap_lines(lines: Vec<OLine>, width: usize) -> Vec<OLine> {
    let mut out = Vec::new();
    for line in lines {
        let runs: Vec<text::Run> = line
            .spans
            .into_iter()
            .map(|span| text::Run::new(span.content.to_string(), span.style))
            .collect();
        out.extend(text::wrap(&runs, width, &[], &[]));
    }
    out
}

/// Tree node addressed by a visible row's path.
fn find_node<'a>(
    nodes: &'a [oino_ui::extras::tree::TreeNode],
    row_path: &str,
    tree_path: &str,
) -> Option<&'a oino_ui::extras::tree::TreeNode> {
    fn walk<'a>(
        nodes: &'a [oino_ui::extras::tree::TreeNode],
        remaining: &[&str],
    ) -> Option<&'a oino_ui::extras::tree::TreeNode> {
        let first = *remaining.first()?;
        let node = nodes.iter().find(|node| node.key == first)?;
        if remaining.len() == 1 {
            return Some(node);
        }
        walk(&node.children, &remaining[1..])
    }
    let rest = row_path.strip_prefix(tree_path).unwrap_or(row_path);
    let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    walk(nodes, &segments)
}
