//! D-DISPATCH / D-REPAIR (core spec §2.1, WP-C2): the tool-dispatch record and the
//! interrupted-turn repair on resume.
//!
//! The dispatch record is one `custom` entry per side-effect tool call, appended before
//! the tool body runs, so a crash between dispatch and result leaves an attributable
//! record. Repair is the resume layer's job and appends closers only: a torn physical
//! tail is terminated with `"\n"` (pi's own repair, `session-manager.js:365-367`) so the
//! fragment stays a permanently skipped line, and every dangling tool call of the
//! trailing assistant message is closed with an unknown-outcome error toolResult —
//! exactly a pi `toolResult` message a stock pi reader understands. Existing bytes are
//! never rewritten or truncated, and open calls are never auto-retried.

use std::collections::HashSet;

use oino_ai::{
    AgentMessage, AssistantContent, JsonObject, ToolCall, ToolResultMessage, UserContent,
};
use serde_json::json;

use super::{SessionManager, files};
use crate::entry::EntryId;
use crate::error::SessionError;
use crate::time::now_ms;

/// `customType` of the dispatch record (D-DISPATCH).
pub const TOOL_DISPATCH_TYPE: &str = "oino.tool-dispatch";

/// `customType` of the repair record (D-REPAIR).
pub const RESUME_REPAIR_TYPE: &str = "oino.resume-repair";

/// Text closing a dangling tool call (D-REPAIR): the outcome is unknown, so the model
/// must verify before retrying — open calls are never auto-retried.
pub const INTERRUPTED_TOOL_TEXT: &str = "Tool execution was interrupted before completion; the outcome is unknown. The effect may or may not have happened. Do not retry blindly: verify the result first, and ask the user if verification is not possible.";

/// Outcome of [`SessionManager::repair_interrupted_turn`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// Ids of the closed dangling tool calls, in append order.
    pub closed_tool_calls: Vec<String>,
    /// Whether a torn tail line was terminated with `"\n"`.
    pub torn_tail: bool,
}

impl SessionManager {
    /// Appends the [`TOOL_DISPATCH_TYPE`] record of a tool call (D-DISPATCH): a `custom`
    /// entry naming the call before its body runs, so resume repair can attribute an
    /// interrupted effect. `custom` entries never project into context (pi
    /// `session-manager.js:164-181`) — inert to stock pi readers.
    ///
    /// # Errors
    ///
    /// I/O when persisting.
    pub fn append_tool_dispatch(
        &mut self,
        tool_call_id: &str,
        tool_name: &str,
        input: &JsonObject,
    ) -> Result<EntryId, SessionError> {
        self.append_custom_entry(
            TOOL_DISPATCH_TYPE,
            Some(json!({
                "toolCallId": tool_call_id,
                "toolName": tool_name,
                "input": input,
                "readOnly": false,
            })),
        )
    }

    /// Repairs an interrupted turn at the leaf (D-REPAIR). Call once, under write
    /// ownership, before the session is used; appends only, never rewrites or truncates:
    ///
    /// 1. a torn last line (non-empty, not valid JSON) is terminated with `"\n"` so it
    ///    stays a permanently skipped line — appended before any closer, so a closer
    ///    never concatenates onto the fragment;
    /// 2. every `toolCall` of the trailing assistant message without a matching
    ///    `toolResult` in the trailing run is closed with an unknown-outcome error
    ///    toolResult;
    /// 3. one [`RESUME_REPAIR_TYPE`] record and a `sync()` follow when anything was
    ///    repaired.
    ///
    /// Repair is idempotent: after the first repair the trailing run is balanced and the
    /// dangling scan finds nothing. In-memory sessions and sessions whose file was never
    /// created have nothing to repair and are left untouched.
    ///
    /// # Errors
    ///
    /// I/O when reading or appending (fail-closed: the caller must not use the session).
    pub fn repair_interrupted_turn(&mut self) -> Result<RepairReport, SessionError> {
        let mut report = RepairReport::default();
        let Some(path) = self
            .session_file
            .clone()
            .filter(|_| self.persist && self.flushed)
        else {
            return Ok(report);
        };
        if torn_tail(&files::read_to_string(&path)?) {
            // `write_line` appends exactly `"\n"` for an empty line.
            self.write_appended(&path, "")?;
            report.torn_tail = true;
        }
        for call in self.dangling_tool_calls() {
            self.append_message(AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                content: vec![UserContent::text(INTERRUPTED_TOOL_TEXT)],
                details: None,
                usage: None,
                nested_calls: None,
                is_error: true,
                timestamp: now_ms(),
                duration_ms: None,
            }))?;
            report.closed_tool_calls.push(call.id);
        }
        if report.torn_tail || !report.closed_tool_calls.is_empty() {
            self.append_custom_entry(
                RESUME_REPAIR_TYPE,
                Some(json!({
                    "closedToolCalls": report.closed_tool_calls,
                    "tornTail": report.torn_tail,
                    "at": now_ms(),
                })),
            )?;
            self.sync()?;
        }
        Ok(report)
    }

    /// Dangling tool calls of the trailing run (D-REPAIR scan): from the end of the leaf
    /// projection, the trailing `toolResult` messages and the assistant message that
    /// issued them — every `toolCall.id` there without a matching `toolResult` in the
    /// trailing run.
    fn dangling_tool_calls(&self) -> Vec<ToolCall> {
        let messages = &self.build_session_projection().messages;
        let mut start = messages.len();
        while start > 0 && matches!(messages[start - 1], AgentMessage::ToolResult(_)) {
            start -= 1;
        }
        let Some(AgentMessage::Assistant(assistant)) =
            start.checked_sub(1).and_then(|index| messages.get(index))
        else {
            return Vec::new();
        };
        let answered: HashSet<&str> = messages[start..]
            .iter()
            .filter_map(|message| match message {
                AgentMessage::ToolResult(result) => Some(result.tool_call_id.as_str()),
                _ => None,
            })
            .collect();
        assistant
            .content
            .iter()
            .filter_map(|block| match block {
                AssistantContent::ToolCall(call) => {
                    (!answered.contains(call.id.as_str())).then(|| call.clone())
                }
                _ => None,
            })
            .collect()
    }
}

/// Whether the file's last line is a torn fragment: non-empty and not valid JSON.
fn torn_tail(text: &str) -> bool {
    match text.split('\n').next_back() {
        Some(last) => !last.is_empty() && serde_json::from_str::<serde_json::Value>(last).is_err(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_torn_tail_only_for_invalid_fragments() {
        // Ends with newline: the trailing fragment is empty.
        assert!(!torn_tail("{}\n"));
        // Valid JSON last line without newline: readable, nothing to repair.
        assert!(!torn_tail("{}\n{\"id\":\"a\"}"));
        // Torn write: non-empty, invalid JSON.
        assert!(torn_tail("{}\n{\"type\":\"mess"));
        assert!(torn_tail("{\"type\":\"mess"));
        assert!(!torn_tail(""));
    }
}
