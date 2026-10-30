//! The [`Node`] enum (§4.1): one variant per node kind, serde tag `"t"`,
//! common props and the tolerant [`Unknown`] catch-all.

use serde::de::Error as DeError;
use serde::ser::Error as SerError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as Json;

use crate::extras::button::Button;
use crate::extras::code::Code;
use crate::extras::diff::Diff;
use crate::extras::empty::Empty;
use crate::extras::image::Image;
use crate::extras::input::Input;
use crate::extras::key_hints::KeyHints;
use crate::extras::log::Log;
use crate::extras::markdown::Markdown;
use crate::extras::select::Select;
use crate::extras::spinner::Spinner;
use crate::extras::toggle::Toggle;
use crate::extras::tree::Tree;
use crate::layout::Layout;
use crate::style::Style;
use crate::widgets::barchart::BarChart;
use crate::widgets::block::{Block, BlockProps};
use crate::widgets::chart::Chart;
use crate::widgets::gauge::{Gauge, LineGauge};
use crate::widgets::list::List;
use crate::widgets::paragraph::Paragraph;
use crate::widgets::scrollbar::Scrollbar;
use crate::widgets::sparkline::Sparkline;
use crate::widgets::table::Table;
use crate::widgets::tabs::Tabs;

/// Node / item identity among siblings (§4.1): 1–128 chars, unique per collection.
pub type Key = String;

/// Action id (§4.6): `^[A-Za-z0-9_.:/-]{1,128}$`, chosen by the extension.
pub type ActionId = String;

/// The common props of every node (Appendix A `Common`): identity, base style,
/// surrounding block and the fallback rendered for unknown / unsupported kinds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Common {
    /// Identity among siblings (§4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    /// Base style of the node's area (§4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
    /// ratatui `.block(..)`: chrome around the widget's inner area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockProps>,
    /// Rendered when the kind is unknown or unsupported by the frontend (§4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Box<Node>>,
}

impl Common {
    /// Empty common props.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            style: None,
            block: None,
            fallback: None,
        }
    }

    /// Sets the node key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Sets the base style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Sets the surrounding block.
    #[must_use]
    pub fn block(mut self, block: BlockProps) -> Self {
        self.block = Some(block);
        self
    }

    /// Sets the fallback node.
    #[must_use]
    pub fn fallback(mut self, node: impl Into<Node>) -> Self {
        self.fallback = Some(Box::new(node.into()));
        self
    }
}

/// An unknown node kind (§4.8): parsers keep the tag (and the common props) so the
/// renderer can draw `fallback` or nothing. Lowercase tags that are OUS v1 kinds
/// are rejected by [`validate`](crate::limits::validate) (§4.8 migration gate).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Unknown {
    /// The raw `t` tag.
    pub t: String,
    /// Identity among siblings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    /// Base style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
    /// Surrounding block, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockProps>,
    /// Rendered instead of nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Box<Node>>,
}

/// One widget-tree node (§4): `{"t": "<Kind>", ...}`.
///
/// Parsing is tolerant (§4.8 / D-U14): unknown props are ignored, unknown kinds
/// become [`Node::Unknown`]. Serialization emits exactly the Appendix A fields.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// Container; children solved on the main axis (§4.3).
    Layout(Layout),
    /// Chrome around a child (§4.5.1).
    Block(Block),
    /// Wrapped text (§4.5.1).
    Paragraph(Paragraph),
    /// Selectable list (§4.5.1).
    List(List),
    /// Column table (§4.5.1).
    Table(Table),
    /// Tab bar (+ optional local panes) (§4.5.1).
    Tabs(Tabs),
    /// Progress bar (§4.5.1).
    Gauge(Gauge),
    /// One-row progress line (§4.5.1).
    LineGauge(LineGauge),
    /// Value series as bars (§4.5.1).
    Sparkline(Sparkline),
    /// Grouped bars (§4.5.1).
    BarChart(BarChart),
    /// Scatter / line / bar plot (§4.5.1).
    Chart(Chart),
    /// Scroll position indicator (§4.5.1).
    Scrollbar(Scrollbar),
    /// Markdown text (§4.5.2).
    Markdown(Markdown),
    /// Syntax-highlighted code (§4.5.2).
    Code(Code),
    /// Unified / split diff (§4.5.2).
    Diff(Diff),
    /// Image (§4.5.2).
    Image(Image),
    /// Tailing log (§4.5.2).
    Log(Log),
    /// Expandable tree (§4.5.2).
    Tree(Tree),
    /// Key hints line (§4.5.2).
    KeyHints(KeyHints),
    /// Text field (§4.5.2).
    Input(Input),
    /// Boolean switch (§4.5.2).
    Toggle(Toggle),
    /// Cycling select (§4.5.2).
    Select(Select),
    /// Action button (§4.5.2).
    Button(Button),
    /// Activity indicator (§4.5.2).
    Spinner(Spinner),
    /// Empty state (§4.5.2).
    Empty(Empty),
    /// Unknown kind (§4.8).
    Unknown(Unknown),
}

macro_rules! for_each_kind {
    ($macro:ident) => {
        $macro! {
            (Layout, "Layout", Layout),
            (Block, "Block", Block),
            (Paragraph, "Paragraph", Paragraph),
            (List, "List", List),
            (Table, "Table", Table),
            (Tabs, "Tabs", Tabs),
            (Gauge, "Gauge", Gauge),
            (LineGauge, "LineGauge", LineGauge),
            (Sparkline, "Sparkline", Sparkline),
            (BarChart, "BarChart", BarChart),
            (Chart, "Chart", Chart),
            (Scrollbar, "Scrollbar", Scrollbar),
            (Markdown, "Markdown", Markdown),
            (Code, "Code", Code),
            (Diff, "Diff", Diff),
            (Image, "Image", Image),
            (Log, "Log", Log),
            (Tree, "Tree", Tree),
            (KeyHints, "KeyHints", KeyHints),
            (Input, "Input", Input),
            (Toggle, "Toggle", Toggle),
            (Select, "Select", Select),
            (Button, "Button", Button),
            (Spinner, "Spinner", Spinner),
            (Empty, "Empty", Empty)
        }
    };
}

macro_rules! impl_node_kind_table {
    ($(($variant:ident, $tag:literal, $ty:ty)),* $(,)?) => {
        /// Every known node tag (Appendix A order).
        pub const NODE_KINDS: [&str; 25] = [$($tag),*];

        impl Node {
            /// The node's tag, or `None` for [`Node::Unknown`] (use `Unknown::t`).
            #[must_use]
            pub fn tag(&self) -> Option<&'static str> {
                match self {
                    $(Self::$variant(_) => Some($tag),)*
                    Self::Unknown(_) => None,
                }
            }

            /// The node's tag, including unknown ones.
            #[must_use]
            pub fn kind(&self) -> &str {
                match self {
                    $(Self::$variant(_) => $tag,)*
                    Self::Unknown(u) => &u.t,
                }
            }

            /// Shared borrow of the common props (the unknown kind keeps a subset).
            #[must_use]
            pub fn common(&self) -> CommonRef<'_> {
                match self {
                    $(Self::$variant(inner) => CommonRef {
                        key: inner.common.key.as_deref(),
                        style: inner.common.style.as_ref(),
                        block: inner.common.block.as_ref(),
                        fallback: inner.common.fallback.as_deref(),
                    },)*
                    Self::Unknown(u) => CommonRef {
                        key: u.key.as_deref(),
                        style: u.style.as_ref(),
                        block: u.block.as_ref(),
                        fallback: u.fallback.as_deref(),
                    },
                }
            }
        }

        /// Borrowed view of a node's common props.
        #[derive(Clone, Copy, Debug)]
        pub struct CommonRef<'a> {
            /// Identity among siblings.
            pub key: Option<&'a str>,
            /// Base style.
            pub style: Option<&'a Style>,
            /// Surrounding block.
            pub block: Option<&'a BlockProps>,
            /// Fallback node.
            pub fallback: Option<&'a Node>,
        }

        impl Serialize for Node {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                fn tagged<S: Serializer>(
                    serializer: S,
                    tag: &str,
                    value: &impl Serialize,
                ) -> Result<S::Ok, S::Error> {
                    use serde::ser::SerializeMap;
                    let inner = serde_json::to_value(value)
                        .map_err(|e| S::Error::custom(e.to_string()))?;
                    let Json::Object(map) = inner else {
                        return Err(S::Error::custom("node props must serialize to an object"));
                    };
                    let mut out = serializer.serialize_map(Some(map.len() + 1))?;
                    out.serialize_entry("t", tag)?;
                    for (k, v) in map {
                        out.serialize_entry(&k, &v)?;
                    }
                    out.end()
                }
                match self {
                    $(Self::$variant(inner) => tagged(serializer, $tag, inner),)*
                    Self::Unknown(u) => u.serialize(serializer),
                }
            }
        }

        impl<'de> Deserialize<'de> for Node {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = Json::deserialize(deserializer)?;
                let tag = match value.get("t") {
                    Some(Json::String(tag)) => tag.clone(),
                    _ if !value.is_object() => {
                        return Err(D::Error::custom("a node must be a JSON object"));
                    }
                    _ => {
                        return Err(D::Error::custom("a node needs a string \"t\" tag"));
                    }
                };
                match tag.as_str() {
                    $($tag => serde_json::from_value::<$ty>(value)
                        .map(Self::$variant)
                        .map_err(|e| D::Error::custom(format!("{tag}: {}", e))),)*
                    _ => serde_json::from_value::<Unknown>(value)
                        .map(Self::Unknown)
                        .map_err(|e| D::Error::custom(format!("{tag}: {}", e))),
                }
            }
        }
    };
}

for_each_kind!(impl_node_kind_table);

/// Converts a builder value into a [`Node`] (§11.2: `w.into()`).
pub trait IntoNode: Into<Node> {
    /// Converts into a node.
    fn into_node(self) -> Node {
        self.into()
    }
}

impl IntoNode for Node {}

/// Extension-side common-props builder for any node (§4.1): `node.with(|c| ...)`.
pub trait WithCommon: Sized {
    /// Mutable access to the node's common props.
    fn common_mut(&mut self) -> &mut Common;

    /// Sets the node key.
    #[must_use]
    fn key(mut self, key: impl Into<Key>) -> Self {
        self.common_mut().key = Some(key.into());
        self
    }

    /// Sets the base style.
    #[must_use]
    fn style(mut self, style: Style) -> Self {
        self.common_mut().style = Some(style);
        self
    }

    /// Sets the surrounding block.
    #[must_use]
    fn block(mut self, block: impl Into<BlockProps>) -> Self {
        self.common_mut().block = Some(block.into());
        self
    }

    /// Sets the fallback node.
    #[must_use]
    fn fallback(mut self, node: impl Into<Node>) -> Self {
        self.common_mut().fallback = Some(Box::new(node.into()));
        self
    }
}

macro_rules! impl_with_common {
    ($(($variant:ident, $tag:literal, $ty:ty)),* $(,)?) => {
        $(impl WithCommon for $ty {
            fn common_mut(&mut self) -> &mut Common {
                &mut self.common
            }
        })*
    };
}

for_each_kind!(impl_with_common);

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
    fn unknown_kind_is_kept_with_fallback() {
        let raw = r#"{"t":"Holgram","style":{"fg":"accent"},"fallback":{"t":"Spinner"}}"#;
        let node: Node = serde_json::from_str(raw).expect("unknown node parses");
        assert_eq!(node.kind(), "Holgram");
        let fallback = node.common().fallback.expect("fallback kept");
        assert_eq!(fallback.tag(), Some("Spinner"));
        let round: Node = serde_json::from_str(&serde_json::to_string(&node).expect("serializes"))
            .expect("re-parses");
        assert_eq!(node, round);
    }

    #[test]
    fn missing_tag_is_an_error() {
        assert!(
            serde_json::from_str::<Node>(r#"{"text":"hi"}"#).is_err(),
            "missing t must not parse"
        );
    }

    #[test]
    fn all_tags_round_trip() {
        let samples: Vec<Node> = vec![
            Layout::vertical([crate::layout::Constraint::Fill(1)]).into(),
            Block::bordered().into(),
            Paragraph::new("x").into(),
            List::new(["a"]).into(),
            Table::new(
                [crate::widgets::table::Row::new(["a"])],
                [crate::layout::Constraint::Length(4)],
            )
            .into(),
            Tabs::new([crate::widgets::tabs::TabTitle::new("a", "A")]).into(),
            Gauge::new().into(),
            LineGauge::new(0.5).into(),
            Sparkline::new([Some(1), None]).into(),
            BarChart::new([crate::widgets::barchart::BarGroup::new([
                crate::widgets::barchart::Bar::new(1),
            ])])
            .into(),
            Chart::new([crate::widgets::chart::Dataset::new([[0.0, 1.0]])]).into(),
            Scrollbar::new(crate::widgets::scrollbar::ScrollbarState::new(10)).into(),
            Markdown::new("# x").into(),
            Code::new("let x = 1;").into(),
            Diff::patch("@@ -1 +1 @@\n-a\n+b").into(),
            Image::new(crate::extras::image::ImageSource::path("a.png"), "an image").into(),
            Log::new(["line"]).into(),
            Tree::new([crate::extras::tree::TreeNode::new("a", "A")]).into(),
            KeyHints::new([("enter", "open")]).into(),
            Input::new("name").into(),
            Toggle::new("on", "Enabled", true).into(),
            Select::new("mode", [("a", "A")]).into(),
            Button::new("Go", "go").into(),
            Spinner::new("working").into(),
            Empty::new().title("Nothing").into(),
        ];
        assert_eq!(samples.len(), NODE_KINDS.len());
        for node in samples {
            let json = serde_json::to_value(&node).unwrap_or(Json::Null);
            let back: Node = serde_json::from_value(json).unwrap_or(Node::Unknown(Unknown {
                t: "boom".into(),
                ..Unknown::default()
            }));
            assert_eq!(node, back, "round trip for {}", node.kind());
        }
    }
}
