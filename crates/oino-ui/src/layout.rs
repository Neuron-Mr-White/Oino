//! Layout node (§4.3): `Layout`, [`Direction`], [`Constraint`], [`Flex`] and [`Margin`].
//!
//! Data only — the solver lives in the renderers (ratatui 0.29 in the TUI, a port of
//! the same formulation in the app, checked against `tests/layout_vectors.json` in WP-U2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};

/// Layout main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// Top to bottom.
    #[default]
    Vertical,
    /// Left to right.
    Horizontal,
}

/// ratatui `Flex`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Flex {
    /// Legacy ratatui behaviour (extra space to the last element).
    Legacy,
    /// Extra space after the children (ratatui 0.29 default).
    #[default]
    Start,
    /// Extra space before the children.
    End,
    /// Extra space split around the children.
    Center,
    /// Equal space between children.
    SpaceBetween,
    /// Equal space around children.
    SpaceAround,
}

/// ratatui `Constraint` in its serde form: `{"Length":3}`; a bare integer `n`
/// deserializes as `{"Length": n}` (§4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Constraint {
    /// Exactly `n` cells.
    Length(u16),
    /// Percentage of the whole area (0..=100).
    Percentage(u16),
    /// `a / max(b, 1)` of the whole area, serialized as `[a, b]`.
    Ratio(u32, u32),
    /// At least `n` cells; grows like `Fill(1)` when `Flex != Legacy`.
    Min(u16),
    /// At most `n` cells (prefers exactly `n`).
    Max(u16),
    /// Excess space proportional to the weight.
    Fill(u16),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ConstraintRaw {
    Bare(u16),
    Tagged(ConstraintTagged),
}

#[derive(Deserialize)]
enum ConstraintTagged {
    Length(u16),
    Percentage(u16),
    /// `[a, b]`.
    Ratio([u32; 2]),
    Min(u16),
    Max(u16),
    Fill(u16),
}

impl<'de> Deserialize<'de> for Constraint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match ConstraintRaw::deserialize(deserializer)? {
            ConstraintRaw::Bare(n) => Ok(Self::Length(n)),
            ConstraintRaw::Tagged(tagged) => Ok(match tagged {
                ConstraintTagged::Length(n) => Self::Length(n),
                ConstraintTagged::Percentage(p) => Self::Percentage(p),
                ConstraintTagged::Ratio([a, b]) => Self::Ratio(a, b),
                ConstraintTagged::Min(n) => Self::Min(n),
                ConstraintTagged::Max(n) => Self::Max(n),
                ConstraintTagged::Fill(w) => Self::Fill(w),
            }),
        }
    }
}

/// Inset of the whole layout area: a number (all sides) or `{horizontal, vertical}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Margin {
    /// Same inset on all sides.
    Uniform(u16),
    /// Separate horizontal / vertical insets.
    Axes {
        /// Left and right inset.
        horizontal: u16,
        /// Top and bottom inset.
        vertical: u16,
    },
}

impl Margin {
    /// Same inset on all sides.
    #[must_use]
    pub fn uniform(n: u16) -> Self {
        Self::Uniform(n)
    }

    /// Separate horizontal / vertical insets.
    #[must_use]
    pub fn hv(horizontal: u16, vertical: u16) -> Self {
        Self::Axes {
            horizontal,
            vertical,
        }
    }
}

/// ratatui `Layout` as a node: data, not geometry (§4.3). There is no `split`;
/// children are attached and the renderer solves the main-axis sizes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Main axis (default `Vertical`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// One constraint per child; missing ones are `Fill(1)`, extras ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Constraint>,
    /// Children (≤ 64, §4.8).
    pub children: Vec<Node>,
    /// Gap between children, in cells.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<u16>,
    /// Inset of the whole area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<Margin>,
    /// ratatui `Flex` (default `Start`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex: Option<Flex>,
    /// Horizontal only: below this many columns, children lay out vertically in
    /// auto mode (§4.3). `0` disables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapse: Option<u16>,
}

impl Layout {
    /// Vertical layout with one constraint per child.
    #[must_use]
    pub fn vertical(constraints: impl IntoIterator<Item = Constraint>) -> Self {
        Self {
            direction: Some(Direction::Vertical),
            constraints: constraints.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Horizontal layout with one constraint per child.
    #[must_use]
    pub fn horizontal(constraints: impl IntoIterator<Item = Constraint>) -> Self {
        Self {
            direction: Some(Direction::Horizontal),
            constraints: constraints.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the constraints.
    #[must_use]
    pub fn constraints(mut self, constraints: impl IntoIterator<Item = Constraint>) -> Self {
        self.constraints = constraints.into_iter().collect();
        self
    }

    /// Attaches children (`header.into()` converts builder values to [`Node`]).
    #[must_use]
    pub fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// Sets the gap between children.
    #[must_use]
    pub fn spacing(mut self, spacing: u16) -> Self {
        self.spacing = Some(spacing);
        self
    }

    /// Sets the inset of the whole area.
    #[must_use]
    pub fn margin(mut self, margin: Margin) -> Self {
        self.margin = Some(margin);
        self
    }

    /// Sets the `Flex` mode.
    #[must_use]
    pub fn flex(mut self, flex: Flex) -> Self {
        self.flex = Some(flex);
        self
    }

    /// Sets the horizontal collapse threshold (§4.3).
    #[must_use]
    pub fn collapse(mut self, collapse: u16) -> Self {
        self.collapse = Some(collapse);
        self
    }
}

impl IntoNode for Layout {}
impl From<Layout> for Node {
    fn from(l: Layout) -> Self {
        Node::Layout(l)
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

    #[test]
    fn bare_integer_is_length() {
        let c: Constraint = serde_json::from_str("3").expect("bare int parses");
        assert_eq!(c, Constraint::Length(3));
        let c: Constraint = serde_json::from_str(r#"{"Fill":1}"#).expect("tagged parses");
        assert_eq!(c, Constraint::Fill(1));
        let c: Constraint = serde_json::from_str(r#"{"Ratio":[1,4]}"#).expect("ratio parses");
        assert_eq!(c, Constraint::Ratio(1, 4));
    }
}
