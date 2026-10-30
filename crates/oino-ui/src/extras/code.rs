//! `Code` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};

/// Syntax-highlighted code (§4.5.2). TUI: syntect; app: the chat code-block renderer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Code {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Code source.
    pub source: String,
    /// Language id (`rust`, `mermaid`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Render line numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    /// First line number (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    /// Highlighted (1-based) line numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight_lines: Vec<u32>,
    /// Wrap long lines (default `false` = horizontal scroll).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
}

impl Code {
    /// A code node.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            common: Common::new(),
            source: source.into(),
            language: None,
            line_numbers: None,
            start_line: None,
            highlight_lines: Vec::new(),
            wrap: None,
        }
    }

    /// Sets the language id.
    #[must_use]
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Renders line numbers.
    #[must_use]
    pub fn line_numbers(mut self, line_numbers: bool) -> Self {
        self.line_numbers = Some(line_numbers);
        self
    }

    /// Sets the first line number.
    #[must_use]
    pub fn start_line(mut self, start_line: u32) -> Self {
        self.start_line = Some(start_line);
        self
    }

    /// Sets the highlighted (1-based) lines.
    #[must_use]
    pub fn highlight_lines(mut self, lines: impl IntoIterator<Item = u32>) -> Self {
        self.highlight_lines = lines.into_iter().collect();
        self
    }

    /// Wraps long lines.
    #[must_use]
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = Some(wrap);
        self
    }
}

impl IntoNode for Code {}
impl From<Code> for Node {
    fn from(c: Code) -> Self {
        Node::Code(c)
    }
}
