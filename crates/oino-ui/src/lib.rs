//! `oino-ui` — wire types and the ratatui-mirroring builder for the Oino
//! extension UI protocol v2 (docs/dev/v2/specs/ui-protocol.md).
//!
//! This crate is the single serde-only definition of the protocol: node trees
//! (§4), the entry registry envelopes (§6.1, §9.2), patch ops (§4.7), validation
//! limits and the OUS v1 gate (§4.8), the plain-text fallback (§10) and — behind
//! the `schema` feature — the normative JSON Schema (Appendix A). It builds for
//! `wasm32-wasip2` (deps: `serde`, `serde_json` only, §11).
//!
//! ```
//! use oino_ui::prelude::*;
//!
//! let list = List::new(["first", "second"])
//!     .highlight_symbol("› ")
//!     .on_activate("open");
//! let node: Node = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)])
//!     .children([Paragraph::new("Tasks").into(), list.into()])
//!     .into();
//! let json = serde_json::to_value(&node).unwrap_or_default();
//! assert_eq!(json["t"], "Layout");
//! ```

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-256.png",
    html_favicon_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/oino.ico"
)]

pub mod extras;
pub mod layout;
pub mod limits;
pub mod node;
pub mod patch;
pub mod plain;
pub mod settings;
pub mod slot;
pub mod style;
pub mod text;
pub mod widgets;

#[cfg(feature = "schema")]
pub mod schema;

/// Everything an extension author usually wants (§11): node kinds, the prelude
/// types and the [`Stylize`] helpers.
pub mod prelude {
    pub use crate::extras::button::{Button, ButtonVariant};
    pub use crate::extras::code::Code;
    pub use crate::extras::diff::{Diff, DiffMode};
    pub use crate::extras::empty::Empty;
    pub use crate::extras::icon::Icon;
    pub use crate::extras::image::{Fit, Image, ImageSource};
    pub use crate::extras::input::Input;
    pub use crate::extras::key_hints::{Hint, KeyHints};
    pub use crate::extras::log::Log;
    pub use crate::extras::markdown::Markdown;
    pub use crate::extras::select::{Select, SelectOption};
    pub use crate::extras::spinner::Spinner;
    pub use crate::extras::toggle::Toggle;
    pub use crate::extras::tree::{Tree, TreeNode, TreeState};
    pub use crate::layout::{Constraint, Direction, Flex, Layout, Margin};
    pub use crate::node::{ActionId, Common, IntoNode, Key, Node, Unknown, WithCommon};
    pub use crate::patch::{NodeOp, NodePath, PatchField, PatchItem};
    pub use crate::style::{Color, Modifier, Modifiers, Style, Stylize};
    pub use crate::text::{Alignment, Line, Span, Text};
    pub use crate::widgets::barchart::{Bar, BarChart, BarGroup};
    pub use crate::widgets::block::{
        Block, BlockProps, BlockTitle, BorderSide, BorderType, Borders, Padding, Title,
        TitlePosition, borders,
    };
    pub use crate::widgets::chart::{Axis, Chart, Dataset, GraphType, LegendPosition, Marker};
    pub use crate::widgets::gauge::{Gauge, LineGauge, LineSet};
    pub use crate::widgets::list::{
        HighlightSpacing, List, ListDirection, ListItem, ListState, Selection,
    };
    pub use crate::widgets::paragraph::{Paragraph, Wrap};
    pub use crate::widgets::scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState};
    pub use crate::widgets::sparkline::{BarSet, SparkDirection, Sparkline};
    pub use crate::widgets::table::{Cell, Row, Sticky, Table};
    pub use crate::widgets::tabs::{TabTitle, Tabs};
}

pub use layout::{Constraint, Direction, Flex, Layout, Margin};
pub use node::{ActionId, Common, IntoNode, Key, Node, Unknown, WithCommon};
pub use style::{Color, Modifier, Modifiers, Style, Stylize};
pub use text::{Alignment, Line, Span, Text};

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    #[test]
    fn prelude_example_round_trips() {
        use crate::prelude::*;
        let list = List::new(["first", "second"]).on_activate("open");
        let node: Node = Layout::vertical([Constraint::Fill(1)])
            .children([list.into()])
            .into();
        let json = serde_json::to_value(&node).unwrap_or_default();
        let back: Node = serde_json::from_value(json).unwrap_or(Node::Unknown(Unknown {
            t: "boom".into(),
            ..Unknown::default()
        }));
        assert_eq!(node, back);
    }
}
