//! ratatui `Gauge` and `LineGauge` (§4.5.1).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};
use crate::style::Style;
use crate::text::{Line, Span};

/// ratatui `Gauge` (§4.5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gauge {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Fill ratio `0.0..=1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// Percentage shorthand `0..=100`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u16>,
    /// Label (default: the percentage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Span>,
    /// Fill style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gauge_style: Option<Style>,
    /// Use unicode half blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_unicode: Option<bool>,
}

impl Gauge {
    /// Empty gauge (`ratio(0.0)`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            common: Common::new(),
            ratio: None,
            percent: None,
            label: None,
            gauge_style: None,
            use_unicode: None,
        }
    }

    /// Sets the fill ratio.
    #[must_use]
    pub fn ratio(mut self, ratio: f64) -> Self {
        self.ratio = Some(ratio);
        self
    }

    /// Sets the fill percentage.
    #[must_use]
    pub fn percent(mut self, percent: u16) -> Self {
        self.percent = Some(percent);
        self
    }

    /// Sets the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Span>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the fill style.
    #[must_use]
    pub fn gauge_style(mut self, style: Style) -> Self {
        self.gauge_style = Some(style);
        self
    }

    /// Uses unicode half blocks.
    #[must_use]
    pub fn use_unicode(mut self, use_unicode: bool) -> Self {
        self.use_unicode = Some(use_unicode);
        self
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoNode for Gauge {}
impl From<Gauge> for Node {
    fn from(g: Gauge) -> Self {
        Node::Gauge(g)
    }
}

/// Line glyph set of a [`LineGauge`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineSet {
    /// `━`.
    #[default]
    Normal,
    /// `█`.
    Thick,
    /// `═`.
    Double,
}

/// ratatui `LineGauge` (§4.5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineGauge {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Fill ratio `0.0..=1.0`.
    pub ratio: f64,
    /// Label line (default: the percentage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Line>,
    /// Filled-part style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filled_style: Option<Style>,
    /// Unfilled-part style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unfilled_style: Option<Style>,
    /// Line glyph set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_set: Option<LineSet>,
}

impl LineGauge {
    /// A gauge with the given ratio.
    #[must_use]
    pub fn new(ratio: f64) -> Self {
        Self {
            common: Common::new(),
            ratio,
            label: None,
            filled_style: None,
            unfilled_style: None,
            line_set: None,
        }
    }

    /// Sets the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Line>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the filled-part style.
    #[must_use]
    pub fn filled_style(mut self, style: Style) -> Self {
        self.filled_style = Some(style);
        self
    }

    /// Sets the unfilled-part style.
    #[must_use]
    pub fn unfilled_style(mut self, style: Style) -> Self {
        self.unfilled_style = Some(style);
        self
    }

    /// Sets the line glyph set.
    #[must_use]
    pub fn line_set(mut self, line_set: LineSet) -> Self {
        self.line_set = Some(line_set);
        self
    }
}

impl Default for LineGauge {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl IntoNode for LineGauge {}
impl From<LineGauge> for Node {
    fn from(g: LineGauge) -> Self {
        Node::LineGauge(g)
    }
}
