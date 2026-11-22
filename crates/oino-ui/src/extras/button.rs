//! `Button` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::extras::icon::Icon;
use crate::node::{ActionId, Common, IntoNode, Node};
use crate::text::Line;
use serde_json::Value as Json;

/// Button visual weight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ButtonVariant {
    /// Primary action.
    Primary,
    /// Default weight.
    #[default]
    Secondary,
    /// Quiet text button.
    Ghost,
    /// Destructive action.
    Danger,
}

/// A button (§4.5.2). Fires its `action` with `value` (§4.6); `confirm` shows an
/// inline y/n popover first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Button {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Button label.
    pub label: Line,
    /// Action id fired on click.
    pub action: ActionId,
    /// Action value (default `null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Json>,
    /// Visual weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<ButtonVariant>,
    /// Leading icon (required for `Danger` in the app).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Greyed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Ask for confirmation first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

impl Button {
    /// A button.
    #[must_use]
    pub fn new(label: impl Into<Line>, action: impl Into<ActionId>) -> Self {
        Self {
            common: Common::new(),
            label: label.into(),
            action: action.into(),
            value: None,
            variant: None,
            icon: None,
            disabled: None,
            confirm: None,
        }
    }

    /// Sets the action value.
    #[must_use]
    pub fn value(mut self, value: impl Into<Json>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Sets the visual weight.
    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = Some(variant);
        self
    }

    /// Sets the leading icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Greys the button out.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    /// Asks for confirmation first.
    #[must_use]
    pub fn confirm(mut self, confirm: impl Into<String>) -> Self {
        self.confirm = Some(confirm.into());
        self
    }
}

impl IntoNode for Button {}
impl From<Button> for Node {
    fn from(b: Button) -> Self {
        Node::Button(b)
    }
}
