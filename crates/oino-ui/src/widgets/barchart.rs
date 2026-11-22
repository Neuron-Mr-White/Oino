//! ratatui `BarChart` (§4.5.1).

use serde::{Deserialize, Serialize};

use crate::layout::Direction;
use crate::node::{ActionId, Common, IntoNode, Node};
use crate::style::Style;
use crate::text::Line;

/// One bar of a group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bar {
    /// Bar height.
    pub value: u64,
    /// Bar label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Line>,
    /// Text printed over the bar instead of the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_value: Option<String>,
    /// Bar style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
    /// Value-label style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_style: Option<Style>,
}

impl Bar {
    /// A bar of the given height.
    #[must_use]
    pub fn new(value: u64) -> Self {
        Self {
            value,
            label: None,
            text_value: None,
            style: None,
            value_style: None,
        }
    }

    /// Sets the bar label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Line>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the text printed over the bar.
    #[must_use]
    pub fn text_value(mut self, text: impl Into<String>) -> Self {
        self.text_value = Some(text.into());
        self
    }

    /// Sets the bar style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Sets the value-label style.
    #[must_use]
    pub fn value_style(mut self, style: Style) -> Self {
        self.value_style = Some(style);
        self
    }
}

/// A group of bars with an optional group label.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarGroup {
    /// Group label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Line>,
    /// Bars of the group.
    pub bars: Vec<Bar>,
}

impl BarGroup {
    /// A group from bars.
    #[must_use]
    pub fn new(bars: impl IntoIterator<Item = Bar>) -> Self {
        Self {
            label: None,
            bars: bars.into_iter().collect(),
        }
    }

    /// Sets the group label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Line>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// ratatui `BarChart` (§4.5.1). Actions report `"<groupIndex>:<barIndex>"` (§4.5.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarChart {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Bar groups.
    pub data: Vec<BarGroup>,
    /// Bar width in columns (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_width: Option<u16>,
    /// Gap between bars of a group (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_gap: Option<u16>,
    /// Gap between groups (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_gap: Option<u16>,
    /// Orientation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// Maximum value (auto from data when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
    /// Default bar style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_style: Option<Style>,
    /// Default value-label style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_style: Option<Style>,
    /// Default label style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_style: Option<Style>,
    /// GUI extra: action on bar select.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_select: Option<ActionId>,
}

impl BarChart {
    /// A chart from groups.
    #[must_use]
    pub fn new(data: impl IntoIterator<Item = BarGroup>) -> Self {
        Self {
            data: data.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Convenience: one group from `(label, value)` bars.
    #[must_use]
    pub fn from_pairs(pairs: impl IntoIterator<Item = (impl Into<Line>, u64)>) -> Self {
        Self::new(vec![BarGroup::new(
            pairs
                .into_iter()
                .map(|(label, value)| Bar::new(value).label(label)),
        )])
    }

    /// Sets the bar width.
    #[must_use]
    pub fn bar_width(mut self, width: u16) -> Self {
        self.bar_width = Some(width);
        self
    }

    /// Sets the gap between bars of a group.
    #[must_use]
    pub fn bar_gap(mut self, gap: u16) -> Self {
        self.bar_gap = Some(gap);
        self
    }

    /// Sets the gap between groups.
    #[must_use]
    pub fn group_gap(mut self, gap: u16) -> Self {
        self.group_gap = Some(gap);
        self
    }

    /// Sets the orientation.
    #[must_use]
    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Sets the maximum value.
    #[must_use]
    pub fn max(mut self, max: u64) -> Self {
        self.max = Some(max);
        self
    }

    /// Sets the default bar style.
    #[must_use]
    pub fn bar_style(mut self, style: Style) -> Self {
        self.bar_style = Some(style);
        self
    }

    /// Sets the default value-label style.
    #[must_use]
    pub fn value_style(mut self, style: Style) -> Self {
        self.value_style = Some(style);
        self
    }

    /// Sets the default label style.
    #[must_use]
    pub fn label_style(mut self, style: Style) -> Self {
        self.label_style = Some(style);
        self
    }

    /// Sets the select action.
    #[must_use]
    pub fn on_select(mut self, action: impl Into<ActionId>) -> Self {
        self.on_select = Some(action.into());
        self
    }
}

impl IntoNode for BarChart {}
impl From<BarChart> for Node {
    fn from(c: BarChart) -> Self {
        Node::BarChart(c)
    }
}
