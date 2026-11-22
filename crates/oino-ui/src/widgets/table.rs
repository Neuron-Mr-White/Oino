//! ratatui `Table` (§4.5.1): [`Table`], [`Row`] and [`Cell`].

use serde::{Deserialize, Serialize};

use crate::layout::Constraint;
use crate::node::{ActionId, Common, IntoNode, Key, Node};
use crate::style::Style;
use crate::text::Text;
use crate::widgets::list::{HighlightSpacing, ListState};

/// ratatui `Cell`: a `Text` shorthand or `{content, style}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    /// Shorthand: cell text.
    Text(Text),
    /// Full form.
    Full {
        /// Cell text.
        content: Text,
        /// Cell style.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<Style>,
    },
}

impl Cell {
    /// Cell from any text-like value.
    #[must_use]
    pub fn new(content: impl Into<Text>) -> Self {
        Self::Full {
            content: content.into(),
            style: None,
        }
    }

    /// Sets the cell style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        if let Self::Full { style: slot, .. } = &mut self {
            *slot = Some(style);
        }
        self
    }

    /// The cell's text.
    #[must_use]
    pub fn content(&self) -> &Text {
        match self {
            Self::Text(text) => text,
            Self::Full { content, .. } => content,
        }
    }
}

impl From<Text> for Cell {
    fn from(text: Text) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for Cell {
    fn from(s: &str) -> Self {
        Self::Text(Text::from(s))
    }
}

impl From<String> for Cell {
    fn from(s: String) -> Self {
        Self::Text(Text::from(s))
    }
}

/// ratatui `Row`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Row identity among siblings (defaults to the positional key, §4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    /// Row cells.
    pub cells: Vec<Cell>,
    /// Row height in rows (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
    /// Blank rows below the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_margin: Option<u16>,
    /// Row style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
}

impl Row {
    /// A row from cells.
    #[must_use]
    pub fn new(cells: impl IntoIterator<Item = impl Into<Cell>>) -> Self {
        Self {
            cells: cells.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    /// Sets the row key.
    #[must_use]
    pub fn key(mut self, key: impl AsRef<str>) -> Self {
        self.key = Some(key.as_ref().to_owned());
        self
    }

    /// Sets the row height.
    #[must_use]
    pub fn height(mut self, height: u16) -> Self {
        self.height = Some(height);
        self
    }

    /// Sets the bottom margin.
    #[must_use]
    pub fn bottom_margin(mut self, margin: u16) -> Self {
        self.bottom_margin = Some(margin);
        self
    }

    /// Sets the row style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
}

/// Sticky column mode (GUI extra).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sticky {
    /// First column stays visible during horizontal scroll.
    #[default]
    FirstColumn,
}

/// ratatui `Table` (§4.5.1). Column widths are solved horizontally like a Layout.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Header row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Row>,
    /// Body rows (≤ 5 000, §4.8).
    pub rows: Vec<Row>,
    /// Footer row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<Row>,
    /// One width constraint per column.
    pub widths: Vec<Constraint>,
    /// Gap between columns (default 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_spacing: Option<u16>,
    /// Selected-row style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_highlight_style: Option<Style>,
    /// Prefix drawn before the selected row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_symbol: Option<String>,
    /// Space reserved for the highlight symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight_spacing: Option<HighlightSpacing>,
    /// Selection / offset state (wins when it changes, §4.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ListState>,
    /// GUI extra: sticky first column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticky: Option<Sticky>,
    /// GUI extra: native scrollbar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollbar: Option<bool>,
    /// Action on selection change (row key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_select: Option<ActionId>,
    /// Action on Enter (row key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_activate: Option<ActionId>,
}

impl Table {
    /// A table from rows and column widths.
    #[must_use]
    pub fn new(
        rows: impl IntoIterator<Item = impl Into<Row>>,
        widths: impl IntoIterator<Item = Constraint>,
    ) -> Self {
        Self {
            rows: rows.into_iter().map(Into::into).collect(),
            widths: widths.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Sets the header row.
    #[must_use]
    pub fn header(mut self, header: impl Into<Row>) -> Self {
        self.header = Some(header.into());
        self
    }

    /// Sets the footer row.
    #[must_use]
    pub fn footer(mut self, footer: impl Into<Row>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Sets the column width constraints.
    #[must_use]
    pub fn widths(mut self, widths: impl IntoIterator<Item = Constraint>) -> Self {
        self.widths = widths.into_iter().collect();
        self
    }

    /// Sets the column gap.
    #[must_use]
    pub fn column_spacing(mut self, spacing: u16) -> Self {
        self.column_spacing = Some(spacing);
        self
    }

    /// Sets the selected-row style.
    #[must_use]
    pub fn row_highlight_style(mut self, style: Style) -> Self {
        self.row_highlight_style = Some(style);
        self
    }

    /// Sets the prefix drawn before the selected row.
    #[must_use]
    pub fn highlight_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.highlight_symbol = Some(symbol.into());
        self
    }

    /// Sets the highlight-symbol spacing policy.
    #[must_use]
    pub fn highlight_spacing(mut self, spacing: HighlightSpacing) -> Self {
        self.highlight_spacing = Some(spacing);
        self
    }

    /// Sets the selection / offset state.
    #[must_use]
    pub fn state(mut self, state: ListState) -> Self {
        self.state = Some(state);
        self
    }

    /// GUI extra: sticky first column.
    #[must_use]
    pub fn sticky(mut self, sticky: Sticky) -> Self {
        self.sticky = Some(sticky);
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

impl IntoNode for Table {}
impl From<Table> for Node {
    fn from(t: Table) -> Self {
        Node::Table(t)
    }
}
