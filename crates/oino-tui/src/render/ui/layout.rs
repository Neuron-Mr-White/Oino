//! Node `Layout` → ratatui layout solver (§4.3, D-U4).
//!
//! The TUI uses ratatui 0.29's `Layout` directly: constraints, `Flex`, spacing and
//! margin have one meaning in both renderers. `collapse` (§4.3 phone rule) is applied
//! by the caller, which knows the intrinsic heights of the children.

use oino_ui::layout::{Constraint, Direction, Flex, Margin};
use ratatui::layout::{
    Constraint as RConstraint, Direction as RDirection, Flex as RFlex, Layout, Margin as RMargin,
    Rect,
};

/// Direction conversion.
#[must_use]
pub(crate) fn direction(direction: Option<Direction>) -> RDirection {
    match direction.unwrap_or_default() {
        Direction::Vertical => RDirection::Vertical,
        Direction::Horizontal => RDirection::Horizontal,
    }
}

/// Constraint conversion (ratatui serde form, §4.3).
#[must_use]
pub(crate) fn constraint(constraint: Constraint) -> RConstraint {
    match constraint {
        Constraint::Length(n) => RConstraint::Length(n),
        Constraint::Percentage(p) => RConstraint::Percentage(p),
        Constraint::Ratio(a, b) => RConstraint::Ratio(a, b),
        Constraint::Min(n) => RConstraint::Min(n),
        Constraint::Max(n) => RConstraint::Max(n),
        Constraint::Fill(w) => RConstraint::Fill(w),
    }
}

/// Flex conversion.
#[must_use]
pub(crate) fn flex(flex: Option<Flex>) -> RFlex {
    match flex.unwrap_or_default() {
        Flex::Legacy => RFlex::Legacy,
        Flex::Start => RFlex::Start,
        Flex::End => RFlex::End,
        Flex::Center => RFlex::Center,
        Flex::SpaceBetween => RFlex::SpaceBetween,
        Flex::SpaceAround => RFlex::SpaceAround,
    }
}

/// Margin conversion.
#[must_use]
pub(crate) fn margin(margin: Margin) -> RMargin {
    match margin {
        Margin::Uniform(n) => RMargin {
            horizontal: n,
            vertical: n,
        },
        Margin::Axes {
            horizontal,
            vertical,
        } => RMargin {
            horizontal,
            vertical,
        },
    }
}

/// One constraint per child: missing ones are `Fill(1)`, extras are ignored (§4.3).
#[must_use]
pub(crate) fn constraints(layout: &oino_ui::layout::Layout) -> Vec<RConstraint> {
    (0..layout.children.len())
        .map(|i| {
            layout
                .constraints
                .get(i)
                .copied()
                .map_or(RConstraint::Fill(1), constraint)
        })
        .collect()
}

/// Solves the layout inside `area` (definite mode, §4.3). No `collapse` handling: the
/// renderer measures the children first (§4.4) and lays them out vertically when the
/// area is narrower than `collapse`.
#[must_use]
pub(crate) fn split(layout: &oino_ui::layout::Layout, area: Rect) -> Vec<Rect> {
    let mut builder = Layout::default()
        .direction(direction(layout.direction))
        .constraints(constraints(layout))
        .flex(flex(layout.flex));
    if let Some(spacing) = layout.spacing {
        builder = builder.spacing(spacing);
    }
    if let Some(m) = layout.margin {
        let margin = margin(m);
        builder = builder
            .horizontal_margin(margin.horizontal)
            .vertical_margin(margin.vertical);
    }
    builder.split(area).to_vec()
}

/// Vertical auto-mode split: `heights` one row each, clipped to `area` (§4.4, §4.3
/// `collapse`). Returns one rect per child, zero-height children included.
#[must_use]
pub(crate) fn split_vertical_auto(heights: &[u16], spacing: u16, area: Rect) -> Vec<Rect> {
    let mut y = area.y;
    let mut out = Vec::with_capacity(heights.len());
    for (index, height) in heights.iter().enumerate() {
        if index > 0 {
            y = y.saturating_add(spacing);
        }
        let remaining = area.bottom().saturating_sub(y);
        let h = (*height).min(remaining);
        out.push(Rect {
            x: area.x,
            y,
            width: area.width,
            height: h,
        });
        y = y.saturating_add(h);
    }
    out
}

/// Whether a `Layout` collapses at `width` (§4.3 `collapse`, horizontal only).
#[must_use]
pub(crate) fn collapses(layout: &oino_ui::layout::Layout, width: u16) -> bool {
    let collapse = layout.collapse.unwrap_or(60);
    collapse > 0 && matches!(layout.direction, Some(Direction::Horizontal)) && width < collapse
}
