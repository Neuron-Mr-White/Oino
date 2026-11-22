//! `Spinner` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};
use crate::text::Line;

/// An activity indicator (§4.5.2). TUI: braille spinner on the shared ticker;
/// app: CSS animation (static dot under `prefers-reduced-motion`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spinner {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Optional label next to the spinner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Line>,
}

impl Spinner {
    /// A spinner with a label.
    #[must_use]
    pub fn new(label: impl Into<Line>) -> Self {
        Self {
            common: Common::new(),
            label: Some(label.into()),
        }
    }

    /// Sets the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Line>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl IntoNode for Spinner {}
impl From<Spinner> for Node {
    fn from(s: Spinner) -> Self {
        Node::Spinner(s)
    }
}
