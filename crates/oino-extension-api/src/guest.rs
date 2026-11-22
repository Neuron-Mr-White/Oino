//! `guest.wit` records: init input, tool invocations, completion items.

use serde::{Deserialize, Serialize};

use crate::json::RawJson;

/// `init-input`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitInput {
    /// Manifest JSON (as installed).
    pub manifest: RawJson,
    /// Resolved settings for the extension's namespaces (`{namespace: {...}}`).
    pub settings: RawJson,
    /// CLI flag values (`{name: value}`).
    pub flags: RawJson,
}

impl Default for InitInput {
    fn default() -> Self {
        let empty = RawJson::from_json_value(&serde_json::Value::Object(serde_json::Map::new()));
        Self {
            manifest: empty.clone(),
            settings: empty.clone(),
            flags: empty,
        }
    }
}

/// `tool-invocation`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInvocation {
    /// Tool call id.
    pub tool_call_id: String,
    /// Tool name.
    pub name: String,
    /// Validated arguments (JSON object).
    pub args: RawJson,
    /// Set for nested calls made through `control.execute-tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
}

/// `completion-item` (command argument completion).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    /// Inserted text.
    pub value: String,
    /// Label (defaults to `value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
