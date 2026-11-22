//! ratatui text model (§4.2): [`Span`], [`Line`], [`Text`] and [`Alignment`].
//!
//! Every type accepts the shorthand forms from Appendix A: a bare string, an array
//! (of spans / lines), or the full object. A string splits on `\n` when rendered.

use serde::{Deserialize, Serialize};

use crate::style::{Style, Stylize};

/// Text alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Alignment {
    /// Left.
    #[default]
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
}

/// ratatui `Span`: a string or `{content, style?, link?}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Span {
    /// Shorthand: plain string.
    Str(String),
    /// Full form.
    Full {
        /// Span text.
        content: String,
        /// Span style.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<Style>,
        /// Hyperlink target (GUI `<a>` / OSC 8).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
    },
}

impl Default for Span {
    fn default() -> Self {
        Self::Str(String::new())
    }
}

impl Span {
    /// Builds a styled span.
    #[must_use]
    pub fn from_string(content: impl Into<String>) -> Self {
        Self::Full {
            content: content.into(),
            style: None,
            link: None,
        }
    }

    /// Styled span with a hyperlink target.
    #[must_use]
    pub fn linked(content: impl Into<String>, link: impl Into<String>) -> Self {
        Self::Full {
            content: content.into(),
            style: None,
            link: Some(link.into()),
        }
    }

    /// Sets a hyperlink target, keeping the current style.
    #[must_use]
    pub fn with_link(self, link: impl Into<String>) -> Self {
        match self {
            Self::Str(content) => Self::Full {
                content,
                style: None,
                link: Some(link.into()),
            },
            Self::Full {
                content,
                style,
                link: _,
            } => Self::Full {
                content,
                style,
                link: Some(link.into()),
            },
        }
    }

    /// The span's text.
    #[must_use]
    pub fn content(&self) -> &str {
        match self {
            Self::Str(content) | Self::Full { content, .. } => content,
        }
    }

    /// The span's style, if any.
    #[must_use]
    pub fn style(&self) -> Option<&Style> {
        match self {
            Self::Str(_) => None,
            Self::Full { style, .. } => style.as_ref(),
        }
    }

    /// The span's link target, if any.
    #[must_use]
    pub fn link(&self) -> Option<&str> {
        match self {
            Self::Str(_) => None,
            Self::Full { link, .. } => link.as_deref(),
        }
    }

    /// Effective style: the stored one or the default.
    #[must_use]
    pub fn effective_style(&self) -> Style {
        self.style().cloned().unwrap_or_default()
    }
}

impl From<&str> for Span {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}

impl From<String> for Span {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl Stylize<Span> for Span {
    fn style(self, style: Style) -> Span {
        match self {
            Self::Str(content) => Self::Full {
                content,
                style: Some(style),
                link: None,
            },
            Self::Full {
                content,
                style: existing,
                link,
            } => Self::Full {
                content,
                style: Some(existing.unwrap_or_default().patch(style)),
                link,
            },
        }
    }
}

impl Stylize<Span> for &str {
    fn style(self, style: Style) -> Span {
        Span::Full {
            content: self.to_owned(),
            style: Some(style),
            link: None,
        }
    }
}

impl Stylize<Span> for String {
    fn style(self, style: Style) -> Span {
        Span::Full {
            content: self,
            style: Some(style),
            link: None,
        }
    }
}

/// ratatui `Line`: a string, an array of spans, or `{spans, style?, alignment?}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Line {
    /// Shorthand: plain string.
    Str(String),
    /// Shorthand: array of spans.
    Spans(Vec<Span>),
    /// Full form.
    Full {
        /// Line spans.
        spans: Vec<Span>,
        /// Line style.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<Style>,
        /// Line alignment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<Alignment>,
    },
}

impl Default for Line {
    fn default() -> Self {
        Self::Str(String::new())
    }
}

impl Line {
    /// Full-form line from spans.
    #[must_use]
    pub fn from_spans(spans: Vec<Span>) -> Self {
        Self::Full {
            spans,
            style: None,
            alignment: None,
        }
    }

    /// Flattens the line to plain text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let spans: &[Span] = match self {
            Self::Str(_) => return String::new(),
            Self::Spans(spans) | Self::Full { spans, .. } => spans,
        };
        let mut out = String::new();
        for span in spans {
            out.push_str(span.content());
        }
        out
    }

    /// A single-span line holding `text` (empty for shorthand string lines).
    #[must_use]
    pub fn as_span(&self) -> Option<&Span> {
        match self {
            Self::Str(_) => None,
            Self::Spans(spans) | Self::Full { spans, .. } => spans.first(),
        }
    }

    /// The line's spans (a shorthand string becomes one span).
    #[must_use]
    pub fn spans(&self) -> Vec<Span> {
        match self {
            Self::Str(text) => vec![Span::Str(text.clone())],
            Self::Spans(spans) | Self::Full { spans, .. } => spans.clone(),
        }
    }

    /// The line's style, if any.
    #[must_use]
    pub fn style(&self) -> Option<&Style> {
        match self {
            Self::Str(_) | Self::Spans(_) => None,
            Self::Full { style, .. } => style.as_ref(),
        }
    }

    /// The line's alignment, if any.
    #[must_use]
    pub fn alignment(&self) -> Option<Alignment> {
        match self {
            Self::Str(_) | Self::Spans(_) => None,
            Self::Full { alignment, .. } => *alignment,
        }
    }
}

impl From<&str> for Line {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}

impl From<String> for Line {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<Span> for Line {
    fn from(span: Span) -> Self {
        Self::Spans(vec![span])
    }
}

impl From<Vec<Span>> for Line {
    fn from(spans: Vec<Span>) -> Self {
        Self::Spans(spans)
    }
}

impl Stylize<Line> for Line {
    fn style(self, style: Style) -> Line {
        let spans = match self {
            Self::Str(text) => vec![Span::Str(text)],
            Self::Spans(spans) => spans,
            Self::Full {
                spans,
                style: existing,
                alignment,
            } => {
                return Self::Full {
                    spans,
                    style: Some(existing.unwrap_or_default().patch(style)),
                    alignment,
                };
            }
        };
        Self::Full {
            spans,
            style: Some(style),
            alignment: None,
        }
    }
}

impl Stylize<Line> for Vec<Span> {
    fn style(self, style: Style) -> Line {
        Line::Full {
            spans: self,
            style: Some(style),
            alignment: None,
        }
    }
}

/// ratatui `Text`: a string, an array of lines, or `{lines, style?, alignment?}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    /// Shorthand: plain string (splits on `\n`).
    Str(String),
    /// Shorthand: array of lines.
    Lines(Vec<Line>),
    /// Full form.
    Full {
        /// Text lines.
        lines: Vec<Line>,
        /// Text style.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<Style>,
        /// Text alignment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<Alignment>,
    },
}

impl Default for Text {
    fn default() -> Self {
        Self::Str(String::new())
    }
}

impl Text {
    /// Full-form text from lines.
    #[must_use]
    pub fn from_lines(lines: Vec<Line>) -> Self {
        Self::Full {
            lines,
            style: None,
            alignment: None,
        }
    }

    /// The text's lines (a shorthand string splits on `\n`).
    #[must_use]
    pub fn lines(&self) -> Vec<Line> {
        match self {
            Self::Str(text) => text
                .split('\n')
                .map(|line| Line::Str(line.to_owned()))
                .collect(),
            Self::Lines(lines) | Self::Full { lines, .. } => lines.clone(),
        }
    }

    /// The text's style, if any.
    #[must_use]
    pub fn style(&self) -> Option<&Style> {
        match self {
            Self::Str(_) | Self::Lines(_) => None,
            Self::Full { style, .. } => style.as_ref(),
        }
    }

    /// The text's alignment, if any.
    #[must_use]
    pub fn alignment(&self) -> Option<Alignment> {
        match self {
            Self::Str(_) | Self::Lines(_) => None,
            Self::Full { alignment, .. } => *alignment,
        }
    }

    /// Flattens to plain text joined by `\n`.
    #[must_use]
    pub fn to_text(&self) -> String {
        match self {
            Self::Str(text) => text.clone(),
            Self::Lines(lines) | Self::Full { lines, .. } => lines
                .iter()
                .map(|line| match line {
                    Line::Str(text) => text.clone(),
                    _ => line.to_text(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<Line> for Text {
    fn from(line: Line) -> Self {
        Self::Lines(vec![line])
    }
}

impl From<Span> for Text {
    fn from(span: Span) -> Self {
        Self::Lines(vec![Line::Spans(vec![span])])
    }
}

impl From<Vec<Line>> for Text {
    fn from(lines: Vec<Line>) -> Self {
        Self::Lines(lines)
    }
}

impl Stylize<Text> for Text {
    fn style(self, style: Style) -> Text {
        let lines = match self {
            Self::Str(text) => text
                .split('\n')
                .map(|line| Line::Str(line.to_owned()))
                .collect(),
            Self::Lines(lines) => lines,
            Self::Full {
                lines,
                style: existing,
                alignment,
            } => {
                return Self::Full {
                    lines,
                    style: Some(existing.unwrap_or_default().patch(style)),
                    alignment,
                };
            }
        };
        Self::Full {
            lines,
            style: Some(style),
            alignment: None,
        }
    }
}

impl Stylize<Text> for Vec<Line> {
    fn style(self, style: Style) -> Text {
        Text::Full {
            lines: self,
            style: Some(style),
            alignment: None,
        }
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
    use crate::style::Modifier;

    #[test]
    fn shorthand_forms_parse() {
        let span: Span = serde_json::from_str("\"hi\"").unwrap_or_default();
        assert_eq!(span.content(), "hi");
        let line: Line = serde_json::from_str("[\"a\",\"b\"]").unwrap_or_default();
        assert_eq!(line.to_text(), "ab");
        let text: Text = serde_json::from_str("\"x\\ny\"").unwrap_or_default();
        assert_eq!(text.lines().len(), 2);
    }

    #[test]
    fn stylize_chain_matches_ratatui_shape() {
        let span = "Board ".bold().fg(crate::style::Color::Accent);
        assert_eq!(span.content(), "Board ");
        assert!(span.effective_style().add_modifier.contains(Modifier::BOLD));
    }
}
