//! In-app browser control (permission `browser`, browser-control.md §8.3).

use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::browser as wbr;
use crate::error::parse;

/// A screenshot returned by a command.
#[derive(Clone, Debug)]
pub struct Image {
    /// `image/png` / `image/jpeg`.
    pub mime_type: String,
    /// Bytes.
    pub data: Vec<u8>,
    /// Absolute path of the saved original.
    pub path: Option<String>,
}

/// Runs one command (`{"op": "...", ...args, "approval"?: "once"|"site"}`); returns the
/// `BrowserResult` JSON (without image) and the image, if any.
pub async fn execute(command: &Value) -> Result<(Value, Option<Image>)> {
    let out = wbr::execute(command.to_string(), None).await?;
    let image = out.image.map(|i| Image {
        mime_type: i.mime_type,
        data: i.data,
        path: i.path,
    });
    Ok((parse(&out.result)?, image))
}

/// `BrowserStatus` JSON.
#[must_use]
pub fn status() -> Value {
    serde_json::from_str(&wbr::status()).unwrap_or(Value::Null)
}
