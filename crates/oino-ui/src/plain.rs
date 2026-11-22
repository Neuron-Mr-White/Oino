//! Plain-text fallback (§10): `to_lines(node, width)` renders a tree for clients
//! without a v2 renderer. Char-count based wrapping (no `unicode-width`
//! dependency — this crate stays serde-only per §11); wide chars may render
//! slightly narrow.

use crate::extras::button::ButtonVariant;
use crate::layout::Direction;
use crate::node::Node;
use crate::text::{Line, Span, Text};
use crate::widgets::block::BlockTitle;
use crate::widgets::list::ListItem;

/// Renders a node to at most `width`-column lines.
#[must_use]
pub fn to_lines(node: &Node, width: u16) -> Vec<String> {
    let mut out = Vec::new();
    render(node, width, &mut out);
    out
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per node kind is the clearest shape for the fallback renderer"
)]
fn render(node: &Node, width: u16, out: &mut Vec<String>) {
    match node {
        Node::Layout(l) => {
            let vertical = matches!(l.direction, Some(Direction::Vertical) | None)
                || width < l.collapse.unwrap_or(60);
            if vertical {
                for child in &l.children {
                    render(child, width, out);
                }
            } else {
                let lines: Vec<Vec<String>> =
                    l.children.iter().map(|c| to_lines(c, width)).collect();
                let rows = lines.iter().map(Vec::len).max().unwrap_or_default();
                for row in 0..rows {
                    let mut merged = String::new();
                    for chunk in &lines {
                        let cell = chunk.get(row).map_or(String::new(), String::clone);
                        let truncated: String = cell.chars().take(width as usize).collect();
                        merged.push_str(&truncated);
                        for _ in truncated.chars().count()..width as usize {
                            merged.push(' ');
                        }
                    }
                    out.push(merged);
                }
            }
        }
        Node::Block(b) => {
            if let Some(title) = block_title(b.title.as_ref()) {
                out.push(title);
            }
            if let Some(child) = &b.child {
                render(child, width, out);
            }
        }
        Node::Paragraph(p) => {
            for line in p.text.lines() {
                let text = line_text(&line);
                wrap(&text, width, out);
            }
        }
        Node::List(l) => {
            for item in &l.items {
                let text = item_text(item);
                wrap(&format!("• {text}"), width, out);
                if let ListItem::Full {
                    detail: Some(detail),
                    ..
                } = item
                {
                    wrap(&format!("    {}", line_text(detail)), width, out);
                }
            }
        }
        Node::Table(t) => {
            for row in t.header.iter().chain(t.rows.iter()).chain(t.footer.iter()) {
                let cells: Vec<String> = row
                    .cells
                    .iter()
                    .map(|cell| text_text(cell.content()))
                    .collect();
                wrap(&cells.join(" | "), width, out);
            }
        }
        Node::Tabs(t) => {
            let titles: Vec<String> = t.titles.iter().map(|ti| line_text(&ti.title)).collect();
            let current = t
                .selected
                .as_deref()
                .and_then(|sel| t.titles.iter().position(|ti| ti.key == sel))
                .unwrap_or_default();
            out.push(titles.join(" │ "));
            if let Some(panes) = &t.panes
                && let Some(key) = t.titles.get(current).map(|ti| ti.key.clone())
                && let Some(pane) = panes.get(&key)
            {
                render(pane, width, out);
            }
        }
        Node::Gauge(g) => {
            let ratio = g.ratio.unwrap_or_default();
            let label = span_text(g.label.as_ref().unwrap_or(&Span::default()));
            out.push(bar_line(ratio, label, width));
        }
        Node::LineGauge(g) => {
            let label = g
                .label
                .as_ref()
                .map_or_else(|| format!("{:.0}%", g.ratio * 100.0), line_text);
            out.push(format!(
                "{label} {}",
                bar_line(g.ratio, String::new(), width)
            ));
        }
        Node::Sparkline(s) => {
            let values: Vec<String> = s
                .data
                .iter()
                .map(|v| v.map_or_else(|| "·".to_owned(), |v| v.to_string()))
                .collect();
            wrap(&values.join(" "), width, out);
        }
        Node::BarChart(c) => {
            for group in &c.data {
                for bar in &group.bars {
                    let label = bar.label.as_ref().map_or_else(String::new, line_text);
                    out.push(format!("{label}: {}", bar.value));
                }
            }
        }
        Node::Chart(c) => {
            for dataset in &c.datasets {
                let name = dataset
                    .name
                    .as_ref()
                    .map_or_else(|| "series".to_owned(), line_text);
                out.push(format!("{name}: {} points", dataset.data.len()));
            }
        }
        Node::Scrollbar(_) => {}
        Node::Markdown(m) => {
            for line in m.source.split('\n') {
                wrap(line, width, out);
            }
        }
        Node::Code(c) => {
            for line in c.source.split('\n') {
                wrap(line, width, out);
            }
        }
        Node::Diff(d) => {
            let source = d.patch.as_ref().map_or_else(
                || {
                    format!(
                        "- {}\n+ {}",
                        d.old.as_deref().unwrap_or_default(),
                        d.new.as_deref().unwrap_or_default()
                    )
                },
                String::clone,
            );
            for line in source.split('\n') {
                wrap(line, width, out);
            }
        }
        Node::Image(i) => {
            out.push(format!("[image: {}]", i.alt));
        }
        Node::Log(l) => {
            let max = l.max_lines.unwrap_or(2_000) as usize;
            let skip = l.lines.len().saturating_sub(max);
            for line in l.lines.iter().skip(skip) {
                wrap(&line_text(line), width, out);
            }
        }
        Node::Tree(t) => {
            for child in &t.nodes {
                render_tree_node(child, 0, width, out);
            }
        }
        Node::KeyHints(k) => {
            let hints: Vec<String> = k
                .hints
                .iter()
                .map(|h| format!("{} {}", h.keys, h.label))
                .collect();
            out.push(hints.join(" · "));
        }
        Node::Input(i) => {
            let label = i
                .label
                .as_ref()
                .map_or_else(String::new, |l| format!("{}: ", line_text(l)));
            let masked = i.secret.unwrap_or_default();
            let value = i.value.as_deref().unwrap_or_default();
            let value = if masked {
                "•".repeat(value.chars().count())
            } else {
                value.to_owned()
            };
            wrap(&format!("{label}{value}"), width, out);
        }
        Node::Toggle(t) => {
            let mark = if t.value { 'x' } else { ' ' };
            wrap(&format!("[{mark}] {}", line_text(&t.label)), width, out);
        }
        Node::Select(s) => {
            let current = s
                .value
                .as_deref()
                .unwrap_or(s.placeholder.as_deref().unwrap_or_default());
            wrap(&format!("‹ {current} ›"), width, out);
        }
        Node::Button(b) => {
            let label = line_text(&b.label);
            out.push(match b.variant.unwrap_or(ButtonVariant::Secondary) {
                ButtonVariant::Primary => format!("⟦ {label} ⟧"),
                _ => format!("[ {label} ]"),
            });
        }
        Node::Spinner(s) => {
            let label = s
                .label
                .as_ref()
                .map_or_else(String::new, |l| format!(" {}", line_text(l)));
            out.push(format!("⠿{label}"));
        }
        Node::Empty(e) => {
            if let Some(title) = &e.title {
                out.push(title.clone());
            }
            if let Some(message) = &e.message {
                wrap(message, width, out);
            }
        }
        Node::Unknown(u) => {
            if let Some(fallback) = &u.fallback {
                render(fallback, width, out);
            }
        }
    }
}

fn render_tree_node(
    node: &crate::extras::tree::TreeNode,
    depth: usize,
    width: u16,
    out: &mut Vec<String>,
) {
    let expanded = node.expanded.unwrap_or_default();
    let arrow = if node.children.is_empty() {
        " "
    } else if expanded {
        "▾"
    } else {
        "▸"
    };
    let indent = "  ".repeat(depth);
    wrap(
        &format!("{indent}{arrow} {}", line_text(&node.label)),
        width,
        out,
    );
    if expanded {
        for child in &node.children {
            render_tree_node(child, depth + 1, width, out);
        }
    }
}

fn block_title(title: Option<&BlockTitle>) -> Option<String> {
    match title {
        Some(BlockTitle::Line(line)) => Some(line_text(line)),
        Some(BlockTitle::Titles(titles)) => titles.first().map(|t| line_text(&t.content)),
        None => None,
    }
}

fn item_text(item: &ListItem) -> String {
    match item {
        ListItem::Text(text) => text_text(text),
        ListItem::Full { content, .. } => text_text(content),
    }
}

fn line_text(line: &Line) -> String {
    match line {
        Line::Str(text) => text.clone(),
        Line::Spans(spans) | Line::Full { spans, .. } => {
            spans.iter().map(Span::content).collect::<Vec<_>>().join("")
        }
    }
}

fn span_text(span: &Span) -> String {
    span.content().to_owned()
}

fn text_text(text: &Text) -> String {
    text.to_text()
}

fn bar_line(ratio: f64, label: String, width: u16) -> String {
    let label = if label.is_empty() {
        format!("{:.0}%", ratio * 100.0)
    } else {
        label
    };
    let label_len = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
    if width <= label_len + 2 {
        return label;
    }
    let bar_width = width - label_len - 1;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "ratio is clamped to 0.0..=1.0 before rounding"
    )]
    let filled = ((ratio.clamp(0.0, 1.0) * f64::from(bar_width)).round() as u16).min(bar_width);
    let mut out = String::new();
    for _ in 0..filled {
        out.push('█');
    }
    for _ in filled..bar_width {
        out.push('░');
    }
    out.push(' ');
    out.push_str(&label);
    out
}

fn wrap(text: &str, width: u16, out: &mut Vec<String>) {
    let max = width.max(1) as usize;
    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in raw_line.split(' ') {
            let candidate_len =
                current.chars().count() + word.chars().count() + usize::from(!current.is_empty());
            if candidate_len > max && !current.is_empty() {
                out.push(current.clone());
                current.clear();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            while current.chars().count() > max {
                let boundary = current
                    .char_indices()
                    .nth(max)
                    .map_or(current.len(), |(i, _)| i);
                out.push(current[..boundary].to_owned());
                current.drain(..boundary);
            }
        }
        out.push(current);
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use super::*;
    use crate::extras::log::Log;
    use crate::layout::{Constraint, Layout};
    use crate::widgets::paragraph::Paragraph;

    #[test]
    fn vertical_layout_stacks_children() {
        let node: Node = Layout::vertical([Constraint::Length(1), Constraint::Length(1)])
            .children([
                Paragraph::new("header").into(),
                Log::new(["row one", "row two"]).into(),
            ])
            .into();
        let lines = to_lines(&node, 80);
        assert_eq!(lines, vec!["header", "row one", "row two"]);
    }

    #[test]
    fn wraps_at_width() {
        let lines = to_lines(&Paragraph::new("aaaa bbbb cccc dddd").into(), 9);
        assert!(lines.iter().all(|l| l.chars().count() <= 9));
        assert_eq!(lines.first().map(String::as_str), Some("aaaa bbbb"));
    }

    #[test]
    fn gauge_bar() {
        let lines = to_lines(&crate::widgets::gauge::Gauge::new().ratio(0.5).into(), 10);
        assert_eq!(lines, vec!["███░░░ 50%"]);
    }
}
