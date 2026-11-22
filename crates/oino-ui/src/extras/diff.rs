//! `Diff` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};

/// Diff layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiffMode {
    /// Unified (default).
    #[default]
    Unified,
    /// Side by side (desktop ≥ 900 px only).
    Split,
}

/// A unified patch, or old/new text pairs (§4.5.2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Unified diff patch (either this, or `old` + `new`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    /// Old text (with `new`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    /// New text (with `old`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<String>,
    /// File path shown in the header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Context lines for `old`/`new` (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u16>,
    /// Layout mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DiffMode>,
}

impl Diff {
    /// Diff from a unified patch.
    #[must_use]
    pub fn patch(patch: impl Into<String>) -> Self {
        Self {
            common: Common::new(),
            patch: Some(patch.into()),
            old: None,
            new: None,
            path: None,
            context: None,
            mode: None,
        }
    }

    /// Diff from old / new text.
    #[must_use]
    pub fn old_new(old: impl Into<String>, new: impl Into<String>) -> Self {
        Self {
            common: Common::new(),
            patch: None,
            old: Some(old.into()),
            new: Some(new.into()),
            path: None,
            context: None,
            mode: None,
        }
    }

    /// Sets the file path.
    #[must_use]
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Sets the context-line count.
    #[must_use]
    pub fn context(mut self, context: u16) -> Self {
        self.context = Some(context);
        self
    }

    /// Sets the layout mode.
    #[must_use]
    pub fn mode(mut self, mode: DiffMode) -> Self {
        self.mode = Some(mode);
        self
    }
}

impl IntoNode for Diff {}
impl From<Diff> for Node {
    fn from(d: Diff) -> Self {
        Node::Diff(d)
    }
}
