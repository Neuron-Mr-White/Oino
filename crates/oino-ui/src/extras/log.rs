//! `Log` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};
use crate::text::Line;

/// A tailing log (§4.5.2). `lines` is a ring of `maxLines`; older lines are
/// dropped silently (§4.8). Follow pauses when the user scrolls up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Log lines (ring-trimmed to `maxLines`).
    pub lines: Vec<Line>,
    /// Ring capacity (default 2 000, ≤ 5 000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u32>,
    /// Stick to the tail (default `true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<bool>,
    /// Wrap long lines (default `false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
    /// Render line numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
}

impl Log {
    /// A log from lines.
    #[must_use]
    pub fn new(lines: impl IntoIterator<Item = impl Into<Line>>) -> Self {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    /// Appends lines (the wire keeps ≤ `maxLines`, §4.8).
    #[must_use]
    pub fn lines(mut self, lines: impl IntoIterator<Item = impl Into<Line>>) -> Self {
        self.lines.extend(lines.into_iter().map(Into::into));
        self
    }

    /// Sets the ring capacity.
    #[must_use]
    pub fn max_lines(mut self, max_lines: u32) -> Self {
        self.max_lines = Some(max_lines);
        self
    }

    /// Sets tail following.
    #[must_use]
    pub fn follow(mut self, follow: bool) -> Self {
        self.follow = Some(follow);
        self
    }

    /// Wraps long lines.
    #[must_use]
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = Some(wrap);
        self
    }

    /// Renders line numbers.
    #[must_use]
    pub fn line_numbers(mut self, line_numbers: bool) -> Self {
        self.line_numbers = Some(line_numbers);
        self
    }
}

impl IntoNode for Log {}
impl From<Log> for Node {
    fn from(l: Log) -> Self {
        Node::Log(l)
    }
}
