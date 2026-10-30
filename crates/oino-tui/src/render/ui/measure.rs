//! Intrinsic heights of nodes at a width (§4.4), plus block geometry.
//!
//! Auto slots (widgets, turn footers, headers, dialogs' `detail`, …) allocate the height
//! this module returns, capped by the slot maximum. Definite slots pass their own rect and
//! only use the block helpers.

use oino_ui::node::Node;
use ratatui::layout::Rect;

use super::Ctx;
use super::style::{BlockSpec, block};
use crate::theme::Theme;

/// Block spec of a node's common `block` prop (leaf widgets, §4.1).
#[must_use]
pub(crate) fn common_block(node: &Node) -> Option<BlockSpec<'_>> {
    node.common().block.map(BlockSpec::from)
}

/// The inner area of a node's block chrome (the whole area when it has none).
#[must_use]
pub(crate) fn inner(area: Rect, spec: Option<BlockSpec<'_>>, theme: &Theme) -> Rect {
    match spec {
        None => area,
        Some(spec) => block(&spec, None, ratatui::style::Style::default(), theme).inner(area),
    }
}

/// Height spent on a block's top/bottom borders and padding.
#[must_use]
pub(crate) fn block_chrome(spec: &BlockSpec<'_>) -> u16 {
    let mut height = 0;
    if let Some(borders) = spec.borders {
        let sides: &[oino_ui::widgets::block::BorderSide] = match borders {
            oino_ui::widgets::block::Borders::All => &[
                oino_ui::widgets::block::BorderSide::TOP,
                oino_ui::widgets::block::BorderSide::BOTTOM,
            ],
            oino_ui::widgets::block::Borders::Sides(sides) => sides,
            _ => &[],
        };
        for side in sides {
            if matches!(
                side,
                oino_ui::widgets::block::BorderSide::TOP
                    | oino_ui::widgets::block::BorderSide::BOTTOM
            ) {
                height += 1;
            }
        }
    }
    if let Some(padding) = spec.padding {
        height += match padding {
            oino_ui::widgets::block::Padding::Uniform(n) => n.saturating_mul(2),
            oino_ui::widgets::block::Padding::Rect { top, bottom, .. } => {
                top.unwrap_or(0) + bottom.unwrap_or(0)
            }
        };
    }
    height
}

/// Intrinsic height of `node` at `width` columns (§4.4), never zero for a visible node.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per node kind (ui-protocol §4.4)"
)]
#[must_use]
pub(crate) fn height(node: &Node, width: u16, cx: &Ctx<'_>) -> u16 {
    if width == 0 {
        return 0;
    }
    let spec = common_block(node);
    let chrome = spec.map_or(0, |spec| block_chrome(&spec));
    let inner_width = inner(
        Rect {
            x: 0,
            y: 0,
            width,
            height: 1,
        },
        spec,
        cx.theme,
    )
    .width;
    let content = match node {
        Node::Layout(layout) => layout_height(layout, inner_width, cx),
        Node::Block(block) => {
            let spec = BlockSpec::from(block);
            let chrome = block_chrome(&spec);
            let collapsed = block.collapsed.unwrap_or(false);
            if collapsed {
                chrome
            } else if let Some(child) = &block.child {
                let area = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height: 1,
                };
                let inner =
                    super::style::block(&spec, None, ratatui::style::Style::default(), cx.theme)
                        .inner(area);
                height(child, inner.width, cx).saturating_add(chrome)
            } else {
                chrome
            }
        }
        Node::Paragraph(paragraph) => paragraph_height(paragraph, inner_width, cx)
            .max(1)
            .saturating_add(chrome),
        Node::Tree(tree) => u16::try_from(super::focus::tree_rows(tree, "", cx.local).len())
            .unwrap_or(u16::MAX)
            .max(1)
            .saturating_add(chrome),
        Node::List(list) => {
            // An item is its content lines plus one muted detail line (§4.5.1).
            let rows: u16 = list
                .items
                .iter()
                .map(|item| match item {
                    oino_ui::widgets::list::ListItem::Text(text) => {
                        u16::try_from(text.lines().len()).unwrap_or(u16::MAX)
                    }
                    oino_ui::widgets::list::ListItem::Full {
                        content, detail, ..
                    } => u16::try_from(content.lines().len())
                        .unwrap_or(u16::MAX)
                        .saturating_add(u16::from(detail.is_some())),
                })
                .sum();
            rows.max(1).saturating_add(chrome)
        }
        Node::Table(table) => {
            let rows = |row: &oino_ui::widgets::table::Row| {
                row.height.unwrap_or(1) + row.bottom_margin.unwrap_or(0)
            };
            let mut n = table.rows.iter().map(rows).sum::<u16>();
            if let Some(header) = &table.header {
                n += rows(header);
            }
            if let Some(footer) = &table.footer {
                n += rows(footer);
            }
            n.max(1).saturating_add(chrome)
        }
        Node::Tabs(tabs) => {
            let selected = super::extras::selected_tab(tabs);
            let pane = selected
                .and_then(|key| tabs.panes.as_ref().and_then(|panes| panes.get(key)))
                .map_or(0, |pane| height(pane, inner_width, cx));
            1_u16.saturating_add(pane).saturating_add(chrome)
        }
        Node::Gauge(_)
        | Node::LineGauge(_)
        | Node::KeyHints(_)
        | Node::Spinner(_)
        | Node::Button(_)
        | Node::Toggle(_)
        | Node::Select(_) => 1_u16.saturating_add(chrome),
        // A vertical scrollbar's intrinsic height is its parent's; auto slots have none, so
        // the renderer reserves a 3-row minimum (§4.4).
        Node::Scrollbar(scrollbar) => {
            let height: u16 = match scrollbar.orientation.unwrap_or_default() {
                oino_ui::widgets::scrollbar::ScrollbarOrientation::HorizontalTop
                | oino_ui::widgets::scrollbar::ScrollbarOrientation::HorizontalBottom => 1,
                _ => 3,
            };
            height.saturating_add(chrome)
        }
        // Sparklines are 3 rows; the TUI has no image protocol, so an `Image` is the
        // documented 3-row `[image: alt]` dim box; `Empty` is 3 rows too (§4.4).
        Node::Sparkline(_) | Node::Image(_) | Node::Empty(_) => 3_u16.saturating_add(chrome),
        Node::BarChart(_) => 8_u16.saturating_add(chrome),
        Node::Chart(_) => 12_u16.saturating_add(chrome),
        Node::Log(log) => {
            let cap = log.max_lines.unwrap_or(2_000).min(5_000);
            u16::try_from(log.lines.len())
                .unwrap_or(u16::MAX)
                .min(u16::try_from(cap).unwrap_or(u16::MAX))
                .max(1)
                .saturating_add(chrome)
        }
        Node::Markdown(markdown) => {
            u16::try_from(super::extras::markdown_lines(markdown, inner_width, cx).len())
                .unwrap_or(u16::MAX)
                .max(1)
                .saturating_add(chrome)
        }
        Node::Code(code) => u16::try_from(super::extras::code_lines(code, cx).len())
            .unwrap_or(u16::MAX)
            .max(1)
            .saturating_add(chrome),
        Node::Diff(diff) => u16::try_from(super::extras::diff_lines(diff, cx).len())
            .unwrap_or(u16::MAX)
            .max(1)
            .saturating_add(chrome),
        Node::Input(input) => {
            let rows = if input.multiline.unwrap_or(false) {
                input.rows.unwrap_or(3).max(1)
            } else {
                1
            };
            rows.saturating_add(chrome)
        }
        Node::Unknown(unknown) => unknown
            .fallback
            .as_deref()
            .map_or(0, |fallback| height(fallback, width, cx)),
    };
    content.max(1)
}

/// Vertical: sum + spacing + margin; horizontal: the tallest child (§4.4).
fn layout_height(layout: &oino_ui::layout::Layout, width: u16, cx: &Ctx<'_>) -> u16 {
    let collapsed = super::layout::collapses(layout, width);
    let horizontal = matches!(
        layout.direction,
        Some(oino_ui::layout::Direction::Horizontal)
    ) && !collapsed;
    let (horizontal_margin, vertical_margin) = match layout.margin {
        None => (0, 0),
        Some(oino_ui::layout::Margin::Uniform(n)) => (n, n),
        Some(oino_ui::layout::Margin::Axes {
            horizontal,
            vertical,
        }) => (horizontal, vertical),
    };
    let content_width = width.saturating_sub(horizontal_margin.saturating_mul(2));
    let heights: Vec<u16> = layout
        .children
        .iter()
        .map(|child| height(child, content_width, cx))
        .collect();
    let spacing = layout.spacing.unwrap_or(0);
    if horizontal {
        let tallest = heights.iter().copied().max().unwrap_or(1);
        tallest.saturating_add(vertical_margin.saturating_mul(2))
    } else {
        let sum: u16 = heights.iter().copied().sum();
        let gaps =
            spacing.saturating_mul(u16::try_from(heights.len().saturating_sub(1)).unwrap_or(0));
        sum.saturating_add(gaps)
            .saturating_add(vertical_margin.saturating_mul(2))
            .max(1)
    }
}

/// Wrapped line count of a `Paragraph` at `width` (§4.4; ratatui's own `line_count` is
/// unstable, so the renderer counts with its own width-aware wrapper).
fn paragraph_height(
    paragraph: &oino_ui::widgets::paragraph::Paragraph,
    width: u16,
    cx: &Ctx<'_>,
) -> u16 {
    use crate::render::text::{Run, wrap};
    if matches!(
        paragraph.wrap,
        Some(oino_ui::widgets::paragraph::Wrap::Disabled(false))
    ) {
        return u16::try_from(paragraph.text.lines().len()).unwrap_or(u16::MAX);
    }
    let base = cx.base();
    let mut total = 0_usize;
    for line in paragraph.text.lines() {
        let runs: Vec<Run> = line
            .spans()
            .iter()
            .map(|span| {
                let style = super::style::patch(span.style(), base, cx.theme);
                Run::new(span.content().to_owned(), style)
            })
            .collect();
        total += wrap(&runs, usize::from(width.max(1)), &[], &[]).len();
    }
    u16::try_from(total).unwrap_or(u16::MAX)
}
