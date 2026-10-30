//! ratatui `Scrollbar` (§4.5.1).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};
use crate::style::Style;

/// Scrollbar orientation and edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScrollbarOrientation {
    /// Vertical, right edge (ratatui default).
    #[default]
    VerticalRight,
    /// Vertical, left edge.
    VerticalLeft,
    /// Horizontal, bottom edge.
    HorizontalBottom,
    /// Horizontal, top edge.
    HorizontalTop,
}

/// ratatui `ScrollbarState`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollbarState {
    /// Total content length.
    pub content_length: u32,
    /// Current position.
    pub position: u32,
    /// Viewport length (derived from the area when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_content_length: Option<u32>,
}

impl ScrollbarState {
    /// State over the given content length.
    #[must_use]
    pub fn new(content_length: u32) -> Self {
        Self {
            content_length,
            position: 0,
            viewport_content_length: None,
        }
    }

    /// Sets the position.
    #[must_use]
    pub fn position(mut self, position: u32) -> Self {
        self.position = position;
        self
    }

    /// Sets the viewport length.
    #[must_use]
    pub fn viewport_content_length(mut self, length: u32) -> Self {
        self.viewport_content_length = Some(length);
        self
    }
}

/// ratatui `Scrollbar` (§4.5.1): a read-only position indicator; scrollable
/// widgets use their own native `scrollbar` flag instead.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scrollbar {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Orientation (default `VerticalRight`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<ScrollbarOrientation>,
    /// Content / position state.
    pub state: ScrollbarState,
    /// Thumb style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_style: Option<Style>,
    /// Track style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_style: Option<Style>,
    /// Begin symbol (`null` = none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub begin_symbol: Option<String>,
    /// End symbol (`null` = none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_symbol: Option<String>,
}

impl Scrollbar {
    /// A scrollbar over the given state.
    #[must_use]
    pub fn new(state: ScrollbarState) -> Self {
        Self {
            common: Common::new(),
            orientation: None,
            state,
            thumb_style: None,
            track_style: None,
            begin_symbol: None,
            end_symbol: None,
        }
    }

    /// Sets the orientation.
    #[must_use]
    pub fn orientation(mut self, orientation: ScrollbarOrientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    /// Sets the thumb style.
    #[must_use]
    pub fn thumb_style(mut self, style: Style) -> Self {
        self.thumb_style = Some(style);
        self
    }

    /// Sets the track style.
    #[must_use]
    pub fn track_style(mut self, style: Style) -> Self {
        self.track_style = Some(style);
        self
    }

    /// Sets the begin symbol.
    #[must_use]
    pub fn begin_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.begin_symbol = Some(symbol.into());
        self
    }

    /// Sets the end symbol.
    #[must_use]
    pub fn end_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.end_symbol = Some(symbol.into());
        self
    }
}

impl IntoNode for Scrollbar {}
impl From<Scrollbar> for Node {
    fn from(s: Scrollbar) -> Self {
        Node::Scrollbar(s)
    }
}
