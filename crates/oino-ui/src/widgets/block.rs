//! ratatui `Block` (§4.5.1): [`Block`] node, [`BlockProps`] (the common `block`
//! prop of every widget), borders, padding and titles.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

use crate::node::{ActionId, Common, IntoNode, Node};
use crate::text::{Alignment, Line};
use crate::{style::Color, style::Style};

/// ratatui `BorderType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BorderType {
    /// Single line.
    #[default]
    Plain,
    /// Rounded corners.
    Rounded,
    /// Double line.
    Double,
    /// Thick line.
    Thick,
}

/// One border side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BorderSide {
    /// Top.
    TOP,
    /// Right.
    RIGHT,
    /// Bottom.
    BOTTOM,
    /// Left.
    LEFT,
}

/// Which borders to draw: `"ALL"`, `"NONE"` or a list of sides (§4.5.1).
/// Unknown strings parse tolerantly as [`Borders::Other`] (dropped with a diagnostic
/// at render time).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Borders {
    /// All sides (`Block::bordered()`).
    All,
    /// No sides (the default).
    #[default]
    None_,
    /// Explicit side list.
    Sides(Vec<BorderSide>),
    /// Unknown string: renderers drop it instead of rejecting the tree (§4.8).
    Other(String),
}

/// Marker constants for the [`Borders`] shorthand strings.
pub mod borders {
    /// `"ALL"` shorthand.
    pub const ALL: &str = "ALL";
    /// `"NONE"` shorthand.
    pub const NONE: &str = "NONE";
}

impl Serialize for Borders {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_str(borders::ALL),
            Self::None_ => serializer.serialize_str(borders::NONE),
            Self::Sides(sides) => sides.serialize(serializer),
            Self::Other(name) => serializer.serialize_str(name),
        }
    }
}

impl<'de> Deserialize<'de> for Borders {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        match raw {
            serde_json::Value::String(name) => match name.as_str() {
                borders::ALL => Ok(Self::All),
                borders::NONE => Ok(Self::None_),
                _ => Ok(Self::Other(name)),
            },
            serde_json::Value::Array(_) => {
                let sides = Vec::<BorderSide>::deserialize(raw).map_err(D::Error::custom)?;
                Ok(Self::Sides(sides))
            }
            _ => Err(D::Error::custom(
                "borders must be \"ALL\", \"NONE\" or an array of sides",
            )),
        }
    }
}

/// Title position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TitlePosition {
    /// Top edge.
    #[default]
    Top,
    /// Bottom edge.
    Bottom,
}

/// One positioned block title.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Title {
    /// Title line.
    pub content: Line,
    /// Title alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    /// Edge the title sits on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<TitlePosition>,
}

impl Title {
    /// Title on the top edge.
    #[must_use]
    pub fn top(content: impl Into<Line>) -> Self {
        Self {
            content: content.into(),
            alignment: None,
            position: Some(TitlePosition::Top),
        }
    }

    /// Title on the bottom edge.
    #[must_use]
    pub fn bottom(content: impl Into<Line>) -> Self {
        Self {
            content: content.into(),
            alignment: None,
            position: Some(TitlePosition::Bottom),
        }
    }

    /// Sets the alignment.
    #[must_use]
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }
}

/// Block `title` prop: a single [`Line`] or a list of positioned [`Title`]s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockTitle {
    /// Single top title.
    Line(Line),
    /// Positioned titles.
    Titles(Vec<Title>),
}

impl From<&str> for BlockTitle {
    fn from(s: &str) -> Self {
        Self::Line(Line::from(s))
    }
}

impl From<String> for BlockTitle {
    fn from(s: String) -> Self {
        Self::Line(Line::from(s))
    }
}

impl From<Line> for BlockTitle {
    fn from(line: Line) -> Self {
        Self::Line(line)
    }
}

impl From<Vec<Title>> for BlockTitle {
    fn from(titles: Vec<Title>) -> Self {
        Self::Titles(titles)
    }
}

/// Block padding: a number (all sides) or `{left, right, top, bottom}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Padding {
    /// Same padding on all sides.
    Uniform(u16),
    /// Per-side padding.
    Rect {
        /// Left padding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        left: Option<u16>,
        /// Right padding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        right: Option<u16>,
        /// Top padding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        top: Option<u16>,
        /// Bottom padding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bottom: Option<u16>,
    },
}

impl Padding {
    /// Same padding on all sides.
    #[must_use]
    pub fn uniform(n: u16) -> Self {
        Self::Uniform(n)
    }

    /// Per-side padding.
    #[must_use]
    pub fn new(left: u16, right: u16, top: u16, bottom: u16) -> Self {
        Self::Rect {
            left: Some(left),
            right: Some(right),
            top: Some(top),
            bottom: Some(bottom),
        }
    }

    /// Horizontal (left / right) padding.
    #[must_use]
    pub fn horizontal(n: u16) -> Self {
        Self::Rect {
            left: Some(n),
            right: Some(n),
            top: None,
            bottom: None,
        }
    }

    /// Vertical (top / bottom) padding.
    #[must_use]
    pub fn vertical(n: u16) -> Self {
        Self::Rect {
            left: None,
            right: None,
            top: Some(n),
            bottom: Some(n),
        }
    }
}

/// The `block` prop common to every widget that ratatui allows a block on
/// (Appendix A `BlockProps`; same props as the [`Block`] node minus `child`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockProps {
    /// One top title, or positioned titles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BlockTitle>,
    /// Default title alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_alignment: Option<Alignment>,
    /// Which borders to draw (default `"NONE"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<Borders>,
    /// Border glyph set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_type: Option<BorderType>,
    /// Border style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_style: Option<Style>,
    /// Title style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_style: Option<Style>,
    /// Inner padding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    /// GUI extra: show a disclosure toggle in the title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsible: Option<bool>,
    /// GUI extra: start collapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// GUI extra: left rail in that colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rail: Option<Color>,
    /// Base style of the block's area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
}

impl BlockProps {
    /// Block with all borders (`Block::bordered()`).
    #[must_use]
    pub fn bordered() -> Self {
        Self {
            borders: Some(Borders::All),
            ..Self::default()
        }
    }

    /// Sets the (top) title.
    #[must_use]
    pub fn title(mut self, title: impl Into<BlockTitle>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the default title alignment.
    #[must_use]
    pub fn title_alignment(mut self, alignment: Alignment) -> Self {
        self.title_alignment = Some(alignment);
        self
    }

    /// Sets which borders to draw.
    #[must_use]
    pub fn borders(mut self, borders: Borders) -> Self {
        self.borders = Some(borders);
        self
    }

    /// Sets the border glyph set.
    #[must_use]
    pub fn border_type(mut self, border_type: BorderType) -> Self {
        self.border_type = Some(border_type);
        self
    }

    /// Sets the border style.
    #[must_use]
    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = Some(style);
        self
    }

    /// Sets the title style.
    #[must_use]
    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = Some(style);
        self
    }

    /// Sets the inner padding.
    #[must_use]
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = Some(padding);
        self
    }

    /// GUI extra: show a disclosure toggle in the title.
    #[must_use]
    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = Some(collapsible);
        self
    }

    /// GUI extra: start collapsed.
    #[must_use]
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = Some(collapsed);
        self
    }

    /// GUI extra: left rail in that colour.
    #[must_use]
    pub fn rail(mut self, color: Color) -> Self {
        self.rail = Some(color);
        self
    }

    /// Sets the base style of the block's area.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
}

/// ratatui `Block` node: draws chrome around an optional child (§4.5.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// One top title, or positioned titles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<BlockTitle>,
    /// Default title alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_alignment: Option<Alignment>,
    /// Which borders to draw (default `"NONE"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<Borders>,
    /// Border glyph set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_type: Option<BorderType>,
    /// Border style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_style: Option<Style>,
    /// Title style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_style: Option<Style>,
    /// Inner padding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    /// GUI extra: show a disclosure toggle in the title (`onToggle` action → `bool`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsible: Option<bool>,
    /// GUI extra: start collapsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// GUI extra: left rail in that colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rail: Option<Color>,
    /// The widget rendered inside the block's inner area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child: Option<Box<Node>>,
    /// Action fired when the disclosure toggle flips.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_toggle: Option<ActionId>,
}

impl Block {
    /// Block with all borders.
    #[must_use]
    pub fn bordered() -> Self {
        Self {
            borders: Some(Borders::All),
            ..Self::default()
        }
    }

    /// Sets the (top) title.
    #[must_use]
    pub fn title(mut self, title: impl Into<BlockTitle>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the default title alignment.
    #[must_use]
    pub fn title_alignment(mut self, alignment: Alignment) -> Self {
        self.title_alignment = Some(alignment);
        self
    }

    /// Sets which borders to draw.
    #[must_use]
    pub fn borders(mut self, borders: Borders) -> Self {
        self.borders = Some(borders);
        self
    }

    /// Sets the border glyph set.
    #[must_use]
    pub fn border_type(mut self, border_type: BorderType) -> Self {
        self.border_type = Some(border_type);
        self
    }

    /// Sets the border style.
    #[must_use]
    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = Some(style);
        self
    }

    /// Sets the title style.
    #[must_use]
    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = Some(style);
        self
    }

    /// Sets the inner padding.
    #[must_use]
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = Some(padding);
        self
    }

    /// GUI extra: show a disclosure toggle in the title.
    #[must_use]
    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = Some(collapsible);
        self
    }

    /// GUI extra: start collapsed.
    #[must_use]
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = Some(collapsed);
        self
    }

    /// GUI extra: left rail in that colour.
    #[must_use]
    pub fn rail(mut self, color: Color) -> Self {
        self.rail = Some(color);
        self
    }

    /// Sets the toggle action.
    #[must_use]
    pub fn on_toggle(mut self, action: impl Into<ActionId>) -> Self {
        self.on_toggle = Some(action.into());
        self
    }

    /// Sets the child widget.
    #[must_use]
    pub fn child(mut self, child: impl IntoNode) -> Self {
        self.child = Some(Box::new(child.into_node()));
        self
    }
}

impl From<Block> for BlockProps {
    fn from(block: Block) -> Self {
        Self {
            title: block.title,
            title_alignment: block.title_alignment,
            borders: block.borders,
            border_type: block.border_type,
            border_style: block.border_style,
            title_style: block.title_style,
            padding: block.padding,
            collapsible: block.collapsible,
            collapsed: block.collapsed,
            rail: block.rail,
            style: block.common.style,
        }
    }
}

impl IntoNode for Block {}
impl From<Block> for Node {
    fn from(b: Block) -> Self {
        Node::Block(b)
    }
}
#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use super::*;

    #[test]
    fn borders_shorthand() {
        let b: Borders = serde_json::from_str("\"ALL\"").unwrap_or_default();
        assert_eq!(b, Borders::All);
        let b: Borders = serde_json::from_str("[\"TOP\",\"LEFT\"]").unwrap_or_default();
        assert_eq!(b, Borders::Sides(vec![BorderSide::TOP, BorderSide::LEFT]));
    }
}
