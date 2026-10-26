//! Transcript rows (pi `interactive-mode.ts` chat container).
//!
//! [`Transcript`] mirrors what pi's interactive mode puts into its chat container: user
//! messages (with skill blocks split out), assistant messages (text + thinking), one row per
//! tool call (args stream in, results attach by tool-call id), `!` command runs, custom
//! messages, custom entries, branch/compaction summaries and notices. Rows carry a revision
//! counter so frontends can cache rendered output.

use std::collections::{HashMap, HashSet};

use oino_agent::AgentToolResult;
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, ImageContent, JsonObject, JsonValue,
    StopReason, UserContent, UserContentField,
};
use oino_core::events::AgentSessionEvent;
use oino_session::{EntryId, EntryIndex, EntryKind, SessionEntry};

/// Stable id of a row inside one [`Transcript`].
pub type RowId = u64;

/// One transcript row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Stable id.
    pub id: RowId,
    /// Bumped on every change (render cache key).
    pub rev: u64,
    /// Content.
    pub kind: RowKind,
}

/// Row content.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    /// User message.
    User(UserRow),
    /// Skill invocation block (`<skill name=… location=…>`), collapsible.
    Skill(SkillRow),
    /// Assistant message (text and thinking blocks).
    Assistant(AssistantRow),
    /// One tool call.
    Tool(ToolRow),
    /// A `!`/`!!` command.
    Bash(BashRow),
    /// Extension message with `display: true`.
    Custom(CustomRow),
    /// Extension entry (`custom`); rendered only when a renderer yields content.
    CustomEntry(CustomEntryRow),
    /// Branch summary.
    BranchSummary(SummaryRow),
    /// Compaction summary.
    Compaction(SummaryRow),
    /// Status/warning/error line added by the frontend.
    Notice(NoticeRow),
}

/// User message row.
#[derive(Clone, Debug, PartialEq)]
pub struct UserRow {
    /// Text blocks joined.
    pub text: String,
    /// Attached images.
    pub images: Vec<ImageContent>,
}

/// pi `parseSkillBlock` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRow {
    /// Skill name.
    pub name: String,
    /// `SKILL.md` location.
    pub location: String,
    /// Skill body.
    pub content: String,
}

/// One block of an assistant message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantBlock {
    /// Markdown text.
    Text(String),
    /// Consecutive thinking blocks joined with a blank line.
    Thinking(String),
}

/// Assistant message row.
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantRow {
    /// Visible blocks in order.
    pub blocks: Vec<AssistantBlock>,
    /// The message contains tool calls (errors are shown on the tool rows then).
    pub has_tool_calls: bool,
    /// Stop reason (`None` while streaming).
    pub stop_reason: Option<StopReason>,
    /// Provider error text.
    pub error_message: Option<String>,
    /// Still streaming.
    pub streaming: bool,
}

/// Result attached to a tool row.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    /// Text and image blocks.
    pub content: Vec<UserContent>,
    /// Tool-specific details (`diff`, `truncation`, …).
    pub details: Option<JsonValue>,
    /// Error flag.
    pub is_error: bool,
}

impl ToolOutput {
    /// pi `getTextOutput`: text blocks joined with newlines.
    #[must_use]
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text(t) => Some(t.text.as_str()),
                UserContent::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Image blocks.
    #[must_use]
    pub fn images(&self) -> Vec<&ImageContent> {
        self.content
            .iter()
            .filter_map(|c| match c {
                UserContent::Image(i) => Some(i),
                UserContent::Text(_) => None,
            })
            .collect()
    }

    fn from_result(result: &AgentToolResult, is_error: bool) -> Self {
        Self {
            content: result.content.clone(),
            details: result.details.clone(),
            is_error,
        }
    }
}

/// Tool call row (pi `ToolExecutionComponent`).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRow {
    /// Tool-call id.
    pub tool_call_id: String,
    /// Tool name.
    pub name: String,
    /// Arguments (partial while streaming).
    pub args: JsonObject,
    /// Arguments finished streaming.
    pub args_complete: bool,
    /// Execution started.
    pub started: bool,
    /// Latest (partial or final) result.
    pub output: Option<ToolOutput>,
    /// `output` is a partial update (pi `isPartial`).
    pub partial: bool,
    /// Wall time of the finished call.
    pub duration_ms: Option<u64>,
}

/// `!` command row (pi `BashExecutionComponent`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors pi BashExecutionMessage flags plus the running state"
)]
pub struct BashRow {
    /// Request id used for `bash_execution_update` routing.
    pub request_id: Option<String>,
    /// Command line.
    pub command: String,
    /// Output so far.
    pub output: String,
    /// Exit code (when finished and not killed).
    pub exit_code: Option<i32>,
    /// Cancelled by the user.
    pub cancelled: bool,
    /// Output was truncated.
    pub truncated: bool,
    /// Full output file.
    pub full_output_path: Option<String>,
    /// `!!`: excluded from context.
    pub exclude_from_context: bool,
    /// Still running.
    pub running: bool,
}

/// Custom message row.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomRow {
    /// Entry id (when known) for renderer requests.
    pub entry_id: Option<String>,
    /// Extension-defined type.
    pub custom_type: String,
    /// Content.
    pub content: UserContentField,
    /// Details.
    pub details: Option<JsonValue>,
}

impl CustomRow {
    /// Text blocks joined with newlines.
    #[must_use]
    pub fn text(&self) -> String {
        self.content.text("\n")
    }
}

/// Custom entry row.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomEntryRow {
    /// Entry id.
    pub entry_id: String,
    /// Extension-defined type.
    pub custom_type: String,
    /// Payload.
    pub data: Option<JsonValue>,
}

/// Branch or compaction summary row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryRow {
    /// Summary text.
    pub summary: String,
    /// Tokens before compaction (compaction only).
    pub tokens_before: u64,
}

/// Severity of a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Dim status line.
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
    /// Markdown document (`/hotkeys`, `/session`).
    Document,
}

/// Notice row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeRow {
    /// Severity.
    pub level: NoticeLevel,
    /// Text.
    pub text: String,
}

/// Finished `!` command (subset of core `BashResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BashOutcome {
    /// Final output.
    pub output: String,
    /// Exit code.
    pub exit_code: Option<i32>,
    /// Cancelled.
    pub cancelled: bool,
    /// Truncated.
    pub truncated: bool,
    /// Full output file.
    pub full_output_path: Option<String>,
}

/// What a reducer step requires from the frontend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Rows changed (redraw).
    pub changed: bool,
    /// The context changed shape (compaction); reload from `get_entries`.
    pub reload: bool,
}

/// The transcript model.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    rows: Vec<Row>,
    next_id: RowId,
    next_rev: u64,
    /// Tool rows awaiting results: tool-call id → row id.
    pending_tools: HashMap<String, RowId>,
    /// The streaming assistant row.
    streaming: Option<RowId>,
}

/// pi `parseSkillBlock`.
#[must_use]
pub fn parse_skill_block(text: &str) -> Option<(SkillRow, Option<String>)> {
    let rest = text.strip_prefix("<skill name=\"")?;
    let (name, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(" location=\"")?;
    let (location, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(">\n")?;
    let end = rest.find("\n</skill>")?;
    let content = &rest[..end];
    let tail = &rest[end + "\n</skill>".len()..];
    let user = if tail.is_empty() {
        None
    } else {
        let message = tail.strip_prefix("\n\n")?;
        let trimmed = message.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };
    if name.is_empty() || location.is_empty() {
        return None;
    }
    Some((
        SkillRow {
            name: name.to_owned(),
            location: location.to_owned(),
            content: content.to_owned(),
        },
        user,
    ))
}

fn user_text(content: &UserContentField) -> (String, Vec<ImageContent>) {
    match content {
        UserContentField::Text(text) => (text.clone(), Vec::new()),
        UserContentField::Blocks(blocks) => {
            let mut texts = Vec::new();
            let mut images = Vec::new();
            for block in blocks {
                match block {
                    UserContent::Text(t) => texts.push(t.text.as_str()),
                    UserContent::Image(i) => images.push(i.clone()),
                }
            }
            (texts.join(""), images)
        }
    }
}

fn assistant_row(message: &AssistantMessage, streaming: bool) -> AssistantRow {
    let mut blocks = Vec::new();
    let mut thinking: Vec<String> = Vec::new();
    let mut in_thinking_run = false;
    let flush = |blocks: &mut Vec<AssistantBlock>, thinking: &mut Vec<String>| {
        if !thinking.is_empty() {
            blocks.push(AssistantBlock::Thinking(thinking.join("\n\n")));
            thinking.clear();
        }
    };
    let mut has_tool_calls = false;
    for content in &message.content {
        match content {
            AssistantContent::Thinking(t) => {
                in_thinking_run = true;
                let text = t.thinking.trim();
                if !text.is_empty() {
                    thinking.push(text.to_owned());
                }
            }
            AssistantContent::Text(t) => {
                if in_thinking_run {
                    flush(&mut blocks, &mut thinking);
                    in_thinking_run = false;
                }
                let text = t.text.trim();
                if !text.is_empty() {
                    blocks.push(AssistantBlock::Text(text.to_owned()));
                }
            }
            AssistantContent::ToolCall(_) => {
                has_tool_calls = true;
                if in_thinking_run {
                    flush(&mut blocks, &mut thinking);
                    in_thinking_run = false;
                }
            }
        }
    }
    flush(&mut blocks, &mut thinking);
    AssistantRow {
        blocks,
        has_tool_calls,
        stop_reason: (!streaming).then_some(message.stop_reason),
        error_message: message.error_message.clone(),
        streaming,
    }
}

/// Error text pi puts on tool rows of an aborted/failed assistant message.
fn tool_error_text(message: &AssistantMessage) -> String {
    if message.stop_reason == StopReason::Aborted {
        "Operation aborted".to_owned()
    } else {
        message
            .error_message
            .clone()
            .unwrap_or_else(|| "Error".to_owned())
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

    /// Whether an assistant message is streaming.
    #[must_use]
    pub const fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    /// Removes every row.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.pending_tools.clear();
        self.streaming = None;
    }

    fn push(&mut self, kind: RowKind) -> RowId {
        let id = self.next_id;
        self.next_id += 1;
        self.next_rev += 1;
        self.rows.push(Row {
            id,
            rev: self.next_rev,
            kind,
        });
        id
    }

    fn index(&self, id: RowId) -> Option<usize> {
        // Rows are appended with increasing ids; search from the end (recent rows change).
        self.rows.iter().rposition(|row| row.id == id)
    }

    fn update(&mut self, id: RowId, f: impl FnOnce(&mut RowKind)) -> bool {
        let Some(index) = self.index(id) else {
            return false;
        };
        self.next_rev += 1;
        let row = &mut self.rows[index];
        row.rev = self.next_rev;
        f(&mut row.kind);
        true
    }

    /// Forces a re-render of every row (theme or density change).
    pub fn touch_all(&mut self) {
        for row in &mut self.rows {
            self.next_rev += 1;
            row.rev = self.next_rev;
        }
    }

    /// Adds a notice line.
    pub fn push_notice(&mut self, level: NoticeLevel, text: impl Into<String>) -> RowId {
        self.push(RowKind::Notice(NoticeRow {
            level,
            text: text.into(),
        }))
    }

    /// Rebuilds from session entries the way pi `renderInitialMessages` does: the context
    /// entries of the branch ending at `leaf` (latest compaction honoured), custom entries
    /// included.
    pub fn reset_from_entries(&mut self, entries: &[SessionEntry], leaf: Option<&str>) {
        self.clear();
        let index = EntryIndex::new(entries.to_vec());
        let leaf = leaf.map(EntryId::new);
        let mut context = oino_session::build_context_entries(&index, leaf.as_ref());
        if !context
            .first()
            .is_some_and(|e| matches!(e.kind, EntryKind::Compaction(_)))
        {
            self.render_entries(&context);
            return;
        }
        // pi prepends the latest compaction for the model; frontends show kept entries,
        // then the summary, then what followed the compaction.
        let compaction = context.remove(0);
        let path = oino_session::path_to(&index, leaf.as_ref());
        let after: HashSet<&EntryId> = path
            .iter()
            .position(|e| e.id == compaction.id)
            .map(|p| path[p + 1..].iter().map(|e| &e.id).collect())
            .unwrap_or_default();
        let (before, later): (Vec<_>, Vec<_>) =
            context.into_iter().partition(|e| !after.contains(&e.id));
        self.render_entries(&before);
        self.render_entries(std::slice::from_ref(&compaction));
        self.render_entries(&later);
    }

    fn render_entries(&mut self, entries: &[SessionEntry]) {
        let mut rendered_tools: HashMap<String, RowId> = HashMap::new();
        for entry in entries {
            if let EntryKind::Custom { custom_type, data } = &entry.kind {
                self.push(RowKind::CustomEntry(CustomEntryRow {
                    entry_id: entry.id.as_str().to_owned(),
                    custom_type: custom_type.clone(),
                    data: data.clone(),
                }));
                continue;
            }
            for message in oino_session::entry_to_context_messages(entry, None) {
                match &message {
                    AgentMessage::Assistant(assistant) => {
                        self.push(RowKind::Assistant(assistant_row(assistant, false)));
                        let failed = matches!(
                            assistant.stop_reason,
                            StopReason::Aborted | StopReason::Error
                        );
                        for content in &assistant.content {
                            if let AssistantContent::ToolCall(call) = content {
                                let output = failed.then(|| ToolOutput {
                                    content: vec![UserContent::text(tool_error_text(assistant))],
                                    details: None,
                                    is_error: true,
                                });
                                let id = self.push(RowKind::Tool(ToolRow {
                                    tool_call_id: call.id.clone(),
                                    name: call.name.clone(),
                                    args: call.arguments.clone(),
                                    args_complete: true,
                                    started: true,
                                    output,
                                    partial: false,
                                    duration_ms: None,
                                }));
                                if !failed {
                                    rendered_tools.insert(call.id.clone(), id);
                                }
                            }
                        }
                    }
                    AgentMessage::ToolResult(result) => {
                        if let Some(id) = rendered_tools.remove(&result.tool_call_id) {
                            let output = ToolOutput {
                                content: result.content.clone(),
                                details: result.details.clone(),
                                is_error: result.is_error,
                            };
                            let duration = result.duration_ms;
                            self.update(id, |kind| {
                                if let RowKind::Tool(tool) = kind {
                                    tool.output = Some(output);
                                    tool.partial = false;
                                    tool.duration_ms = duration;
                                }
                            });
                        }
                    }
                    other => self.add_message(other, Some(entry.id.as_str())),
                }
            }
        }
        self.pending_tools.extend(rendered_tools);
    }

    /// pi `addMessageToChat` for non-assistant, non-tool-result messages.
    fn add_message(&mut self, message: &AgentMessage, entry_id: Option<&str>) {
        match message {
            AgentMessage::User(user) => {
                let (text, images) = user_text(&user.content);
                if text.is_empty() && images.is_empty() {
                    return;
                }
                if let Some((skill, user_message)) = parse_skill_block(&text) {
                    self.push(RowKind::Skill(skill));
                    if let Some(user_message) = user_message {
                        self.push(RowKind::User(UserRow {
                            text: user_message,
                            images,
                        }));
                    }
                } else {
                    self.push(RowKind::User(UserRow { text, images }));
                }
            }
            AgentMessage::BashExecution(bash) => {
                self.push(RowKind::Bash(BashRow {
                    request_id: None,
                    command: bash.command.clone(),
                    output: bash.output.clone(),
                    exit_code: bash.exit_code,
                    cancelled: bash.cancelled,
                    truncated: bash.truncated,
                    full_output_path: bash.full_output_path.clone(),
                    exclude_from_context: bash.exclude_from_context.unwrap_or(false),
                    running: false,
                }));
            }
            AgentMessage::Custom(custom) => {
                if custom.display {
                    self.push(RowKind::Custom(CustomRow {
                        entry_id: entry_id.map(str::to_owned),
                        custom_type: custom.custom_type.clone(),
                        content: custom.content.clone(),
                        details: custom.details.clone(),
                    }));
                }
            }
            AgentMessage::BranchSummary(summary) => {
                self.push(RowKind::BranchSummary(SummaryRow {
                    summary: summary.summary.clone(),
                    tokens_before: 0,
                }));
            }
            AgentMessage::CompactionSummary(summary) => {
                self.push(RowKind::Compaction(SummaryRow {
                    summary: summary.summary.clone(),
                    tokens_before: summary.tokens_before,
                }));
            }
            AgentMessage::Assistant(assistant) => {
                self.push(RowKind::Assistant(assistant_row(assistant, false)));
            }
            AgentMessage::System(_) | AgentMessage::ToolResult(_) => {}
        }
    }

    fn ensure_tool(&mut self, id: &str, name: &str, args: &JsonObject) -> RowId {
        if let Some(row) = self.pending_tools.get(id).copied()
            && self.index(row).is_some()
        {
            return row;
        }
        let row = self.push(RowKind::Tool(ToolRow {
            tool_call_id: id.to_owned(),
            name: name.to_owned(),
            args: args.clone(),
            args_complete: false,
            started: false,
            output: None,
            partial: true,
            duration_ms: None,
        }));
        self.pending_tools.insert(id.to_owned(), row);
        row
    }

    /// Applies one session event (pi `handleEvent` chat-container effects).
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per pi session event, mirroring interactive-mode handleEvent"
    )]
    pub fn apply(&mut self, event: &AgentSessionEvent) -> Applied {
        let mut applied = Applied::default();
        match event {
            AgentSessionEvent::AgentStart => {
                self.pending_tools.clear();
            }
            AgentSessionEvent::MessageStart { message } => match message {
                AgentMessage::Assistant(assistant) => {
                    let id = self.push(RowKind::Assistant(assistant_row(assistant, true)));
                    self.streaming = Some(id);
                    applied.changed = true;
                }
                AgentMessage::User(_) | AgentMessage::Custom(_) => {
                    let before = self.rows.len();
                    self.add_message(message, None);
                    applied.changed = self.rows.len() != before;
                }
                _ => {}
            },
            AgentSessionEvent::MessageUpdate { message, .. } => {
                if let (Some(id), AgentMessage::Assistant(assistant)) = (self.streaming, message) {
                    let row = assistant_row(assistant, true);
                    self.update(id, |kind| *kind = RowKind::Assistant(row));
                    for content in &assistant.content {
                        if let AssistantContent::ToolCall(call) = content {
                            let tool = self.ensure_tool(&call.id, &call.name, &call.arguments);
                            let args = call.arguments.clone();
                            self.update(tool, |kind| {
                                if let RowKind::Tool(t) = kind
                                    && t.args != args
                                {
                                    t.args = args;
                                }
                            });
                        }
                    }
                    applied.changed = true;
                }
            }
            AgentSessionEvent::MessageEnd { message } => {
                if let (Some(id), AgentMessage::Assistant(assistant)) = (self.streaming, message) {
                    let mut row = assistant_row(assistant, false);
                    if assistant.stop_reason == StopReason::Aborted {
                        row.error_message = Some("Operation aborted".to_owned());
                    }
                    self.update(id, |kind| *kind = RowKind::Assistant(row));
                    let failed = matches!(
                        assistant.stop_reason,
                        StopReason::Aborted | StopReason::Error
                    );
                    let pending: Vec<RowId> = self.pending_tools.values().copied().collect();
                    let text = tool_error_text(assistant);
                    for tool in pending {
                        let text = text.clone();
                        self.update(tool, |kind| {
                            if let RowKind::Tool(t) = kind {
                                if failed {
                                    t.output = Some(ToolOutput {
                                        content: vec![UserContent::text(text)],
                                        details: None,
                                        is_error: true,
                                    });
                                    t.partial = false;
                                } else {
                                    t.args_complete = true;
                                }
                            }
                        });
                    }
                    if failed {
                        self.pending_tools.clear();
                    }
                    self.streaming = None;
                    applied.changed = true;
                }
            }
            AgentSessionEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
                parent_tool_call_id,
            } => {
                if parent_tool_call_id.is_none() {
                    let row = self.ensure_tool(tool_call_id, tool_name, args);
                    self.update(row, |kind| {
                        if let RowKind::Tool(t) = kind {
                            t.started = true;
                            t.args_complete = true;
                        }
                    });
                    applied.changed = true;
                }
            }
            AgentSessionEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => {
                if let Some(row) = self.pending_tools.get(tool_call_id).copied() {
                    let output = ToolOutput::from_result(partial_result, false);
                    applied.changed = self.update(row, |kind| {
                        if let RowKind::Tool(t) = kind {
                            t.output = Some(output);
                            t.partial = true;
                        }
                    });
                }
            }
            AgentSessionEvent::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                duration_ms,
                ..
            } => {
                if let Some(row) = self.pending_tools.remove(tool_call_id) {
                    let output = ToolOutput::from_result(result, *is_error);
                    let duration = *duration_ms;
                    applied.changed = self.update(row, |kind| {
                        if let RowKind::Tool(t) = kind {
                            t.output = Some(output);
                            t.partial = false;
                            t.duration_ms = duration;
                        }
                    });
                }
            }
            AgentSessionEvent::AgentEnd { .. } => {
                if let Some(id) = self.streaming.take()
                    && let Some(index) = self.index(id)
                {
                    self.rows.remove(index);
                    applied.changed = true;
                }
                self.pending_tools.clear();
            }
            AgentSessionEvent::EntryAppended { entry } => match &entry.kind {
                EntryKind::Custom { custom_type, data } => {
                    self.push(RowKind::CustomEntry(CustomEntryRow {
                        entry_id: entry.id.as_str().to_owned(),
                        custom_type: custom_type.clone(),
                        data: data.clone(),
                    }));
                    applied.changed = true;
                }
                EntryKind::CustomMessage {
                    custom_type,
                    content,
                    display: true,
                    details,
                } => {
                    self.push(RowKind::Custom(CustomRow {
                        entry_id: Some(entry.id.as_str().to_owned()),
                        custom_type: custom_type.clone(),
                        content: content.clone(),
                        details: details.clone(),
                    }));
                    applied.changed = true;
                }
                EntryKind::Compaction(_) => applied.reload = true,
                _ => {}
            },
            AgentSessionEvent::CompactionEnd {
                result: Some(_),
                aborted: false,
                ..
            } => applied.reload = true,
            _ => {}
        }
        applied
    }

    /// Starts a `!` command row (frontend-initiated; output streams via
    /// [`Transcript::bash_delta`]).
    pub fn begin_bash(
        &mut self,
        request_id: Option<String>,
        command: impl Into<String>,
        exclude_from_context: bool,
    ) -> RowId {
        self.push(RowKind::Bash(BashRow {
            request_id,
            command: command.into(),
            output: String::new(),
            exit_code: None,
            cancelled: false,
            truncated: false,
            full_output_path: None,
            exclude_from_context,
            running: true,
        }))
    }

    fn bash_row(&self, request_id: Option<&str>) -> Option<RowId> {
        self.rows.iter().rev().find_map(|row| match &row.kind {
            RowKind::Bash(b) if b.running && b.request_id.as_deref() == request_id => Some(row.id),
            _ => None,
        })
    }

    /// Appends streamed output of a running `!` command.
    pub fn bash_delta(&mut self, request_id: Option<&str>, delta: &str) -> bool {
        let Some(id) = self.bash_row(request_id) else {
            return false;
        };
        self.update(id, |kind| {
            if let RowKind::Bash(b) = kind {
                b.output.push_str(delta);
            }
        })
    }

    /// Completes a `!` command row.
    pub fn finish_bash(&mut self, request_id: Option<&str>, outcome: BashOutcome) -> bool {
        let Some(id) = self.bash_row(request_id) else {
            return false;
        };
        self.update(id, |kind| {
            if let RowKind::Bash(b) = kind {
                b.output = outcome.output;
                b.exit_code = outcome.exit_code;
                b.cancelled = outcome.cancelled;
                b.truncated = outcome.truncated;
                b.full_output_path = outcome.full_output_path;
                b.running = false;
            }
        })
    }

    /// Marks a running `!` command row as failed with `error`.
    pub fn fail_bash(&mut self, request_id: Option<&str>, error: &str) -> bool {
        let Some(id) = self.bash_row(request_id) else {
            return false;
        };
        let error = error.to_owned();
        self.update(id, |kind| {
            if let RowKind::Bash(b) = kind {
                if !b.output.is_empty() && !b.output.ends_with('\n') {
                    b.output.push('\n');
                }
                b.output.push_str(&error);
                b.running = false;
            }
        })
    }

    /// Text of the last finished assistant message (pi `/copy`).
    #[must_use]
    pub fn last_assistant_text(&self) -> Option<String> {
        self.rows.iter().rev().find_map(|row| match &row.kind {
            RowKind::Assistant(a) if !a.streaming => {
                let text: Vec<&str> = a
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        AssistantBlock::Text(t) => Some(t.as_str()),
                        AssistantBlock::Thinking(_) => None,
                    })
                    .collect();
                (!text.is_empty()).then(|| text.join("\n\n"))
            }
            _ => None,
        })
    }

    /// Texts of user rows in order (editor history seed, pi `populateHistory`).
    #[must_use]
    pub fn user_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::User(u) if !u.text.is_empty() => Some(u.text.clone()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_ai::{TextContent, ToolCall, Usage, UserMessage};
    use serde_json::json;

    fn assistant(content: &[AssistantContent], stop: StopReason) -> AssistantMessage {
        serde_json::from_value(json!({
            "content": serde_json::to_value(content).unwrap(),
            "api": "faux", "provider": "faux", "model": "m",
            "usage": serde_json::to_value(Usage::default()).unwrap(),
            "stopReason": stop, "timestamp": 0
        }))
        .unwrap()
    }

    fn call(id: &str, name: &str, args: JsonValue) -> AssistantContent {
        AssistantContent::ToolCall(ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: serde_json::from_value(args).unwrap(),
            thought_signature: None,
            namespace: None,
            arguments_error: None,
        })
    }

    #[test]
    fn skill_block_parsing() {
        let (skill, user) = parse_skill_block(
            "<skill name=\"git\" location=\"/s/SKILL.md\">\nbody\n</skill>\n\nhi",
        )
        .unwrap();
        assert_eq!(skill.name, "git");
        assert_eq!(skill.content, "body");
        assert_eq!(user.as_deref(), Some("hi"));
        assert!(parse_skill_block("plain").is_none());
    }

    #[test]
    fn streaming_reply_with_tool() {
        let mut t = Transcript::new();
        let user = AgentMessage::User(UserMessage {
            content: UserContentField::Text("hello".into()),
            timestamp: 0,
        });
        t.apply(&AgentSessionEvent::AgentStart);
        t.apply(&AgentSessionEvent::MessageStart {
            message: user.clone(),
        });
        let partial = assistant(
            &[AssistantContent::Text(TextContent::new("Let me look"))],
            StopReason::Stop,
        );
        t.apply(&AgentSessionEvent::MessageStart {
            message: AgentMessage::Assistant(partial.clone()),
        });
        let with_call = assistant(
            &[
                AssistantContent::Text(TextContent::new("Let me look")),
                call("c1", "read", json!({"path": "a.rs"})),
            ],
            StopReason::ToolUse,
        );
        t.apply(&AgentSessionEvent::MessageUpdate {
            message: AgentMessage::Assistant(with_call.clone()),
            assistant_message_event: serde_json::from_value(
                json!({"type":"start","partial": with_call}),
            )
            .unwrap(),
        });
        assert!(t.is_streaming());
        t.apply(&AgentSessionEvent::MessageEnd {
            message: AgentMessage::Assistant(with_call),
        });
        t.apply(&AgentSessionEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "read".into(),
            args: serde_json::from_value(json!({"path": "a.rs"})).unwrap(),
            parent_tool_call_id: None,
        });
        t.apply(&AgentSessionEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "read".into(),
            result: AgentToolResult {
                content: vec![UserContent::text("fn main() {}")],
                ..AgentToolResult::default()
            },
            is_error: false,
            duration_ms: Some(5),
            parent_tool_call_id: None,
        });
        let kinds: Vec<&RowKind> = t.rows().iter().map(|r| &r.kind).collect();
        assert!(matches!(kinds[0], RowKind::User(u) if u.text == "hello"));
        assert!(matches!(kinds[1], RowKind::Assistant(a) if a.has_tool_calls && !a.streaming));
        let RowKind::Tool(tool) = kinds[2] else {
            panic!("tool row")
        };
        assert_eq!(tool.output.as_ref().unwrap().text(), "fn main() {}");
        assert!(!tool.partial);
        assert_eq!(t.rows().len(), 3);
    }

    #[test]
    fn aborted_message_marks_pending_tools() {
        let mut t = Transcript::new();
        let msg = assistant(
            &[call("c1", "bash", json!({"command": "ls"}))],
            StopReason::Aborted,
        );
        t.apply(&AgentSessionEvent::MessageStart {
            message: AgentMessage::Assistant(msg.clone()),
        });
        t.apply(&AgentSessionEvent::MessageUpdate {
            message: AgentMessage::Assistant(msg.clone()),
            assistant_message_event: serde_json::from_value(json!({"type":"start","partial": msg}))
                .unwrap(),
        });
        t.apply(&AgentSessionEvent::MessageEnd {
            message: AgentMessage::Assistant(msg),
        });
        let RowKind::Tool(tool) = &t.rows()[1].kind else {
            panic!()
        };
        let out = tool.output.as_ref().unwrap();
        assert!(out.is_error);
        assert_eq!(out.text(), "Operation aborted");
    }

    #[test]
    fn bash_rows_stream_and_finish() {
        let mut t = Transcript::new();
        t.begin_bash(Some("b1".into()), "ls", false);
        assert!(t.bash_delta(Some("b1"), "a\n"));
        assert!(t.finish_bash(
            Some("b1"),
            BashOutcome {
                output: "a\nb\n".into(),
                exit_code: Some(0),
                ..BashOutcome::default()
            }
        ));
        let RowKind::Bash(b) = &t.rows()[0].kind else {
            panic!()
        };
        assert!(!b.running);
        assert_eq!(b.output, "a\nb\n");
    }

    #[test]
    fn rebuilds_from_entries_with_tool_results() {
        let entries: Vec<SessionEntry> = serde_json::from_value(json!([
            {"type":"message","id":"a1","parentId":null,"timestamp":"2026-01-01T00:00:00.000Z",
             "message":{"role":"user","content":"hi","timestamp":0}},
            {"type":"message","id":"a2","parentId":"a1","timestamp":"2026-01-01T00:00:01.000Z",
             "message":{"role":"assistant","content":[{"type":"toolCall","id":"t1","name":"ls","arguments":{}}],
             "api":"x","provider":"x","model":"m","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
             "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":0}},
            {"type":"message","id":"a3","parentId":"a2","timestamp":"2026-01-01T00:00:02.000Z",
             "message":{"role":"toolResult","toolCallId":"t1","toolName":"ls","content":[{"type":"text","text":"x.rs"}],"isError":false,"timestamp":0}},
            {"type":"custom","id":"a4","parentId":"a3","timestamp":"2026-01-01T00:00:03.000Z","customType":"probe","data":{"n":1}}
        ]))
        .unwrap();
        let mut t = Transcript::new();
        t.reset_from_entries(&entries, Some("a4"));
        assert_eq!(t.rows().len(), 4);
        let RowKind::Tool(tool) = &t.rows()[2].kind else {
            panic!()
        };
        assert_eq!(tool.output.as_ref().unwrap().text(), "x.rs");
        assert!(matches!(&t.rows()[3].kind, RowKind::CustomEntry(c) if c.custom_type == "probe"));
        assert_eq!(t.user_texts(), vec!["hi".to_owned()]);
    }
}
