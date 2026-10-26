//! OUS v1 nodes → terminal lines (ui-protocol §6.1, TUI column).

use std::fmt::Write as _;

use std::collections::HashMap;

use oino_ai::JsonValue;
use oino_view::ous::{Align, DiffContent, Direction, ImageSource, OusNode, Rich, Tone, TreeItem};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::markdown::{self, MdStyle};
use super::text::{OLine, Run, line_width, wrap};
use crate::theme::Theme;

const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Interactive focus and live values (custom dialogs, panels).
#[derive(Clone, Debug, Default)]
pub(crate) struct Focus<'a> {
    /// Focused interactive index.
    pub(crate) index: Option<usize>,
    /// Cursor inside a focused list.
    pub(crate) choice: usize,
    /// Field values overriding spec values.
    pub(crate) values: Option<&'a HashMap<String, JsonValue>>,
    /// Text being typed into the focused input.
    pub(crate) editing: Option<&'a str>,
}

struct Ctx<'a> {
    theme: &'a Theme,
    focus: Focus<'a>,
    counter: usize,
    tick: u64,
}

impl Ctx<'_> {
    fn next_focus(&mut self) -> bool {
        let focused = self.focus.index == Some(self.counter);
        self.counter += 1;
        focused
    }

    fn tone(&self, tone: Tone) -> Style {
        Style::default().fg(self.theme.tone(tone))
    }

    fn runs(&self, rich: &Rich, base: Style) -> Vec<Run> {
        rich.0
            .iter()
            .map(|span| {
                let mut style = base;
                if span.tone != Tone::Default {
                    style = style.fg(self.theme.tone(span.tone));
                }
                if span.bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if span.italic {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if span.dim {
                    style = style.add_modifier(Modifier::DIM);
                }
                if span.underline || span.link.is_some() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if span.code {
                    style = style.fg(self.theme.md_code);
                }
                Run::new(span.text.clone(), style)
            })
            .collect()
    }

    fn rich_line(&self, rich: &Rich, base: Style, width: usize) -> Vec<OLine> {
        wrap(&self.runs(rich, base), width, &[], &[])
    }

    fn focus_style(&self, focused: bool, style: Style) -> Style {
        if focused {
            style
                .bg(self.theme.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }
}

fn indent(lines: Vec<OLine>, prefix: &str, style: Style) -> Vec<OLine> {
    lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::styled(prefix.to_owned(), style)];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// Renders `node` to at most `width` cells.
pub(crate) fn render(
    node: &OusNode,
    width: usize,
    theme: &Theme,
    focus: Focus<'_>,
    tick: u64,
) -> Vec<OLine> {
    let mut cx = Ctx {
        theme,
        focus,
        counter: 0,
        tick,
    };
    node_lines(node, width.max(4), &mut cx)
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per OUS node kind (ui-protocol §6.1)"
)]
fn node_lines(node: &OusNode, width: usize, cx: &mut Ctx<'_>) -> Vec<OLine> {
    let theme = cx.theme;
    let text_style = Style::default().fg(theme.text);
    let muted = Style::default().fg(theme.muted);
    match node {
        OusNode::Text { value, wrap: wraps } => {
            let lines = cx.rich_line(
                value,
                text_style,
                if *wraps { width } else { usize::MAX / 4 },
            );
            lines
                .into_iter()
                .map(|l| super::text::truncate_line(l, width))
                .collect()
        }
        OusNode::Markdown { value } => {
            markdown::render(value, width, MdStyle { base: text_style }, theme)
        }
        OusNode::Code {
            value,
            language,
            line_numbers,
        } => {
            let lines = super::highlight::highlight(
                value,
                language.as_deref(),
                theme,
                Style::default().fg(theme.md_code_block),
            );
            let digits = lines.len().to_string().len();
            lines
                .into_iter()
                .enumerate()
                .map(|(i, spans)| {
                    let mut out = Vec::new();
                    if *line_numbers {
                        out.push(Span::styled(
                            format!("{:>digits$} ", i + 1),
                            Style::default().fg(theme.dim),
                        ));
                    }
                    out.extend(spans);
                    super::text::truncate_line(Line::from(out), width)
                })
                .collect()
        }
        OusNode::Diff(content) => {
            let patch = match content {
                DiffContent::Patch(p) => p.clone(),
                DiffContent::Texts {
                    old_text,
                    new_text,
                    path,
                } => {
                    let mut p = format!(
                        "--- {}\n+++ {}\n",
                        path.as_deref().unwrap_or("a"),
                        path.as_deref().unwrap_or("b")
                    );
                    for line in old_text.lines() {
                        let _ = writeln!(p, "-{line}");
                    }
                    for line in new_text.lines() {
                        let _ = writeln!(p, "+{line}");
                    }
                    p
                }
            };
            patch
                .lines()
                .map(|line| {
                    let color = match line.chars().next() {
                        Some('+') => theme.diff_added,
                        Some('-') => theme.diff_removed,
                        Some('@') => theme.accent,
                        _ => theme.diff_context,
                    };
                    super::text::truncate_line(
                        Line::from(Span::styled(line.to_owned(), Style::default().fg(color))),
                        width,
                    )
                })
                .collect()
        }
        OusNode::Image { source, alt } => {
            let what = match source {
                Some(ImageSource::Data { mime_type, .. }) => mime_type.clone(),
                Some(ImageSource::Path(path)) => path.clone(),
                None => String::new(),
            };
            let label = if alt.is_empty() {
                what
            } else {
                format!("{alt} ({what})")
            };
            vec![Line::from(Span::styled(format!("[image: {label}]"), muted))]
        }
        OusNode::Log {
            lines,
            tail: _,
            max_lines,
        } => {
            let max = max_lines.unwrap_or(20);
            let start = lines.len().saturating_sub(max);
            lines[start..]
                .iter()
                .map(|l| {
                    super::text::truncate_line(
                        Line::from(Span::styled(
                            l.clone(),
                            Style::default().fg(theme.tool_output),
                        )),
                        width,
                    )
                })
                .collect()
        }
        OusNode::Kv { items } => {
            let key_width = items
                .iter()
                .map(|(k, _)| k.text().width())
                .max()
                .unwrap_or(0)
                .min(width / 2);
            let mut out = Vec::new();
            for (key, value) in items {
                let key_text = oino_view::format::truncate(&key.text(), key_width, "…");
                let pad = key_width.saturating_sub(key_text.width());
                let prefix = vec![
                    Run::new(key_text, muted),
                    Run::new(format!("{}  ", " ".repeat(pad)), Style::default()),
                ];
                let rest = vec![Run::new(" ".repeat(key_width + 2), Style::default())];
                out.extend(wrap(&cx.runs(value, text_style), width, &prefix, &rest));
            }
            out
        }
        OusNode::Table {
            columns,
            rows,
            selectable,
        } => {
            let focused = selectable.is_some() && cx.next_focus();
            let n = columns
                .len()
                .max(rows.iter().map(Vec::len).max().unwrap_or(0));
            if n == 0 {
                return Vec::new();
            }
            let mut widths = vec![0usize; n];
            for (i, c) in columns.iter().enumerate() {
                widths[i] = c.title.width();
            }
            for row in rows {
                for (i, cell) in row.iter().enumerate() {
                    widths[i] = widths[i].max(cell.text().width());
                }
            }
            let budget = width.saturating_sub(2 * n);
            while widths.iter().sum::<usize>() > budget {
                let Some((i, _)) = widths.iter().enumerate().max_by_key(|(_, w)| **w) else {
                    break;
                };
                if widths[i] <= 3 {
                    break;
                }
                widths[i] -= 1;
            }
            let cell = |text: &str, i: usize, align: Align| {
                let t = oino_view::format::truncate(text, widths[i], "…");
                let pad = widths[i].saturating_sub(t.width());
                match align {
                    Align::Right => format!("{}{t}  ", " ".repeat(pad)),
                    Align::Center => {
                        format!("{}{t}{}  ", " ".repeat(pad / 2), " ".repeat(pad - pad / 2))
                    }
                    Align::Left => format!("{t}{}  ", " ".repeat(pad)),
                }
            };
            let mut out = Vec::new();
            if !columns.is_empty() {
                let header: String = (0..n)
                    .map(|i| {
                        cell(
                            columns.get(i).map_or("", |c| c.title.as_str()),
                            i,
                            columns.get(i).map_or(Align::Left, |c| c.align),
                        )
                    })
                    .collect();
                out.push(Line::from(Span::styled(
                    header.trim_end().to_owned(),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )));
            }
            for (r, row) in rows.iter().enumerate() {
                let text: String = (0..n)
                    .map(|i| {
                        cell(
                            &row.get(i).map(Rich::text).unwrap_or_default(),
                            i,
                            columns.get(i).map_or(Align::Left, |c| c.align),
                        )
                    })
                    .collect();
                let style = cx.focus_style(focused && r == cx.focus.choice, text_style);
                out.push(Line::from(Span::styled(text.trim_end().to_owned(), style)));
            }
            out
        }
        OusNode::List {
            items,
            selectable,
            selected,
            multi: _,
        } => {
            let focused = selectable.is_some() && cx.next_focus();
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let is_cursor = focused && i == cx.focus.choice;
                let is_selected = selected.as_deref() == Some(item.id.as_str());
                let marker = if is_cursor {
                    "› "
                } else if is_selected {
                    "• "
                } else if selectable.is_some() {
                    "  "
                } else {
                    "- "
                };
                let base = cx.focus_style(is_cursor, cx.tone(item.tone).patch(text_style));
                let mut runs = cx.runs(&item.label, base);
                if let Some(badge) = &item.badge {
                    runs.push(Run::new(" ", base));
                    runs.push(Run::new(
                        format!("[{}]", badge.text()),
                        Style::default().fg(theme.accent),
                    ));
                }
                if let Some(detail) = &item.detail {
                    runs.push(Run::new("  ", base));
                    runs.extend(cx.runs(detail, muted));
                }
                out.extend(wrap(
                    &runs,
                    width,
                    &[Run::new(marker, Style::default().fg(theme.accent))],
                    &[Run::new("  ", Style::default())],
                ));
            }
            out
        }
        OusNode::Tree { nodes, selectable } => {
            let _ = selectable.is_some() && cx.next_focus();
            let mut out = Vec::new();
            tree_walk(nodes, "", &mut out, cx, width);
            out
        }
        OusNode::Tabs { tabs, active, .. } => {
            let active = active
                .clone()
                .or_else(|| tabs.first().map(|t| t.id.clone()));
            let mut header = Vec::new();
            for tab in tabs {
                let is_active = Some(&tab.id) == active.as_ref();
                let style = if is_active {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    muted
                };
                header.push(Span::styled(format!(" {} ", tab.title), style));
            }
            let mut out = vec![super::text::truncate_line(Line::from(header), width)];
            if let Some(tab) = tabs.iter().find(|t| Some(&t.id) == active.as_ref()) {
                out.extend(node_lines(&tab.body, width, cx));
            }
            out
        }
        OusNode::Split {
            direction: Direction::Horizontal,
            children,
            ratios,
        } if !children.is_empty() => horizontal(children, ratios, width, cx),
        OusNode::Stack {
            direction: Direction::Horizontal,
            children,
            ..
        } if !children.is_empty() => horizontal(children, &[], width, cx),
        OusNode::Split { children, .. } => children
            .iter()
            .flat_map(|c| node_lines(c, width, cx))
            .collect(),
        OusNode::Stack { children, gap, .. } => {
            let mut out = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    for _ in 0..*gap {
                        out.push(Line::default());
                    }
                }
                out.extend(node_lines(child, width, cx));
            }
            out
        }
        OusNode::Section {
            title,
            body,
            collapsed,
        } => {
            let marker = if *collapsed { "▸ " } else { "▾ " };
            let mut out = wrap(
                &cx.runs(
                    title,
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
                width,
                &[Run::new(marker, Style::default().fg(theme.accent))],
                &[],
            );
            if !collapsed {
                out.extend(indent(
                    node_lines(body, width.saturating_sub(2), cx),
                    "  ",
                    Style::default(),
                ));
            }
            out
        }
        OusNode::Card {
            title,
            rail,
            source,
            body,
        } => {
            let rail_color = theme.tone(rail.unwrap_or(Tone::Muted));
            let rail_style = Style::default().fg(rail_color);
            let mut out = Vec::new();
            if title.is_some() || source.is_some() {
                let mut runs = Vec::new();
                if let Some(title) = title {
                    runs.extend(cx.runs(
                        title,
                        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                    ));
                }
                if let Some(source) = source {
                    runs.push(Run::new(
                        format!("  {source}"),
                        Style::default().fg(theme.dim),
                    ));
                }
                out.extend(indent(
                    wrap(&runs, width.saturating_sub(2), &[], &[]),
                    "▌ ",
                    rail_style,
                ));
            }
            out.extend(indent(
                node_lines(body, width.saturating_sub(2), cx),
                "▌ ",
                rail_style,
            ));
            out
        }
        OusNode::Badge { value, tone } => vec![Line::from(Span::styled(
            format!("[{}]", value.text()),
            Style::default()
                .fg(theme.tone(*tone))
                .add_modifier(Modifier::BOLD),
        ))],
        OusNode::Meter {
            value,
            max,
            label,
            tone,
        } => {
            let ratio = if *max > 0.0 {
                (value / max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            vec![bar_line(
                label.as_deref(),
                ratio,
                &format!("{value}/{max}"),
                theme.tone(*tone),
                width,
                theme,
            )]
        }
        OusNode::Gauge { value, label } => {
            vec![bar_line(
                label.as_deref(),
                *value,
                &format!("{:.0}%", value * 100.0),
                theme.accent,
                width,
                theme,
            )]
        }
        OusNode::Sparkline { values, label } => {
            let max = values.iter().copied().fold(f64::MIN, f64::max);
            let min = values.iter().copied().fold(f64::MAX, f64::min);
            let range = (max - min).max(f64::EPSILON);
            let available = width.saturating_sub(label.as_ref().map_or(0, |l| l.width() + 1));
            let start = values.len().saturating_sub(available);
            let bars: String = values[start..]
                .iter()
                .map(|v| {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "bucket index 0..8"
                    )]
                    let i = (((v - min) / range) * 7.0).round() as usize;
                    SPARK[i.min(7)]
                })
                .collect();
            let mut spans = Vec::new();
            if let Some(label) = label {
                spans.push(Span::styled(format!("{label} "), muted));
            }
            spans.push(Span::styled(bars, Style::default().fg(theme.accent)));
            vec![Line::from(spans)]
        }
        OusNode::BarChart { bars } => {
            let label_width = bars
                .iter()
                .map(|b| b.label.width())
                .max()
                .unwrap_or(0)
                .min(width / 3);
            let max = bars
                .iter()
                .map(|b| b.value)
                .fold(0.0_f64, f64::max)
                .max(f64::EPSILON);
            bars.iter()
                .map(|bar| {
                    let available = width.saturating_sub(label_width + 10);
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::cast_precision_loss,
                        reason = "bar length rounding"
                    )]
                    let len = ((bar.value / max) * available as f64).round() as usize;
                    Line::from(vec![
                        Span::styled(
                            format!(
                                "{:<label_width$} ",
                                oino_view::format::truncate(&bar.label, label_width, "…")
                            ),
                            muted,
                        ),
                        Span::styled("█".repeat(len), Style::default().fg(theme.tone(bar.tone))),
                        Span::styled(format!(" {}", bar.value), muted),
                    ])
                })
                .collect()
        }
        OusNode::Area { series } => series
            .iter()
            .flat_map(|s| {
                node_lines(
                    &OusNode::Sparkline {
                        values: s.values.clone(),
                        label: Some(s.label.clone()),
                    },
                    width,
                    cx,
                )
            })
            .collect(),
        OusNode::Spinner { label } => {
            let frame = SPINNER[usize::try_from(cx.tick % 10).unwrap_or(0)];
            vec![Line::from(vec![
                Span::styled(format!("{frame} "), Style::default().fg(theme.accent)),
                Span::styled(label.clone().unwrap_or_default(), muted),
            ])]
        }
        OusNode::Divider { label } => {
            let style = Style::default().fg(theme.border_muted);
            match label {
                Some(label) => {
                    let rest = width.saturating_sub(label.width() + 4);
                    vec![Line::from(vec![
                        Span::styled("── ", style),
                        Span::styled(label.clone(), muted),
                        Span::styled(format!(" {}", "─".repeat(rest)), style),
                    ])]
                }
                None => vec![Line::from(Span::styled("─".repeat(width), style))],
            }
        }
        OusNode::KeyHints { hints } => {
            let mut spans = Vec::new();
            for (i, hint) in hints.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" · ", muted));
                }
                spans.push(Span::styled(
                    hint.keys.clone(),
                    Style::default().fg(theme.dim),
                ));
                spans.push(Span::styled(format!(" {}", hint.label), muted));
            }
            vec![super::text::truncate_line(Line::from(spans), width)]
        }
        OusNode::Input {
            id,
            value,
            placeholder,
            ..
        } => {
            let focused = cx.next_focus();
            let current = cx
                .focus
                .values
                .and_then(|v| v.get(id))
                .and_then(JsonValue::as_str)
                .unwrap_or(value)
                .to_owned();
            let shown = match (focused, cx.focus.editing) {
                (true, Some(editing)) => format!("{editing}▏"),
                _ if current.is_empty() => placeholder.clone().unwrap_or_default(),
                _ => current.clone(),
            };
            let style = if current.is_empty() && !(focused && cx.focus.editing.is_some()) {
                muted
            } else {
                text_style
            };
            vec![Line::from(vec![
                Span::styled("[ ", Style::default().fg(theme.border_muted)),
                Span::styled(shown, cx.focus_style(focused, style)),
                Span::styled(" ]", Style::default().fg(theme.border_muted)),
            ])]
        }
        OusNode::Toggle {
            id, label, value, ..
        } => {
            let focused = cx.next_focus();
            let on = cx
                .focus
                .values
                .and_then(|v| v.get(id))
                .and_then(JsonValue::as_bool)
                .unwrap_or(*value);
            vec![Line::from(Span::styled(
                format!("[{}] {label}", if on { "x" } else { " " }),
                cx.focus_style(focused, text_style),
            ))]
        }
        OusNode::Select {
            id, options, value, ..
        } => {
            let focused = cx.next_focus();
            let current = cx
                .focus
                .values
                .and_then(|v| v.get(id))
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
                .or_else(|| value.clone());
            let label = options
                .iter()
                .find(|o| Some(&o.value) == current.as_ref())
                .or_else(|| options.first())
                .map_or_else(String::new, |o| o.label.clone());
            vec![Line::from(Span::styled(
                format!("‹ {label} ›"),
                cx.focus_style(focused, text_style),
            ))]
        }
        OusNode::Button {
            label,
            tone,
            primary,
            ..
        } => {
            let focused = cx.next_focus();
            let mut style = Style::default().fg(if *primary {
                theme.accent
            } else {
                theme.tone(*tone)
            });
            if *primary {
                style = style.add_modifier(Modifier::BOLD);
            }
            vec![Line::from(Span::styled(
                format!("[ {label} ]"),
                cx.focus_style(focused, style),
            ))]
        }
        OusNode::Empty { message } => match message {
            Some(m) => vec![Line::from(Span::styled(
                m.clone(),
                Style::default().fg(theme.dim),
            ))],
            None => Vec::new(),
        },
        OusNode::Unknown { fallback, .. } => fallback
            .as_ref()
            .map(|f| node_lines(f, width, cx))
            .unwrap_or_default(),
    }
}

fn tree_walk(items: &[TreeItem], prefix: &str, out: &mut Vec<OLine>, cx: &Ctx<'_>, width: usize) {
    for (i, item) in items.iter().enumerate() {
        let last = i + 1 == items.len();
        let connector = if last { "└─ " } else { "├─ " };
        let fold = if item.children.is_empty() {
            ""
        } else if item.expanded {
            "▾ "
        } else {
            "▸ "
        };
        let first = vec![Run::new(
            format!("{prefix}{connector}{fold}"),
            Style::default().fg(cx.theme.dim),
        )];
        out.extend(wrap(
            &cx.runs(&item.label, Style::default().fg(cx.theme.text)),
            width,
            &first,
            &[],
        ));
        if item.expanded {
            let next = format!("{prefix}{}", if last { "   " } else { "│  " });
            tree_walk(&item.children, &next, out, cx, width);
        }
    }
}

fn horizontal(children: &[OusNode], ratios: &[f64], width: usize, cx: &mut Ctx<'_>) -> Vec<OLine> {
    let shares: Vec<f64> = if ratios.len() == children.len() {
        ratios.to_vec()
    } else {
        vec![1.0; children.len()]
    };
    let total: f64 = shares.iter().sum::<f64>().max(f64::EPSILON);
    let gap = 2;
    let usable = width.saturating_sub(gap * (children.len() - 1));
    let widths: Vec<usize> = shares
        .iter()
        .map(|share| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                reason = "column width rounding"
            )]
            let w = ((usable as f64) * share / total).floor() as usize;
            w.max(1)
        })
        .collect();
    let columns: Vec<Vec<OLine>> = children
        .iter()
        .zip(&widths)
        .map(|(c, w)| node_lines(c, *w, cx))
        .collect();
    let height = columns.iter().map(Vec::len).max().unwrap_or(0);
    (0..height)
        .map(|row| {
            let mut spans = Vec::new();
            for (i, column) in columns.iter().enumerate() {
                let line = column.get(row).cloned().unwrap_or_default();
                let w = line_width(&line);
                spans.extend(line.spans);
                if i + 1 < columns.len() {
                    spans.push(Span::raw(" ".repeat(widths[i].saturating_sub(w) + gap)));
                }
            }
            Line::from(spans)
        })
        .collect()
}

fn bar_line(
    label: Option<&str>,
    ratio: f64,
    suffix: &str,
    color: ratatui::style::Color,
    width: usize,
    theme: &Theme,
) -> OLine {
    let label_text = label.map(|l| format!("{l} ")).unwrap_or_default();
    let available = width
        .saturating_sub(label_text.width() + suffix.width() + 1)
        .max(1);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "bar length rounding"
    )]
    let filled = ((ratio.clamp(0.0, 1.0)) * available as f64).round() as usize;
    Line::from(vec![
        Span::styled(label_text, Style::default().fg(theme.muted)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled(
            "░".repeat(available - filled.min(available)),
            Style::default().fg(theme.border_muted),
        ),
        Span::styled(format!(" {suffix}"), Style::default().fg(theme.muted)),
    ])
}
