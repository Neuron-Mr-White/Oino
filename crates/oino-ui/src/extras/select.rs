//! `Select` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::text::Line;

/// One select option.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectOption {
    /// Option value (sent as the action value).
    pub value: String,
    /// Option label.
    pub label: Line,
    /// Second-line description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SelectOption {
    /// An option with value and label.
    #[must_use]
    pub fn new(value: impl Into<String>, label: impl Into<Line>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
        }
    }

    /// Sets the description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl From<(&str, &str)> for SelectOption {
    fn from((value, label): (&str, &str)) -> Self {
        Self::new(value, label)
    }
}

/// A cycling select field (§4.5.2): `‹ value ›`; Enter opens the list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Select {
    /// Common node props (`key` is required for `Select`).
    #[serde(flatten)]
    pub common: Common,
    /// Options.
    pub options: Vec<SelectOption>,
    /// Current value (wins when it changes, §4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Placeholder when nothing is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Greyed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Action on change (the option value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<ActionId>,
}

impl Select {
    /// A select with the given `key` and options.
    #[must_use]
    pub fn new(
        key: impl Into<Key>,
        options: impl IntoIterator<Item = impl Into<SelectOption>>,
    ) -> Self {
        let mut common = Common::new();
        common.key = Some(key.into());
        Self {
            common,
            options: options.into_iter().map(Into::into).collect(),
            value: None,
            placeholder: None,
            disabled: None,
            on_change: None,
        }
    }

    /// Sets the current value.
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

    /// Greys the select out.
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

impl IntoNode for Select {}
impl From<Select> for Node {
    fn from(s: Select) -> Self {
        Node::Select(s)
    }
}
