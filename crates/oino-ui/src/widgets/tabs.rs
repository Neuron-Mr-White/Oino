//! ratatui `Tabs` (§4.5.1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::style::Style;
use crate::text::{Line, Span};

/// One tab title.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabTitle {
    /// Tab identity (drives `selected` and the action value).
    pub key: Key,
    /// Tab title.
    pub title: Line,
    /// GUI extra: right-aligned badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Line>,
}

impl TabTitle {
    /// A tab title.
    #[must_use]
    pub fn new(key: impl Into<Key>, title: impl Into<Line>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            badge: None,
        }
    }

    /// Sets the right-aligned badge.
    #[must_use]
    pub fn badge(mut self, badge: impl Into<Line>) -> Self {
        self.badge = Some(badge.into());
        self
    }
}

/// ratatui `Tabs` (§4.5.1). With `panes`, the selected pane renders below the bar
/// and switching is local (no round-trip) unless `onSelect` is set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tabs {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Tab titles.
    pub titles: Vec<TabTitle>,
    /// Selected tab key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Key>,
    /// Divider between titles (ratatui default `"│"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divider: Option<Span>,
    /// Left / right padding lines around the bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<[Line; 2]>,
    /// Selected-title style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_style: Option<Style>,
    /// Pane contents keyed by tab key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<BTreeMap<Key, Node>>,
    /// Action on selection change (tab key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_select: Option<ActionId>,
}

impl Tabs {
    /// Tabs from titles.
    #[must_use]
    pub fn new(titles: impl IntoIterator<Item = TabTitle>) -> Self {
        Self {
            titles: titles.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the selected tab key.
    #[must_use]
    pub fn selected(mut self, key: impl Into<Key>) -> Self {
        self.selected = Some(key.into());
        self
    }

    /// Sets the divider span.
    #[must_use]
    pub fn divider(mut self, divider: impl Into<Span>) -> Self {
        self.divider = Some(divider.into());
        self
    }

    /// Sets the left / right padding lines.
    #[must_use]
    pub fn padding(mut self, padding: [Line; 2]) -> Self {
        self.padding = Some(padding);
        self
    }

    /// Sets the selected-title style.
    #[must_use]
    pub fn highlight_style(mut self, style: Style) -> Self {
        self.highlight_style = Some(style);
        self
    }

    /// Attaches pane contents keyed by tab key.
    #[must_use]
    pub fn panes(mut self, panes: impl IntoIterator<Item = (Key, impl IntoNode)>) -> Self {
        self.panes = Some(panes.into_iter().map(|(k, n)| (k, n.into_node())).collect());
        self
    }

    /// Sets the selection action.
    #[must_use]
    pub fn on_select(mut self, action: impl Into<ActionId>) -> Self {
        self.on_select = Some(action.into());
        self
    }
}

impl IntoNode for Tabs {}
impl From<Tabs> for Node {
    fn from(t: Tabs) -> Self {
        Node::Tabs(t)
    }
}
