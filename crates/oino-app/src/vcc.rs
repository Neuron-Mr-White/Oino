use oino_session::{SessionEntry, SessionEntryKind};
use oino_types::{ContentBlock, Message, OinoId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_SECTION_ITEMS: usize = 12;
const MAX_TRANSCRIPT_ITEMS: usize = 24;
const MAX_SNIPPET_CHARS: usize = 260;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VccCompaction {
    pub summary: String,
    pub replaces: Vec<OinoId>,
    pub compacted_entries: usize,
    pub kept_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VccRecallOptions {
    pub query: Option<String>,
    pub scope_all: bool,
    pub offset: usize,
    pub limit: usize,
    pub expand: bool,
}

impl Default for VccRecallOptions {
    fn default() -> Self {
        Self {
            query: None,
            scope_all: false,
            offset: 0,
            limit: 5,
            expand: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VccRecallResult {
    pub output: String,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

pub fn compact_branch(branch: &[SessionEntry]) -> Option<VccCompaction> {
    let last_user_index = branch.iter().rposition(is_user_message_entry)?;
    if last_user_index == 0 {
        return None;
    }
    let compacted = &branch[..last_user_index];
    if compacted.is_empty() {
        return None;
    }
    let kept_entries = branch.len().saturating_sub(compacted.len());
    let summary = build_summary(compacted, kept_entries);
    let replaces = compacted.iter().map(|entry| entry.id).collect::<Vec<_>>();
    Some(VccCompaction {
        summary,
        replaces,
        compacted_entries: compacted.len(),
        kept_entries,
    })
}

pub fn recall(
    branch: &[SessionEntry],
    all_entries: &[SessionEntry],
    options: VccRecallOptions,
) -> VccRecallResult {
    let entries = if options.scope_all {
        all_entries
    } else {
        branch
    };
    let mut rows = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| recall_row(index, entry))
        .collect::<Vec<_>>();

    if let Some(query) = options
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        let terms = query
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        rows = rows
            .into_iter()
            .filter_map(|mut row| {
                let haystack = row.search_text.to_ascii_lowercase();
                let mut score = 0usize;
                for term in &terms {
                    if haystack.contains(term) {
                        score += haystack.matches(term).count().max(1);
                    }
                }
                (score > 0).then(|| {
                    row.score = score;
                    row
                })
            })
            .collect();
        rows.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.index.cmp(&left.index))
        });
    } else {
        rows.sort_by_key(|row| std::cmp::Reverse(row.index));
    }

    let total = rows.len();
    let limit = options.limit.clamp(1, 20);
    let offset = options.offset.min(total);
    let selected = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let scope = if options.scope_all {
        "all session entries"
    } else {
        "active branch"
    };
    let query = options
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .unwrap_or("<recent>");
    let mut output = format!(
        "# VCC recall\n\nScope: {scope}\nQuery: {query}\nMatches: {total}\nShowing: {}-{}\n",
        if selected.is_empty() { 0 } else { offset + 1 },
        offset + selected.len()
    );
    if selected.is_empty() {
        output.push_str("\nNo matching session history found.");
    } else {
        for row in selected {
            output.push('\n');
            output.push_str(&format!("## #{} {}\n", row.index + 1, row.title));
            if options.expand {
                output.push_str(row.full.trim());
            } else {
                output.push_str(row.snippet.trim());
            }
            output.push('\n');
        }
    }
    VccRecallResult {
        output,
        total,
        offset,
        limit,
    }
}

fn is_user_message_entry(entry: &SessionEntry) -> bool {
    matches!(
        &entry.kind,
        SessionEntryKind::Message {
            message: Message::User { .. }
        } | SessionEntryKind::CustomMessage {
            message: Message::User { .. }
        }
    )
}

fn build_summary(entries: &[SessionEntry], kept_entries: usize) -> String {
    let goals = extract_goals(entries);
    let files = extract_files(entries);
    let commits = extract_commits(entries);
    let preferences = extract_preferences(entries);
    let outstanding = extract_outstanding(entries);
    let transcript = brief_transcript(entries);
    let mut output = String::new();
    output.push_str("[Session Goal]\n");
    push_items_or_placeholder(&mut output, &goals, "- Unknown from compacted history");
    output.push_str("\n[Files And Changes]\n");
    push_items_or_placeholder(&mut output, &files, "- No file paths or changes detected");
    output.push_str("\n[Commits]\n");
    push_items_or_placeholder(&mut output, &commits, "- No commits detected");
    output.push_str("\n[Outstanding Context]\n");
    push_items_or_placeholder(&mut output, &outstanding, "- No obvious blockers detected");
    output.push_str("\n[User Preferences]\n");
    push_items_or_placeholder(
        &mut output,
        &preferences,
        "- No explicit preferences detected",
    );
    output.push_str("\n[Recent Compacted Transcript]\n");
    push_items_or_placeholder(&mut output, &transcript, "- No transcript content detected");
    output.push_str("\n[Recall]\n");
    output.push_str(&format!(
        "- Compacted {} entries; kept {kept_entries} live tail entries. Use `/recall <query>` or the `vcc_recall` tool to search raw session history.\n",
        entries.len()
    ));
    output
}

fn push_items_or_placeholder(output: &mut String, items: &[String], placeholder: &str) {
    if items.is_empty() {
        output.push_str(placeholder);
        output.push('\n');
    } else {
        for item in items.iter().take(MAX_SECTION_ITEMS) {
            output.push_str(item);
            output.push('\n');
        }
    }
}

fn extract_goals(entries: &[SessionEntry]) -> Vec<String> {
    let mut goals = Vec::new();
    for entry in entries {
        let Some(message @ Message::User { .. }) = message_for_entry(entry) else {
            continue;
        };
        let text = message_text(message);
        let snippet = compact_snippet(&text, MAX_SNIPPET_CHARS);
        if !snippet.is_empty() {
            goals.push(format!("- {snippet}"));
            if goals.len() >= 3 {
                break;
            }
        }
    }
    goals
}

fn extract_files(entries: &[SessionEntry]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for entry in entries {
        let Some(message) = message_for_entry(entry) else {
            continue;
        };
        for block in content_blocks(message) {
            if let ContentBlock::ToolCall {
                name, arguments, ..
            } = block
            {
                if matches!(name.as_str(), "read" | "write" | "edit") {
                    if let Some(path) = arguments.get("path").and_then(Value::as_str) {
                        push_unique(&mut files, &mut seen, format!("- `{path}` via `{name}`"));
                    }
                }
                if name == "bash" {
                    if let Some(command) = arguments.get("command").and_then(Value::as_str) {
                        for path in shellish_paths(command) {
                            push_unique(
                                &mut files,
                                &mut seen,
                                format!("- `{path}` mentioned in `bash`"),
                            );
                        }
                    }
                }
            }
        }
    }
    files
}

fn extract_commits(entries: &[SessionEntry]) -> Vec<String> {
    let mut commits = Vec::new();
    for entry in entries {
        let Some(message) = message_for_entry(entry) else {
            continue;
        };
        let text = message_text(message);
        for token in text.split_whitespace() {
            let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_hexdigit());
            if (7..=40).contains(&trimmed.len())
                && trimmed.chars().all(|ch| ch.is_ascii_hexdigit())
                && text.to_ascii_lowercase().contains("commit")
            {
                commits.push(format!("- `{}`", &trimmed[..trimmed.len().min(12)]));
                break;
            }
        }
        for block in content_blocks(message) {
            if let ContentBlock::ToolCall {
                name, arguments, ..
            } = block
            {
                if name == "bash" {
                    if let Some(command) = arguments.get("command").and_then(Value::as_str) {
                        if command.contains("git commit") {
                            commits.push(format!("- `{}`", compact_snippet(command, 160)));
                        }
                    }
                }
            }
        }
    }
    commits.truncate(MAX_SECTION_ITEMS);
    commits
}

fn extract_preferences(entries: &[SessionEntry]) -> Vec<String> {
    let markers = [
        "prefer",
        "always",
        "never",
        "please use",
        "please avoid",
        "don't",
        "do not",
    ];
    entries
        .iter()
        .filter_map(message_for_entry)
        .filter(|message| matches!(message, Message::User { .. }))
        .flat_map(|message| {
            message_text(message)
                .lines()
                .map(str::trim)
                .filter(|line| {
                    let lower = line.to_ascii_lowercase();
                    markers.iter().any(|marker| lower.contains(marker))
                })
                .map(|line| format!("- {}", compact_snippet(line, MAX_SNIPPET_CHARS)))
                .collect::<Vec<_>>()
        })
        .take(MAX_SECTION_ITEMS)
        .collect()
}

fn extract_outstanding(entries: &[SessionEntry]) -> Vec<String> {
    let markers = [
        "error", "failed", "blocked", "todo", "fixme", "panic", "warning",
    ];
    let mut out = Vec::new();
    for entry in entries.iter().rev() {
        let Some(message) = message_for_entry(entry) else {
            continue;
        };
        let is_error = matches!(message, Message::ToolResult { is_error: true, .. });
        let text = message_text(message);
        let lower = text.to_ascii_lowercase();
        if is_error || markers.iter().any(|marker| lower.contains(marker)) {
            let snippet = compact_snippet(&text, MAX_SNIPPET_CHARS);
            if !snippet.is_empty() {
                out.push(format!("- {snippet}"));
            }
        }
        if out.len() >= MAX_SECTION_ITEMS {
            break;
        }
    }
    out.reverse();
    out
}

fn brief_transcript(entries: &[SessionEntry]) -> Vec<String> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let message = message_for_entry(entry)?;
            let title = message_title(message);
            let body = match message {
                Message::Assistant { content, .. } => {
                    let tool_calls = content
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::ToolCall {
                                name, arguments, ..
                            } => Some(format!(
                                "tool `{name}` {}",
                                compact_snippet(&arguments.to_string(), 120)
                            )),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let text = compact_snippet(&message_text(message), MAX_SNIPPET_CHARS);
                    if text.is_empty() && !tool_calls.is_empty() {
                        tool_calls.join("; ")
                    } else if tool_calls.is_empty() {
                        text
                    } else {
                        format!("{text} [{}]", tool_calls.join("; "))
                    }
                }
                _ => compact_snippet(&message_text(message), MAX_SNIPPET_CHARS),
            };
            (!body.is_empty()).then(|| format!("- #{} {title}: {body}", index + 1))
        })
        .rev()
        .take(MAX_TRANSCRIPT_ITEMS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[derive(Debug, Clone)]
struct RecallRow {
    index: usize,
    title: String,
    snippet: String,
    full: String,
    search_text: String,
    score: usize,
}

fn recall_row(index: usize, entry: &SessionEntry) -> Option<RecallRow> {
    let title;
    let full;
    match &entry.kind {
        SessionEntryKind::Message { message } | SessionEntryKind::CustomMessage { message } => {
            title = message_title(message).to_string();
            full = message_full_text(message);
        }
        SessionEntryKind::Compaction { summary, .. } => {
            title = "compaction".into();
            full = summary.clone();
        }
        SessionEntryKind::BranchSummary { summary } => {
            title = "branch summary".into();
            full = summary.clone();
        }
        SessionEntryKind::ModelChange { model } => {
            title = "model".into();
            full = model.identifier();
        }
        SessionEntryKind::ThinkingLevelChange { thinking_level } => {
            title = "thinking".into();
            full = format!("{thinking_level:?}");
        }
        SessionEntryKind::Label { label } => {
            title = "label".into();
            full = label.clone();
        }
        SessionEntryKind::SessionInfo { name, cwd } => {
            title = "session info".into();
            full = format!("name={name:?} cwd={cwd:?}");
        }
        SessionEntryKind::Custom { name, payload } => {
            title = format!("custom:{name}");
            full = payload.to_string();
        }
        SessionEntryKind::ExtensionCustom { entry } => {
            title = format!("extension:{}", entry.owner_extension_id);
            full = entry.payload.to_string();
        }
        SessionEntryKind::LeafMove { leaf_id } => {
            title = "leaf move".into();
            full = leaf_id.to_string();
        }
    }
    let snippet = compact_snippet(&full, MAX_SNIPPET_CHARS);
    (!snippet.is_empty()).then(|| RecallRow {
        index,
        title,
        snippet,
        search_text: full.clone(),
        full,
        score: 0,
    })
}

fn message_for_entry(entry: &SessionEntry) -> Option<&Message> {
    match &entry.kind {
        SessionEntryKind::Message { message } | SessionEntryKind::CustomMessage { message } => {
            Some(message)
        }
        _ => None,
    }
}

fn message_title(message: &Message) -> &'static str {
    match message {
        Message::User { .. } => "user",
        Message::Assistant { .. } => "assistant",
        Message::ToolResult { is_error: true, .. } => "tool error",
        Message::ToolResult { .. } => "tool result",
        Message::Custom { .. } => "custom",
        Message::CompactionSummary { .. } => "compaction",
        Message::BranchSummary { .. } => "branch summary",
    }
}

fn message_full_text(message: &Message) -> String {
    match message {
        Message::ToolResult {
            tool_name,
            content,
            is_error,
            ..
        } => format!(
            "tool={tool_name} error={is_error}\n{}",
            blocks_text(content, true)
        ),
        Message::CompactionSummary { summary, .. } | Message::BranchSummary { summary, .. } => {
            summary.clone()
        }
        Message::Custom { name, payload, .. } => format!("custom:{name}\n{payload}"),
        Message::User { content, .. } | Message::Assistant { content, .. } => {
            blocks_text(content, true)
        }
    }
}

fn message_text(message: &Message) -> String {
    match message {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::ToolResult { content, .. } => blocks_text(content, false),
        Message::CompactionSummary { summary, .. } | Message::BranchSummary { summary, .. } => {
            summary.clone()
        }
        Message::Custom { payload, .. } => payload.to_string(),
    }
}

fn content_blocks(message: &Message) -> &[ContentBlock] {
    match message {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::ToolResult { content, .. } => content,
        Message::Custom { .. }
        | Message::CompactionSummary { .. }
        | Message::BranchSummary { .. } => &[],
    }
}

fn blocks_text(blocks: &[ContentBlock], include_tools: bool) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::Image { media_type, .. } => Some(format!("[image:{media_type}]")),
            ContentBlock::Thinking { .. } => None,
            ContentBlock::ToolCall {
                name, arguments, ..
            } if include_tools => Some(format!("tool_call {name} {arguments}")),
            ContentBlock::ToolCall { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_snippet(text: &str, limit: usize) -> String {
    let normalized = text
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= limit {
        normalized
    } else {
        let mut out = normalized
            .chars()
            .take(limit.saturating_sub(1))
            .collect::<String>();
        out.push('…');
        out
    }
}

fn push_unique(items: &mut Vec<String>, seen: &mut BTreeSet<String>, item: String) {
    if items.len() >= MAX_SECTION_ITEMS {
        return;
    }
    if seen.insert(item.clone()) {
        items.push(item);
    }
}

fn shellish_paths(command: &str) -> Vec<String> {
    command
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, '\'' | '"' | ',' | ';' | ':' | '(' | ')')
        })
        .filter(|token| {
            (token.contains('/') || token.contains('.'))
                && !token.starts_with('-')
                && !token.starts_with("http")
                && token.chars().any(|ch| ch.is_ascii_alphabetic())
        })
        .map(|token| {
            token
                .trim_matches(|ch: char| matches!(ch, '`' | '[' | ']' | '{' | '}'))
                .to_string()
        })
        .filter(|token| !token.is_empty())
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_session::{SessionHeader, SessionManager};
    use oino_types::StopReason;
    use uuid::Uuid;

    #[test]
    fn compacts_before_latest_user_and_keeps_tail() {
        let mut session = SessionManager::new(SessionHeader::new("test", "/tmp".into()));
        session.append_message(Message::user_text(
            "Please edit README.md and prefer simple code",
        ));
        session.append_message(Message::assistant_text(
            "I'll inspect it",
            StopReason::ToolUse,
        ));
        session.append_message(Message::Assistant {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::ToolCall {
                id: Uuid::new_v4(),
                name: "read".into(),
                arguments: serde_json::json!({"path":"README.md"}),
            }],
            stop_reason: Some(StopReason::ToolUse),
            usage: None,
            provider: None,
        });
        session.append_message(Message::user_text("Now continue"));
        let branch = session
            .get_branch(session.get_leaf_id())
            .unwrap_or_else(|err| panic!("branch failed: {err}"));
        let compacted = compact_branch(&branch).unwrap_or_else(|| panic!("should compact"));
        assert_eq!(compacted.kept_entries, 1);
        assert_eq!(compacted.compacted_entries, 3);
        assert!(compacted.summary.contains("README.md"));
        assert!(compacted.summary.contains("prefer simple code"));
    }

    #[test]
    fn recall_searches_active_branch() {
        let mut session = SessionManager::new(SessionHeader::new("test", "/tmp".into()));
        session.append_message(Message::user_text("alpha task"));
        session.append_message(Message::assistant_text("beta result", StopReason::EndTurn));
        let branch = session
            .get_branch(session.get_leaf_id())
            .unwrap_or_else(|err| panic!("branch failed: {err}"));
        let result = recall(
            &branch,
            &branch,
            VccRecallOptions {
                query: Some("beta".into()),
                ..Default::default()
            },
        );
        assert_eq!(result.total, 1);
        assert!(result.output.contains("beta result"));
    }
}
