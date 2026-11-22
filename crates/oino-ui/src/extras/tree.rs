//! `Tree` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::extras::icon::Icon;
use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::text::Line;

/// One tree row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    /// Node identity.
    pub key: Key,
    /// Row label.
    pub label: Line,
    /// Leading icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Right-aligned badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Line>,
    /// Child rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
    /// Start expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// GUI extra: children load on first expand (extension replies with a patch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lazy: Option<bool>,
}

impl TreeNode {
    /// A leaf row.
    #[must_use]
    pub fn new(key: impl Into<Key>, label: impl Into<Line>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            icon: None,
            badge: None,
            children: Vec::new(),
            expanded: None,
            lazy: None,
        }
    }

    /// Sets the leading icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the right-aligned badge.
    #[must_use]
    pub fn badge(mut self, badge: impl Into<Line>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Attaches child rows.
    #[must_use]
    pub fn children(mut self, children: impl IntoIterator<Item = TreeNode>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// Starts expanded.
    #[must_use]
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Loads children lazily on first expand.
    #[must_use]
    pub fn lazy(mut self, lazy: bool) -> Self {
        self.lazy = Some(lazy);
        self
    }
}

/// Tree selection state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeState {
    /// Selected node key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Key>,
}

impl TreeState {
    /// State with one selected key.
    #[must_use]
    pub fn with_selected(key: impl Into<Key>) -> Self {
        Self {
            selected: Some(key.into()),
        }
    }
}

/// A tree with expand / collapse and optional lazy children (§4.5.2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tree {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Root rows (≤ 5 000 counting descendants, §4.8).
    pub nodes: Vec<TreeNode>,
    /// Selection state (wins when it changes, §4.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<TreeState>,
    /// Action on selection change (node key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_select: Option<ActionId>,
    /// Action on Enter (node key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<ActionId>,
    /// Action on expand / collapse (`{key, expanded}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_toggle: Option<ActionId>,
}

impl Tree {
    /// A tree from root rows.
    #[must_use]
    pub fn new(nodes: impl IntoIterator<Item = TreeNode>) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the selection state.
    #[must_use]
    pub fn state(mut self, state: TreeState) -> Self {
        self.state = Some(state);
        self
    }

    /// Sets the selection action.
    #[must_use]
    pub fn on_select(mut self, action: impl Into<ActionId>) -> Self {
        self.on_select = Some(action.into());
        self
    }

    /// Sets the activate action.
    #[must_use]
    pub fn on_activate(mut self, action: impl Into<ActionId>) -> Self {
        self.on_activate = Some(action.into());
        self
    }

    /// Sets the toggle action.
    #[must_use]
    pub fn on_toggle(mut self, action: impl Into<ActionId>) -> Self {
        self.on_toggle = Some(action.into());
        self
    }
}

impl IntoNode for Tree {}
impl From<Tree> for Node {
    fn from(t: Tree) -> Self {
        Node::Tree(t)
    }
}
