//! `Image` node (§4.5.2).

use serde::{Deserialize, Serialize};

use crate::node::{Common, IntoNode, Node};

/// Where the image bytes come from (§4.5.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageSource {
    /// Inline base64 bytes (≤ 1 MiB decoded; > 4 MiB decoded rejected, §4.8).
    #[serde(rename_all = "camelCase")]
    Data {
        /// Base64-encoded bytes.
        data: String,
        /// MIME type.
        mime_type: String,
    },
    /// Host file path (fetched with `fs_read`, host.md §9.1).
    #[serde(rename_all = "camelCase")]
    Path {
        /// Path in the session workspace.
        path: String,
    },
    /// Host-side blob id (fetched with `blob_get`, host.md §9.2).
    #[serde(rename_all = "camelCase")]
    Blob {
        /// Blob id.
        blob: String,
    },
}

impl ImageSource {
    /// Inline base64 source.
    #[must_use]
    pub fn data(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::Data {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    /// Host file-path source.
    #[must_use]
    pub fn path(path: impl Into<String>) -> Self {
        Self::Path { path: path.into() }
    }

    /// Host blob-id source.
    #[must_use]
    pub fn blob(blob: impl Into<String>) -> Self {
        Self::Blob { blob: blob.into() }
    }

    /// Base64 payload length estimate in decoded bytes (only meaningful for
    /// [`ImageSource::Data`]).
    #[must_use]
    pub fn decoded_len_estimate(&self) -> usize {
        match self {
            Self::Data { data, .. } => data.len() * 3 / 4,
            _ => 0,
        }
    }
}

/// Object fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Fit {
    /// Keep aspect ratio, fit inside (default).
    #[default]
    Contain,
    /// Keep aspect ratio, cover the area.
    Cover,
}

/// An image (§4.5.2). TUI: kitty / iTerm2 / sixel when supported, else a dim
/// `[image: alt]` box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    /// Common node props (`key`, `style`, `block`, `fallback`).
    #[serde(flatten)]
    pub common: Common,
    /// Image source.
    pub src: ImageSource,
    /// Alt text (the TUI fallback box).
    pub alt: String,
    /// Object fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
    /// Maximum width in columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u16>,
    /// Maximum height in rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u16>,
}

impl Image {
    /// An image with source and alt text.
    #[must_use]
    pub fn new(src: ImageSource, alt: impl Into<String>) -> Self {
        Self {
            common: Common::new(),
            src,
            alt: alt.into(),
            fit: None,
            max_width: None,
            max_height: None,
        }
    }

    /// Sets the object fit.
    #[must_use]
    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = Some(fit);
        self
    }

    /// Sets the maximum width.
    #[must_use]
    pub fn max_width(mut self, width: u16) -> Self {
        self.max_width = Some(width);
        self
    }

    /// Sets the maximum height.
    #[must_use]
    pub fn max_height(mut self, height: u16) -> Self {
        self.max_height = Some(height);
        self
    }
}

impl IntoNode for Image {}
impl From<Image> for Node {
    fn from(i: Image) -> Self {
        Node::Image(i)
    }
}
