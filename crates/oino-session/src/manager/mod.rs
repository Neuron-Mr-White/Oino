//! [`SessionManager`]: append-only session tree persistence (pi `SessionManager`, D20).
//!
//! `session_dir` arguments are always the **sessions root** (e.g. `~/.oino/sessions`); new
//! files go to `<root>/--<cwd>--/<ts>_<id>.jsonl` (session-v3 §6.1).

mod files;
mod list;
mod tree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use oino_ai::{
    AgentMessage, JsonValue, SystemMessage, TextContent, Usage, UserContent, UserContentField,
};
use uuid::Uuid;

pub use files::{
    LEGACY_DIR_NAME, MigratedSession, cwd_session_dir, default_agent_dir, default_sessions_root,
    encode_cwd, migrate_legacy_file, resolve_path, session_file_name, validate_session_id,
};
pub use list::{ProgressSink, SessionInfo, session_info};
pub use tree::SessionTreeNode;

use crate::context::{
    EntryIndex, SessionContext, SessionProjection, build_context_entries, build_session_context,
    build_session_projection, current_system_message, walk_up,
};
use crate::entry::{
    BranchSummaryEntry, CURRENT_SESSION_VERSION, CompactionEntry, ContextReplacement,
    EditableContent, EntryId, EntryKind, IsoTimestamp, SessionEntry, SessionHeader,
};
use crate::error::SessionError;
use crate::migrate::parse_session_text;

/// Options for new sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreateOptions {
    /// Explicit session id (validated); default `UUIDv7`.
    pub id: Option<String>,
    /// Source session file recorded as `parentSession`.
    pub parent_session: Option<PathBuf>,
}

/// Input of [`SessionManager::append_compaction`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactionDraft {
    /// Summary text.
    pub summary: String,
    /// First kept entry; `None` ⇒ the compaction's own id (retain none).
    pub first_kept_entry_id: Option<EntryId>,
    /// Context tokens before compaction.
    pub tokens_before: u64,
    /// Implementation data.
    pub details: Option<JsonValue>,
    /// Produced by an extension.
    pub from_hook: Option<bool>,
    /// Usage of the summary request.
    pub usage: Option<Usage>,
    /// Checkpoint; `None` ⇒ folded from the current projection (pi behaviour).
    pub system_message: Option<SystemMessage>,
}

/// Input of [`SessionManager::branch_with_summary`].
#[derive(Clone, Debug, PartialEq)]
pub struct BranchSummaryDraft {
    /// Previous leaf whose path was summarised.
    pub from_id: EntryId,
    /// Summary text.
    pub summary: String,
    /// File tracking or extension data.
    pub details: Option<JsonValue>,
    /// Produced by an extension.
    pub from_hook: Option<bool>,
    /// Usage of the summary request.
    pub usage: Option<Usage>,
}

/// Read-only view handed to extensions (extension-api §5.2).
pub trait ReadonlySession {
    /// Session cwd.
    fn cwd(&self) -> &Path;
    /// Session id.
    fn session_id(&self) -> &str;
    /// Session file, when persisted.
    fn session_file(&self) -> Option<&Path>;
    /// Header.
    fn header(&self) -> &SessionHeader;
    /// Latest session name.
    fn session_name(&self) -> Option<String>;
    /// Current leaf.
    fn leaf_id(&self) -> Option<&EntryId>;
    /// Entry by id.
    fn entry(&self, id: &EntryId) -> Option<&SessionEntry>;
    /// Children in file order.
    fn children(&self, parent: Option<&EntryId>) -> Vec<&SessionEntry>;
    /// Current label of an entry.
    fn label(&self, id: &EntryId) -> Option<&str>;
    /// Root → `from` (default leaf).
    fn branch(&self, from: Option<&EntryId>) -> Vec<&SessionEntry>;
    /// All entries in file order.
    fn entries(&self) -> &[SessionEntry];
    /// Tree view.
    fn tree(&self) -> Vec<SessionTreeNode>;
    /// Context-selected entries at the leaf.
    fn build_context_entries(&self) -> Vec<SessionEntry>;
    /// Projection at the leaf.
    fn build_session_projection(&self) -> SessionProjection;
    /// Context at the leaf.
    fn build_session_context(&self) -> SessionContext;
}

/// Session tree with append-only JSONL persistence.
#[derive(Clone, Debug)]
pub struct SessionManager {
    cwd: PathBuf,
    session_dir: PathBuf,
    persist: bool,
    session_file: Option<PathBuf>,
    flushed: bool,
    has_conversation: bool,
    header: SessionHeader,
    index: EntryIndex,
    labels: HashMap<EntryId, (String, IsoTimestamp)>,
    leaf: Option<EntryId>,
}

fn new_header(cwd: &Path, opts: &CreateOptions) -> Result<SessionHeader, SessionError> {
    if let Some(id) = &opts.id {
        validate_session_id(id)?;
    }
    Ok(header_with(
        cwd,
        opts.id.clone(),
        opts.parent_session.as_deref(),
    ))
}

fn header_with(cwd: &Path, id: Option<String>, parent_session: Option<&Path>) -> SessionHeader {
    SessionHeader {
        version: CURRENT_SESSION_VERSION,
        id: id.unwrap_or_else(|| Uuid::now_v7().hyphenated().to_string()),
        timestamp: IsoTimestamp::now(),
        cwd: cwd.to_string_lossy().into_owned(),
        parent_session: parent_session.map(|path| path.to_string_lossy().into_owned()),
    }
}

fn is_conversation(entry: &SessionEntry) -> bool {
    matches!(
        entry.message(),
        Some(AgentMessage::User(_) | AgentMessage::Assistant(_))
    )
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_break = false;
    for ch in name.chars() {
        if matches!(ch, '\r' | '\n') {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
        } else {
            out.push(ch);
            in_break = false;
        }
    }
    out.trim().to_owned()
}

impl SessionManager {
    fn blank(cwd: &Path, session_dir: &Path, persist: bool, header: SessionHeader) -> Self {
        Self {
            cwd: resolve_path(cwd),
            session_dir: session_dir.to_path_buf(),
            persist,
            session_file: None,
            flushed: false,
            has_conversation: false,
            header,
            index: EntryIndex::default(),
            labels: HashMap::new(),
            leaf: None,
        }
    }

    /// New persisted session for `cwd` (file created on the first user/assistant message).
    ///
    /// # Errors
    ///
    /// [`SessionError::InvalidSessionId`] for a bad explicit id.
    pub fn create(
        cwd: &Path,
        session_dir: &Path,
        opts: CreateOptions,
    ) -> Result<Self, SessionError> {
        let cwd = resolve_path(cwd);
        let mut manager = Self::blank(&cwd, session_dir, true, new_header(&cwd, &opts)?);
        manager.new_session(opts)?;
        Ok(manager)
    }

    /// Opens a session file (pi `open`): migrates pi v1/v2 in place and pre-v1 Oino files to
    /// the v3 layout; a missing file starts a new session at that path.
    ///
    /// # Errors
    ///
    /// I/O, [`SessionError::UnsupportedVersion`], [`SessionError::MissingHeader`] (non-empty
    /// file without header), [`SessionError::MissingCwd`].
    pub fn open(
        path: &Path,
        session_dir: &Path,
        cwd_override: Option<&Path>,
    ) -> Result<Self, SessionError> {
        let path = Self::migrate_if_legacy(&resolve_path(path), session_dir)?;
        let cwd = match cwd_override {
            Some(cwd) => resolve_path(cwd),
            None if path.exists() => match files::read_header(&path) {
                Some(header) if header.cwd.is_empty() => {
                    return Err(SessionError::MissingCwd(path));
                }
                Some(header) => PathBuf::from(header.cwd),
                None => std::env::current_dir().map_err(|e| SessionError::io(&path, e))?,
            },
            None => std::env::current_dir().map_err(|e| SessionError::io(&path, e))?,
        };
        let mut manager = Self::blank(
            &cwd,
            session_dir,
            true,
            new_header(&cwd, &CreateOptions::default())?,
        );
        manager.set_session_file(&path)?;
        Ok(manager)
    }

    fn migrate_if_legacy(path: &Path, session_dir: &Path) -> Result<PathBuf, SessionError> {
        if !path.is_file() || !files::is_legacy_file(path) {
            return Ok(path.to_path_buf());
        }
        let root = path.parent().unwrap_or(session_dir);
        let migrated = migrate_legacy_file(path, root)?;
        Ok(migrated.path.unwrap_or(migrated.legacy_path))
    }

    /// Opens the newest session of `cwd`, else creates one.
    ///
    /// # Errors
    ///
    /// As [`SessionManager::open`] / [`SessionManager::create`].
    pub fn continue_recent(cwd: &Path, session_dir: &Path) -> Result<Self, SessionError> {
        match Self::find_most_recent(cwd, session_dir)? {
            Some(path) => Self::open(&path, session_dir, Some(cwd)),
            None => Self::create(cwd, session_dir, CreateOptions::default()),
        }
    }

    /// In-memory session (never written). `entries` are appended after a fresh header; an
    /// invalid explicit id is replaced by a generated one.
    #[must_use]
    pub fn in_memory(cwd: &Path, opts: CreateOptions, entries: Vec<SessionEntry>) -> Self {
        let cwd = resolve_path(cwd);
        let id = opts.id.filter(|id| validate_session_id(id).is_ok());
        let header = header_with(&cwd, id, opts.parent_session.as_deref());
        let mut manager = Self::blank(&cwd, Path::new(""), false, header);
        manager.load_entries(entries);
        manager
    }

    /// Copies all entries of `source` into a new session for `target_cwd` with
    /// `parentSession = source` (pi `forkFrom`). Written immediately.
    ///
    /// # Errors
    ///
    /// I/O, invalid source ([`SessionError::MissingHeader`]), invalid id.
    pub fn fork_from(
        source: &Path,
        target_cwd: &Path,
        session_dir: &Path,
        opts: CreateOptions,
    ) -> Result<Self, SessionError> {
        let source = Self::migrate_if_legacy(&resolve_path(source), session_dir)?;
        let parsed = parse_session_text(&files::read_to_string(&source)?)?
            .ok_or_else(|| SessionError::MissingHeader(source.clone()))?;
        let target_cwd = resolve_path(target_cwd);
        let header = new_header(
            &target_cwd,
            &CreateOptions {
                parent_session: Some(source),
                ..opts
            },
        )?;
        let path = cwd_session_dir(session_dir, &target_cwd).join(session_file_name(&header));
        files::write_new(&path, &files::serialize_lines(&header, &parsed.entries)?)?;
        Self::open(&path, session_dir, Some(&target_cwd))
    }

    /// Finds a session by id: exact header id, then a unique prefix (≥ 4 chars) in the cwd directory, then over all directories.
    ///
    /// # Errors
    ///
    /// [`SessionError::AmbiguousId`].
    pub fn find_by_id(
        cwd: &Path,
        id: &str,
        session_dir: &Path,
    ) -> Result<Option<PathBuf>, SessionError> {
        list::find_by_id(cwd, id, session_dir)
    }

    /// Sessions of `cwd`, newest modified first.
    ///
    /// # Errors
    ///
    /// [`SessionError::Cancelled`].
    pub fn list(
        cwd: &Path,
        session_dir: &Path,
        progress: Option<&dyn ProgressSink>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<SessionInfo>, SessionError> {
        list::list(cwd, session_dir, progress, cancel)
    }

    /// Sessions of all cwds, newest modified first.
    ///
    /// # Errors
    ///
    /// [`SessionError::Cancelled`].
    pub fn list_all(
        session_dir: &Path,
        progress: Option<&dyn ProgressSink>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<SessionInfo>, SessionError> {
        list::list_all(session_dir, progress, cancel)
    }

    /// Newest valid session file of `cwd`.
    ///
    /// # Errors
    ///
    /// Never fails today; `Result` keeps room for I/O reporting.
    pub fn find_most_recent(
        cwd: &Path,
        session_dir: &Path,
    ) -> Result<Option<PathBuf>, SessionError> {
        Ok(list::find_most_recent(cwd, session_dir))
    }

    /// Migrates every pre-v1 `*.jsonl` directly in `sessions_root` (D21).
    ///
    /// # Errors
    ///
    /// The first I/O error; files migrated before it stay migrated.
    pub fn migrate_legacy_dir(sessions_root: &Path) -> Result<Vec<MigratedSession>, SessionError> {
        let mut migrated = Vec::new();
        let mut candidates = list::jsonl_files(sessions_root);
        candidates.sort();
        for path in candidates {
            if files::is_legacy_file(&path) {
                migrated.push(migrate_legacy_file(&path, sessions_root)?);
            }
        }
        Ok(migrated)
    }

    /// Fresh header and empty tree (pi `newSession`).
    ///
    /// # Errors
    ///
    /// [`SessionError::InvalidSessionId`].
    #[allow(
        clippy::needless_pass_by_value,
        reason = "contract signature (session-v3 §6.2) mirrors pi newSession(options)"
    )]
    pub fn new_session(&mut self, opts: CreateOptions) -> Result<(), SessionError> {
        self.header = new_header(&self.cwd, &opts)?;
        self.index = EntryIndex::default();
        self.labels.clear();
        self.leaf = None;
        self.flushed = false;
        self.has_conversation = false;
        if self.persist {
            self.session_file = Some(
                cwd_session_dir(&self.session_dir, &self.cwd).join(session_file_name(&self.header)),
            );
        }
        Ok(())
    }

    /// Binds to `path` (pi `setSessionFile`): loads it when it exists, otherwise starts a new
    /// session that will be written there.
    ///
    /// # Errors
    ///
    /// As [`SessionManager::open`].
    pub fn set_session_file(&mut self, path: &Path) -> Result<(), SessionError> {
        let path = Self::migrate_if_legacy(&resolve_path(path), &self.session_dir)?;
        if !path.exists() {
            self.new_session(CreateOptions::default())?;
            self.session_file = Some(path);
            return Ok(());
        }
        let text = files::read_to_string(&path)?;
        match parse_session_text(&text)? {
            None if !text.is_empty() => Err(SessionError::MissingHeader(path)),
            None => {
                self.new_session(CreateOptions::default())?;
                self.session_file = Some(path.clone());
                files::write_atomic(&path, &files::serialize_lines(&self.header, &[])?)?;
                self.flushed = true;
                Ok(())
            }
            Some(parsed) => {
                self.header = parsed.header;
                self.session_file = Some(path.clone());
                self.load_entries(parsed.entries);
                self.flushed = true;
                if parsed.migrated {
                    files::write_atomic(
                        &path,
                        &files::serialize_lines(&self.header, self.index.entries())?,
                    )?;
                }
                Ok(())
            }
        }
    }

    fn load_entries(&mut self, entries: Vec<SessionEntry>) {
        self.index = EntryIndex::new(entries);
        self.labels.clear();
        self.leaf = None;
        self.has_conversation = false;
        let mut labels = HashMap::new();
        for entry in self.index.entries() {
            self.leaf = Some(entry.id.clone());
            self.has_conversation |= is_conversation(entry);
            if let EntryKind::Label { target_id, label } = &entry.kind {
                match label.as_deref().filter(|l| !l.is_empty()) {
                    Some(label) => {
                        labels.insert(
                            target_id.clone(),
                            (label.to_owned(), entry.timestamp.clone()),
                        );
                    }
                    None => {
                        labels.remove(target_id);
                    }
                }
            }
        }
        self.labels = labels;
    }

    /// Whether this manager writes to disk (pi `isPersisted`: `false` for in-memory).
    #[must_use]
    pub const fn is_persisted(&self) -> bool {
        self.persist
    }

    /// Whether the file has been created (header + buffered entries written).
    #[must_use]
    pub const fn is_flushed(&self) -> bool {
        self.flushed
    }

    /// Session cwd.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Sessions root.
    #[must_use]
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// Whether the sessions root is `<default agent dir>/sessions`.
    #[must_use]
    pub fn uses_default_session_dir(&self) -> bool {
        default_sessions_root()
            .is_some_and(|root| resolve_path(&root) == resolve_path(&self.session_dir))
    }

    /// Session id.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.header.id
    }

    /// Session file path (planned path before the first flush).
    #[must_use]
    pub fn session_file(&self) -> Option<&Path> {
        self.session_file.as_deref()
    }

    /// Header.
    #[must_use]
    pub const fn header(&self) -> &SessionHeader {
        &self.header
    }

    /// Number of distinct entry ids.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.index.id_count()
    }

    /// The pure entry index (for the free functions in [`crate::context`]).
    #[must_use]
    pub const fn index(&self) -> &EntryIndex {
        &self.index
    }

    fn push(
        &mut self,
        kind: EntryKind,
        id: Option<EntryId>,
        parent: Option<EntryId>,
        timestamp: Option<IsoTimestamp>,
    ) -> Result<EntryId, SessionError> {
        let id = id.unwrap_or_else(|| self.generate_id());
        let entry = SessionEntry {
            id: id.clone(),
            parent_id: parent,
            timestamp: timestamp.unwrap_or_else(IsoTimestamp::now),
            kind,
        };
        self.has_conversation |= is_conversation(&entry);
        let line = serde_json::to_string(&entry).map_err(|source| SessionError::Json {
            path: self.session_file.clone().unwrap_or_default(),
            line: 0,
            source,
        })?;
        self.index.push(entry);
        self.leaf = Some(id.clone());
        self.persist_line(&line)?;
        Ok(id)
    }

    fn generate_id(&self) -> EntryId {
        EntryId::generate(|candidate| self.index.contains_str(candidate))
    }

    fn persist_line(&mut self, line: &str) -> Result<(), SessionError> {
        let Some(path) = self.session_file.clone().filter(|_| self.persist) else {
            return Ok(());
        };
        if self.flushed {
            return files::append_line(&path, line);
        }
        if !self.has_conversation {
            return Ok(());
        }
        files::write_new(
            &path,
            &files::serialize_lines(&self.header, self.index.entries())?,
        )?;
        self.flushed = true;
        Ok(())
    }

    /// Generic append at the leaf (used by boundary drafts).
    ///
    /// # Errors
    ///
    /// I/O when persisting.
    pub fn append_entry(&mut self, kind: EntryKind) -> Result<EntryId, SessionError> {
        let parent = self.leaf.clone();
        self.push(kind, None, parent, None)
    }

    /// Appends a message; summary roles are rejected (use compaction/branch entries).
    ///
    /// # Errors
    ///
    /// [`SessionError::InvalidAppend`], I/O.
    pub fn append_message(&mut self, message: AgentMessage) -> Result<EntryId, SessionError> {
        if matches!(
            message,
            AgentMessage::BranchSummary(_) | AgentMessage::CompactionSummary(_)
        ) {
            return Err(SessionError::InvalidAppend(
                "summary messages are stored as branch_summary/compaction entries",
            ));
        }
        self.append_entry(EntryKind::Message { message })
    }

    /// Appends a `thinking_level_change`.
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_thinking_level_change(&mut self, level: &str) -> Result<EntryId, SessionError> {
        self.append_entry(EntryKind::ThinkingLevelChange {
            thinking_level: level.to_owned(),
        })
    }

    /// Appends a `model_change`.
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_model_change(
        &mut self,
        provider: &str,
        model_id: &str,
    ) -> Result<EntryId, SessionError> {
        self.append_entry(EntryKind::ModelChange {
            provider: provider.to_owned(),
            model_id: model_id.to_owned(),
        })
    }

    /// Appends a `usage` entry (empty note omitted).
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_usage(
        &mut self,
        kind: &str,
        provider: &str,
        model: &str,
        usage: Usage,
        note: Option<String>,
    ) -> Result<EntryId, SessionError> {
        self.append_entry(EntryKind::Usage {
            kind: kind.to_owned(),
            provider: provider.to_owned(),
            model: model.to_owned(),
            usage,
            note: note.filter(|n| !n.is_empty()),
        })
    }

    /// Appends a compaction. Without an explicit checkpoint the current projection's system
    /// state is stored, stamped with the entry time (pi `appendCompaction`).
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_compaction(&mut self, draft: CompactionDraft) -> Result<EntryId, SessionError> {
        let id = self.generate_id();
        let now = IsoTimestamp::now();
        let system_message = draft.system_message.or_else(|| {
            current_system_message(&self.build_session_projection().messages).map(|mut system| {
                system.timestamp = now.to_ms().unwrap_or(0);
                system
            })
        });
        let kind = EntryKind::Compaction(CompactionEntry {
            summary: draft.summary,
            first_kept_entry_id: draft.first_kept_entry_id.unwrap_or_else(|| id.clone()),
            tokens_before: draft.tokens_before,
            details: draft.details,
            usage: draft.usage,
            from_hook: draft.from_hook,
            system_message,
        });
        let parent = self.leaf.clone();
        self.push(kind, Some(id), parent, Some(now))
    }

    /// Appends a `custom` entry (extension state).
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_custom_entry(
        &mut self,
        custom_type: &str,
        data: Option<JsonValue>,
    ) -> Result<EntryId, SessionError> {
        self.append_entry(EntryKind::Custom {
            custom_type: custom_type.to_owned(),
            data,
        })
    }

    /// Appends a `session_info` (CR/LF runs → one space, trimmed).
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_session_info(&mut self, name: Option<&str>) -> Result<EntryId, SessionError> {
        self.append_entry(EntryKind::SessionInfo {
            name: name.map(sanitize_name),
        })
    }

    /// Appends a `custom_message` (participates in context).
    ///
    /// # Errors
    ///
    /// I/O.
    pub fn append_custom_message_entry(
        &mut self,
        custom_type: &str,
        content: UserContentField,
        display: bool,
        details: Option<JsonValue>,
    ) -> Result<EntryId, SessionError> {
        self.append_entry(EntryKind::CustomMessage {
            custom_type: custom_type.to_owned(),
            content,
            display,
            details,
        })
    }

    /// Appends a `context_edit` of an editable entry on the active branch; string replacements
    /// of assistant/toolResult targets are normalised to one text block (pi).
    ///
    /// # Errors
    ///
    /// [`SessionError::UnknownEntry`], [`SessionError::InvalidAppend`], I/O.
    pub fn append_context_edit(
        &mut self,
        target: &EntryId,
        replacement: Option<EditableContent>,
    ) -> Result<EntryId, SessionError> {
        let entry = self
            .index
            .get(target)
            .ok_or_else(|| SessionError::UnknownEntry(target.clone()))?;
        if !self.branch(None).iter().any(|e| e.id == *target) {
            return Err(SessionError::InvalidAppend(
                "context edit target is not on the active branch",
            ));
        }
        let role = match &entry.kind {
            EntryKind::CustomMessage { .. } => "custom",
            EntryKind::Message { message } => message.role(),
            _ => "",
        };
        if !matches!(role, "custom" | "user" | "assistant" | "toolResult") {
            return Err(SessionError::InvalidAppend(
                "context edit target does not contribute editable model content",
            ));
        }
        let replacement = replacement.map(|content| ContextReplacement {
            content: match (role, content) {
                ("assistant", EditableContent::Text(text)) => {
                    EditableContent::Assistant(vec![oino_ai::AssistantContent::Text(
                        TextContent::new(text),
                    )])
                }
                ("toolResult", EditableContent::Text(text)) => {
                    EditableContent::User(vec![UserContent::Text(TextContent::new(text))])
                }
                (_, content) => content,
            },
        });
        self.append_entry(EntryKind::ContextEdit {
            target_id: target.clone(),
            replacement,
        })
    }

    /// Sets (or with `None`/empty clears) the label of `target`.
    ///
    /// # Errors
    ///
    /// [`SessionError::UnknownEntry`], I/O.
    pub fn append_label_change(
        &mut self,
        target: &EntryId,
        label: Option<&str>,
    ) -> Result<EntryId, SessionError> {
        if !self.index.contains(target) {
            return Err(SessionError::UnknownEntry(target.clone()));
        }
        let label = label.filter(|l| !l.is_empty()).map(str::to_owned);
        let id = self.append_entry(EntryKind::Label {
            target_id: target.clone(),
            label: label.clone(),
        })?;
        match label {
            Some(label) => {
                let timestamp = self
                    .index
                    .get(&id)
                    .map(|e| e.timestamp.clone())
                    .unwrap_or_default();
                self.labels.insert(target.clone(), (label, timestamp));
            }
            None => {
                self.labels.remove(target);
            }
        }
        Ok(id)
    }

    /// Latest `session_info` name in file order (trimmed; empty ⇒ `None`).
    #[must_use]
    pub fn session_name(&self) -> Option<String> {
        self.index
            .entries()
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                EntryKind::SessionInfo { name } => Some(
                    name.as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned),
                ),
                _ => None,
            })?
    }

    /// Current leaf id (`None` after [`SessionManager::reset_leaf`] or when empty).
    #[must_use]
    pub const fn leaf_id(&self) -> Option<&EntryId> {
        self.leaf.as_ref()
    }

    /// Current leaf entry.
    #[must_use]
    pub fn leaf_entry(&self) -> Option<&SessionEntry> {
        self.leaf.as_ref().and_then(|id| self.index.get(id))
    }

    /// Entry by id.
    #[must_use]
    pub fn entry(&self, id: &EntryId) -> Option<&SessionEntry> {
        self.index.get(id)
    }

    /// Entries whose parent is `parent` (`None` ⇒ roots), file order.
    #[must_use]
    pub fn children(&self, parent: Option<&EntryId>) -> Vec<&SessionEntry> {
        self.index
            .entries()
            .iter()
            .filter(|entry| entry.parent_id.as_ref() == parent)
            .collect()
    }

    /// Current label of an entry.
    #[must_use]
    pub fn label(&self, id: &EntryId) -> Option<&str> {
        self.labels.get(id).map(|(label, _)| label.as_str())
    }

    /// Root → `from` (default leaf), all entry types; unknown id ⇒ empty.
    #[must_use]
    pub fn branch(&self, from: Option<&EntryId>) -> Vec<&SessionEntry> {
        from.or(self.leaf.as_ref())
            .and_then(|id| self.index.get(id))
            .map(|start| walk_up(&self.index, start))
            .unwrap_or_default()
    }

    /// All entries in file order.
    #[must_use]
    pub fn entries(&self) -> &[SessionEntry] {
        self.index.entries()
    }

    /// Tree of all entries (roots and orphans in file order, children by timestamp).
    #[must_use]
    pub fn tree(&self) -> Vec<SessionTreeNode> {
        tree::build(self.index.entries(), &self.labels)
    }

    /// pi `buildContextEntries` at the leaf.
    #[must_use]
    pub fn build_context_entries(&self) -> Vec<SessionEntry> {
        build_context_entries(&self.index, self.leaf.as_ref())
    }

    /// pi `buildSessionProjection` at the leaf.
    #[must_use]
    pub fn build_session_projection(&self) -> SessionProjection {
        build_session_projection(&self.index, self.leaf.as_ref())
    }

    /// pi `buildSessionContext` at the leaf.
    #[must_use]
    pub fn build_session_context(&self) -> SessionContext {
        build_session_context(&self.index, self.leaf.as_ref())
    }

    /// Moves the leaf (pi `branch(fromId)`); nothing is written.
    ///
    /// # Errors
    ///
    /// [`SessionError::UnknownEntry`].
    pub fn set_leaf(&mut self, id: &EntryId) -> Result<(), SessionError> {
        if !self.index.contains(id) {
            return Err(SessionError::UnknownEntry(id.clone()));
        }
        self.leaf = Some(id.clone());
        Ok(())
    }

    /// Clears the leaf; the next entry becomes a new root.
    pub fn reset_leaf(&mut self) {
        self.leaf = None;
    }

    /// Moves the leaf to `from` (`None` ⇒ new root) and appends a `branch_summary` there.
    ///
    /// # Errors
    ///
    /// [`SessionError::UnknownEntry`], I/O.
    pub fn branch_with_summary(
        &mut self,
        from: Option<&EntryId>,
        summary: BranchSummaryDraft,
    ) -> Result<EntryId, SessionError> {
        if let Some(from) = from.filter(|id| !self.index.contains(id)) {
            return Err(SessionError::UnknownEntry(from.clone()));
        }
        self.leaf = from.cloned();
        self.append_entry(EntryKind::BranchSummary(BranchSummaryEntry {
            from_id: summary.from_id,
            summary: summary.summary,
            details: summary.details,
            usage: summary.usage,
            from_hook: summary.from_hook,
        }))
    }

    /// Header and entries of a new session holding the path root → `leaf` (labels re-emitted
    /// at the end, compaction `firstKeptEntryId`s that pointed at labels remapped).
    fn branched(&self, leaf: &EntryId) -> Result<(SessionHeader, Vec<SessionEntry>), SessionError> {
        let path = self.branch(Some(leaf));
        if path.is_empty() {
            return Err(SessionError::UnknownEntry(leaf.clone()));
        }
        let mut kept: Vec<SessionEntry> = Vec::with_capacity(path.len());
        let mut label_replacement: HashMap<EntryId, EntryId> = HashMap::new();
        let mut pending_labels: Vec<EntryId> = Vec::new();
        let mut parent: Option<EntryId> = None;
        for entry in path {
            if matches!(entry.kind, EntryKind::Label { .. }) {
                pending_labels.push(entry.id.clone());
                continue;
            }
            for label in pending_labels.drain(..) {
                label_replacement.insert(label, entry.id.clone());
            }
            let mut copy = entry.clone();
            copy.parent_id.clone_from(&parent);
            if let EntryKind::Compaction(data) = &mut copy.kind
                && data.first_kept_entry_id != entry.id
                && let Some(replacement) = label_replacement.get(&data.first_kept_entry_id)
            {
                data.first_kept_entry_id = replacement.clone();
            }
            parent = Some(entry.id.clone());
            kept.push(copy);
        }
        let header = new_header(
            &self.cwd,
            &CreateOptions {
                id: None,
                parent_session: self.session_file.clone().filter(|_| self.persist),
            },
        )?;
        let mut labels: Vec<(&EntryId, &(String, IsoTimestamp))> = self
            .labels
            .iter()
            .filter(|(target, _)| kept.iter().any(|e| &e.id == *target))
            .collect();
        labels.sort_by(|a, b| a.1.1.cmp(&b.1.1).then_with(|| a.0.cmp(b.0)));
        let mut taken: std::collections::HashSet<String> =
            kept.iter().map(|e| e.id.to_string()).collect();
        let mut previous = kept.last().map(|e| e.id.clone());
        for (target, (label, timestamp)) in labels {
            let id = EntryId::generate(|candidate| taken.contains(candidate));
            taken.insert(id.to_string());
            kept.push(SessionEntry {
                id: id.clone(),
                parent_id: previous.replace(id),
                timestamp: timestamp.clone(),
                kind: EntryKind::Label {
                    target_id: target.clone(),
                    label: Some(label.clone()),
                },
            });
        }
        Ok((header, kept))
    }

    /// Writes a new session file containing the path root → `leaf` (`/clone`), with
    /// `parentSession` = this file. `self` is unchanged.
    ///
    /// # Errors
    ///
    /// [`SessionError::UnknownEntry`], [`SessionError::InvalidAppend`] for in-memory
    /// sessions (use [`SessionManager::branch_into_new_session`]), I/O.
    pub fn create_branched_session(&self, leaf: &EntryId) -> Result<PathBuf, SessionError> {
        if !self.persist {
            return Err(SessionError::InvalidAppend(
                "in-memory sessions have no file to branch into",
            ));
        }
        let (header, entries) = self.branched(leaf)?;
        let path = cwd_session_dir(&self.session_dir, &self.cwd).join(session_file_name(&header));
        files::write_new(&path, &files::serialize_lines(&header, &entries)?)?;
        Ok(path)
    }

    /// pi `createBranchedSession` semantics: replaces `self` with the branched session (the
    /// file is written only if it contains a user/assistant message). Returns the new path
    /// for persisted sessions.
    ///
    /// # Errors
    ///
    /// [`SessionError::UnknownEntry`], I/O.
    pub fn branch_into_new_session(
        &mut self,
        leaf: &EntryId,
    ) -> Result<Option<PathBuf>, SessionError> {
        let (header, entries) = self.branched(leaf)?;
        self.header = header;
        self.load_entries(entries);
        self.flushed = false;
        if !self.persist {
            self.session_file = None;
            return Ok(None);
        }
        let path =
            cwd_session_dir(&self.session_dir, &self.cwd).join(session_file_name(&self.header));
        self.session_file = Some(path.clone());
        if self.has_conversation {
            files::write_atomic(
                &path,
                &files::serialize_lines(&self.header, self.index.entries())?,
            )?;
            self.flushed = true;
        }
        Ok(Some(path))
    }
}

impl ReadonlySession for SessionManager {
    fn cwd(&self) -> &Path {
        Self::cwd(self)
    }
    fn session_id(&self) -> &str {
        Self::session_id(self)
    }
    fn session_file(&self) -> Option<&Path> {
        Self::session_file(self)
    }
    fn header(&self) -> &SessionHeader {
        Self::header(self)
    }
    fn session_name(&self) -> Option<String> {
        Self::session_name(self)
    }
    fn leaf_id(&self) -> Option<&EntryId> {
        Self::leaf_id(self)
    }
    fn entry(&self, id: &EntryId) -> Option<&SessionEntry> {
        Self::entry(self, id)
    }
    fn children(&self, parent: Option<&EntryId>) -> Vec<&SessionEntry> {
        Self::children(self, parent)
    }
    fn label(&self, id: &EntryId) -> Option<&str> {
        Self::label(self, id)
    }
    fn branch(&self, from: Option<&EntryId>) -> Vec<&SessionEntry> {
        Self::branch(self, from)
    }
    fn entries(&self) -> &[SessionEntry] {
        Self::entries(self)
    }
    fn tree(&self) -> Vec<SessionTreeNode> {
        Self::tree(self)
    }
    fn build_context_entries(&self) -> Vec<SessionEntry> {
        Self::build_context_entries(self)
    }
    fn build_session_projection(&self) -> SessionProjection {
        Self::build_session_projection(self)
    }
    fn build_session_context(&self) -> SessionContext {
        Self::build_session_context(self)
    }
}
