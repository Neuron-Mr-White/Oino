//! Session reads and writes (`sessionManager`, `appendEntry`, `sendMessage`).

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
/// Which entries an [`EntryQuery`] starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryScope {
    /// Every entry in file order (all branches), like [`entries`].
    #[default]
    All,
    /// Root → leaf, like [`branch`].
    Branch,
}

/// A bounded session read ([`query`]): the host filters before it serializes, so only the
/// matching entries cross into this extension's memory (which never shrinks). Prefer it over
/// [`entries`] / [`branch`] anywhere a hook runs on every session.
///
/// Filters: `types` (entry `type`), `custom_types` (checked on `custom` / `custom_message`
/// entries only), `roles` (checked on `message` entries only). `last` keeps the newest N
/// matches, `max_string` clips longer strings. Results are oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryQuery {
    /// Starting set.
    pub scope: QueryScope,
    /// Start at the live context of the newest `compaction`: its first kept entry (when in
    /// scope), else the compaction entry itself.
    pub since_compaction: bool,
    /// Entry `type`s to keep (empty: all).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// `customType`s to keep (empty: all).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_types: Vec<String>,
    /// Message roles to keep (empty: all).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Keep only the newest N matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<usize>,
    /// Clip string values longer than this many bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_string: Option<usize>,
}

impl EntryQuery {
    /// The `custom` entries of these `customType`s (all branches, file order) — the
    /// extension-state restore at `session_start`.
    #[must_use]
    pub fn custom(custom_types: &[&str]) -> Self {
        Self {
            types: vec!["custom".to_owned()],
            custom_types: custom_types.iter().map(|t| (*t).to_owned()).collect(),
            ..Self::default()
        }
    }

    /// The messages of these roles on the current branch.
    #[must_use]
    pub fn messages(roles: &[&str]) -> Self {
        Self {
            scope: QueryScope::Branch,
            types: vec!["message".to_owned()],
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            ..Self::default()
        }
    }

    /// Keep only the newest `n` matches.
    #[must_use]
    pub const fn last(mut self, n: usize) -> Self {
        self.last = Some(n);
        self
    }

    /// Clip strings longer than `bytes`.
    #[must_use]
    pub const fn max_string(mut self, bytes: usize) -> Self {
        self.max_string = Some(bytes);
        self
    }
}

/// The entries matching `query`, oldest first (`query-entries`).
pub fn query(query: &EntryQuery) -> Result<Vec<Value>> {
    parse(&ws::query_entries(&serde_json::to_string(query)?))
}

/// The newest `custom` entry of `custom_type`, if any.
pub fn last_custom(custom_type: &str) -> Result<Option<Value>> {
    Ok(query(&EntryQuery::custom(&[custom_type]).last(1))?.pop())
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
/// Sets the session name (permission `sessionControl`).
pub fn set_session_name(name: Option<&str>) -> Result<()> {
    Ok(ws::set_session_name(name)?)
}
/// Sets the entry label (permission `sessionControl`).
pub fn set_label(id: &str, label: Option<&str>) -> Result<()> {
    Ok(ws::set_label(id, label)?)
}
/// Appends an entry (`appendEntry(customType, data)`); returns the entry id.
pub fn append_entry(custom_type: &str, data: Option<&impl Serialize>) -> Result<String> {
    let data = data.map(serde_json::to_string).transpose()?;
    Ok(ws::append_entry(custom_type, data.as_deref())?)
}

/// Append trusted harness context without changing any earlier message.
/// The key must start with this extension's id followed by `/` and a topic.
pub fn append_reminder(key: &str, text: &str) -> Result<Value> {
    parse(&ws::append_reminder(key, text)?)
}

/// A custom message (`sendMessage` draft).
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

/// Sends a message into the session.
pub async fn send_message(message: &CustomMessage, options: SendOptions) -> Result<()> {
    let opts = ws::SendOptions {
        trigger_turn: options.trigger_turn,
        deliver_as: options.deliver_as,
        provenance: options.provenance,
    };
    Ok(ws::send_message(serde_json::to_string(message)?, opts).await?)
}

/// Sends a user message (permission `sessionControl`). `content` = string or content blocks.
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
