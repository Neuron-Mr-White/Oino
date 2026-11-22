//! `Markdown` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};

/// Markdown text (§4.5.2). TUI: pulldown-cmark; app: `renderChatMarkdown`
/// (marked + `DOMPurify`). Links open through `openUrl`, never navigate the webview.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Markdown {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Markdown source (≤ 256 KiB, §4.8).
    pub source: String,
}

impl Markdown {
    /// A markdown node.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            common: Common::new(),
            source: source.into(),
        }
    }
}

impl IntoNode for Markdown {}
impl From<Markdown> for Node {
    fn from(m: Markdown) -> Self {
        Node::Markdown(m)
    }
}
