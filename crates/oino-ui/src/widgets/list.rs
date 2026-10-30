//! ratatui `List` (§4.5.1): [`List`], [`ListItem`], [`ListState`] and helpers.

use serde::{Deserialize, Serialize};

use crate::extras::icon::Icon;
use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::style::Style;
use crate::text::{Line, Text};

/// ratatui `ListDirection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ListDirection {
    /// First item at the top.
    #[default]
    TopToBottom,
    /// First item at the bottom.
    BottomToTop,
}

/// ratatui `HighlightSpacing`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HighlightSpacing {
    /// Always reserve space for the highlight symbol.
    #[default]
    Always,
    /// Reserve space only when an item is selected.
    WhenSelected,
    /// Never reserve space.
    Never,
}

/// Selection state: one key, or several when `multi` (§4.5.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListState {
    /// Selected item key(s) (by `key`, §4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Selection>,
    /// Scroll offset in items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

impl ListState {
    /// State with one selected key.
    #[must_use]
    pub fn with_selected(key: impl Into<Key>) -> Self {
        Self {
            selected: Some(Selection::One(key.into())),
            offset: None,
        }
    }

    /// State with several selected keys (`multi` lists).
    #[must_use]
    pub fn with_selected_many(keys: Vec<Key>) -> Self {
        Self {
            selected: Some(Selection::Many(keys)),
            offset: None,
        }
    }
}

/// Selected item keys: one, or several for `multi` lists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Selection {
    /// Single selection.
    One(Key),
    /// Multi selection (`List.multi`).
    Many(Vec<Key>),
}

/// ratatui `ListItem`: a `Text` shorthand or the full object with key / detail /
/// badge / icon.
#[allow(
    clippy::large_enum_variant,
    reason = "list items are built once per list; boxing the content adds noise"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ListItem {
    /// Shorthand: item content as text.
    Text(Text),
    /// Full form.
    Full {
        /// Identity among siblings (defaults to the positional key, §4.1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<Key>,
        /// Item content.
        content: Text,
        /// Item style.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<Style>,
        /// GUI extra: muted second line.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<Line>,
        /// GUI extra: right-aligned badge.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        badge: Option<Line>,
        /// GUI extra: leading icon.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<Icon>,
        /// GUI extra: greyed out, not activatable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        disabled: Option<bool>,
    },
}

impl ListItem {
    /// Item from any text-like value.
    #[must_use]
    pub fn new(content: impl Into<Text>) -> Self {
        Self::Full {
            key: None,
            content: content.into(),
            style: None,
            detail: None,
            badge: None,
            icon: None,
            disabled: None,
        }
    }

    /// Sets the item key.
    #[must_use]
    pub fn key(mut self, key: impl AsRef<str>) -> Self {
        if let Self::Full { key: slot, .. } = &mut self {
            *slot = Some(key.as_ref().to_owned());
        }
        self
    }

    /// Sets the muted second line.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<Line>) -> Self {
        if let Self::Full { detail: slot, .. } = &mut self {
            *slot = Some(detail.into());
        }
        self
    }

    /// Sets the right-aligned badge.
    #[must_use]
    pub fn badge(mut self, badge: impl Into<Line>) -> Self {
        if let Self::Full { badge: slot, .. } = &mut self {
            *slot = Some(badge.into());
        }
        self
    }

    /// Sets the leading icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        if let Self::Full { icon: slot, .. } = &mut self {
            *slot = Some(icon);
        }
        self
    }

    /// Greys the item out.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        if let Self::Full { disabled: slot, .. } = &mut self {
            *slot = Some(disabled);
        }
        self
    }

    /// Sets the item style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        if let Self::Full { style: slot, .. } = &mut self {
            *slot = Some(style);
        }
        self
    }

    /// The item's content text.
    #[must_use]
    pub fn content(&self) -> &Text {
        match self {
            Self::Text(text) => text,
            Self::Full { content, .. } => content,
        }
    }
}

impl From<Text> for ListItem {
    fn from(text: Text) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for ListItem {
    fn from(s: &str) -> Self {
        Self::Text(Text::from(s))
    }
}

impl From<String> for ListItem {
    fn from(s: String) -> Self {
        Self::Text(Text::from(s))
    }
}

/// ratatui `List` (§4.5.1). Selection is by item key; actions fire on select /
/// activate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Items (≤ 5 000, §4.8).
    pub items: Vec<ListItem>,
    /// Selection / offset state (wins when it changes, §4.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ListState>,
    /// Style of the selected item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_style: Option<Style>,
    /// Prefix drawn before the selected item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_symbol: Option<String>,
    /// Draw the highlight symbol on unselected items too (ratatui `repeat`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_highlight_symbol: Option<bool>,
    /// Space reserved for the highlight symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_spacing: Option<HighlightSpacing>,
    /// Item order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<ListDirection>,
    /// Items kept above / below the selection when scrolling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_padding: Option<u16>,
    /// GUI extra: multi-select (Space toggles; `onSelect` → `key[]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi: Option<bool>,
    /// GUI extra: native scrollbar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollbar: Option<bool>,
    /// Action on selection change (item key, or `key[]` when `multi`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_select: Option<ActionId>,
    /// Action on Enter (item key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<ActionId>,
}

impl List {
    /// A list from items.
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = impl Into<ListItem>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    /// Replaces the items.
    #[must_use]
    pub fn items(mut self, items: impl IntoIterator<Item = impl Into<ListItem>>) -> Self {
        self.items = items.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the selection / offset state.
    #[must_use]
    pub fn state(mut self, state: ListState) -> Self {
        self.state = Some(state);
        self
    }

    /// Sets the selected-item style.
    #[must_use]
    pub fn highlight_style(mut self, style: Style) -> Self {
        self.highlight_style = Some(style);
        self
    }

    /// Sets the prefix drawn before the selected item.
    #[must_use]
    pub fn highlight_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.highlight_symbol = Some(symbol.into());
        self
    }

    /// Draws the highlight symbol on unselected items too.
    #[must_use]
    pub fn repeat_highlight_symbol(mut self, repeat: bool) -> Self {
        self.repeat_highlight_symbol = Some(repeat);
        self
    }

    /// Sets the highlight-symbol spacing policy.
    #[must_use]
    pub fn highlight_spacing(mut self, spacing: HighlightSpacing) -> Self {
        self.highlight_spacing = Some(spacing);
        self
    }

    /// Sets the item order.
    #[must_use]
    pub fn direction(mut self, direction: ListDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Sets the scroll padding.
    #[must_use]
    pub fn scroll_padding(mut self, padding: u16) -> Self {
        self.scroll_padding = Some(padding);
        self
    }

    /// Enables multi-select.
    #[must_use]
    pub fn multi(mut self, multi: bool) -> Self {
        self.multi = Some(multi);
        self
    }

    /// GUI extra: native scrollbar.
    #[must_use]
    pub fn scrollbar(mut self, scrollbar: bool) -> Self {
        self.scrollbar = Some(scrollbar);
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
}

impl IntoNode for List {}
impl From<List> for Node {
    fn from(l: List) -> Self {
        Node::List(l)
    }
}

/// Compatibility alias: the table keeps the same state shape as a list
/// (Appendix A `Table.state` → `ListState`).
pub type TableState = ListState;
