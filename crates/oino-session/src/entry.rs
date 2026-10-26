//! Session v3 line types: [`SessionHeader`], [`SessionEntry`] and [`EntryKind`].

use std::fmt;

use oino_ai::json::keep_null;
use oino_ai::{
    AgentMessage, AssistantContent, JsonValue, SystemMessage, Usage, UserContent, UserContentField,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time::{format_iso, now_ms, parse_iso};

/// Current session format version.
pub const CURRENT_SESSION_VERSION: u32 = 3;

/// Entry id: 8 lowercase hex chars normally; any string is accepted on read.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(String);

impl EntryId {
    /// Wraps an id string as-is.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// pi `generateId`: first 8 hex chars of a random UUID, retried up to 100 times against
    /// `taken`, then a full UUID.
    #[must_use]
    pub fn generate(taken: impl Fn(&str) -> bool) -> Self {
        for _ in 0..100 {
            let uuid = Uuid::new_v4().simple().to_string();
            let short = &uuid[..8];
            if !taken(short) {
                return Self(short.to_owned());
            }
        }
        Self(Uuid::new_v4().hyphenated().to_string())
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for EntryId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl From<String> for EntryId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

/// ISO-8601 UTC timestamp with millisecond precision and `Z` (JS `toISOString`). The original
/// text is preserved on round trips.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IsoTimestamp(String);

impl IsoTimestamp {
    /// Current time.
    #[must_use]
    pub fn now() -> Self {
        Self::from_ms(now_ms())
    }

    /// From Unix milliseconds.
    #[must_use]
    pub fn from_ms(ms: i64) -> Self {
        Self(format_iso(ms))
    }

    /// Unix milliseconds, `None` when the text is not a parseable date (JS `NaN`).
    #[must_use]
    pub fn to_ms(&self) -> Option<i64> {
        parse_iso(&self.0)
    }

    /// Wraps text as-is.
    #[must_use]
    pub fn from_text(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The timestamp text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// File-name form: `:` and `.` replaced by `-` (`2026-10-26T03-00-07-623Z`).
    #[must_use]
    pub fn file_stem(&self) -> String {
        self.0.replace([':', '.'], "-")
    }
}

impl fmt::Display for IsoTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const fn default_version() -> u32 {
    1
}

/// First line of a session file (`"type":"session"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "session", rename_all = "camelCase")]
pub struct SessionHeader {
    /// Format version (missing ⇒ 1; 3 after migration).
    #[serde(default = "default_version")]
    pub version: u32,
    /// Session id (`UUIDv7` by default).
    pub id: String,
    /// Creation time.
    #[serde(default)]
    pub timestamp: IsoTimestamp,
    /// Working directory the session belongs to.
    #[serde(default)]
    pub cwd: String,
    /// Source session file for forks/clones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
}

/// One tree entry (every line after the header).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    /// Entry id.
    pub id: EntryId,
    /// Parent entry; `null` for roots (always serialised).
    #[serde(default)]
    pub parent_id: Option<EntryId>,
    /// Creation time.
    #[serde(default)]
    pub timestamp: IsoTimestamp,
    /// Type-specific payload (`type` tag).
    #[serde(flatten)]
    pub kind: EntryKind,
}

impl SessionEntry {
    /// pi `type` string of this entry.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }

    /// The message of a `message` entry.
    #[must_use]
    pub const fn message(&self) -> Option<&AgentMessage> {
        match &self.kind {
            EntryKind::Message { message } => Some(message),
            _ => None,
        }
    }

    /// `true` for `message` entries whose role is `system`.
    #[must_use]
    pub const fn is_system_message(&self) -> bool {
        matches!(self.message(), Some(AgentMessage::System(_)))
    }
}

/// Entry payloads (`"type"` discriminant, snake case).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "variants mirror pi entry shapes; entries are stored by value in one Vec"
)]
pub enum EntryKind {
    /// Conversation message.
    Message {
        /// The message.
        message: AgentMessage,
    },
    /// Thinking level change (free string; parse with `ThinkingLevel::from_str`).
    ThinkingLevelChange {
        /// New level.
        #[serde(rename = "thinkingLevel")]
        thinking_level: String,
    },
    /// Model change.
    ModelChange {
        /// Provider id.
        provider: String,
        /// Model id.
        #[serde(rename = "modelId")]
        model_id: String,
    },
    /// Usage outside assistant messages (e.g. `cache_warm`).
    Usage {
        /// Operation kind (unknown kinds are normal usage).
        kind: String,
        /// Provider id.
        provider: String,
        /// Model id.
        model: String,
        /// Usage.
        usage: Usage,
        /// Free-form note.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Compaction.
    Compaction(CompactionEntry),
    /// Append-only context edit of an earlier entry.
    ContextEdit {
        /// Edited entry.
        #[serde(rename = "targetId")]
        target_id: EntryId,
        /// Replacement content, `null` (always serialised) to omit the target.
        #[serde(default)]
        replacement: Option<ContextReplacement>,
    },
    /// Branch summary.
    BranchSummary(BranchSummaryEntry),
    /// Extension state (never in context).
    Custom {
        /// Extension-defined type.
        #[serde(rename = "customType")]
        custom_type: String,
        /// Payload.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "keep_null"
        )]
        data: Option<JsonValue>,
    },
    /// Extension message that participates in context.
    CustomMessage {
        /// Extension-defined type.
        #[serde(rename = "customType")]
        custom_type: String,
        /// String or blocks.
        #[serde(default)]
        content: UserContentField,
        /// Whether frontends render it.
        #[serde(default)]
        display: bool,
        /// Metadata (not sent to the LLM).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "keep_null"
        )]
        details: Option<JsonValue>,
    },
    /// Label set/cleared on an entry.
    Label {
        /// Labelled entry.
        #[serde(rename = "targetId")]
        target_id: EntryId,
        /// Label; absent/empty clears.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Session metadata (display name).
    SessionInfo {
        /// Session name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl EntryKind {
    /// pi `type` string.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Message { .. } => "message",
            Self::ThinkingLevelChange { .. } => "thinking_level_change",
            Self::ModelChange { .. } => "model_change",
            Self::Usage { .. } => "usage",
            Self::Compaction(_) => "compaction",
            Self::ContextEdit { .. } => "context_edit",
            Self::BranchSummary(_) => "branch_summary",
            Self::Custom { .. } => "custom",
            Self::CustomMessage { .. } => "custom_message",
            Self::Label { .. } => "label",
            Self::SessionInfo { .. } => "session_info",
        }
    }
}

/// `compaction` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionEntry {
    /// Summary text.
    pub summary: String,
    /// First retained entry; the compaction's own id means "retain none".
    pub first_kept_entry_id: EntryId,
    /// Context tokens before compaction.
    #[serde(default)]
    pub tokens_before: u64,
    /// Implementation data (e.g. `{readFiles, modifiedFiles}`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "keep_null"
    )]
    pub details: Option<JsonValue>,
    /// Usage of the summary request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// `true` when produced by an extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
    /// Complete prompt/tool checkpoint at the boundary (serialised with `"role":"system"`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "system_with_role"
    )]
    pub system_message: Option<SystemMessage>,
}

#[allow(
    clippy::ref_option,
    reason = "signature required by serde(serialize_with)"
)]
fn system_with_role<S: serde::Serializer>(
    value: &Option<SystemMessage>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .clone()
        .map(oino_ai::Message::System)
        .serialize(serializer)
}

/// `branch_summary` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryEntry {
    /// Previous leaf whose path was summarised (`"root"` when there was none).
    pub from_id: EntryId,
    /// Summary text.
    #[serde(default)]
    pub summary: String,
    /// File tracking or extension data.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "keep_null"
    )]
    pub details: Option<JsonValue>,
    /// Usage of the summary request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// `true` when produced by an extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
}

/// `context_edit.replacement` object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextReplacement {
    /// New content.
    pub content: EditableContent,
}

/// Replacement content: a string or a block array.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EditableContent {
    /// String form.
    Text(String),
    /// Text/image blocks.
    User(Vec<UserContent>),
    /// Assistant blocks (text/thinking/tool calls).
    Assistant(Vec<AssistantContent>),
}

/// One parsed line of a session file.
#[allow(
    clippy::large_enum_variant,
    reason = "short-lived parse result; boxing would only add an allocation per line"
)]
#[derive(Clone, Debug, PartialEq)]
pub enum FileLine {
    /// `"type":"session"` header.
    Header(SessionHeader),
    /// Tree entry.
    Entry(SessionEntry),
}

impl FileLine {
    /// Parses one JSON line (header when `type == "session"`).
    ///
    /// # Errors
    ///
    /// Returns the serde error when the line is not a valid header or entry.
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        let value: JsonValue = serde_json::from_str(line)?;
        Self::from_value(value)
    }

    /// Like [`FileLine::parse`] from an already parsed value.
    ///
    /// # Errors
    ///
    /// Returns the serde error when the value is not a valid header or entry.
    pub fn from_value(value: JsonValue) -> Result<Self, serde_json::Error> {
        if value.get("type").and_then(JsonValue::as_str) == Some("session") {
            serde_json::from_value(value).map(Self::Header)
        } else {
            serde_json::from_value(value).map(Self::Entry)
        }
    }

    /// Serialises to one JSON line (no trailing newline).
    ///
    /// # Errors
    ///
    /// Propagates serde errors (not expected for these types).
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Header(header) => serde_json::to_string(header),
            Self::Entry(entry) => serde_json::to_string(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn header_has_type_first_and_requires_session_tag() {
        let header = SessionHeader {
            version: 3,
            id: "u".into(),
            timestamp: IsoTimestamp::from_ms(0),
            cwd: "/p".into(),
            parent_session: None,
        };
        let line = serde_json::to_string(&header).unwrap();
        assert!(
            line.starts_with("{\"type\":\"session\",\"version\":3"),
            "{line}"
        );
        let v1: SessionHeader =
            serde_json::from_value(json!({"type":"session","id":"x","timestamp":"t","cwd":"/"}))
                .unwrap();
        assert_eq!(v1.version, 1);
    }

    #[test]
    fn generated_ids_are_short_hex() {
        let id = EntryId::generate(|_| false);
        assert_eq!(id.as_str().len(), 8);
        assert!(id.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(EntryId::generate(|_| true).as_str().len(), 36);
    }

    #[test]
    fn null_fields_are_always_present() {
        let entry = SessionEntry {
            id: "a".into(),
            parent_id: None,
            timestamp: IsoTimestamp::from_ms(0),
            kind: EntryKind::ContextEdit {
                target_id: "b".into(),
                replacement: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            json!({"id":"a","parentId":null,"timestamp":"1970-01-01T00:00:00.000Z","type":"context_edit","targetId":"b","replacement":null})
        );
    }
}
