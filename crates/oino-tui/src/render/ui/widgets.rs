//! ratatui widgets of the v2 node tree (§4.5.1): `Block`, `Paragraph`, `List` + `ListState`,
//! `Table` + `TableState`, `Tabs`, `Gauge`, `LineGauge`, `Sparkline`, `BarChart`, `Chart`,
//! `Scrollbar`.
//!
//! Blocks are attached with the builder (`.block(..)`) so the widget renders inside the
//! block's inner area (§4.1).

use oino_ui::layout::Direction;
use oino_ui::node::Node;
use oino_ui::widgets::block::BlockProps;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Axis, Bar, BarChart, BarGroup, Block, Chart, Dataset, Gauge, LegendPosition, LineGauge, List,
    ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Sparkline, StatefulWidget, Table, Tabs as TabsWidget, Widget, Wrap,
};

use super::style;
use super::{Ctx, RenderState, focus, measure};

/// The ratatui block of a node's common `block` prop (§4.1).
pub(crate) fn block_of(node: &Node, base: Style, cx: &Ctx<'_>) -> Option<Block<'static>> {
    measure::common_block(node).map(|spec| style::block(&spec, None, base, cx.theme).style(base))
}

/// The block style of common `block` props (background fill of the area).
fn block_style(props: Option<&BlockProps>, base: Style, cx: &Ctx<'_>) -> Style {
    props
        .and_then(|props| props.style.as_ref())
        .map_or(base, |s| style::style(s, base, cx.theme))
}

/// `Paragraph` conversion (also used for measuring, §4.4).
pub(crate) fn paragraph(
    paragraph: &oino_ui::widgets::paragraph::Paragraph,
    cx: &Ctx<'_>,
) -> Paragraph<'static> {
    let base = style::patch(paragraph.common.style.as_ref(), cx.base(), cx.theme);
    let mut widget = Paragraph::new(style::text(&paragraph.text, base, cx.theme))
        .style(base)
        .alignment(style::alignment(paragraph.alignment));
    if let Some(wrap) = paragraph.wrap {
        widget = match wrap {
            oino_ui::widgets::paragraph::Wrap::Disabled(false) => widget.wrap(Wrap { trim: true }),
            oino_ui::widgets::paragraph::Wrap::Disabled(true) => widget,
            oino_ui::widgets::paragraph::Wrap::Enabled { trim } => widget.wrap(Wrap { trim }),
        };
    } else {
        widget = widget.wrap(Wrap { trim: true });
    }
    if let Some([y, x]) = paragraph.scroll {
        widget = widget.scroll((y, x));
    }
    widget
}

/// Rendering of every ratatui-backed node kind.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per ratatui widget (ui-protocol §4.5.1)"
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
    buffer.set_style(area, block_style(node.common().block, base, cx));
    match node {
        Node::Paragraph(paragraph) => {
            let mut widget = self::paragraph(paragraph, cx);
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            widget.render(area, buffer);
        }
        Node::List(list) => {
            state.next_focus();
            let items: Vec<ListItem<'static>> = list
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| list_item(item, index, path, cx))
                .collect();
            let selected = local_selection(list, path, cx);
            let selected = selected.min(list.items.len().saturating_sub(1));
            let mut widget = List::new(items)
                .style(base)
                .direction(match list.direction.unwrap_or_default() {
                    oino_ui::widgets::list::ListDirection::TopToBottom => {
                        ratatui::widgets::ListDirection::TopToBottom
                    }
                    oino_ui::widgets::list::ListDirection::BottomToTop => {
                        ratatui::widgets::ListDirection::BottomToTop
                    }
                })
                .repeat_highlight_symbol(list.repeat_highlight_symbol.unwrap_or(false))
                .highlight_spacing(match list.highlight_spacing.unwrap_or_default() {
                    oino_ui::widgets::list::HighlightSpacing::Always => {
                        ratatui::widgets::HighlightSpacing::Always
                    }
                    oino_ui::widgets::list::HighlightSpacing::WhenSelected => {
                        ratatui::widgets::HighlightSpacing::WhenSelected
                    }
                    oino_ui::widgets::list::HighlightSpacing::Never => {
                        ratatui::widgets::HighlightSpacing::Never
                    }
                });
            if let Some(style) = list.highlight_style.as_ref() {
                widget = widget.highlight_style(style::style(style, base, cx.theme));
            }
            if let Some(symbol) = &list.highlight_symbol {
                widget = widget.highlight_symbol(symbol.as_str());
            }
            if let Some(padding) = list.scroll_padding {
                widget = widget.scroll_padding(usize::from(padding));
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            let mut list_state = ListState::default().with_selected(Some(selected));
            StatefulWidget::render(&widget, area, buffer, &mut list_state);
            if list.scrollbar.unwrap_or(false) {
                let inner = block_of(node, base, cx).map_or(area, |b| b.inner(area));
                render_scrollbar(
                    inner,
                    buffer,
                    cx,
                    list.items.len(),
                    selected,
                    list.state
                        .as_ref()
                        .and_then(|s| s.offset)
                        .map_or(0, u32::from),
                );
            }
        }
        Node::Table(table) => {
            state.next_focus();
            let mut widget = Table::new(
                table
                    .rows
                    .iter()
                    .map(|row| table_row(row, cx, base))
                    .collect::<Vec<_>>(),
                table
                    .widths
                    .iter()
                    .copied()
                    .map(super::layout::constraint)
                    .collect::<Vec<Constraint>>(),
            )
            .style(base)
            .column_spacing(table.column_spacing.unwrap_or(1));
            if let Some(header) = &table.header {
                widget = widget.header(table_row(header, cx, base));
            }
            if let Some(footer) = &table.footer {
                widget = widget.footer(table_row(footer, cx, base));
            }
            if let Some(style) = table.row_highlight_style.as_ref() {
                widget = widget.row_highlight_style(style::style(style, base, cx.theme));
            }
            if let Some(symbol) = &table.highlight_symbol {
                widget = widget.highlight_symbol(symbol.clone());
            }
            widget = widget.highlight_spacing(match table.highlight_spacing.unwrap_or_default() {
                oino_ui::widgets::list::HighlightSpacing::Always => {
                    ratatui::widgets::HighlightSpacing::Always
                }
                oino_ui::widgets::list::HighlightSpacing::WhenSelected => {
                    ratatui::widgets::HighlightSpacing::WhenSelected
                }
                oino_ui::widgets::list::HighlightSpacing::Never => {
                    ratatui::widgets::HighlightSpacing::Never
                }
            });
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            let selected = table_selection(table, path, cx).min(table.rows.len().saturating_sub(1));
            let mut table_state =
                ratatui::widgets::TableState::default().with_selected(Some(selected));
            StatefulWidget::render(&widget, area, buffer, &mut table_state);
        }
        Node::Tabs(tabs) => {
            state.next_focus();
            let titles: Vec<Line<'static>> = tabs
                .titles
                .iter()
                .map(|title| tabs_title(title, base, cx))
                .collect();
            let selected = local_tabs_selection(tabs, path, cx);
            let mut widget = TabsWidget::new(titles)
                .style(base)
                .select(Some(selected))
                .divider(
                    tabs.divider
                        .as_ref()
                        .map_or_else(|| Span::from("│"), |span| style::span(span, base, cx.theme)),
                );
            let (left, right) = tabs.padding.as_ref().map_or_else(
                || (Line::from(""), Line::from("")),
                |[left, right]| {
                    (
                        style::line(left, base, cx.theme),
                        style::line(right, base, cx.theme),
                    )
                },
            );
            widget = widget.padding(left, right);
            if let Some(style) = tabs.highlight_style.as_ref() {
                widget = widget.highlight_style(style::style(style, base, cx.theme));
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            if let Some(panes) = &tabs.panes {
                let inner = block_of(node, base, cx).map_or(area, |b| b.inner(area));
                let [bar, pane] =
                    ratatui::layout::Layout::vertical([Constraint::Length(1), Constraint::Fill(1)])
                        .areas(inner);
                widget.render(bar, buffer);
                if let Some(key) = tabs.titles.get(selected).map(|title| &title.key)
                    && let Some(pane_node) = panes.get(key)
                {
                    super::render_node(
                        pane_node,
                        &focus::join(path, key),
                        pane,
                        buffer,
                        cx,
                        state,
                        base,
                    );
                }
            } else {
                widget.render(area, buffer);
            }
        }
        Node::Gauge(gauge) => {
            let ratio = gauge
                .ratio
                .or_else(|| gauge.percent.map(|p| f64::from(p) / 100.0))
                .unwrap_or_default();
            let mut widget = Gauge::default()
                .ratio(ratio.clamp(0.0, 1.0))
                .style(base)
                .use_unicode(gauge.use_unicode.unwrap_or(false));
            if let Some(label) = &gauge.label {
                widget = widget.label(style::span(label, base, cx.theme));
            }
            if let Some(style) = gauge.gauge_style.as_ref() {
                widget = widget.gauge_style(style::style(style, base, cx.theme));
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            widget.render(area, buffer);
        }
        Node::LineGauge(gauge) => {
            let mut widget = LineGauge::default()
                .ratio(gauge.ratio.clamp(0.0, 1.0))
                .style(base)
                .line_set(match gauge.line_set.unwrap_or_default() {
                    oino_ui::widgets::gauge::LineSet::Normal => ratatui::symbols::line::NORMAL,
                    oino_ui::widgets::gauge::LineSet::Thick => ratatui::symbols::line::THICK,
                    oino_ui::widgets::gauge::LineSet::Double => ratatui::symbols::line::DOUBLE,
                });
            if let Some(label) = &gauge.label {
                widget = widget.label(style::line(label, base, cx.theme));
            }
            if let Some(style) = gauge.filled_style.as_ref() {
                widget = widget.filled_style(style::style(style, base, cx.theme));
            }
            if let Some(style) = gauge.unfilled_style.as_ref() {
                widget = widget.unfilled_style(style::style(style, base, cx.theme));
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            widget.render(area, buffer);
        }
        Node::Sparkline(sparkline) => {
            let mut widget = Sparkline::default()
                .data(sparkline.data.iter().copied().map(|v| v.unwrap_or(0)))
                .style(base);
            if let Some(max) = sparkline.max {
                widget = widget.max(max);
            }
            widget = widget.direction(match sparkline.direction.unwrap_or_default() {
                oino_ui::widgets::sparkline::SparkDirection::LeftToRight => {
                    ratatui::widgets::RenderDirection::LeftToRight
                }
                oino_ui::widgets::sparkline::SparkDirection::RightToLeft => {
                    ratatui::widgets::RenderDirection::RightToLeft
                }
            });
            widget = widget.bar_set(match sparkline.bar_set.unwrap_or_default() {
                oino_ui::widgets::sparkline::BarSet::Nine => ratatui::symbols::bar::NINE_LEVELS,
                oino_ui::widgets::sparkline::BarSet::Three => ratatui::symbols::bar::THREE_LEVELS,
            });
            if let Some(symbol) = &sparkline.absent_value_symbol
                && let Some(c) = symbol.chars().next()
            {
                widget = widget.absent_value_symbol(c);
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            widget.render(area, buffer);
        }
        Node::BarChart(chart) => {
            let groups: Vec<BarGroup<'static>> = chart
                .data
                .iter()
                .map(|group| {
                    let bars: Vec<Bar<'static>> = group
                        .bars
                        .iter()
                        .map(|bar| {
                            let mut widget = Bar::default().value(bar.value);
                            widget = widget.text_value(
                                bar.text_value
                                    .clone()
                                    .unwrap_or_else(|| bar.value.to_string()),
                            );
                            if let Some(label) = &bar.label {
                                widget = widget.label(style::line(label, base, cx.theme));
                            }
                            if let Some(style) = bar.style.as_ref() {
                                widget = widget.style(style::style(style, base, cx.theme));
                            }
                            if let Some(style) = bar.value_style.as_ref() {
                                widget = widget.value_style(style::style(style, base, cx.theme));
                            }
                            widget
                        })
                        .collect();
                    let mut widget = BarGroup::default().bars(&bars);
                    if let Some(label) = &group.label {
                        widget = widget.label(style::line(label, base, cx.theme));
                    }
                    widget
                })
                .collect();
            let mut widget = BarChart::default()
                .style(base)
                .bar_width(chart.bar_width.unwrap_or(1))
                .bar_gap(chart.bar_gap.unwrap_or(1))
                .group_gap(chart.group_gap.unwrap_or(0))
                .direction(match chart.direction.unwrap_or(Direction::Vertical) {
                    Direction::Vertical => ratatui::layout::Direction::Vertical,
                    Direction::Horizontal => ratatui::layout::Direction::Horizontal,
                });
            for group in groups {
                widget = widget.data(group);
            }
            if let Some(max) = chart.max {
                widget = widget.max(max);
            }
            if let Some(style) = chart.bar_style.as_ref() {
                widget = widget.bar_style(style::style(style, base, cx.theme));
            }
            if let Some(style) = chart.value_style.as_ref() {
                widget = widget.value_style(style::style(style, base, cx.theme));
            }
            if let Some(style) = chart.label_style.as_ref() {
                widget = widget.label_style(style::style(style, base, cx.theme));
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            widget.render(area, buffer);
        }
        Node::Chart(chart) => {
            let points: Vec<Vec<(f64, f64)>> = chart
                .datasets
                .iter()
                .map(|dataset| dataset.data.iter().map(|[x, y]| (*x, *y)).collect())
                .collect();
            let datasets: Vec<Dataset<'_>> = chart
                .datasets
                .iter()
                .zip(&points)
                .map(|(dataset, data)| {
                    let mut widget = Dataset::default()
                        .data(data)
                        .marker(match dataset.marker.unwrap_or_default() {
                            oino_ui::widgets::chart::Marker::Dot => ratatui::symbols::Marker::Dot,
                            oino_ui::widgets::chart::Marker::Braille => {
                                ratatui::symbols::Marker::Braille
                            }
                            oino_ui::widgets::chart::Marker::Block => {
                                ratatui::symbols::Marker::Block
                            }
                            oino_ui::widgets::chart::Marker::Bar => ratatui::symbols::Marker::Bar,
                            oino_ui::widgets::chart::Marker::HalfBlock => {
                                ratatui::symbols::Marker::HalfBlock
                            }
                        })
                        .graph_type(match dataset.graph_type.unwrap_or_default() {
                            oino_ui::widgets::chart::GraphType::Scatter => {
                                ratatui::widgets::GraphType::Scatter
                            }
                            oino_ui::widgets::chart::GraphType::Line => {
                                ratatui::widgets::GraphType::Line
                            }
                            oino_ui::widgets::chart::GraphType::Bar => {
                                ratatui::widgets::GraphType::Bar
                            }
                        });
                    if let Some(name) = &dataset.name {
                        widget = widget.name(style::line(name, base, cx.theme));
                    }
                    if let Some(style) = dataset.style.as_ref() {
                        widget = widget.style(style::style(style, base, cx.theme));
                    }
                    widget
                })
                .collect();
            let mut widget = Chart::new(datasets)
                .style(base)
                .x_axis(axis(&chart.x_axis, base, cx))
                .y_axis(axis(&chart.y_axis, base, cx));
            if let Some(position) = chart.legend_position {
                widget = widget.legend_position(Some(match position {
                    oino_ui::widgets::chart::LegendPosition::Top => LegendPosition::Top,
                    oino_ui::widgets::chart::LegendPosition::TopRight => LegendPosition::TopRight,
                    oino_ui::widgets::chart::LegendPosition::TopLeft => LegendPosition::TopLeft,
                    oino_ui::widgets::chart::LegendPosition::Left => LegendPosition::Left,
                    oino_ui::widgets::chart::LegendPosition::Right => LegendPosition::Right,
                    oino_ui::widgets::chart::LegendPosition::Bottom => LegendPosition::Bottom,
                    oino_ui::widgets::chart::LegendPosition::BottomRight => {
                        LegendPosition::BottomRight
                    }
                    oino_ui::widgets::chart::LegendPosition::BottomLeft => {
                        LegendPosition::BottomLeft
                    }
                }));
            } else {
                widget = widget.legend_position(None);
            }
            if let Some(block) = block_of(node, base, cx) {
                widget = widget.block(block);
            }
            widget.render(area, buffer);
        }
        Node::Scrollbar(scrollbar) => {
            let mut state =
                ScrollbarState::new(usize::try_from(scrollbar.state.content_length).unwrap_or(0))
                    .position(usize::try_from(scrollbar.state.position).unwrap_or(0));
            if let Some(viewport) = scrollbar.state.viewport_content_length {
                state = state.viewport_content_length(usize::try_from(viewport).unwrap_or(0));
            }
            let mut widget = Scrollbar::new(match scrollbar.orientation.unwrap_or_default() {
                oino_ui::widgets::scrollbar::ScrollbarOrientation::VerticalRight => {
                    ScrollbarOrientation::VerticalRight
                }
                oino_ui::widgets::scrollbar::ScrollbarOrientation::VerticalLeft => {
                    ScrollbarOrientation::VerticalLeft
                }
                oino_ui::widgets::scrollbar::ScrollbarOrientation::HorizontalBottom => {
                    ScrollbarOrientation::HorizontalBottom
                }
                oino_ui::widgets::scrollbar::ScrollbarOrientation::HorizontalTop => {
                    ScrollbarOrientation::HorizontalTop
                }
            })
            .style(base);
            if let Some(style) = scrollbar.thumb_style.as_ref() {
                widget = widget.thumb_style(style::style(style, base, cx.theme));
            }
            if let Some(style) = scrollbar.track_style.as_ref() {
                widget = widget.track_style(style::style(style, base, cx.theme));
            }
            if let Some(symbol) = &scrollbar.begin_symbol {
                widget = widget.begin_symbol(Some(symbol.as_str()));
            }
            if let Some(symbol) = &scrollbar.end_symbol {
                widget = widget.end_symbol(Some(symbol.as_str()));
            }
            StatefulWidget::render(widget, area, buffer, &mut state);
        }
        _ => {}
    }
}

fn axis(axis: &oino_ui::widgets::chart::Axis, base: Style, cx: &Ctx<'_>) -> Axis<'static> {
    let mut widget = Axis::default().bounds(axis.bounds).style(style::patch(
        axis.style.as_ref(),
        base,
        cx.theme,
    ));
    if let Some(title) = &axis.title {
        widget = widget.title(style::line(title, base, cx.theme));
    }
    if !axis.labels.is_empty() {
        widget = widget.labels(
            axis.labels
                .iter()
                .map(|label| style::span(label, base, cx.theme))
                .collect::<Vec<_>>(),
        );
    }
    if let Some(alignment) = axis.labels_alignment {
        widget = widget.labels_alignment(style::alignment(Some(alignment)));
    }
    widget
}

fn local_selection(list: &oino_ui::widgets::list::List, path: &str, cx: &Ctx<'_>) -> usize {
    let declared = list.state.as_ref().and_then(|state| {
        state.selected.as_ref().and_then(|selection| match selection {
            oino_ui::widgets::list::Selection::One(key) => list.items.iter().position(|item| {
                matches!(item, oino_ui::widgets::list::ListItem::Full { key: Some(k), .. } if k == key)
            }),
            oino_ui::widgets::list::Selection::Many(_) => None,
        })
    });
    cx.local.selected(path, declared.unwrap_or(0))
}

fn table_selection(table: &oino_ui::widgets::table::Table, path: &str, cx: &Ctx<'_>) -> usize {
    let declared = table.state.as_ref().and_then(|state| {
        state
            .selected
            .as_ref()
            .and_then(|selection| match selection {
                oino_ui::widgets::list::Selection::One(key) => table
                    .rows
                    .iter()
                    .position(|row| row.key.as_deref() == Some(key.as_str())),
                oino_ui::widgets::list::Selection::Many(_) => None,
            })
    });
    cx.local.selected(path, declared.unwrap_or(0))
}

fn local_tabs_selection(tabs: &oino_ui::widgets::tabs::Tabs, path: &str, cx: &Ctx<'_>) -> usize {
    let declared = tabs
        .selected
        .as_ref()
        .and_then(|key| tabs.titles.iter().position(|title| &title.key == key))
        .unwrap_or(0);
    cx.local
        .selected(path, declared)
        .min(tabs.titles.len().saturating_sub(1))
}

fn tabs_title(
    title: &oino_ui::widgets::tabs::TabTitle,
    base: Style,
    cx: &Ctx<'_>,
) -> Line<'static> {
    let mut line = style::line(&title.title, base, cx.theme);
    if let Some(badge) = &title.badge {
        let mut spans = line.spans;
        spans.push(Span::raw(" "));
        spans.extend(style::line(badge, base, cx.theme).spans);
        line = Line::from(spans);
    }
    line
}

fn list_item(
    item: &oino_ui::widgets::list::ListItem,
    index: usize,
    path: &str,
    cx: &Ctx<'_>,
) -> ListItem<'static> {
    use oino_ui::widgets::list::ListItem as Item;
    let base = cx.base();
    match item {
        Item::Text(text) => {
            let _ = (index, path);
            ListItem::new(style::text(text, base, cx.theme))
        }
        Item::Full {
            key,
            content,
            style: item_style,
            detail,
            badge,
            icon,
            disabled,
        } => {
            let item_path = focus::join(path, &key.clone().unwrap_or_else(|| format!("#{index}")));
            let style = style::patch(item_style.as_ref(), base, cx.theme);
            let mut lines: Vec<Line<'static>> = style::text(content, style, cx.theme).lines;
            if let Some(detail) = detail {
                let muted = Style::default().fg(cx.theme.muted);
                lines.push(style::line(detail, muted, cx.theme));
            }
            if let Some(badge) = badge
                && let Some(first) = lines.first_mut()
            {
                let badge_line = style::line(badge, style, cx.theme);
                let mut spans = first.spans.clone();
                spans.push(Span::raw("  "));
                spans.extend(badge_line.spans);
                *first = Line::from(spans);
            }
            if let Some(icon) = icon
                && let Some(first) = lines.first_mut()
            {
                let mut spans = vec![Span::styled(
                    format!("{} ", super::extras::glyph(*icon)),
                    style,
                )];
                spans.extend(first.spans.clone());
                *first = Line::from(spans);
            }
            if disabled.unwrap_or(false) {
                for line in &mut lines {
                    line.style = line.style.add_modifier(Modifier::DIM);
                }
            }
            if cx
                .local
                .multi
                .get(path)
                .is_some_and(|set| set.contains(&index))
                && let Some(first) = lines.first_mut()
            {
                let mut spans = vec![Span::styled("✔ ", style.add_modifier(Modifier::BOLD))];
                spans.extend(first.spans.clone());
                *first = Line::from(spans);
            }
            let _ = item_path;
            ListItem::new(Text::from(lines))
        }
    }
}

fn table_row(row: &oino_ui::widgets::table::Row, cx: &Ctx<'_>, base: Style) -> Row<'static> {
    let style = style::patch(row.style.as_ref(), base, cx.theme);
    let cells: Vec<ratatui::widgets::Cell<'static>> = row
        .cells
        .iter()
        .map(|cell| {
            let (text, cell_style) = match cell {
                oino_ui::widgets::table::Cell::Text(text) => (text, None),
                oino_ui::widgets::table::Cell::Full {
                    content,
                    style: cell_style,
                } => (content, cell_style.as_ref()),
            };
            ratatui::widgets::Cell::from(style::text(text, style, cx.theme))
                .style(style::patch(cell_style, style, cx.theme))
        })
        .collect();
    let mut out = Row::new(cells).style(style);
    if let Some(height) = row.height {
        out = out.height(height);
    }
    if let Some(margin) = row.bottom_margin {
        out = out.bottom_margin(margin);
    }
    out
}

fn render_scrollbar(
    area: Rect,
    buffer: &mut Buffer,
    cx: &Ctx<'_>,
    content: usize,
    position: usize,
    offset: u32,
) {
    let mut state = ScrollbarState::new(content)
        .position(position.saturating_add(usize::try_from(offset).unwrap_or(0)));
    let widget = Scrollbar::default()
        .orientation(ScrollbarOrientation::VerticalRight)
        .style(cx.base());
    StatefulWidget::render(widget, area, buffer, &mut state);
}
