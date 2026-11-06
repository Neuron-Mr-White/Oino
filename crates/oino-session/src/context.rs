//! Context building and projection (`buildContextEntries`, `buildSessionProjection`,
//! `buildSessionContext`). Pure functions over an [`EntryIndex`].
//!
//! Leaf convention: `leaf = None` is the `null` leaf (empty path, e.g. after `resetLeaf`);
//! an id that is not in the index falls back to the last entry. Use
//! [`EntryIndex::last_id`] for the "default leaf".

use std::collections::{HashMap, HashSet};

use oino_ai::transcript::SystemState;
use oino_ai::{
    AgentMessage, AssistantContent, SystemMessage, TextContent, UserContent, UserContentField,
};
use serde::{Deserialize, Serialize};

use crate::entry::{ContextReplacement, EditableContent, EntryId, EntryKind, SessionEntry};
use crate::messages::{branch_summary_message, compaction_summary_message, custom_message};

/// Entries in file order plus an id index (latest entry wins on duplicate ids).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryIndex {
    entries: Vec<SessionEntry>,
    by_id: HashMap<EntryId, usize>,
}

impl EntryIndex {
    /// Indexes `entries` (file order).
    #[must_use]
    pub fn new(entries: Vec<SessionEntry>) -> Self {
        let by_id = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id.clone(), index))
            .collect();
        Self { entries, by_id }
    }

    /// Appends an entry.
    pub fn push(&mut self, entry: SessionEntry) {
        self.by_id.insert(entry.id.clone(), self.entries.len());
        self.entries.push(entry);
    }

    /// Entry by id.
    #[must_use]
    pub fn get(&self, id: &EntryId) -> Option<&SessionEntry> {
        self.by_id.get(id).map(|&index| &self.entries[index])
    }

    /// Whether `id` is present.
    #[must_use]
    pub fn contains(&self, id: &EntryId) -> bool {
        self.by_id.contains_key(id)
    }

    /// Whether the id text is present.
    #[must_use]
    pub fn contains_str(&self, id: &str) -> bool {
        self.by_id.contains_key(&EntryId::new(id))
    }

    /// All entries in file order.
    #[must_use]
    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    /// Consumes the index.
    #[must_use]
    pub fn into_entries(self) -> Vec<SessionEntry> {
        self.entries
    }

    /// Number of distinct ids.
    #[must_use]
    pub fn id_count(&self) -> usize {
        self.by_id.len()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Id of the last entry in file order (the default leaf).
    #[must_use]
    pub fn last_id(&self) -> Option<&EntryId> {
        self.entries.last().map(|entry| &entry.id)
    }
}

impl FromIterator<SessionEntry> for EntryIndex {
    fn from_iter<I: IntoIterator<Item = SessionEntry>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

/// `{provider, modelId}` selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model_id: String,
}

/// One selected entry and the messages it contributes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedEntry {
    /// Selected raw entry.
    pub source_entry: SessionEntry,
    /// Model-visible messages after context edits.
    pub messages: Vec<AgentMessage>,
}

/// The session projection record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProjection {
    /// Selected entries with provenance.
    pub entries: Vec<ProjectedEntry>,
    /// Recoverable invalid compaction records ignored while preserving the prior view.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Flattened messages.
    pub messages: Vec<AgentMessage>,
    /// Thinking level string (default `"off"`).
    pub thinking_level: String,
    /// Current model.
    pub model: Option<ModelRef>,
}

/// The session context record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContext {
    /// Messages for the agent.
    pub messages: Vec<AgentMessage>,
    /// Thinking level string (default `"off"`).
    pub thinking_level: String,
    /// Current model.
    pub model: Option<ModelRef>,
}

/// Latest explicit selections on a path (input to virtual-model aware resolution).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchSelection {
    /// Latest `model_change`.
    pub model: Option<ModelRef>,
    /// Id of that `model_change` entry.
    pub model_entry_id: Option<EntryId>,
    /// Latest `thinking_level_change`.
    pub thinking_level: Option<String>,
}

/// Root → leaf path. Stops at a missing parent (orphans become roots)
/// and at cycles.
#[must_use]
pub fn path_to<'a>(entries: &'a EntryIndex, leaf: Option<&EntryId>) -> Vec<&'a SessionEntry> {
    let Some(leaf) = leaf else {
        return Vec::new();
    };
    let Some(start) = entries.get(leaf).or_else(|| entries.entries.last()) else {
        return Vec::new();
    };
    walk_up(entries, start)
}

pub(crate) fn walk_up<'a>(
    entries: &'a EntryIndex,
    start: &'a SessionEntry,
) -> Vec<&'a SessionEntry> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(start);
    while let Some(entry) = current {
        if !seen.insert(&entry.id) {
            break;
        }
        path.push(entry);
        current = entry
            .parent_id
            .as_ref()
            .and_then(|parent| entries.get(parent));
    }
    path.reverse();
    path
}

/// Latest `compaction` entry on `path`.
#[must_use]
pub fn latest_compaction<'a>(path: &[&'a SessionEntry]) -> Option<&'a SessionEntry> {
    path.iter()
        .rev()
        .find(|entry| matches!(entry.kind, EntryKind::Compaction(_)))
        .copied()
}

/// Latest explicit `model_change` and `thinking_level_change` on `path`.
#[must_use]
pub fn branch_selection(path: &[&SessionEntry]) -> BranchSelection {
    let mut selection = BranchSelection::default();
    for entry in path {
        match &entry.kind {
            EntryKind::ModelChange { provider, model_id } => {
                selection.model = Some(ModelRef {
                    provider: provider.clone(),
                    model_id: model_id.clone(),
                });
                selection.model_entry_id = Some(entry.id.clone());
            }
            EntryKind::ThinkingLevelChange { thinking_level } => {
                selection.thinking_level = Some(thinking_level.clone());
            }
            _ => {}
        }
    }
    selection
}

/// Thinking level (default `"off"`) and model from the full
/// path; assistant messages also update the model (the physical model that answered).
fn context_settings(path: &[&SessionEntry]) -> (String, Option<ModelRef>) {
    let mut thinking_level = "off".to_owned();
    let mut model = None;
    for entry in path {
        match &entry.kind {
            EntryKind::ThinkingLevelChange {
                thinking_level: level,
            } => {
                thinking_level.clone_from(level);
            }
            EntryKind::ModelChange { provider, model_id } => {
                model = Some(ModelRef {
                    provider: provider.clone(),
                    model_id: model_id.clone(),
                });
            }
            EntryKind::Message {
                message: AgentMessage::Assistant(assistant),
            } => {
                model = Some(ModelRef {
                    provider: assistant.provider.clone(),
                    model_id: assistant.model.clone(),
                });
            }
            _ => {}
        }
    }
    (thinking_level, model)
}

/// Honours the latest compaction on the path.
#[must_use]
pub fn build_context_entries(entries: &EntryIndex, leaf: Option<&EntryId>) -> Vec<SessionEntry> {
    select_context(&path_to(entries, leaf))
        .into_iter()
        .cloned()
        .collect()
}

/// Validate an inclusive anchor against the branch *before* the state record.
/// User turns are the only safe boundary: an assistant call and all its results stay together.
///
/// # Errors
/// Missing/off-branch anchors, stale indexes, and mid-turn anchors.
pub fn validate_compaction_anchor(
    path: &[&SessionEntry],
    anchor: &crate::entry::CompactionAnchor,
) -> Result<(), &'static str> {
    let Some(entry) = path.get(anchor.index).filter(|entry| entry.id == anchor.id) else {
        return Err("compaction anchor is missing, off branch, or has an invalid index");
    };
    if !matches!(entry.message(), Some(AgentMessage::User(_))) {
        return Err("compaction anchor must start a user turn, never split tool calls and results");
    }
    // A malformed transcript may place a user message before pending results.
    let mut pending = HashSet::new();
    for earlier in &path[..anchor.index] {
        match earlier.message() {
            Some(AgentMessage::Assistant(assistant)) => {
                for block in &assistant.content {
                    if let AssistantContent::ToolCall(call) = block {
                        pending.insert(call.id.as_str());
                    }
                }
            }
            Some(AgentMessage::ToolResult(result)) => {
                pending.remove(result.tool_call_id.as_str());
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        return Err("compaction anchor splits pending tool calls and results");
    }
    Ok(())
}

fn select_context_with_errors<'a>(
    path: &[&'a SessionEntry],
) -> (Vec<&'a SessionEntry>, Vec<String>) {
    let mut selected = path.to_vec();
    let mut errors = Vec::new();
    // Process in order: a corrupt later record must never replace an earlier valid view.
    for (position, entry) in path.iter().enumerate() {
        let anchor_index = match &entry.kind {
            EntryKind::CompactionState(state) => {
                if let Err(error) =
                    validate_compaction_anchor(&path[..position], &state.anchor_message)
                {
                    errors.push(format!("Ignored invalid compaction {}: {error}", entry.id));
                    continue;
                }
                state.anchor_message.index
            }
            EntryKind::Compaction(legacy) => {
                // Old writers used their own record id for an empty retained range.
                if legacy.first_kept_entry_id == entry.id
                    || legacy.first_kept_entry_id.as_str().is_empty()
                {
                    position
                } else {
                    let Some(index) = path[..position]
                        .iter()
                        .position(|e| e.id == legacy.first_kept_entry_id)
                    else {
                        errors.push(format!(
                            "Ignored invalid legacy compaction {}: missing anchor",
                            entry.id
                        ));
                        continue;
                    };
                    index
                }
            }
            _ => continue,
        };
        selected = vec![*entry];
        selected.extend(
            path[anchor_index..position]
                .iter()
                .copied()
                .filter(|e| !e.is_system_message()),
        );
        selected.extend_from_slice(&path[position + 1..]);
    }
    // Invalid records are diagnostics, not selected context entries. Avoid
    // identifying records by substring when ids can be arbitrary strings.
    selected.retain(|entry| {
        !errors.iter().any(|error| {
            error.starts_with(&format!("Ignored invalid compaction {}:", entry.id))
                || error.starts_with(&format!("Ignored invalid legacy compaction {}:", entry.id))
        })
    });
    (selected, errors)
}

fn select_context<'a>(path: &[&'a SessionEntry]) -> Vec<&'a SessionEntry> {
    select_context_with_errors(path).0
}

/// Converts entries to context messages. `compaction_system` is the checkpoint used when a
/// compaction entry has no `systemMessage` (projection passes `None`).
#[must_use]
pub fn entry_to_context_messages(
    entry: &SessionEntry,
    compaction_system: Option<&SystemMessage>,
) -> Vec<AgentMessage> {
    let timestamp = entry.timestamp.to_ms().unwrap_or(0);
    match &entry.kind {
        EntryKind::Message { message } => vec![message.clone()],
        EntryKind::CustomMessage {
            custom_type,
            content,
            display,
            details,
        } => vec![custom_message(
            custom_type,
            content.clone(),
            *display,
            details.clone(),
            timestamp,
        )],
        EntryKind::BranchSummary(summary) if !summary.summary.is_empty() => {
            vec![branch_summary_message(
                &summary.summary,
                Some(summary.from_id.as_str()),
                timestamp,
            )]
        }
        EntryKind::CompactionState(state) => vec![AgentMessage::User(oino_ai::UserMessage {
            content: UserContentField::Text(state.injected_text()),
            timestamp,
        })],
        EntryKind::Compaction(compaction) => {
            let system = compaction.system_message.as_ref().or(compaction_system);
            system
                .cloned()
                .map(AgentMessage::System)
                .into_iter()
                .chain(std::iter::once(compaction_summary_message(
                    &compaction.summary,
                    compaction.tokens_before,
                    timestamp,
                )))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn user_blocks(content: &EditableContent) -> UserContentField {
    match content {
        EditableContent::Text(text) => UserContentField::Text(text.clone()),
        EditableContent::User(blocks) => UserContentField::Blocks(blocks.clone()),
        EditableContent::Assistant(blocks) => UserContentField::Blocks(
            blocks
                .iter()
                .filter_map(|block| match block {
                    AssistantContent::Text(text) => Some(UserContent::Text(text.clone())),
                    _ => None,
                })
                .collect(),
        ),
    }
}

fn result_blocks(content: &EditableContent) -> Vec<UserContent> {
    match user_blocks(content) {
        UserContentField::Text(text) => vec![UserContent::Text(TextContent::new(text))],
        UserContentField::Blocks(blocks) => blocks,
    }
}

fn assistant_blocks(content: &EditableContent) -> Vec<AssistantContent> {
    match content {
        EditableContent::Text(text) => vec![AssistantContent::Text(TextContent::new(text.clone()))],
        EditableContent::Assistant(blocks) => blocks.clone(),
        EditableContent::User(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserContent::Text(text) => Some(AssistantContent::Text(text.clone())),
                UserContent::Image(_) => None,
            })
            .collect(),
    }
}

/// Applies a replacement to one message, keeping role and metadata.
/// Blocks that the target role cannot hold (images on assistants, thinking on users) are
/// dropped.
fn replace_content(message: AgentMessage, replacement: &ContextReplacement) -> AgentMessage {
    let content = &replacement.content;
    match message {
        AgentMessage::User(mut user) => {
            user.content = user_blocks(content);
            AgentMessage::User(user)
        }
        AgentMessage::Custom(mut custom) => {
            custom.content = user_blocks(content);
            AgentMessage::Custom(custom)
        }
        AgentMessage::Assistant(mut assistant) => {
            assistant.content = assistant_blocks(content);
            AgentMessage::Assistant(assistant)
        }
        AgentMessage::ToolResult(mut result) => {
            result.content = result_blocks(content);
            AgentMessage::ToolResult(result)
        }
        other => other,
    }
}

fn project_entry(
    entry: &SessionEntry,
    edit: Option<&Option<ContextReplacement>>,
) -> Vec<AgentMessage> {
    let messages = entry_to_context_messages(entry, None);
    match edit {
        None => messages,
        Some(None) => Vec::new(),
        Some(Some(replacement)) => messages
            .into_iter()
            .map(|message| replace_content(message, replacement))
            .collect(),
    }
}

/// Context selection, latest context edit per target (among the
/// selected entries), entry → messages.
#[must_use]
pub fn build_session_projection(entries: &EntryIndex, leaf: Option<&EntryId>) -> SessionProjection {
    let path = path_to(entries, leaf);
    let (thinking_level, model) = context_settings(&path);
    let (selected, errors) = select_context_with_errors(&path);
    let mut edits: HashMap<&EntryId, &Option<ContextReplacement>> = HashMap::new();
    for entry in &selected {
        if let EntryKind::ContextEdit {
            target_id,
            replacement,
        } = &entry.kind
        {
            edits.insert(target_id, replacement);
        }
    }
    let projected: Vec<ProjectedEntry> = selected
        .iter()
        .enumerate()
        .map(|(index, entry)| ProjectedEntry {
            source_entry: (*entry).clone(),
            messages: if matches!(
                entry.kind,
                EntryKind::Compaction(_) | EntryKind::CompactionState(_)
            ) && index > 0
            {
                Vec::new()
            } else {
                project_entry(entry, edits.get(&entry.id).copied())
            },
        })
        .collect();
    let messages = projected
        .iter()
        .flat_map(|entry| entry.messages.iter().cloned())
        .collect();
    SessionProjection {
        errors,
        entries: projected,
        messages,
        thinking_level,
        model,
    }
}

/// B u i l d   S e s s i o n   C o n t e x t.
#[must_use]
pub fn build_session_context(entries: &EntryIndex, leaf: Option<&EntryId>) -> SessionContext {
    let SessionProjection {
        messages,
        thinking_level,
        model,
        ..
    } = build_session_projection(entries, leaf);
    SessionContext {
        messages,
        thinking_level,
        model,
    }
}

/// Folded system checkpoint of a projection.
#[must_use]
pub fn current_system_message(messages: &[AgentMessage]) -> Option<SystemMessage> {
    SystemState::fold(messages.iter().filter_map(AgentMessage::as_system)).into_message()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn index(lines: &serde_json::Value) -> EntryIndex {
        serde_json::from_value::<Vec<SessionEntry>>(lines.clone())
            .unwrap()
            .into_iter()
            .collect()
    }

    fn user(id: &str, parent: Option<&str>, text: &str) -> serde_json::Value {
        json!({"type":"message","id":id,"parentId":parent,"timestamp":"2024-01-01T00:00:00.000Z",
            "message":{"role":"user","content":text,"timestamp":1}})
    }

    fn texts(context: &SessionContext) -> Vec<String> {
        context
            .messages
            .iter()
            .map(|m| match m {
                AgentMessage::User(u) => u.content.text(""),
                other => other.role().to_owned(),
            })
            .collect()
    }

    #[test]
    fn compaction_keeps_range_and_drops_system() {
        let entries = index(&json!([
            {"type":"message","id":"s","parentId":null,"timestamp":"2024-01-01T00:00:00.000Z",
             "message":{"role":"system","content":"sys","timestamp":0}},
            user("a", Some("s"), "a"),
            user("b", Some("a"), "b"),
            {"type":"message","id":"s2","parentId":"b","timestamp":"2024-01-01T00:00:00.000Z",
             "message":{"role":"system","content":"patch","timestamp":0}},
            {"type":"compaction","id":"c","parentId":"s2","timestamp":"2024-01-01T00:00:01.000Z",
             "summary":"sum","firstKeptEntryId":"b","tokensBefore":5,
             "systemMessage":{"role":"system","content":"sys\n\npatch","timestamp":1000}},
            user("d", Some("c"), "d"),
        ]));
        let context = build_session_context(&entries, entries.last_id());
        assert_eq!(texts(&context), ["system", "compactionSummary", "b", "d"]);
        let selected: Vec<_> = build_context_entries(&entries, Some(&"d".into()))
            .into_iter()
            .map(|e| e.id.to_string())
            .collect();
        assert_eq!(selected, ["c", "b", "d"]);
        assert_eq!(build_session_context(&entries, None).messages, []);
    }

    #[test]
    fn context_edits_are_branch_relative() {
        let entries = index(&json!([
            user("a", None, "a"),
            {"type":"context_edit","id":"e","parentId":"a","timestamp":"2024-01-01T00:00:00.000Z","targetId":"a",
             "replacement":{"content":"edited"}},
            {"type":"context_edit","id":"f","parentId":"e","timestamp":"2024-01-01T00:00:00.000Z","targetId":"a","replacement":null},
        ]));
        assert_eq!(
            texts(&build_session_context(&entries, Some(&"e".into()))),
            ["edited"]
        );
        assert_eq!(
            build_session_context(&entries, Some(&"f".into())).messages,
            []
        );
        assert_eq!(
            texts(&build_session_context(&entries, Some(&"a".into()))),
            ["a"]
        );
    }

    #[test]
    fn settings_come_from_full_path() {
        let entries = index(&json!([
            {"type":"model_change","id":"m","parentId":null,"timestamp":"t","provider":"p","modelId":"x"},
            {"type":"thinking_level_change","id":"t","parentId":"m","timestamp":"t","thinkingLevel":"high"},
        ]));
        let context = build_session_context(&entries, entries.last_id());
        assert_eq!(context.thinking_level, "high");
        assert_eq!(context.model.map(|m| m.model_id), Some("x".to_owned()));
        let path = path_to(&entries, entries.last_id());
        assert_eq!(branch_selection(&path).model_entry_id, Some("m".into()));
    }

    #[test]
    fn cycles_terminate() {
        let entries = index(&json!([
            user("a", Some("b"), "a"),
            user("b", Some("a"), "b")
        ]));
        assert_eq!(path_to(&entries, Some(&"b".into())).len(), 2);
    }
}
