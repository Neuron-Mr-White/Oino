//! ratatui `Paragraph` (§4.5.1).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};
use crate::text::{Alignment, Text};

/// Paragraph wrapping: `false` disables wrapping, `{trim: bool}` enables it
/// (ratatui `Wrap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Wrap {
    /// `false`: no wrapping, long lines scroll horizontally.
    Disabled(bool),
    /// ratatui `Wrap { trim }`.
    Enabled {
        /// Trim leading whitespace on wrapped lines (ratatui default `true`).
        trim: bool,
    },
}

impl Wrap {
    /// ratatui `Wrap { trim }`.
    #[must_use]
    pub fn trim(trim: bool) -> Self {
        Self::Enabled { trim }
    }
}

/// ratatui `Paragraph` (§4.5.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Paragraph text.
    pub text: Text,
    /// Wrapping (default `{trim: true}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<Wrap>,
    /// Initial scroll offset `[y, x]` in rows / columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<[u16; 2]>,
    /// Text alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    /// GUI extra: native scrollbar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollbar: Option<bool>,
}

impl Paragraph {
    /// A paragraph from any text-like value.
    #[must_use]
    pub fn new(text: impl Into<Text>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Sets wrapping.
    #[must_use]
    pub fn wrap(mut self, wrap: Wrap) -> Self {
        self.wrap = Some(wrap);
        self
    }

    /// Sets the initial scroll offset.
    #[must_use]
    pub fn scroll(mut self, y: u16, x: u16) -> Self {
        self.scroll = Some([y, x]);
        self
    }

    /// Sets the alignment.
    #[must_use]
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }

    /// GUI extra: native scrollbar.
    #[must_use]
    pub fn scrollbar(mut self, scrollbar: bool) -> Self {
        self.scrollbar = Some(scrollbar);
        self
    }
}

impl IntoNode for Paragraph {}
impl From<Paragraph> for Node {
    fn from(p: Paragraph) -> Self {
        Node::Paragraph(p)
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
    fn wrap_false_form() {
        let w: Wrap = serde_json::from_str("false").expect("false parses");
        assert_eq!(w, Wrap::Disabled(false));
        let w: Wrap = serde_json::from_str(r#"{"trim":true}"#).expect("object parses");
        assert_eq!(w, Wrap::Enabled { trim: true });
    }
}
