//! Conversation rows (gpui-app §5.2).
//!
//! Rows have stable ids (a sequence id for messages, the tool-call id for tool rows) and a
//! version counter that bumps on every change, so a virtualised list re-measures only the rows
//! a batch of events touched. Streaming patches the tail assistant row; tool rows are created
//! from `toolcall` content while the model is still emitting arguments, then advance through
//! `tool_execution_*`.

use std::collections::HashMap;
use std::time::Instant;

/// Unix ms (0 on clock failure; only display labels depend on it).
fn wall_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

use oino_agent::AgentToolResult;
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, JsonObject, JsonValue, StopReason,
    UserContent, UserContentField,
};
use oino_core::events::AgentSessionEvent;

use crate::model::tools;

/// Cap on retained tool output per row (the full output stays in the session file).
pub const MAX_TOOL_OUTPUT: usize = 64 * 1024;

/// Lifecycle of a tool row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    /// Arguments are still streaming.
    Preparing,
    /// Executing.
    Running,
    /// Finished successfully.
    Done,
    /// Finished with `isError`.
    Error,
}

/// A user prompt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserRow {
    /// Text.
    pub text: String,
    /// Number of attached images.
    pub images: usize,
}

/// An assistant message (text + thinking; tool calls are separate rows).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssistantRow {
    /// Visible text.
    pub text: String,
    /// Thinking text (collapsed by default).
    pub thinking: String,
    /// Wall time of the thinking phase once it is over (the label reads "Thought for 12.3s");
    /// `None` for history rows loaded from a session file, which carry no timing.
    pub thinking_ms: Option<u64>,
    /// Creation wall time (Unix ms), stamped when the live message starts; `None` for
    /// history rows (`AgentMessage` carries no timestamps). Drives the hover-bar time label
    /// ([`ZCode`] `messageTimeLabel`).
    pub created_ms: Option<i64>,
    /// Still streaming.
    pub streaming: bool,
    /// Error / abort text.
    pub error: Option<String>,
    /// `provider/model` that produced it.
    pub model: String,
}

/// A tool call.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRow {
    /// Tool-call id.
    pub call_id: String,
    /// Tool name.
    pub name: String,
    /// Arguments (partial while preparing).
    pub args: JsonObject,
    /// Status.
    pub status: ToolStatus,
    /// Latest result text (capped at [`MAX_TOOL_OUTPUT`], tail kept).
    pub output: String,
    /// Tool details (edit: `patch`).
    pub details: Option<JsonValue>,
    /// Wall time.
    pub duration_ms: Option<u64>,
    /// Calling tool for nested calls.
    pub parent: Option<String>,
    /// Expanded in the transcript.
    pub expanded: bool,
}

impl ToolRow {
    fn new(call_id: String, name: String, args: JsonObject, status: ToolStatus) -> Self {
        Self {
            call_id,
            name,
            args,
            status,
            output: String::new(),
            details: None,
            duration_ms: None,
            parent: None,
            expanded: false,
        }
    }

    /// One-line title.
    #[must_use]
    pub fn title(&self) -> String {
        tools::title(&self.name, &self.args)
    }

    /// Unified patch for edit results.
    #[must_use]
    pub fn patch(&self) -> Option<String> {
        tools::patch_of(self.details.as_ref())
    }

    fn set_output(&mut self, result: &AgentToolResult) {
        let mut text = tools::result_text(&result.content);
        if text.len() > MAX_TOOL_OUTPUT {
            let mut cut = text.len() - MAX_TOOL_OUTPUT;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text.drain(..cut);
        }
        self.output = text;
        if result.details.is_some() {
            self.details.clone_from(&result.details);
        }
    }
}

/// A `!` command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BashRow {
    /// Command line.
    pub command: String,
    /// Output.
    pub output: String,
    /// Exit code.
    pub exit_code: Option<i32>,
}

/// An extension (`custom`) message with `display: true`.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomRow {
    /// `customType`.
    pub custom_type: String,
    /// Text content.
    pub text: String,
    /// Details (provenance, renderer data).
    pub details: Option<JsonValue>,
}

impl CustomRow {
    /// Provenance label (`details.oinoProvenance.source`) when present (ui-protocol §6.4).
    #[must_use]
    pub fn provenance(&self) -> Option<&str> {
        let provenance = self.details.as_ref()?.get("oinoProvenance")?;
        provenance
            .get("source")
            .and_then(JsonValue::as_str)
            .or_else(|| provenance.as_str())
    }
}

/// Summary card kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryKind {
    /// Compaction summary.
    Compaction,
    /// Branch summary.
    Branch,
}

/// Compaction / branch summary card.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryRow {
    /// Kind.
    pub kind: SummaryKind,
    /// Summary text.
    pub summary: String,
    /// Tokens before compaction.
    pub tokens_before: u64,
    /// Expanded.
    pub expanded: bool,
}

/// Tone of a notice row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    /// Neutral.
    Info,
    /// Warning (retries).
    Warning,
    /// Error.
    Error,
}

/// Row payload.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    /// User prompt.
    User(UserRow),
    /// Assistant message.
    Assistant(AssistantRow),
    /// Tool call.
    Tool(Box<ToolRow>),
    /// `!` command.
    Bash(BashRow),
    /// Extension message.
    Custom(CustomRow),
    /// Summary card.
    Summary(SummaryRow),
    /// Status notice (compaction, retry, errors).
    Notice(NoticeTone, String),
}

/// One transcript row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Stable id.
    pub id: String,
    /// Bumped on every change (render caches key on `(id, version)`).
    pub version: u64,
    /// Payload.
    pub kind: RowKind,
}

/// What a reducer step changed, for minimal list updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowChange {
    /// `count` rows inserted at `at`.
    Inserted {
        /// First new index.
        at: usize,
        /// Number of rows.
        count: usize,
    },
    /// Row `at` changed in place.
    Updated {
        /// Index.
        at: usize,
    },
    /// Everything changed.
    Reset,
}

/// The transcript of one session.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    rows: Vec<Row>,
    index: HashMap<String, usize>,
    seq: u64,
    streaming: Option<usize>,
    /// When the live thinking phase of the streaming assistant message started; resolved into
    /// [`AssistantRow::thinking_ms`] at the first non-thinking delta or at `message_end`.
    thinking_at: Option<Instant>,
}

fn user_text(content: &UserContentField) -> (String, usize) {
    match content {
        UserContentField::Text(text) => (text.clone(), 0),
        UserContentField::Blocks(blocks) => {
            let mut text = String::new();
            let mut images = 0;
            for block in blocks {
                match block {
                    UserContent::Text(t) => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&t.text);
                    }
                    UserContent::Image(_) => images += 1,
                }
            }
            (text, images)
        }
    }
}

fn assistant_parts(message: &AssistantMessage) -> (String, String) {
    let mut text = String::new();
    let mut thinking = String::new();
    for block in &message.content {
        match block {
            AssistantContent::Text(t) => {
                if !text.is_empty() && !t.text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&t.text);
            }
            AssistantContent::Thinking(t) => {
                if !thinking.is_empty() {
                    thinking.push_str("\n\n");
                }
                thinking.push_str(&t.thinking);
            }
            AssistantContent::ToolCall(_) => {}
        }
    }
    (text, thinking)
}

fn assistant_error(message: &AssistantMessage) -> Option<String> {
    match message.stop_reason {
        StopReason::Error => Some(
            message
                .error_message
                .clone()
                .unwrap_or_else(|| "Request failed".to_owned()),
        ),
        StopReason::Aborted => Some(
            message
                .error_message
                .clone()
                .unwrap_or_else(|| "Aborted".to_owned()),
        ),
        _ => None,
    }
}

impl Transcript {
    /// An empty transcript.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rows in display order.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Row> {
        self.index.get(id).map(|&i| &self.rows[i])
    }

    /// Index of a row id.
    #[must_use]
    pub fn position(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// Indices of user rows (the query rail jumps between prompts).
    #[must_use]
    pub fn prompt_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.kind, RowKind::User(_)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether an assistant row is streaming.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}{}", self.seq)
    }

    fn push(&mut self, id: String, kind: RowKind) -> RowChange {
        let at = self.rows.len();
        self.index.insert(id.clone(), at);
        self.rows.push(Row {
            id,
            version: 0,
            kind,
        });
        RowChange::Inserted { at, count: 1 }
    }

    fn touch(&mut self, at: usize) -> RowChange {
        self.rows[at].version += 1;
        RowChange::Updated { at }
    }

    fn tool_mut(&mut self, call_id: &str) -> Option<(usize, &mut ToolRow)> {
        let at = *self.index.get(call_id)?;
        match &mut self.rows[at].kind {
            RowKind::Tool(tool) => Some((at, tool)),
            _ => None,
        }
    }

    /// Clears every row.
    pub fn clear(&mut self) -> RowChange {
        self.rows.clear();
        self.index.clear();
        self.streaming = None;
        RowChange::Reset
    }

    /// Rebuilds from a full message list (`get_messages`, session switch).
    pub fn load(&mut self, messages: &[AgentMessage]) -> RowChange {
        self.clear();
        for message in messages {
            self.push_message(message, false);
        }
        // QA-3: history has no run events, so tool calls that never got a result (the app
        // quit before the tool ran, e.g. an approval that was never answered) must be
        // settled here, not counted as successful edits in the turn summary.
        let _ = self.settle_preparing();
        RowChange::Reset
    }

    /// Toggles the expanded state of a tool or summary row.
    pub fn toggle_expanded(&mut self, id: &str) -> Option<RowChange> {
        let at = *self.index.get(id)?;
        match &mut self.rows[at].kind {
            RowKind::Tool(tool) => tool.expanded = !tool.expanded,
            RowKind::Summary(summary) => summary.expanded = !summary.expanded,
            _ => return None,
        }
        Some(self.touch(at))
    }

    /// Appends a notice row (frontend-originated errors, e.g. a failed command).
    pub fn notice(&mut self, tone: NoticeTone, text: impl Into<String>) -> RowChange {
        let id = self.next_id("n");
        self.push(id, RowKind::Notice(tone, text.into()))
    }

    /// Adds rows for one complete message (history or `message_start`).
    fn push_message(&mut self, message: &AgentMessage, streaming: bool) -> Vec<RowChange> {
        let mut changes = Vec::new();
        match message {
            AgentMessage::User(user) => {
                let (text, images) = user_text(&user.content);
                let id = self.next_id("u");
                changes.push(self.push(id, RowKind::User(UserRow { text, images })));
            }
            AgentMessage::Assistant(assistant) => {
                let (text, thinking) = assistant_parts(assistant);
                let id = self.next_id("a");
                let row = AssistantRow {
                    text,
                    thinking,
                    thinking_ms: None,
                    // ZCode `messageTimeLabel`: the hover bar shows when the answer arrived;
                    // history rows carry no timestamp and stay quiet.
                    created_ms: streaming.then(wall_ms),
                    streaming,
                    error: if streaming {
                        None
                    } else {
                        assistant_error(assistant)
                    },
                    model: format!("{}/{}", assistant.provider, assistant.model),
                };
                let change = self.push(id, RowKind::Assistant(row));
                if streaming {
                    self.streaming = Some(self.rows.len() - 1);
                    self.thinking_at = None;
                }
                changes.push(change);
                changes.extend(self.sync_tool_calls(assistant));
            }
            AgentMessage::ToolResult(result) => {
                let tool_result = AgentToolResult {
                    content: result.content.clone(),
                    details: result.details.clone(),
                    ..AgentToolResult::default()
                };
                if let Some((at, tool)) = self.tool_mut(&result.tool_call_id) {
                    tool.set_output(&tool_result);
                    tool.status = if result.is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Done
                    };
                    tool.duration_ms = result.duration_ms;
                    changes.push(self.touch(at));
                }
            }
            AgentMessage::BashExecution(bash) => {
                let id = self.next_id("b");
                changes.push(self.push(
                    id,
                    RowKind::Bash(BashRow {
                        command: bash.command.clone(),
                        output: bash.output.clone(),
                        exit_code: bash.exit_code,
                    }),
                ));
            }
            AgentMessage::Custom(custom) if custom.display => {
                let (text, _) = user_text(&custom.content);
                let id = self.next_id("c");
                changes.push(self.push(
                    id,
                    RowKind::Custom(CustomRow {
                        custom_type: custom.custom_type.clone(),
                        text,
                        details: custom.details.clone(),
                    }),
                ));
            }
            AgentMessage::CompactionSummary(summary) => {
                let id = self.next_id("s");
                changes.push(self.push(
                    id,
                    RowKind::Summary(SummaryRow {
                        kind: SummaryKind::Compaction,
                        summary: summary.summary.clone(),
                        tokens_before: summary.tokens_before,
                        expanded: false,
                    }),
                ));
            }
            AgentMessage::BranchSummary(summary) => {
                let id = self.next_id("s");
                changes.push(self.push(
                    id,
                    RowKind::Summary(SummaryRow {
                        kind: SummaryKind::Branch,
                        summary: summary.summary.clone(),
                        tokens_before: 0,
                        expanded: false,
                    }),
                ));
            }
            AgentMessage::Custom(_) | AgentMessage::System(_) => {}
        }
        changes
    }

    /// Creates tool rows for tool calls in an assistant message (streaming preview) and
    /// refreshes their arguments while they are still preparing.
    fn sync_tool_calls(&mut self, message: &AssistantMessage) -> Vec<RowChange> {
        let mut changes = Vec::new();
        for block in &message.content {
            let AssistantContent::ToolCall(call) = block else {
                continue;
            };
            if call.id.is_empty() {
                continue;
            }
            if let Some((at, tool)) = self.tool_mut(&call.id) {
                if tool.status == ToolStatus::Preparing && tool.args != call.arguments {
                    tool.args.clone_from(&call.arguments);
                    changes.push(self.touch(at));
                }
            } else {
                // Always starts as Preparing: a result event (live) or `settle_preparing`
                // (history load, QA-3) decides the final state. Marking a result-less call
                // `Done` here made the turn summary claim edits that never happened.
                let row = ToolRow::new(
                    call.id.clone(),
                    call.name.clone(),
                    call.arguments.clone(),
                    ToolStatus::Preparing,
                );
                changes.push(self.push(call.id.clone(), RowKind::Tool(Box::new(row))));
            }
        }
        changes
    }

    /// Applies one session event; returns the changed rows.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per event kind reads better than helpers per arm"
    )]
    pub fn apply(&mut self, event: &AgentSessionEvent) -> Vec<RowChange> {
        match event {
            AgentSessionEvent::MessageStart { message } => match message {
                AgentMessage::Assistant(_) => self.push_message(message, true),
                AgentMessage::ToolResult(_) => Vec::new(),
                _ => self.push_message(message, false),
            },
            AgentSessionEvent::MessageUpdate { message, .. } => {
                let AgentMessage::Assistant(assistant) = message else {
                    return Vec::new();
                };
                let Some(at) = self.streaming else {
                    return self.push_message(message, true);
                };
                let (text, thinking) = assistant_parts(assistant);
                let mut changes = Vec::new();
                // The thinking phase starts at the first thinking delta and ends at the first
                // visible answer text ("Thought for 12.3s", design.md §4.9). Both are decided
                // from state read before the row borrow.
                let started = self.thinking_at;
                let mut start_thinking = false;
                let mut done_thinking = None;
                if let RowKind::Assistant(row) = &mut self.rows[at].kind
                    && (row.text != text || row.thinking != thinking)
                {
                    let first_text = row.text.is_empty() && !text.is_empty();
                    row.text = text;
                    row.thinking = thinking;
                    start_thinking = started.is_none()
                        && row.thinking_ms.is_none()
                        && !row.thinking.trim().is_empty();
                    if first_text && row.thinking_ms.is_none() {
                        done_thinking = started.map(|s| s.elapsed());
                    }
                    changes.push(self.touch(at));
                }
                if start_thinking {
                    self.thinking_at = Some(Instant::now());
                }
                if let Some(elapsed) = done_thinking {
                    self.thinking_at = None;
                    if let RowKind::Assistant(row) = &mut self.rows[at].kind {
                        row.thinking_ms =
                            Some(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
                    }
                }
                changes.extend(self.sync_tool_calls(assistant));
                changes
            }
            AgentSessionEvent::MessageEnd { message } => {
                let AgentMessage::Assistant(assistant) = message else {
                    return Vec::new();
                };
                let Some(at) = self.streaming.take() else {
                    return self.push_message(message, false);
                };
                let (text, thinking) = assistant_parts(assistant);
                if let RowKind::Assistant(row) = &mut self.rows[at].kind {
                    row.text = text;
                    row.thinking = thinking;
                    row.streaming = false;
                    row.error = assistant_error(assistant);
                    if row.thinking_ms.is_none()
                        && !row.thinking.trim().is_empty()
                        && let Some(started) = self.thinking_at.take()
                    {
                        row.thinking_ms =
                            Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
                    }
                }
                self.thinking_at = None;
                let mut changes = vec![self.touch(at)];
                changes.extend(self.sync_tool_calls(assistant));
                changes
            }
            AgentSessionEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
                parent_tool_call_id,
            } => {
                if let Some((at, tool)) = self.tool_mut(tool_call_id) {
                    tool.status = ToolStatus::Running;
                    tool.args.clone_from(args);
                    tool.parent.clone_from(parent_tool_call_id);
                    vec![self.touch(at)]
                } else {
                    let mut row = ToolRow::new(
                        tool_call_id.clone(),
                        tool_name.clone(),
                        args.clone(),
                        ToolStatus::Running,
                    );
                    row.parent.clone_from(parent_tool_call_id);
                    vec![self.push(tool_call_id.clone(), RowKind::Tool(Box::new(row)))]
                }
            }
            AgentSessionEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => match self.tool_mut(tool_call_id) {
                Some((at, tool)) => {
                    tool.set_output(partial_result);
                    vec![self.touch(at)]
                }
                None => Vec::new(),
            },
            AgentSessionEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
                duration_ms,
                parent_tool_call_id,
            } => {
                if self.tool_mut(tool_call_id).is_none() {
                    let row = ToolRow::new(
                        tool_call_id.clone(),
                        tool_name.clone(),
                        JsonObject::new(),
                        ToolStatus::Running,
                    );
                    self.push(tool_call_id.clone(), RowKind::Tool(Box::new(row)));
                }
                let Some((at, tool)) = self.tool_mut(tool_call_id) else {
                    return Vec::new();
                };
                tool.set_output(result);
                tool.status = if *is_error {
                    ToolStatus::Error
                } else {
                    ToolStatus::Done
                };
                tool.duration_ms = *duration_ms;
                tool.parent.clone_from(parent_tool_call_id);
                vec![self.touch(at)]
            }
            AgentSessionEvent::CompactionEnd {
                error_message: Some(error),
                aborted: false,
                ..
            } => {
                // Core and the compactor may already prefix the reason (GA-9).
                let mut reason = error.trim();
                while let Some(rest) = reason.strip_prefix("Compaction failed:") {
                    reason = rest.trim_start();
                }
                let tone = if reason.to_lowercase().starts_with("nothing to compact") {
                    NoticeTone::Info
                } else {
                    NoticeTone::Error
                };
                vec![self.notice(tone, format!("Compaction failed: {reason}"))]
            }
            AgentSessionEvent::AutoRetryStart {
                attempt,
                max_attempts,
                delay_ms,
                error_message,
            } => vec![self.notice(
                NoticeTone::Warning,
                format!(
                    "Retrying ({attempt}/{max_attempts}) in {}.{}s: {error_message}",
                    delay_ms / 1000,
                    (delay_ms % 1000) / 100
                ),
            )],
            AgentSessionEvent::AutoRetryEnd {
                success: false,
                final_error: Some(error),
                ..
            } => vec![self.notice(NoticeTone::Error, error.clone())],
            AgentSessionEvent::AgentEnd { .. } | AgentSessionEvent::AgentSettled { .. } => {
                // A run can end without `message_end` (abort before the first token).
                match self.streaming.take() {
                    Some(at) => {
                        if let RowKind::Assistant(row) = &mut self.rows[at].kind {
                            row.streaming = false;
                        }
                        let mut changes = vec![self.touch(at)];
                        changes.extend(self.settle_preparing());
                        changes
                    }
                    None => self.settle_preparing(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// Tool rows still `Preparing` when a run ends never executed (blocked or aborted).
    fn settle_preparing(&mut self) -> Vec<RowChange> {
        let stale: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                matches!(&r.kind, RowKind::Tool(t) if matches!(t.status, ToolStatus::Preparing | ToolStatus::Running))
            })
            .map(|(i, _)| i)
            .collect();
        stale
            .into_iter()
            .map(|at| {
                if let RowKind::Tool(tool) = &mut self.rows[at].kind {
                    tool.status = ToolStatus::Error;
                    if tool.output.is_empty() {
                        "Not executed".clone_into(&mut tool.output);
                    }
                }
                self.touch(at)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(value: JsonValue) -> AgentMessage {
        serde_json::from_value(value).unwrap()
    }

    fn assistant(content: JsonValue, stop: &str) -> AgentMessage {
        msg(json!({
            "role": "assistant", "content": content, "api": "x", "provider": "p", "model": "m",
            "usage": {"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                      "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason": stop, "timestamp": 0
        }))
    }

    fn result(text: &str) -> AgentToolResult {
        AgentToolResult {
            content: vec![UserContent::text(text)],
            ..AgentToolResult::default()
        }
    }

    fn update(message: AgentMessage) -> AgentSessionEvent {
        let partial = serde_json::to_value(&message).unwrap();
        AgentSessionEvent::MessageUpdate {
            message,
            assistant_message_event: serde_json::from_value(
                json!({"type":"start","partial": partial}),
            )
            .unwrap(),
        }
    }

    #[test]
    fn streaming_patches_only_the_tail_row() {
        let mut t = Transcript::new();
        let changes = t.apply(&AgentSessionEvent::MessageStart {
            message: msg(json!({"role":"user","content":"hi","timestamp":0})),
        });
        assert_eq!(changes, vec![RowChange::Inserted { at: 0, count: 1 }]);
        t.apply(&AgentSessionEvent::MessageStart {
            message: assistant(json!([]), "stop"),
        });
        assert!(t.is_streaming());
        let changes = t.apply(&update(assistant(
            json!([{"type":"text","text":"Hel"}]),
            "stop",
        )));
        assert_eq!(changes, vec![RowChange::Updated { at: 1 }]);
        // Same cumulative text: nothing to redraw.
        assert!(
            t.apply(&update(assistant(
                json!([{"type":"text","text":"Hel"}]),
                "stop"
            )))
            .is_empty()
        );
        t.apply(&AgentSessionEvent::MessageEnd {
            message: assistant(json!([{"type":"text","text":"Hello"}]), "stop"),
        });
        assert!(!t.is_streaming());
        let RowKind::Assistant(row) = &t.rows()[1].kind else {
            panic!()
        };
        assert_eq!(row.text, "Hello");
        assert!(!row.streaming);
        assert_eq!(t.rows()[1].version, 2);
        assert_eq!(t.prompt_indices(), vec![0]);
    }

    #[test]
    fn tool_rows_follow_preview_then_execution() {
        let mut t = Transcript::new();
        t.apply(&AgentSessionEvent::MessageStart {
            message: assistant(json!([]), "stop"),
        });
        let call = json!([{"type":"toolCall","id":"c1","name":"edit","arguments":{"path":"a.rs"}}]);
        let changes = t.apply(&update(assistant(call.clone(), "toolUse")));
        assert_eq!(changes, vec![RowChange::Inserted { at: 1, count: 1 }]);
        let tool = |t: &Transcript| match &t.get("c1").unwrap().kind {
            RowKind::Tool(tool) => (**tool).clone(),
            _ => panic!(),
        };
        assert_eq!(tool(&t).status, ToolStatus::Preparing);
        assert_eq!(tool(&t).title(), "Edit a.rs");
        t.apply(&AgentSessionEvent::MessageEnd {
            message: assistant(call, "toolUse"),
        });
        t.apply(&AgentSessionEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "edit".into(),
            args: serde_json::from_value(json!({"path":"a.rs"})).unwrap(),
            parent_tool_call_id: None,
        });
        assert_eq!(tool(&t).status, ToolStatus::Running);
        let mut done = result("ok");
        done.details = Some(json!({"patch":"@@ -1 +1 @@\n-a\n+b\n"}));
        let changes = t.apply(&AgentSessionEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "edit".into(),
            result: done,
            is_error: false,
            duration_ms: Some(5),
            parent_tool_call_id: None,
        });
        assert_eq!(changes, vec![RowChange::Updated { at: 1 }]);
        let row = tool(&t);
        assert_eq!(row.status, ToolStatus::Done);
        assert_eq!(row.output, "ok");
        assert!(row.patch().unwrap().contains("+b"));
        assert_eq!(t.toggle_expanded("c1"), Some(RowChange::Updated { at: 1 }));
        assert!(tool(&t).expanded);
    }

    #[test]
    fn unexecuted_calls_settle_as_errors() {
        let mut t = Transcript::new();
        t.apply(&AgentSessionEvent::MessageStart {
            message: assistant(json!([]), "stop"),
        });
        t.apply(&update(assistant(
            json!([{"type":"toolCall","id":"c9","name":"bash","arguments":{"command":"ls"}}]),
            "toolUse",
        )));
        t.apply(&AgentSessionEvent::AgentSettled { aborted: true });
        let RowKind::Tool(tool) = &t.get("c9").unwrap().kind else {
            panic!()
        };
        assert_eq!(tool.status, ToolStatus::Error);
        assert!(!t.is_streaming());
    }

    #[test]
    fn loads_history_and_merges_results() {
        let mut t = Transcript::new();
        let messages = vec![
            msg(
                json!({"role":"user","content":[{"type":"text","text":"do"},{"type":"image","data":"AA","mimeType":"image/png"}],"timestamp":0}),
            ),
            assistant(
                json!([{"type":"text","text":"ok"},{"type":"toolCall","id":"t1","name":"bash","arguments":{"command":"ls"}}]),
                "toolUse",
            ),
            msg(
                json!({"role":"toolResult","toolCallId":"t1","toolName":"bash","content":[{"type":"text","text":"a\nb"}],"isError":true,"timestamp":0}),
            ),
            msg(
                json!({"role":"custom","customType":"note","content":"shown","display":true,"details":{"oinoProvenance":{"source":"workflow"}},"timestamp":0}),
            ),
            msg(
                json!({"role":"custom","customType":"hidden","content":"x","display":false,"timestamp":0}),
            ),
            msg(
                json!({"role":"compactionSummary","summary":"sum","tokensBefore":10,"timestamp":0}),
            ),
        ];
        assert_eq!(t.load(&messages), RowChange::Reset);
        assert_eq!(t.len(), 5);
        let RowKind::User(user) = &t.rows()[0].kind else {
            panic!()
        };
        assert_eq!((user.text.as_str(), user.images), ("do", 1));
        let RowKind::Tool(tool) = &t.get("t1").unwrap().kind else {
            panic!()
        };
        assert_eq!(tool.status, ToolStatus::Error);
        assert_eq!(tools::tail(&tool.output, 1), vec!["b"]);
        let RowKind::Custom(custom) = &t.rows()[3].kind else {
            panic!()
        };
        assert_eq!(custom.provenance(), Some("workflow"));
        assert!(matches!(t.rows()[4].kind, RowKind::Summary(_)));
    }

    #[test]
    fn errors_and_retries_become_notices() {
        let mut t = Transcript::new();
        t.apply(&AgentSessionEvent::MessageStart {
            message: assistant(json!([]), "stop"),
        });
        let mut failed = assistant(json!([]), "error");
        if let AgentMessage::Assistant(a) = &mut failed {
            a.error_message = Some("429".into());
        }
        t.apply(&AgentSessionEvent::MessageEnd { message: failed });
        let RowKind::Assistant(row) = &t.rows()[0].kind else {
            panic!()
        };
        assert_eq!(row.error.as_deref(), Some("429"));
        let changes = t.apply(&AgentSessionEvent::AutoRetryStart {
            attempt: 1,
            max_attempts: 3,
            delay_ms: 2000,
            error_message: "429".into(),
        });
        assert_eq!(changes, vec![RowChange::Inserted { at: 1, count: 1 }]);
        let RowKind::Notice(NoticeTone::Warning, text) = &t.rows()[1].kind else {
            panic!()
        };
        assert_eq!(text, "Retrying (1/3) in 2.0s: 429");
    }

    #[test]
    fn huge_outputs_keep_the_tail() {
        let mut t = Transcript::new();
        t.apply(&AgentSessionEvent::ToolExecutionStart {
            tool_call_id: "b".into(),
            tool_name: "bash".into(),
            args: JsonObject::new(),
            parent_tool_call_id: None,
        });
        let big = format!("{}END", "é".repeat(MAX_TOOL_OUTPUT));
        t.apply(&AgentSessionEvent::ToolExecutionUpdate {
            tool_call_id: "b".into(),
            tool_name: "bash".into(),
            args: JsonObject::new(),
            partial_result: result(&big),
            parent_tool_call_id: None,
        });
        let RowKind::Tool(tool) = &t.get("b").unwrap().kind else {
            panic!()
        };
        assert!(tool.output.len() <= MAX_TOOL_OUTPUT);
        assert!(tool.output.ends_with("END"));
    }

    /// QA-3: a tool call whose result never arrived (the app quit before the tool ran, e.g.
    /// an unanswered approval) loads as an error, and the turn summary must not count it as
    /// an applied change.
    #[test]
    fn resultless_tool_calls_load_as_errors_and_never_count_as_changes() {
        let mut t = Transcript::new();
        let messages = vec![
            msg(json!({"role":"user","content":[{"type":"text","text":"write it"}],"timestamp":0})),
            assistant(
                json!([{"type":"toolCall","id":"w1","name":"write","arguments":{"path":"qa-note2.md","content":"hi\n"}}]),
                "toolUse",
            ),
        ];
        assert_eq!(t.load(&messages), RowChange::Reset);
        let RowKind::Tool(tool) = &t.get("w1").unwrap().kind else {
            panic!()
        };
        assert_eq!(tool.status, ToolStatus::Error);
        assert!(!tool.output.is_empty());
        assert!(crate::model::timeline::file_changes(t.rows()).is_empty());
    }
}
