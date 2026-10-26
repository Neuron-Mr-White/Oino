//! Shared helpers (pi `compaction/utils.js`): file-operation tracking and conversation
//! serialisation for summary prompts.

use indexmap::IndexSet;
use oino_ai::{AgentMessage, AssistantContent, JsonObject, JsonValue, Message, UserContent};
use serde::{Deserialize, Serialize};

use super::js;

/// Files touched by tool calls (pi `FileOperations`; sets keep insertion order).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileOperations {
    /// Paths passed to `read`.
    pub read: IndexSet<String>,
    /// Paths passed to `write`.
    pub written: IndexSet<String>,
    /// Paths passed to `edit`.
    pub edited: IndexSet<String>,
}

/// Sorted file lists (pi `computeFileLists` result, also `CompactionDetails` /
/// `BranchSummaryDetails`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLists {
    /// Files only read.
    pub read_files: Vec<String>,
    /// Files written or edited.
    pub modified_files: Vec<String>,
}

impl FileOperations {
    /// pi `createFileOps`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// pi `extractFileOpsFromMessage`: tool calls of an assistant message, or the nested calls
    /// recorded on a tool result (codemode scripts).
    pub fn extract_from_message(&mut self, message: &AgentMessage) {
        match message {
            AgentMessage::ToolResult(result) => {
                for call in result.nested_calls.iter().flat_map(|nested| &nested.calls) {
                    self.add(&call.name, call.arguments.as_ref());
                }
            }
            AgentMessage::Assistant(assistant) => {
                for block in &assistant.content {
                    if let AssistantContent::ToolCall(call) = block {
                        self.add(&call.name, Some(&call.arguments));
                    }
                }
            }
            _ => {}
        }
    }

    fn add(&mut self, tool: &str, arguments: Option<&JsonObject>) {
        let Some(path) = arguments
            .and_then(|args| args.get("path"))
            .and_then(JsonValue::as_str)
            .filter(|path| !path.is_empty())
        else {
            return;
        };
        let set = match tool {
            "read" => &mut self.read,
            "write" => &mut self.written,
            "edit" => &mut self.edited,
            _ => return,
        };
        set.insert(path.to_owned());
    }

    /// pi `computeFileLists`: read-only files and modified (edited ∪ written) files, each in
    /// JS default sort order.
    #[must_use]
    pub fn file_lists(&self) -> FileLists {
        let modified: IndexSet<&String> = self.edited.iter().chain(&self.written).collect();
        let mut read_files: Vec<String> = self
            .read
            .iter()
            .filter(|path| !modified.contains(path))
            .cloned()
            .collect();
        let mut modified_files: Vec<String> = modified.into_iter().cloned().collect();
        read_files.sort_by(|a, b| js::cmp16(a, b));
        modified_files.sort_by(|a, b| js::cmp16(a, b));
        FileLists {
            read_files,
            modified_files,
        }
    }

    /// Seeds from stored file lists (previous pi-generated compaction / branch summary
    /// `details`): `readFiles` → read, `modifiedFiles` → edited. Non-array fields and non-string
    /// items are ignored like pi's `Array.isArray` guard (strings only).
    pub fn seed_from_details(&mut self, details: &JsonValue) {
        let list = |key: &str| {
            details
                .get(key)
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .map(str::to_owned)
        };
        self.read.extend(list("readFiles"));
        self.edited.extend(list("modifiedFiles"));
    }
}

/// pi `formatFileOperations`: `<read-files>` / `<modified-files>` sections, prefixed with a
/// blank line, or `""` when both lists are empty.
#[must_use]
pub fn format_file_operations(read_files: &[String], modified_files: &[String]) -> String {
    let mut sections = Vec::new();
    if !read_files.is_empty() {
        sections.push(format!(
            "<read-files>\n{}\n</read-files>",
            read_files.join("\n")
        ));
    }
    if !modified_files.is_empty() {
        sections.push(format!(
            "<modified-files>\n{}\n</modified-files>",
            modified_files.join("\n")
        ));
    }
    if sections.is_empty() {
        return String::new();
    }
    format!("\n\n{}", sections.join("\n\n"))
}

/// Maximum characters (UTF-16 units) of a tool result in serialised summaries.
pub const TOOL_RESULT_MAX_CHARS: usize = 2000;

fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    let length = js::len16(text);
    if length <= max_chars {
        return text.to_owned();
    }
    format!(
        "{}\n\n[... {} more characters truncated]",
        js::prefix16(text, max_chars),
        length - max_chars
    )
}

fn blocks_text(blocks: &[UserContent], separator: &str) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            UserContent::Text(text) => Some(text.text.as_str()),
            UserContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join(separator)
}

/// pi `serializeConversation`: LLM messages as `[User]:` / `[Assistant]:` /
/// `[Assistant tool calls]:` / `[Tool result]:` paragraphs (thinking omitted, tool results
/// truncated to [`TOOL_RESULT_MAX_CHARS`]). Call [`oino_session::convert_to_llm`] first.
#[must_use]
pub fn serialize_conversation(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        match message {
            Message::User(user) => {
                let content = user.content.text("");
                if !content.is_empty() {
                    parts.push(format!("[User]: {content}"));
                }
            }
            Message::Assistant(assistant) => {
                let mut texts = Vec::new();
                let mut calls = Vec::new();
                for block in &assistant.content {
                    match block {
                        AssistantContent::Text(text) => texts.push(text.text.as_str()),
                        AssistantContent::ToolCall(call) => {
                            let args = call
                                .arguments
                                .iter()
                                .map(|(key, value)| format!("{key}={}", js::stringify(value)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            calls.push(format!("{}({args})", call.name));
                        }
                        AssistantContent::Thinking(_) => {}
                    }
                }
                if !texts.is_empty() {
                    parts.push(format!("[Assistant]: {}", texts.join("\n")));
                }
                if !calls.is_empty() {
                    parts.push(format!("[Assistant tool calls]: {}", calls.join("; ")));
                }
            }
            Message::ToolResult(result) => {
                let content = blocks_text(&result.content, "");
                if !content.is_empty() {
                    parts.push(format!(
                        "[Tool result]: {}",
                        truncate_for_summary(&content, TOOL_RESULT_MAX_CHARS)
                    ));
                }
            }
            Message::System(_) => {}
        }
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn seed_ignores_malformed_details() {
        let mut ops = FileOperations::new();
        ops.seed_from_details(&json!({"readFiles": "x", "modifiedFiles": ["a", 1]}));
        assert!(ops.read.is_empty());
        assert_eq!(ops.edited.iter().collect::<Vec<_>>(), ["a"]);
    }
}
