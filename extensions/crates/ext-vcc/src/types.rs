//! Minimal serde mirror of `oino_session::SessionEntry`/`SessionEntryKind`
//! (the guest cannot depend on oino-session — it pulls tokio). Field names
//! and serde shapes are identical so `host.session_entries` JSON parses here.

use oino_types::{ContentBlock, Message, Model, OinoId, ThinkingLevel, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExtensionSessionEntry {
    pub owner_extension_id: String,
    pub key: String,
    #[serde(default)]
    pub schema_version: u32,
    pub payload: Value,
    #[serde(default)]
    pub provenance: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SessionEntryKind {
    Message {
        message: Message,
    },
    ModelChange {
        model: Model,
    },
    ThinkingLevelChange {
        thinking_level: ThinkingLevel,
    },
    Compaction {
        summary: String,
        first_kept: Option<OinoId>,
        #[serde(default)]
        tokens_before: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        replaces: Vec<OinoId>,
    },
    ContextEdit {
        target: OinoId,
        replacement: Option<Vec<ContentBlock>>,
    },
    Usage {
        usage_kind: String,
        model: Model,
        usage: Usage,
    },
    BranchSummary {
        summary: String,
    },
    Custom {
        name: String,
        payload: Value,
    },
    ExtensionCustom {
        entry: Box<ExtensionSessionEntry>,
    },
    CustomMessage {
        message: Message,
    },
    Label {
        label: String,
    },
    SessionInfo {
        name: Option<String>,
        cwd: Option<PathBuf>,
    },
    LeafMove {
        leaf_id: OinoId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: OinoId,
    pub parent: Option<OinoId>,
    pub kind: SessionEntryKind,
}
