#![doc = r#"Append-only Oino session trees and JSONL persistence.

Sessions store runtime history as immutable entries linked by parent identifiers. The manager
tracks a current leaf for navigation, branching, compaction reconstruction, labels, and
context building. It does not own providers, tools, UI, or the agent loop.

## Boundary

`oino-session` owns the in-memory session tree and JSONL load/save format. It records
messages, model/thinking changes, compaction summaries, branch summaries, labels, title
and cwd updates, extension-owned custom entries, and the current leaf. It does not run
providers or tools, decide TUI navigation, choose storage roots, expand resources, or
manage auth/settings; `oino-harness`, `oino-app`, and `oino-tui` turn session data into
runtime behavior and user workflows.

## Public API map

- [`SessionHeader`] stores file-level metadata: format version, session id, display
  name, and starting cwd.
- [`SessionEntryKind`] and [`SessionEntry`] are the append-only tree records. Entries
  are immutable and linked by parent id; moving the current leaf chooses which branch is
  active. Format version 2 adds `System` transcript messages, `ContextEdit`
  (omit/replace a message at context-build time), `Usage` records, and
  `first_kept`-based compaction; v1 files still load (`replaces` compactions are
  resolved at reconstruction).
- [`SessionManager`] appends entries, branches/reset leaves, exposes children/tree data,
  finds labels, filters extension entries, and builds [`SessionContext`] for the active
  branch.
- [`SessionContext`] is the reconstructed model context plus latest model and thinking
  settings; compaction keeps a kept-boundary tail behind a summary message and folds
  dropped `System` messages into one replayed leading system message.
- [`SessionRepository`] owns directory-level helpers for creating, opening, and listing
  `.jsonl` session files; callers own higher-level sorting, previews, and blank-session
  policies.
- [`SessionError`] and [`SessionResult`] keep missing-entry, invalid-record, I/O, and
  JSON failures typed for app/TUI error handling.

## Contributor rules

Keep persistence deterministic and inspectable: a JSONL file starts with one header
record followed by entry records. Preserve branch reconstruction semantics when adding
new entry kinds, and decide explicitly whether a new entry affects model context,
metadata, labels, extension state, or only UI/session bookkeeping. Extension-specific
state should remain typed as [`oino_extension_core::ExtensionSessionEntry`] so sessions
can be inspected without loading extension runtime code. Update the [sessions guide]
when user-visible save, branch, title, import, delete, or migration behavior changes.

[sessions guide]: ../../../docs/sessions.md
"#]
#![forbid(unsafe_code)]

use oino_extension_core::{ExtensionId, ExtensionSessionEntry};
use oino_types::{ContentBlock, Message, Model, OinoId, ThinkingLevel, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncWriteExt, BufWriter},
};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("entry not found: {0}")]
    EntryNotFound(OinoId),
    #[error("invalid jsonl record: {0}")]
    InvalidRecord(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type SessionResult<T> = Result<T, SessionError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionHeader {
    pub version: u32,
    pub session_id: OinoId,
    pub name: String,
    pub cwd: PathBuf,
}

impl SessionHeader {
    #[must_use]
    pub fn new(name: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            version: 2,
            session_id: Uuid::new_v4(),
            name: name.into(),
            cwd,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
        /// First entry on the active path retained after this compaction.
        /// Equal to the compaction entry's own id for retain-none. `None` only
        /// for v1-loaded entries, which carry `replaces` instead.
        first_kept: Option<OinoId>,
        #[serde(default)]
        tokens_before: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        /// v1 on-disk shape only; never written for new entries.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        replaces: Vec<OinoId>,
    },
    /// Context edit: omit (`None`) or replace (`Some`) the content of the
    /// message identified by `target` during context reconstruction. Raw
    /// history is untouched.
    ContextEdit {
        target: OinoId,
        replacement: Option<Vec<ContentBlock>>,
    },
    /// Token/cost usage record (e.g. `cache_warm`, `compaction`). Never enters
    /// model context.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEntry {
    pub id: OinoId,
    pub parent: Option<OinoId>,
    pub kind: SessionEntryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum JsonlRecord {
    Header {
        header: SessionHeader,
        leaf_id: Option<OinoId>,
    },
    Entry {
        entry: Box<SessionEntry>,
    },
}

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub messages: Vec<Message>,
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
    /// Non-fatal reconstruction notes (e.g. a compaction `first_kept` id that
    /// is no longer on the active path).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SessionManager {
    header: SessionHeader,
    entries: BTreeMap<OinoId, SessionEntry>,
    children: BTreeMap<Option<OinoId>, BTreeSet<OinoId>>,
    leaf_id: Option<OinoId>,
}

impl SessionManager {
    #[must_use]
    pub fn new(header: SessionHeader) -> Self {
        Self {
            header,
            entries: BTreeMap::new(),
            children: BTreeMap::new(),
            leaf_id: None,
        }
    }

    #[must_use]
    pub fn header(&self) -> &SessionHeader {
        &self.header
    }
    #[must_use]
    pub fn get_leaf_id(&self) -> Option<OinoId> {
        self.leaf_id
    }
    #[must_use]
    pub fn get_leaf_entry(&self) -> Option<&SessionEntry> {
        self.leaf_id.and_then(|id| self.entries.get(&id))
    }
    #[must_use]
    pub fn get_entry(&self, id: OinoId) -> Option<&SessionEntry> {
        self.entries.get(&id)
    }
    #[must_use]
    pub fn get_entries(&self) -> Vec<SessionEntry> {
        self.entries.values().cloned().collect()
    }

    pub fn append(&mut self, kind: SessionEntryKind) -> OinoId {
        self.append_to(self.leaf_id, kind)
    }

    pub fn append_to(&mut self, parent: Option<OinoId>, kind: SessionEntryKind) -> OinoId {
        let id = Uuid::new_v4();
        let entry = SessionEntry { id, parent, kind };
        self.children.entry(parent).or_default().insert(id);
        self.entries.insert(id, entry);
        self.leaf_id = Some(id);
        id
    }

    pub fn append_message(&mut self, message: Message) -> OinoId {
        self.append(SessionEntryKind::Message { message })
    }
    pub fn append_model(&mut self, model: Model) -> OinoId {
        self.append(SessionEntryKind::ModelChange { model })
    }
    pub fn append_thinking_level(&mut self, thinking_level: ThinkingLevel) -> OinoId {
        self.append(SessionEntryKind::ThinkingLevelChange { thinking_level })
    }
    /// Append a compaction entry. `first_kept` is the id of the first entry on
    /// the active path to retain; `None` means retain-none and is stored as the
    /// compaction entry's own id.
    pub fn append_compaction(
        &mut self,
        summary: impl Into<String>,
        first_kept: Option<OinoId>,
        tokens_before: u64,
    ) -> OinoId {
        let id = Uuid::new_v4();
        let entry = SessionEntry {
            id,
            parent: self.leaf_id,
            kind: SessionEntryKind::Compaction {
                summary: summary.into(),
                first_kept: Some(first_kept.unwrap_or(id)),
                tokens_before,
                details: None,
                replaces: Vec::new(),
            },
        };
        self.children.entry(self.leaf_id).or_default().insert(id);
        self.entries.insert(id, entry);
        self.leaf_id = Some(id);
        id
    }

    pub fn append_context_edit(
        &mut self,
        target: OinoId,
        replacement: Option<Vec<ContentBlock>>,
    ) -> OinoId {
        self.append(SessionEntryKind::ContextEdit {
            target,
            replacement,
        })
    }

    pub fn append_usage(&mut self, kind: impl Into<String>, model: Model, usage: Usage) -> OinoId {
        self.append(SessionEntryKind::Usage {
            usage_kind: kind.into(),
            model,
            usage,
        })
    }

    /// Usage entries recorded on the active branch, in order.
    #[must_use]
    pub fn usage_entries_on_branch(&self) -> Vec<(String, Model, Usage)> {
        self.branch_entry_refs(self.leaf_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| match &entry.kind {
                SessionEntryKind::Usage {
                    usage_kind,
                    model,
                    usage,
                } => Some((usage_kind.clone(), model.clone(), usage.clone())),
                _ => None,
            })
            .collect()
    }
    pub fn append_label(&mut self, label: impl Into<String>) -> OinoId {
        self.append(SessionEntryKind::Label {
            label: label.into(),
        })
    }

    pub fn append_extension_custom(&mut self, entry: ExtensionSessionEntry) -> OinoId {
        self.append(SessionEntryKind::ExtensionCustom {
            entry: Box::new(entry),
        })
    }

    #[must_use]
    pub fn extension_custom_entries(&self, owner: &ExtensionId) -> Vec<ExtensionSessionEntry> {
        self.branch_entry_refs(self.leaf_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| match &entry.kind {
                SessionEntryKind::ExtensionCustom { entry }
                    if &entry.owner_extension_id == owner =>
                {
                    Some((**entry).clone())
                }
                _ => None,
            })
            .collect()
    }

    pub fn branch(&mut self, from: OinoId) -> SessionResult<()> {
        if !self.entries.contains_key(&from) {
            return Err(SessionError::EntryNotFound(from));
        }
        self.leaf_id = Some(from);
        Ok(())
    }

    pub fn branch_with_summary(
        &mut self,
        from: OinoId,
        summary: impl Into<String>,
    ) -> SessionResult<OinoId> {
        if !self.entries.contains_key(&from) {
            return Err(SessionError::EntryNotFound(from));
        }
        Ok(self.append_to(
            Some(from),
            SessionEntryKind::BranchSummary {
                summary: summary.into(),
            },
        ))
    }

    pub fn reset_leaf(&mut self, leaf: Option<OinoId>) -> SessionResult<()> {
        if let Some(id) = leaf {
            if !self.entries.contains_key(&id) {
                return Err(SessionError::EntryNotFound(id));
            }
        }
        self.leaf_id = leaf;
        Ok(())
    }

    #[must_use]
    pub fn get_children(&self, parent: Option<OinoId>) -> Vec<OinoId> {
        self.children
            .get(&parent)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn get_branch(&self, leaf: Option<OinoId>) -> SessionResult<Vec<SessionEntry>> {
        self.branch_entry_refs(leaf)
            .map(|entries| entries.into_iter().cloned().collect())
    }

    fn branch_entry_refs(&self, leaf: Option<OinoId>) -> SessionResult<Vec<&SessionEntry>> {
        let mut out = Vec::new();
        let mut cursor = leaf;
        while let Some(id) = cursor {
            let Some(entry) = self.entries.get(&id) else {
                return Err(SessionError::EntryNotFound(id));
            };
            out.push(entry);
            cursor = entry.parent;
        }
        out.reverse();
        Ok(out)
    }

    pub fn get_tree(&self) -> BTreeMap<Option<OinoId>, Vec<OinoId>> {
        self.children
            .iter()
            .map(|(parent, children)| (*parent, children.iter().copied().collect()))
            .collect()
    }

    #[must_use]
    pub fn get_session_name(&self) -> String {
        let mut cursor = self.leaf_id;
        while let Some(id) = cursor {
            let Some(entry) = self.entries.get(&id) else {
                break;
            };
            if let SessionEntryKind::SessionInfo {
                name: Some(name), ..
            } = &entry.kind
            {
                return name.clone();
            }
            cursor = entry.parent;
        }
        self.header.name.clone()
    }

    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.entries
            .values()
            .filter_map(|entry| match &entry.kind {
                SessionEntryKind::Label { label } => Some(label.clone()),
                _ => None,
            })
            .collect()
    }

    /// Reconstruct the model context for the active branch (root to leaf).
    ///
    /// Compaction keeps the tail from `first_kept` (inclusive) up to the
    /// compaction entry, plus everything after it, behind a
    /// `CompactionSummary` message. `System` messages dropped by the
    /// compaction are folded into one replayed leading `System` so the prompt
    /// and tool declarations survive. `ContextEdit` entries apply last-wins by
    /// target (`None` omits, `Some` replaces content). `Usage` entries never
    /// produce messages.
    pub fn build_session_context(&self) -> SessionResult<SessionContext> {
        let branch = self.branch_entry_refs(self.leaf_id)?;
        let mut warnings: Vec<String> = Vec::new();
        let mut model = None;
        let mut thinking_level = None;
        let mut edits: BTreeMap<OinoId, Option<Vec<ContentBlock>>> = BTreeMap::new();
        for entry in &branch {
            match &entry.kind {
                SessionEntryKind::ModelChange { model: changed } => model = Some(changed.clone()),
                SessionEntryKind::ThinkingLevelChange {
                    thinking_level: changed,
                } => thinking_level = Some(*changed),
                SessionEntryKind::ContextEdit {
                    target,
                    replacement,
                } => {
                    edits.insert(*target, replacement.clone());
                }
                _ => {}
            }
        }

        let mut messages: Vec<Message> = Vec::new();
        let compaction_index = branch
            .iter()
            .rposition(|entry| matches!(entry.kind, SessionEntryKind::Compaction { .. }));
        if let Some(compaction_index) = compaction_index {
            let compaction = branch[compaction_index];
            let SessionEntryKind::Compaction {
                summary,
                first_kept,
                replaces,
                ..
            } = &compaction.kind
            else {
                return Err(SessionError::InvalidRecord(
                    "compaction position mismatch".into(),
                ));
            };
            // v1 shape resolves `first_kept` from `replaces`: the first path
            // entry after the last replaced entry before the compaction.
            let resolved_first_kept = first_kept.or_else(|| {
                let last_replaced = branch[..compaction_index]
                    .iter()
                    .rposition(|entry| replaces.contains(&entry.id));
                match last_replaced {
                    Some(index) if index + 1 < compaction_index => Some(branch[index + 1].id),
                    _ => Some(compaction.id),
                }
            });
            let kept_start = match resolved_first_kept {
                Some(id) if id == compaction.id => compaction_index,
                Some(id) => match branch[..compaction_index]
                    .iter()
                    .position(|entry| entry.id == id)
                {
                    Some(index) => index,
                    None => {
                        warnings.push(format!(
                            "compaction first_kept {id} is not on the active path; retaining none"
                        ));
                        compaction_index
                    }
                },
                None => compaction_index,
            };
            // Fold System messages dropped by the compaction into one replayed
            // leading System message so prompt and tools survive.
            let dropped_messages: Vec<Message> = branch[..kept_start]
                .iter()
                .filter_map(|entry| match &entry.kind {
                    SessionEntryKind::Message { message }
                    | SessionEntryKind::CustomMessage { message } => Some(message.clone()),
                    _ => None,
                })
                .collect();
            if let Some(system) = oino_types::transcript::current_system(&dropped_messages) {
                messages.push(system);
            }
            messages.push(Message::CompactionSummary {
                id: compaction.id,
                summary: summary.clone(),
            });
            for entry in branch[kept_start..compaction_index]
                .iter()
                .chain(branch[compaction_index + 1..].iter())
            {
                push_context_message(&mut messages, entry);
            }
        } else {
            for entry in &branch {
                push_context_message(&mut messages, entry);
            }
        }

        // Apply context edits last-wins by target.
        if !edits.is_empty() {
            messages.retain_mut(|message| match edits.get(&message.id()) {
                Some(None) => false,
                Some(Some(blocks)) => {
                    match message {
                        Message::User { content, .. }
                        | Message::Assistant { content, .. }
                        | Message::ToolResult { content, .. } => *content = blocks.clone(),
                        _ => {}
                    }
                    true
                }
                None => true,
            });
        }

        Ok(SessionContext {
            messages,
            model,
            thinking_level,
            warnings,
        })
    }

    #[must_use]
    pub fn is_persisted(&self) -> bool {
        false
    }

    pub async fn save_jsonl(&mut self, path: impl AsRef<Path>) -> SessionResult<()> {
        // The writer only emits v2 shapes; always stamp version 2, including
        // for sessions loaded from v1 files.
        self.header.version = 2;
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).await?;
        }
        let mut file = BufWriter::new(fs::File::create(path).await?);
        let header = JsonlRecord::Header {
            header: self.header.clone(),
            leaf_id: self.leaf_id,
        };
        file.write_all(serde_json::to_string(&header)?.as_bytes())
            .await?;
        file.write_all(b"\n").await?;
        for entry in self.entries.values() {
            let record = JsonlRecord::Entry {
                entry: Box::new(entry.clone()),
            };
            file.write_all(serde_json::to_string(&record)?.as_bytes())
                .await?;
            file.write_all(b"\n").await?;
        }
        file.flush().await?;
        Ok(())
    }

    pub async fn load_jsonl(path: impl AsRef<Path>) -> SessionResult<Self> {
        let content = fs::read_to_string(path).await?;
        let mut lines = content.lines();
        let Some(first) = lines.next() else {
            return Err(SessionError::InvalidRecord("missing header".into()));
        };
        let JsonlRecord::Header { header, leaf_id } = serde_json::from_str(first)? else {
            return Err(SessionError::InvalidRecord(
                "first record must be header".into(),
            ));
        };
        if header.version > 2 {
            return Err(SessionError::InvalidRecord(format!(
                "unsupported session version {}",
                header.version
            )));
        }
        let mut manager = Self::new(header);
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<JsonlRecord>(line)? {
                JsonlRecord::Entry { entry } => {
                    manager
                        .children
                        .entry(entry.parent)
                        .or_default()
                        .insert(entry.id);
                    manager.entries.insert(entry.id, *entry);
                }
                JsonlRecord::Header { .. } => {
                    return Err(SessionError::InvalidRecord("duplicate header".into()))
                }
            }
        }
        manager.reset_leaf(leaf_id)?;
        Ok(manager)
    }
}

fn push_context_message(messages: &mut Vec<Message>, entry: &SessionEntry) {
    match &entry.kind {
        SessionEntryKind::Message { message } | SessionEntryKind::CustomMessage { message } => {
            messages.push(message.clone());
        }
        SessionEntryKind::BranchSummary { summary } => {
            messages.push(Message::BranchSummary {
                id: entry.id,
                summary: summary.clone(),
            });
        }
        _ => {}
    }
}

pub struct SessionRepository {
    root: PathBuf,
}
impl SessionRepository {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub async fn create(
        &self,
        name: impl Into<String>,
        cwd: PathBuf,
    ) -> SessionResult<(PathBuf, SessionManager)> {
        fs::create_dir_all(&self.root).await?;
        let mut manager = SessionManager::new(SessionHeader::new(name, cwd));
        let path = self
            .root
            .join(format!("{}.jsonl", manager.header.session_id));
        manager.save_jsonl(&path).await?;
        Ok((path, manager))
    }
    pub async fn open(&self, path: impl AsRef<Path>) -> SessionResult<SessionManager> {
        SessionManager::load_jsonl(path).await
    }
    pub async fn list(&self) -> SessionResult<Vec<PathBuf>> {
        let mut out = Vec::new();
        let mut dir = match fs::read_dir(&self.root).await {
            Ok(dir) => dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(err) => return Err(err.into()),
        };
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_extension_core::{Provenance, SourceDescriptor, SourceKind, SourceScope};
    use oino_types::StopReason;

    fn manager() -> SessionManager {
        SessionManager::new(SessionHeader::new("test", PathBuf::from("/tmp")))
    }

    #[test]
    fn branch_navigation_and_context() {
        let mut session = manager();
        let root = session.append_message(Message::user_text("root"));
        let a = session.append_message(Message::assistant_text("a", StopReason::EndTurn));
        let branched = session.branch(root);
        assert!(branched.is_ok());
        let b = session.append_message(Message::assistant_text("b", StopReason::EndTurn));
        let branch = session.get_branch(Some(b));
        let branch = match branch {
            Ok(branch) => branch,
            Err(err) => panic!("branch failed: {err}"),
        };
        assert_eq!(branch.len(), 2);
        assert!(session.get_children(Some(root)).contains(&a));
    }

    #[test]
    fn compaction_replaces_prior_context() {
        let mut session = manager();
        let first = session.append_message(Message::user_text("old"));
        session.append_compaction("summary", None, 0);
        let _ = first;
        session.append_message(Message::user_text("new"));
        let context = session.build_session_context();
        let context = match context {
            Ok(context) => context,
            Err(err) => panic!("context failed: {err}"),
        };
        assert_eq!(context.messages.len(), 2);
        assert!(matches!(
            context.messages[0],
            Message::CompactionSummary { .. }
        ));
    }

    #[tokio::test]
    async fn extension_custom_entries_round_trip_without_runtime_code() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("session.jsonl");
        let extension_id =
            ExtensionId::new("acme.persist").unwrap_or_else(|err| panic!("id: {err}"));
        let mut session = manager();
        session.append_extension_custom(ExtensionSessionEntry {
            owner_extension_id: extension_id.clone(),
            key: "pane".into(),
            schema_version: 2,
            payload: serde_json::json!({ "visible": true }),
            provenance: Some(Provenance {
                source: SourceDescriptor {
                    scope: SourceScope::Project,
                    kind: SourceKind::LocalExtension,
                    path: None,
                    registry: None,
                },
                package_id: None,
                extension_id: Some(extension_id.clone()),
                package_version: None,
                manifest_path: None,
            }),
        });
        session.append_message(Message::user_text("normal context"));
        session
            .save_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("save failed: {err}"));
        let loaded = SessionManager::load_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("load failed: {err}"));
        let entries = loaded.extension_custom_entries(&extension_id);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].schema_version, 2);
        assert_eq!(entries[0].payload["visible"], true);
        let context = loaded
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        assert_eq!(context.messages.len(), 1);
    }

    #[tokio::test]
    async fn jsonl_round_trip() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("session.jsonl");
        let mut session = manager();
        session.append_label("important");
        session.append_message(Message::user_text("persist"));
        let saved = session.save_jsonl(&path).await;
        assert!(saved.is_ok());
        let loaded = SessionManager::load_jsonl(&path).await;
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(err) => panic!("load failed: {err}"),
        };
        assert_eq!(loaded.labels(), vec!["important".to_string()]);
        assert_eq!(loaded.get_entries().len(), session.get_entries().len());
    }

    #[tokio::test]
    async fn save_creates_parent_directory_and_missing_list_is_empty() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let missing_root = dir.path().join("missing");
        let repository = SessionRepository::new(&missing_root);
        let listed = repository.list().await;
        assert!(matches!(listed, Ok(items) if items.is_empty()));

        let path = missing_root.join("nested").join("session.jsonl");
        let mut session = manager();
        session.append_message(Message::user_text("persist"));
        let saved = session.save_jsonl(&path).await;
        assert!(saved.is_ok());
        assert!(path.exists());
    }

    fn system_message(content: &str, tools: Vec<oino_types::ToolDeclaration>) -> Message {
        Message::System {
            id: Uuid::new_v4(),
            content: content.into(),
            sections: Vec::new(),
            tools_added: tools,
            tools_removed: Vec::new(),
        }
    }

    fn tool_decl(name: &str) -> oino_types::ToolDeclaration {
        oino_types::ToolDeclaration {
            name: name.into(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn compaction_keeps_tail_from_first_kept() {
        let mut session = manager();
        session.append_message(Message::user_text("old-1"));
        let kept = session.append_message(Message::user_text("kept"));
        session.append_compaction("summary", Some(kept), 100);
        session.append_message(Message::user_text("new"));
        let context = session
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        assert!(matches!(
            context.messages[0],
            Message::CompactionSummary { .. }
        ));
        assert!(matches!(
            context.messages[1],
            Message::User { ref content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text == "kept")
        ));
        assert!(matches!(
            context.messages[2],
            Message::User { ref content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text == "new")
        ));
        assert_eq!(context.messages.len(), 3);
        assert!(context.warnings.is_empty());
    }

    #[test]
    fn compaction_retain_none_keeps_only_post_compaction() {
        let mut session = manager();
        session.append_message(Message::user_text("old"));
        session.append_compaction("summary", None, 0);
        session.append_message(Message::user_text("new"));
        let context = session
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        assert_eq!(context.messages.len(), 2);
        assert!(matches!(
            context.messages[0],
            Message::CompactionSummary { .. }
        ));
    }

    #[test]
    fn compaction_folds_dropped_system_into_replayed_leading() {
        let mut session = manager();
        session.append_message(system_message("base prompt", vec![tool_decl("read")]));
        session.append_message(Message::user_text("old"));
        session.append_compaction("summary", None, 0);
        session.append_message(Message::user_text("new"));
        let context = session
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        // Replayed leading System keeps prompt + tools; then summary; then tail.
        let Message::System {
            content,
            tools_added,
            ..
        } = &context.messages[0]
        else {
            panic!("expected replayed system message");
        };
        assert_eq!(content, "base prompt");
        assert_eq!(tools_added.len(), 1);
        assert_eq!(tools_added[0].name, "read");
        assert!(matches!(
            context.messages[1],
            Message::CompactionSummary { .. }
        ));
        assert_eq!(context.messages.len(), 3);
    }

    #[test]
    fn context_edit_omits_and_replaces_last_wins() {
        let mut session = manager();
        let target = Message::user_text("first");
        let target_id = target.id();
        let other = Message::user_text("other");
        let other_id = other.id();
        session.append_message(target);
        session.append_message(other);
        session.append_context_edit(
            target_id,
            Some(vec![ContentBlock::Text {
                text: "edited".into(),
                signature: None,
            }]),
        );
        session.append_context_edit(other_id, None);
        // Last-wins: revert the edit.
        session.append_context_edit(target_id, None);
        session.append_context_edit(
            target_id,
            Some(vec![ContentBlock::Text {
                text: "final".into(),
                signature: None,
            }]),
        );
        let context = session
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        let texts: Vec<String> = context
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::User { content, .. } => content.iter().find_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["final".to_string()]);
    }

    #[test]
    fn usage_entries_never_enter_context() {
        let mut session = manager();
        session.append_message(Message::user_text("hi"));
        session.append_usage(
            "compaction",
            Model::new("test", "faux"),
            Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
        );
        let context = session
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        assert_eq!(context.messages.len(), 1);
        let usage = session.usage_entries_on_branch();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].0, "compaction");
        assert_eq!(usage[0].2.input_tokens, 10);
    }

    #[tokio::test]
    async fn v1_session_loads_and_reconstructs_kept_tail() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("session.jsonl");
        let old_id = Uuid::new_v4();
        let kept_id = Uuid::new_v4();
        let new_id = Uuid::new_v4();
        let compaction_id = Uuid::new_v4();
        let user_entry = |id: Uuid, parent: Option<Uuid>, text: &str| {
            serde_json::json!({
                "record": "entry",
                "entry": {
                    "id": id,
                    "parent": parent,
                    "kind": {
                        "kind": "message",
                        "message": {
                            "kind": "user",
                            "id": id,
                            "content": [{"kind": "text", "text": text}]
                        }
                    }
                }
            })
            .to_string()
        };
        let file = [
            serde_json::json!({
                "record": "header",
                "header": {"version": 1, "session_id": "00000000-0000-0000-0000-000000000001", "name": "v1", "cwd": "/tmp"},
                "leaf_id": new_id
            })
            .to_string(),
            user_entry(old_id, None, "old"),
            user_entry(kept_id, Some(old_id), "kept"),
            serde_json::json!({
                "record": "entry",
                "entry": {
                    "id": compaction_id,
                    "parent": kept_id,
                    "kind": {"kind": "compaction", "summary": "v1 summary", "replaces": [old_id]}
                }
            })
            .to_string(),
            user_entry(new_id, Some(compaction_id), "new"),
            String::new(),
        ]
        .join("\n");
        std::fs::write(&path, file).unwrap_or_else(|err| panic!("write failed: {err}"));
        let loaded = SessionManager::load_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("v1 load failed: {err}"));
        let context = loaded
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        assert_eq!(context.messages.len(), 3);
        assert!(matches!(
            context.messages[0],
            Message::CompactionSummary { .. }
        ));
        let texts: Vec<String> = context
            .messages
            .iter()
            .skip(1)
            .filter_map(|message| match message {
                Message::User { content, .. } => content.iter().find_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["kept".to_string(), "new".to_string()]);
    }

    #[tokio::test]
    async fn version_above_two_is_rejected() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            "{\"record\":\"header\",\"header\":{\"version\":3,\"session_id\":\"00000000-0000-0000-0000-000000000001\",\"name\":\"v3\",\"cwd\":\"/tmp\"},\"leaf_id\":null}\n",
        )
        .unwrap_or_else(|err| panic!("write failed: {err}"));
        match SessionManager::load_jsonl(&path).await {
            Err(SessionError::InvalidRecord(message)) => {
                assert!(message.contains("unsupported session version 3"));
            }
            other => panic!("expected invalid record, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn v2_round_trip_preserves_compaction_fields() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("session.jsonl");
        let mut session = manager();
        let kept_message = Message::user_text("kept");
        let kept_message_id = kept_message.id();
        let kept = session.append_message(kept_message);
        session.append_compaction("v2 summary", Some(kept), 42);
        session.append_context_edit(kept_message_id, None);
        session
            .save_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("save failed: {err}"));
        let loaded = SessionManager::load_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("load failed: {err}"));
        assert_eq!(loaded.header().version, 2);
        let entries = loaded.get_entries();
        let compaction = entries
            .iter()
            .find_map(|entry| match &entry.kind {
                SessionEntryKind::Compaction {
                    summary,
                    first_kept,
                    tokens_before,
                    ..
                } => Some((summary.clone(), *first_kept, *tokens_before)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("compaction missing"));
        assert_eq!(compaction.0, "v2 summary");
        assert_eq!(compaction.1, Some(kept));
        assert_eq!(compaction.2, 42);
        let context = loaded
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        // The kept message was then omitted by the context edit.
        assert_eq!(context.messages.len(), 1);
        assert!(matches!(
            context.messages[0],
            Message::CompactionSummary { .. }
        ));
    }

    #[tokio::test]
    async fn v1_load_then_save_writes_version_two() {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("session.jsonl");
        let user_id = Uuid::new_v4();
        std::fs::write(
            &path,
            format!(
                "{{\"record\":\"header\",\"header\":{{\"version\":1,\"session_id\":\"00000000-0000-0000-0000-000000000001\",\"name\":\"v1\",\"cwd\":\"/tmp\"}},\"leaf_id\":\"{user_id}\"}}\n{{\"record\":\"entry\",\"entry\":{{\"id\":\"{user_id}\",\"parent\":null,\"kind\":{{\"kind\":\"message\",\"message\":{{\"kind\":\"user\",\"id\":\"{user_id}\",\"content\":[{{\"kind\":\"text\",\"text\":\"hi\"}}]}}}}}}}}\n"
            ),
        )
        .unwrap_or_else(|err| panic!("write failed: {err}"));
        let mut loaded = SessionManager::load_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("load failed: {err}"));
        let before = loaded
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        loaded
            .save_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("save failed: {err}"));
        let reloaded = SessionManager::load_jsonl(&path)
            .await
            .unwrap_or_else(|err| panic!("reload failed: {err}"));
        assert_eq!(reloaded.header().version, 2);
        let after = reloaded
            .build_session_context()
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        assert_eq!(before.messages, after.messages);
    }
}
