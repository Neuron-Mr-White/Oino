//! `KeyHints` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{ActionId, Common, IntoNode, Node};

/// One key hint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hint {
    /// Key id.
    pub keys: String,
    /// Hint label.
    pub label: String,
    /// Action fired when the hint is tapped (phone shows only tappable hints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ActionId>,
}

impl Hint {
    /// A hint without an action.
    #[must_use]
    pub fn new(keys: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            keys: keys.into(),
            label: label.into(),
            action: None,
        }
    }

    /// A hint that fires an action.
    #[must_use]
    pub fn with_action(
        keys: impl Into<String>,
        label: impl Into<String>,
        action: impl Into<ActionId>,
    ) -> Self {
        Self {
            keys: keys.into(),
            label: label.into(),
            action: Some(action.into()),
        }
    }
}

impl From<(&str, &str)> for Hint {
    fn from((keys, label): (&str, &str)) -> Self {
        Self::new(keys, label)
    }
}

impl From<(&str, &str, &str)> for Hint {
    fn from((keys, label, action): (&str, &str, &str)) -> Self {
        Self::with_action(keys, label, action)
    }
}

/// ratatui-style key hints line (§4.5.2): `enter open · n new`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyHints {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Hints, in display order.
    pub hints: Vec<Hint>,
}

impl KeyHints {
    /// Hints from anything convertible to [`Hint`] (`("enter", "open")` tuples).
    #[must_use]
    pub fn new(hints: impl IntoIterator<Item = impl Into<Hint>>) -> Self {
        Self {
            hints: hints.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    /// Attaches an action to the hint with the given `keys` (§11.3).
    #[must_use]
    pub fn hint_action(mut self, keys: &str, action: impl Into<ActionId>) -> Self {
        for hint in &mut self.hints {
            if hint.keys == keys {
                hint.action = Some(action.into());
                break;
            }
        }
        self
    }
}

impl IntoNode for KeyHints {}
impl From<KeyHints> for Node {
    fn from(k: KeyHints) -> Self {
        Node::KeyHints(k)
    }
}
