//! `Input` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::text::Line;

/// A text field (§4.5.2). Typed text is local state; `values` collects it on
/// submission (§4.6).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    /// Common node props (`key` is required for `Input`).
    #[serde(flatten)]
    pub common: Common,
    /// Initial value (also drives the field when it changes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Field label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Line>,
    /// Multi-line (default `false`; intrinsic height `rows`, default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiline: Option<bool>,
    /// Rows when `multiline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    /// Mask the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<bool>,
    /// Maximum length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// Greyed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Action on Enter (the string value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_submit: Option<ActionId>,
    /// Action on change, debounced 150 ms (the string value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<ActionId>,
}

impl Input {
    /// An input with the given `key` (used to collect `values`, §4.6).
    #[must_use]
    pub fn new(key: impl Into<Key>) -> Self {
        let mut common = Common::new();
        common.key = Some(key.into());
        Self {
            common,
            value: None,
            placeholder: None,
            label: None,
            multiline: None,
            rows: None,
            secret: None,
            max_length: None,
            disabled: None,
            on_submit: None,
            on_change: None,
        }
    }

    /// Sets the initial value.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Sets the placeholder.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Sets the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Line>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Multi-line with the given row count.
    #[must_use]
    pub fn multiline(mut self, rows: u16) -> Self {
        self.multiline = Some(true);
        self.rows = Some(rows);
        self
    }

    /// Masks the text.
    #[must_use]
    pub fn secret(mut self, secret: bool) -> Self {
        self.secret = Some(secret);
        self
    }

    /// Sets the maximum length.
    #[must_use]
    pub fn max_length(mut self, max_length: u32) -> Self {
        self.max_length = Some(max_length);
        self
    }

    /// Greys the field out.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    /// Sets the submit action.
    #[must_use]
    pub fn on_submit(mut self, action: impl Into<ActionId>) -> Self {
        self.on_submit = Some(action.into());
        self
    }

    /// Sets the change action.
    #[must_use]
    pub fn on_change(mut self, action: impl Into<ActionId>) -> Self {
        self.on_change = Some(action.into());
        self
    }
}

impl IntoNode for Input {}
impl From<Input> for Node {
    fn from(i: Input) -> Self {
        Node::Input(i)
    }
}
