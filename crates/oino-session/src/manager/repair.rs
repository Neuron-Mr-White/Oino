//! D-DISPATCH / D-REPAIR (core spec §2.1, WP-C2): the tool-dispatch record and the
//! interrupted-turn repair on resume.
//!
//! The dispatch record is one `custom` entry per side-effect tool call, appended before
//! the tool body runs, so a crash between dispatch and result leaves an attributable
//! record. Repair is the resume layer's job and appends closers only: a torn physical
//! tail is terminated with `"\n"` (the reference repair, `session-manager.js:365-367`) so the
//! fragment stays a permanently skipped line, and every dangling tool call of the
//! trailing assistant message is closed with an unknown-outcome error toolResult —
//! exactly a `toolResult` message any v3-format reader understands. Existing bytes are
//! never rewritten or truncated, and open calls are never auto-retried.

use std::collections::HashSet;

use oino_ai::{
    AgentMessage, AssistantContent, JsonObject, ToolCall, ToolResultMessage, UserContent,
};
use serde_json::json;

use super::{SessionManager, files};
use crate::entry::{EntryId, EntryKind};
use crate::error::SessionError;
use crate::time::now_ms;

/// `customType` of the dispatch record (D-DISPATCH).
pub const TOOL_DISPATCH_TYPE: &str = "oino.tool-dispatch";

/// `customType` of the repair record (D-REPAIR).
pub const RESUME_REPAIR_TYPE: &str = "oino.resume-repair";

/// `customType` of the interrupted-turn marker (perf spec §4.2, T13): appended by repair
/// when the leaf turn was cut off (torn tail, closed tool calls, or a turn that started
/// without an end). A `custom` entry — never sent to the model; the app draws it as an
/// "Interrupted — Continue" row and long-horizon modes auto-continue from it.
pub const TURN_INTERRUPTED_TYPE: &str = "oino.turn-interrupted";

/// Durable terminal boundary, including runs intentionally ending on tool results.
const AGENT_END_TYPE: &str = "oino.agent-end";

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
    /// Reason of the [`TURN_INTERRUPTED_TYPE`] marker appended by this repair
    /// (`tool_calls_closed`, `torn_tail` or `turn_unfinished`), `None` when none was.
    pub interrupted: Option<String>,
}

impl SessionManager {
    /// Persist a run boundary before any asynchronous preparation or provider work.
    /// The small registry is a recovery hint; the JSONL remains authoritative.
    ///
    /// # Errors
    /// Returns an error if the durable run registry cannot be read or written.
    pub fn begin_run(&mut self) -> Result<(), SessionError> {
        self.append_custom_entry("oino.run-begun", None)?;
        self.materialize()?;
        if let Some(path) = self.session_file.as_ref().filter(|_| self.persist) {
            let registry = run_registry(path);
            files::write_atomic(&registry, &path.to_string_lossy())?;
            files::sync_dir(&registry)?;
            files::sync_dir(registry.parent().unwrap_or(path))?;
        }
        Ok(())
    }

    /// Close the durable run only after the settle hooks and pending appends finish.
    ///
    /// # Errors
    /// Returns an error if the durable run registry cannot be read or written.
    pub fn end_run(&mut self) -> Result<(), SessionError> {
        self.append_custom_entry("oino.run-ended", None)?;
        self.sync()?;
        self.clear_open_run()
    }

    fn clear_open_run(&self) -> Result<(), SessionError> {
        if let Some(path) = self.session_file.as_ref().filter(|_| self.persist) {
            remove_hint(&run_registry(path))?;
        }
        Ok(())
    }

    /// Durably reserve an accepted prompt before its runtime has finished waking.
    /// The eventual engine begin/end owns the authoritative JSONL boundaries.
    ///
    /// # Errors
    /// Returns an error if the durable run registry cannot be read or written.
    pub fn register_queued_run(path: &std::path::Path) -> Result<String, SessionError> {
        let id = uuid::Uuid::new_v4().to_string();
        let registry = queued_registry(path, &id);
        files::write_atomic(&registry, &path.to_string_lossy())?;
        files::sync_dir(&registry)?;
        files::sync_dir(registry.parent().unwrap_or(path))?;
        Ok(id)
    }

    /// Release only this accepted command's reservation; identical prompts have distinct ids.
    ///
    /// # Errors
    /// Returns an error when removal or directory synchronization fails.
    pub fn release_queued_run(path: &std::path::Path, id: &str) -> Result<(), SessionError> {
        // Only ids minted by register_queued_run are valid file-name components.
        if uuid::Uuid::parse_str(id).is_err() {
            return Ok(());
        }
        remove_hint(&queued_registry(path, id))
    }

    /// Recover only registered open runs, without waking engines or extensions.
    /// Active writers (for example another CLI) are never modified.
    ///
    /// # Errors
    /// Returns an error if the durable run registry cannot be read or written.
    pub fn recover_open_runs(
        sessions_root: &std::path::Path,
    ) -> Result<Vec<std::path::PathBuf>, SessionError> {
        let mut repaired = Vec::new();
        let Ok(projects) = std::fs::read_dir(sessions_root) else {
            return Ok(repaired);
        };
        for project in projects.flatten() {
            let Ok(runs) = std::fs::read_dir(project.path().join(".open-runs")) else {
                continue;
            };
            let mut registries: Vec<_> = runs.flatten().map(|run| run.path()).collect();
            // A pending command survives an older clean run boundary. Recover it first.
            registries.sort_by_key(|path| path.extension().is_none_or(|ext| ext != "queued"));
            let mut recovered = HashSet::new();
            for registry in &registries {
                if registry
                    .extension()
                    .is_none_or(|ext| ext != "run" && ext != "queued")
                {
                    continue;
                }
                // A preceding repair may already have removed a second hint for this run.
                if !registry.exists() {
                    continue;
                }
                let queued = registry.extension().is_some_and(|ext| ext == "queued");
                let path = std::path::PathBuf::from(files::read_to_string(registry)?);
                if !recovered.insert(path.clone()) {
                    remove_hint(registry)?;
                    continue;
                }
                let holder = super::LeaseInfo::for_process("startup-repair", sessions_root);
                let Ok(_lease) = super::SessionLease::acquire(&path, &holder) else {
                    recovered.remove(&path);
                    continue;
                };
                let mut manager = Self::open(&path, sessions_root, None)?;
                if queued && !manager.turn_unfinished() {
                    manager.begin_run()?;
                }
                let report = manager.repair_interrupted_turn()?;
                if report.interrupted.is_some() {
                    repaired.push(path);
                }
                manager.clear_open_run()?;
                remove_hint(registry)?;
            }
        }
        Ok(repaired)
    }

    /// Appends the [`TOOL_DISPATCH_TYPE`] record of a tool call (D-DISPATCH): a `custom`
    /// entry naming the call before its body runs, so resume repair can attribute an
    /// interrupted effect. `custom` entries never project into context
    /// (`session-manager.js:164-181`) — inert to v3-format readers.
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
    /// 3. one [`RESUME_REPAIR_TYPE`] record follows when anything was repaired;
    /// 4. one [`TURN_INTERRUPTED_TYPE`] marker (`{reason, at}`) follows when the leaf turn
    ///    was cut off — steps 1–2 repaired something, or the branch's last turn message is
    ///    a user prompt, tool result or assistant tool call without a final answer or
    ///    durable agent-end boundary — unless the branch already
    ///    carries a marker after its last message; then a `sync()`.
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
        if file_torn_tail(&path)? {
            // `write_line` appends exactly `"\n"` for an empty line.
            self.write_appended(&path, "")?;
            report.torn_tail = true;
        }
        let unfinished = self.turn_unfinished();
        let dangling = if unfinished {
            self.dangling_tool_calls()
        } else {
            Vec::new()
        };
        for call in dangling {
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
        }
        let reason = if !report.closed_tool_calls.is_empty() {
            Some("tool_calls_closed")
        } else if report.torn_tail && unfinished {
            Some("torn_tail")
        } else if unfinished {
            Some("turn_unfinished")
        } else {
            None
        };
        if let Some(reason) = reason.filter(|_| !self.interrupt_marked()) {
            self.append_custom_entry(
                TURN_INTERRUPTED_TYPE,
                Some(json!({"reason": reason, "at": now_ms()})),
            )?;
            report.interrupted = Some(reason.to_owned());
        }
        if report.torn_tail || !report.closed_tool_calls.is_empty() || report.interrupted.is_some()
        {
            self.sync()?;
        }
        if report.interrupted.is_some() {
            self.clear_open_run()?;
        }
        Ok(report)
    }

    /// Inspect durable branch entries, not the projection: compaction can insert a
    /// synthetic summary assistant which is not a final answer to the current turn.
    /// A tool loop remains in-flight until a final answer or an agent-end boundary.
    fn turn_unfinished(&self) -> bool {
        // Explicit lifecycle records supersede message-shape inference, including
        // final-looking assistant messages followed by hooks or compaction.
        let mut newer_message = false;
        for entry in self.branch(None).into_iter().rev() {
            if matches!(entry.kind, EntryKind::Message { .. }) {
                newer_message = true;
            }
            if let EntryKind::Custom { custom_type, .. } = &entry.kind {
                match custom_type.as_str() {
                    "oino.run-begun" => return true,
                    "oino.run-ended" | TURN_INTERRUPTED_TYPE => {
                        if !newer_message {
                            return false;
                        }
                        break; // A legacy writer may append without lifecycle records.
                    }
                    _ => {}
                }
            }
        }
        for entry in self.branch(None).into_iter().rev() {
            match &entry.kind {
                EntryKind::Custom { custom_type, .. } if custom_type == AGENT_END_TYPE => {
                    return false;
                }
                EntryKind::Message { message } => match message {
                    AgentMessage::User(_) | AgentMessage::ToolResult(_) => return true,
                    AgentMessage::Assistant(assistant) => {
                        return assistant
                            .content
                            .iter()
                            .any(|c| matches!(c, AssistantContent::ToolCall(_)));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        false
    }

    /// A [`TURN_INTERRUPTED_TYPE`] marker sits after the branch's last message (repair is
    /// idempotent: re-opening an unanswered chat never stacks markers).
    fn interrupt_marked(&self) -> bool {
        for entry in self.branch(None).into_iter().rev() {
            match &entry.kind {
                EntryKind::Custom { custom_type, .. } if custom_type == TURN_INTERRUPTED_TYPE => {
                    return true;
                }
                EntryKind::Custom { custom_type, .. } if custom_type == "oino.run-begun" => {
                    return false;
                }
                EntryKind::Message { .. } => return false,
                _ => {}
            }
        }
        false
    }

    /// Dangling tool calls of the trailing run (D-REPAIR scan): from the end of the leaf
    /// projection, the trailing `toolResult` messages and the assistant message that
    /// issued them — every `toolCall.id` there without a matching `toolResult` in the
    /// trailing run.
    fn dangling_tool_calls(&self) -> Vec<ToolCall> {
        let branch = self.branch(None);
        let messages: Vec<_> = branch
            .iter()
            .filter_map(|entry| match &entry.kind {
                EntryKind::Message { message }
                    if matches!(
                        message,
                        AgentMessage::User(_)
                            | AgentMessage::Assistant(_)
                            | AgentMessage::ToolResult(_)
                    ) =>
                {
                    Some(message)
                }
                _ => None,
            })
            .collect();
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

fn queued_registry(path: &std::path::Path, id: &str) -> std::path::PathBuf {
    let registry = run_registry(path);
    registry.with_file_name(format!(
        "{}.{}.queued",
        path.file_name().unwrap_or_default().to_string_lossy(),
        id,
    ))
}

fn remove_hint(path: &std::path::Path) -> Result<(), SessionError> {
    match std::fs::remove_file(path) {
        Ok(()) => files::sync_dir(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SessionError::io(path, error)),
    }
}

fn run_registry(path: &std::path::Path) -> std::path::PathBuf {
    path.parent()
        .unwrap_or(std::path::Path::new("."))
        .join(".open-runs")
        .join(format!(
            "{}.run",
            path.file_name().unwrap_or_default().to_string_lossy()
        ))
}

// Almost every session ends in a newline. Do not read a 72 MB transcript merely
// to discover that its last byte is intact. Grow backwards only for a torn tail.
fn file_torn_tail(path: &std::path::Path) -> Result<bool, SessionError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).map_err(|e| SessionError::io(path, e))?;
    let len = file
        .metadata()
        .map_err(|e| SessionError::io(path, e))?
        .len();
    let mut size = len.min(4096);
    loop {
        file.seek(SeekFrom::Start(len - size))
            .map_err(|e| SessionError::io(path, e))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)
            .map_err(|e| SessionError::io(path, e))?;
        if let Some(at) = tail.iter().rposition(|b| *b == b'\n') {
            return Ok(torn_tail(&String::from_utf8_lossy(&tail[at + 1..])));
        }
        if size == len {
            return Ok(torn_tail(&String::from_utf8_lossy(&tail)));
        }
        size = (size * 2).min(len);
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
