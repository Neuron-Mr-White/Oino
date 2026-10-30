//! `Empty` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::extras::button::Button;
use crate::extras::icon::Icon;
use crate::node::{Common, IntoNode, Node};

/// An empty state (§4.5.2): dim centred title / message with an optional action
/// button.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Empty {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Message under the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Leading icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Action button — a `Button` node (addressed by the `"action"` path
    /// segment, §4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Box<Node>>,
}

impl Empty {
    /// An empty state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            common: Common::new(),
            title: None,
            message: None,
            icon: None,
            action: None,
        }
    }

    /// Sets the title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the message.
    #[must_use]
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Sets the icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the action button (stored as a tagged `Button` node).
    #[must_use]
    pub fn action(mut self, action: Button) -> Self {
        self.action = Some(Box::new(Node::Button(action)));
        self
    }
}

impl Default for Empty {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoNode for Empty {}
impl From<Empty> for Node {
    fn from(e: Empty) -> Self {
        Node::Empty(e)
    }
}
