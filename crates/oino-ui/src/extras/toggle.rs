//! `Toggle` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::text::Line;

/// A boolean switch (§4.5.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toggle {
    /// Common node props (`key` is required for `Toggle`).
    #[serde(flatten)]
    pub common: Common,
    /// Switch label.
    pub label: Line,
    /// Current value (wins when it changes, §4.6).
    pub value: bool,
    /// Greyed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Action on change (the new bool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<ActionId>,
}

impl Toggle {
    /// A toggle with `key`, label and value.
    #[must_use]
    pub fn new(key: impl Into<Key>, label: impl Into<Line>, value: bool) -> Self {
        let mut common = Common::new();
        common.key = Some(key.into());
        Self {
            common,
            label: label.into(),
            value,
            disabled: None,
            on_change: None,
        }
    }

    /// Greys the toggle out.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    /// Sets the change action.
    #[must_use]
    pub fn on_change(mut self, action: impl Into<ActionId>) -> Self {
        self.on_change = Some(action.into());
        self
    }
}

impl IntoNode for Toggle {}
impl From<Toggle> for Node {
    fn from(t: Toggle) -> Self {
        Node::Toggle(t)
    }
}
