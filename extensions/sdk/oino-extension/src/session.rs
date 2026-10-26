//! Session reads and writes (pi `sessionManager`, `appendEntry`, `sendMessage`).

use serde::Serialize;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::session as ws;
pub use crate::bindings::oino::extension::session::{Delivery, Provenance, SessionsScope};
use crate::error::parse;

/// `SessionHeader`.
pub fn header() -> Result<Value> {
    parse(&ws::header())
}
/// Session id.
#[must_use]
pub fn session_id() -> String {
    ws::session_id()
}
/// Session file, when persisted.
#[must_use]
pub fn session_file() -> Option<String> {
    ws::session_file()
}
/// Leaf entry id.
#[must_use]
pub fn leaf_id() -> Option<String> {
    ws::leaf_id()
}
/// One entry.
pub fn entry(id: &str) -> Result<Option<Value>> {
    ws::entry(id).map(|t| parse(&t)).transpose()
}
/// All entries (file order, all branches).
pub fn entries() -> Result<Vec<Value>> {
    parse(&ws::entries())
}
/// Root → `from` (default leaf).
pub fn branch(from: Option<&str>) -> Result<Vec<Value>> {
    parse(&ws::branch(from))
}
/// Entries selected for the model context.
pub fn context_entries() -> Result<Vec<Value>> {
    parse(&ws::context_entries())
}
/// `SessionProjection`.
pub fn projection() -> Result<Value> {
    parse(&ws::projection())
}
/// Session tree.
pub fn tree() -> Result<Value> {
    parse(&ws::tree())
}
/// Label of an entry.
#[must_use]
pub fn label(id: &str) -> Option<String> {
    ws::label(id)
}
/// Session name.
#[must_use]
pub fn session_name() -> Option<String> {
    ws::session_name()
}
/// pi `setSessionName` (permission `sessionControl`).
pub fn set_session_name(name: Option<&str>) -> Result<()> {
    Ok(ws::set_session_name(name)?)
}
/// pi `setLabel` (permission `sessionControl`).
pub fn set_label(id: &str, label: Option<&str>) -> Result<()> {
    Ok(ws::set_label(id, label)?)
}
/// pi `appendEntry(customType, data)`; returns the entry id.
pub fn append_entry(custom_type: &str, data: Option<&impl Serialize>) -> Result<String> {
    let data = data.map(serde_json::to_string).transpose()?;
    Ok(ws::append_entry(custom_type, data.as_deref())?)
}

/// A custom message (pi `sendMessage` draft).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessage {
    /// Extension-defined type.
    pub custom_type: String,
    /// Content (string or content blocks).
    pub content: Value,
    /// Render in frontends.
    pub display: bool,
    /// Metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Options of [`send_message`].
#[derive(Clone, Debug, Default)]
pub struct SendOptions {
    /// Trigger a turn when idle.
    pub trigger_turn: bool,
    /// Delivery while streaming.
    pub deliver_as: Option<Delivery>,
    /// Provenance.
    pub provenance: Option<Provenance>,
}

/// pi `sendMessage`.
pub async fn send_message(message: &CustomMessage, options: SendOptions) -> Result<()> {
    let opts = ws::SendOptions {
        trigger_turn: options.trigger_turn,
        deliver_as: options.deliver_as,
        provenance: options.provenance,
    };
    Ok(ws::send_message(serde_json::to_string(message)?, opts).await?)
}

/// pi `sendUserMessage` (permission `sessionControl`). `content` = string or content blocks.
pub async fn send_user_message(content: &Value, deliver_as: Option<Delivery>) -> Result<()> {
    let opts = ws::UserMessageOptions {
        deliver_as,
        expand_prompt_templates: false,
        provenance: None,
    };
    Ok(ws::send_user_message(content.to_string(), opts).await?)
}

/// Sessions index (permission `sessions.readAll`).
pub async fn list_sessions(scope: SessionsScope) -> Result<Vec<Value>> {
    parse(&ws::sessions_list(scope).await?)
}

/// Reads another session file (permission `sessions.readAll`).
pub async fn read_session(path: &str) -> Result<Value> {
    parse(&ws::session_read(path.to_owned()).await?)
}
