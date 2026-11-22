//! ratatui `Sparkline` (§4.5.1).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};

/// Sparkline drawing direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SparkDirection {
    /// Oldest value on the left.
    #[default]
    LeftToRight,
    /// Oldest value on the right.
    RightToLeft,
}

/// Bar glyph set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BarSet {
    /// Nine levels (ratatui default `▁▂▃▄▅▆▇█`).
    #[default]
    Nine,
    /// Three levels (`░▒▓`).
    Three,
}

/// ratatui `Sparkline` (§4.5.1). `None` data points render the absent-value symbol.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sparkline {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Values (≤ 1 000, §4.8).
    pub data: Vec<Option<u64>>,
    /// Maximum value (auto from data when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
    /// Drawing direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<SparkDirection>,
    /// Bar glyph set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_set: Option<BarSet>,
    /// Symbol for `None` points (ratatui default `" "`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absent_value_symbol: Option<String>,
}

impl Sparkline {
    /// A sparkline from data points.
    #[must_use]
    pub fn new(data: impl IntoIterator<Item = Option<u64>>) -> Self {
        Self {
            data: data.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the maximum value.
    #[must_use]
    pub fn max(mut self, max: u64) -> Self {
        self.max = Some(max);
        self
    }

    /// Sets the drawing direction.
    #[must_use]
    pub fn direction(mut self, direction: SparkDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Sets the bar glyph set.
    #[must_use]
    pub fn bar_set(mut self, bar_set: BarSet) -> Self {
        self.bar_set = Some(bar_set);
        self
    }

    /// Sets the absent-value symbol.
    #[must_use]
    pub fn absent_value_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.absent_value_symbol = Some(symbol.into());
        self
    }
}

impl IntoNode for Sparkline {}
impl From<Sparkline> for Node {
    fn from(s: Sparkline) -> Self {
        Node::Sparkline(s)
    }
}
