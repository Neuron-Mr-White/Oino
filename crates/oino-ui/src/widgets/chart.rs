//! ratatui `Chart` (§4.5.1).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};
use crate::style::Style;
use crate::text::{Alignment, Line, Span};

/// Point marker glyph set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Marker {
    /// One pixel per point.
    Dot,
    /// Braille-density pixels.
    #[default]
    Braille,
    /// Half-block pixels.
    Block,
    /// Whole-block pixels.
    Bar,
    /// Half-block pixels, other half.
    HalfBlock,
}

/// How a dataset is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphType {
    /// Points only.
    #[default]
    Scatter,
    /// Points joined left to right.
    Line,
    /// Vertical bars from the x axis.
    Bar,
}

/// Legend position (Appendix A `legendPosition`; the schema also allows `null` =
/// hide the legend, which parses to `None`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LegendPosition {
    /// Top centre.
    #[default]
    Top,
    /// Top right.
    TopRight,
    /// Top left.
    TopLeft,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Bottom centre.
    Bottom,
    /// Bottom right.
    BottomRight,
    /// Bottom left.
    BottomLeft,
}

/// One chart axis.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Axis {
    /// Axis title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Line>,
    /// Value range `[min, max]`.
    #[serde(default)]
    pub bounds: [f64; 2],
    /// Fixed tick labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Span>,
    /// Tick-label alignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels_alignment: Option<Alignment>,
    /// Axis style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
}

impl Axis {
    /// An axis over the given bounds.
    #[must_use]
    pub fn bounds(min: f64, max: f64) -> Self {
        Self {
            title: None,
            bounds: [min, max],
            labels: Vec::new(),
            labels_alignment: None,
            style: None,
        }
    }

    /// Sets the axis title.
    #[must_use]
    pub fn title(mut self, title: impl Into<Line>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets fixed tick labels.
    #[must_use]
    pub fn labels(mut self, labels: impl IntoIterator<Item = Span>) -> Self {
        self.labels = labels.into_iter().collect();
        self
    }

    /// Sets the tick-label alignment.
    #[must_use]
    pub fn labels_alignment(mut self, alignment: Alignment) -> Self {
        self.labels_alignment = Some(alignment);
        self
    }

    /// Sets the axis style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
}

/// One chart dataset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dataset {
    /// Legend name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Line>,
    /// `[x, y]` points (Σ points ≤ 10 000, §4.8).
    pub data: Vec<[f64; 2]>,
    /// Point marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<Marker>,
    /// Rendering mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_type: Option<GraphType>,
    /// Dataset style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
}

impl Dataset {
    /// A dataset from `[x, y]` points.
    #[must_use]
    pub fn new(data: impl IntoIterator<Item = [f64; 2]>) -> Self {
        Self {
            data: data.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the legend name.
    #[must_use]
    pub fn name(mut self, name: impl Into<Line>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the point marker.
    #[must_use]
    pub fn marker(mut self, marker: Marker) -> Self {
        self.marker = Some(marker);
        self
    }

    /// Sets the rendering mode.
    #[must_use]
    pub fn graph_type(mut self, graph_type: GraphType) -> Self {
        self.graph_type = Some(graph_type);
        self
    }

    /// Sets the dataset style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
}

/// ratatui `Chart` (§4.5.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chart {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Datasets (Σ points ≤ 10 000, §4.8).
    pub datasets: Vec<Dataset>,
    /// X axis.
    #[serde(default)]
    pub x_axis: Axis,
    /// Y axis.
    #[serde(default)]
    pub y_axis: Axis,
    /// Legend position (absent = renderer default; `null` = hide).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_position: Option<LegendPosition>,
}

impl Chart {
    /// A chart from datasets.
    #[must_use]
    pub fn new(datasets: impl IntoIterator<Item = Dataset>) -> Self {
        Self {
            datasets: datasets.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the x axis.
    #[must_use]
    pub fn x_axis(mut self, axis: Axis) -> Self {
        self.x_axis = axis;
        self
    }

    /// Sets the y axis.
    #[must_use]
    pub fn y_axis(mut self, axis: Axis) -> Self {
        self.y_axis = axis;
        self
    }

    /// Sets the legend position.
    #[must_use]
    pub fn legend_position(mut self, position: LegendPosition) -> Self {
        self.legend_position = Some(position);
        self
    }

    /// Hides the legend (wire `null`).
    #[must_use]
    pub fn hide_legend(mut self) -> Self {
        self.legend_position = None;
        self
    }
}

impl IntoNode for Chart {}
impl From<Chart> for Node {
    fn from(c: Chart) -> Self {
        Node::Chart(c)
    }
}
