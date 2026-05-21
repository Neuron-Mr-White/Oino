#![doc = r#"Append-only Oino session trees and JSONL persistence.

Sessions store runtime history as immutable entries linked by parent identifiers. The manager
tracks a current leaf for navigation, branching, compaction reconstruction, labels, and
context building. It does not own providers, tools, UI, or the agent loop.
"#]
#![forbid(unsafe_code)]

use oino_extension_core::{ExtensionId, ExtensionSessionEntry};
use oino_types::{Message, Model, OinoId, ThinkingLevel};
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
            version: 1,
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
        replaces: Vec<OinoId>,
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
        entry: SessionEntry,
    },
}

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub messages: Vec<Message>,
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
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
    pub fn append_compaction(
        &mut self,
        summary: impl Into<String>,
        replaces: Vec<OinoId>,
    ) -> OinoId {
        self.append(SessionEntryKind::Compaction {
            summary: summary.into(),
            replaces,
        })
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

    pub fn build_session_context(&self) -> SessionResult<SessionContext> {
        let branch = self.branch_entry_refs(self.leaf_id)?;
        let mut messages = Vec::new();
        let mut model = None;
        let mut thinking_level = None;
        for entry in branch {
            match &entry.kind {
                SessionEntryKind::Message { message }
                | SessionEntryKind::CustomMessage { message } => messages.push(message.clone()),
                SessionEntryKind::ModelChange { model: changed } => model = Some(changed.clone()),
                SessionEntryKind::ThinkingLevelChange {
                    thinking_level: changed,
                } => thinking_level = Some(*changed),
                SessionEntryKind::Compaction { summary, .. } => {
                    messages.clear();
                    messages.push(Message::CompactionSummary {
                        id: entry.id,
                        summary: summary.clone(),
                    });
                }
                SessionEntryKind::BranchSummary { summary } => {
                    messages.push(Message::BranchSummary {
                        id: entry.id,
                        summary: summary.clone(),
                    })
                }
                SessionEntryKind::Custom { .. }
                | SessionEntryKind::ExtensionCustom { .. }
                | SessionEntryKind::Label { .. }
                | SessionEntryKind::SessionInfo { .. }
                | SessionEntryKind::LeafMove { .. } => {}
            }
        }
        Ok(SessionContext {
            messages,
            model,
            thinking_level,
        })
    }

    #[must_use]
    pub fn is_persisted(&self) -> bool {
        false
    }

    pub async fn save_jsonl(&self, path: impl AsRef<Path>) -> SessionResult<()> {
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
                entry: entry.clone(),
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
                    manager.entries.insert(entry.id, entry);
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
        let manager = SessionManager::new(SessionHeader::new(name, cwd));
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
        session.append_compaction("summary", vec![first]);
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
}
