//! Entry offset index: open a big session at its tail without parsing the whole file.
//!
//! A rebuildable `<file>.idx` cache records offsets, parent links, branch depth,
//! pending tool calls, validated compaction anchors, title, file size and mtime.
//! JSONL remains authoritative. Complete appended lines extend the cached index;
//! truncation, replacement, incompatible or corrupt caches trigger a rebuild.
//! Sidecars use atomic temp + rename. The process cache holds at most 32 indexes.
//!
//! Runtime wake parses the retained branch range and settings/prompt metadata only.
//! Inert prefix records preserve absolute branch indexes used by compaction validation.
//! Full-history APIs materialize original entries lazily, never exposing these records.
//!
//! - **Pages.** [`OffsetIndex::page`] reads the newest `limit` entries of the current
//!   branch before an optional cursor (an entry id), bounded by raw bytes, aligned so a
//!   page starts at a user message whenever the page holds one (turns are never cut when
//!   it can be helped). Lines are read by offset (`pread`) and parsed one by one.
//!
//! Version 1/2 files (no entry ids) are not indexable: [`OffsetIndex::load`] answers
//! [`SessionError::UnsupportedVersion`] and callers fall back to the full open.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::UNIX_EPOCH;

use oino_ai::JsonValue;
use serde::{Deserialize, Serialize};

use crate::error::SessionError;

/// Files at least this big get their index persisted (smaller ones scan in milliseconds).
pub const PERSIST_MIN: u64 = 4 << 20;
/// Suffix of the directory persisted indexes live in: `<parent>/.<root name>.oino-offsets`.
pub const OFFSETS_DIR_SUFFIX: &str = ".oino-offsets";
const FORMAT_VERSION: u32 = 7;
const HASH_BLOCK_BYTES: usize = 1 << 20;
const HASH_BLOCK: u64 = HASH_BLOCK_BYTES as u64;
const READ_CHUNK: usize = 4 << 20;
/// Process-wide in-memory cache size (indexes, not entries).
const MEMORY_CACHE: usize = 32;

/// The entry is a `message` with `role: "user"`.
pub const FLAG_USER: u8 = 1;
/// The entry is a `compaction` or `compaction_state` (a context checkpoint).
pub const FLAG_COMPACTION: u8 = 2;
/// The entry is a `session_info` (the session title).
const FLAG_INFO: u8 = 4;
/// The entry is a `message` with `role: "assistant"` (names the model that answered).
pub(super) const FLAG_ASSISTANT: u8 = 8;
/// The entry is a `model_change`.
const FLAG_MODEL: u8 = 16;
/// The entry is a `thinking_level_change`.
const FLAG_THINKING: u8 = 32;
const FLAG_CONTEXT: u8 = 64;
const FLAG_ANCHOR: u8 = 128;

/// One indexed entry line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPos {
    /// Byte offset of the line.
    #[serde(rename = "o")]
    pub offset: u64,
    /// Line length without the newline.
    #[serde(rename = "l")]
    pub len: u32,
    /// Entry id.
    pub id: String,
    /// Index (into [`OffsetIndex::entries`]) of the parent entry; `None` for a root or a
    /// parent that is not in the file.
    #[serde(rename = "p", default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// [`FLAG_USER`] | [`FLAG_COMPACTION`].
    #[serde(rename = "f", default, skip_serializing_if = "is_zero")]
    pub flags: u8,
    #[serde(default)]
    depth: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending: Vec<String>,
    /// Validated retained-range anchor (file position), or this record for an empty range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compaction_summary_id: Option<crate::entry::EntryId>,
    /// Nearest compaction on this entry's parent chain (including itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch_compaction: Option<u32>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// The offset index of one session file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffsetIndex {
    #[serde(rename = "v")]
    version: u32,
    /// File size when indexed.
    size: u64,
    /// File mtime (ns) when indexed.
    mtime_ns: u64,
    identity: FileStamp,
    generation: u64,
    /// Offset after the last complete line read.
    end: u64,
    /// XXH3-64 of each fixed 1 MiB block before `end`, including its partial tail.
    fingerprint: Vec<u64>,
    /// Every entry line, in file order.
    entries: Vec<EntryPos>,
    /// Index of the last compaction entry in the file, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_compaction: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    moved_cwd: Option<String>,
    #[serde(skip)]
    by_id: OnceLock<HashMap<String, u32>>,
}

/// What a page request asks for.
#[derive(Clone, Debug)]
pub struct PageRequest {
    /// Only entries strictly before this one (an entry id of the branch); `None` = the tail.
    pub before: Option<String>,
    /// At most this many entries.
    pub limit: usize,
    /// Stop growing the page once its raw lines reach this many bytes (≥ 1 entry always).
    pub max_bytes: u64,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            before: None,
            limit: 120,
            max_bytes: 24 << 20,
        }
    }
}

/// One page of the current branch, oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryPage {
    /// Parsed entry lines (raw JSON, exactly as stored), root-most first.
    pub entries: Vec<JsonValue>,
    /// The branch leaf (the file's last entry).
    pub leaf_id: Option<String>,
    /// Entries on the whole current branch.
    pub total: usize,
    /// Older entries exist before this page.
    pub has_more: bool,
    /// The id to pass as `before` for the next older page (the page's first entry).
    pub cursor: Option<String>,
}

impl OffsetIndex {
    /// Rebuildable cache beside the unchanged JSONL file.
    #[must_use]
    pub fn sidecar_path(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_os_string();
        name.push(".idx");
        PathBuf::from(name)
    }

    /// Latest title in file order.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub(super) fn custom_positions(&self, kind: &str) -> impl Iterator<Item = u32> + '_ {
        let kind = kind.to_owned();
        self.entries.iter().enumerate().filter_map(move |(i, p)| {
            (p.custom_type.as_deref() == Some(kind.as_str())).then_some(as_u32(i))
        })
    }

    pub(super) fn moved_cwd(&self) -> Option<&str> {
        self.moved_cwd.as_deref()
    }

    /// Every indexed entry, in file order.
    #[must_use]
    pub fn entries(&self) -> &[EntryPos] {
        &self.entries
    }

    /// The leaf: the file's last entry.
    #[must_use]
    pub fn leaf_id(&self) -> Option<&str> {
        self.entries.last().map(|entry| entry.id.as_str())
    }

    /// Index of the last compaction entry in the file.
    #[must_use]
    pub const fn last_compaction(&self) -> Option<u32> {
        self.last_compaction
    }

    /// Bytes covered (offset after the last complete line).
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.end
    }

    fn by_id(&self) -> &HashMap<String, u32> {
        self.by_id.get_or_init(|| {
            self.entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id.clone(), as_u32(index)))
                .collect()
        })
    }

    /// Position of `id` in the file, if indexed.
    #[must_use]
    pub fn position(&self, id: &str) -> Option<usize> {
        self.by_id().get(id).map(|index| *index as usize)
    }

    /// The current branch as entry indexes, root first (the parent chain of the leaf).
    #[must_use]
    pub fn branch(&self) -> Vec<u32> {
        let mut chain = Vec::new();
        let Some(mut cursor) = self.entries.len().checked_sub(1).map(as_u32) else {
            return chain;
        };
        // A cycle cannot exist in a well-formed file; the bound guards a corrupt one.
        for _ in 0..=self.entries.len() {
            chain.push(cursor);
            match self.entries[cursor as usize].parent {
                Some(parent) if parent < cursor => cursor = parent,
                _ => break,
            }
        }
        chain.reverse();
        chain
    }

    /// Loads the index of `path` — memory cache, then the persisted index under
    /// `offsets_dir` (when given), extended for appended lines or rebuilt when stale.
    ///
    /// # Errors
    ///
    /// I/O failures, [`SessionError::MissingHeader`] (not a session file),
    /// [`SessionError::UnsupportedVersion`] (pre-v3 files have no entry ids).
    pub fn load(path: &Path, offsets_dir: Option<&Path>) -> Result<Arc<Self>, SessionError> {
        let _ = offsets_dir; // Kept for source compatibility with older host callers.
        Self::load_inner(path, Some(&Self::sidecar_path(path)), 0, true)
    }

    /// Load without queuing persistence. Call `persist_after_reply` once the
    /// initial response has been sent, keeping cache IO off the open path.
    ///
    /// # Errors
    /// Returns the same I/O and session-format errors as [`Self::load`].
    pub fn load_deferred(path: &Path) -> Result<Arc<Self>, SessionError> {
        Self::load_inner(path, Some(&Self::sidecar_path(path)), 0, false)
    }

    /// Queue a bounded, best-effort atomic snapshot after sending an open reply.
    pub fn persist_after_reply(path: &Path) {
        schedule_snapshot(path);
    }

    /// [`Self::load`] with the persisted index at `persisted` (written when the file is at
    /// least `persist_min` bytes and the index changed).
    fn load_inner(
        path: &Path,
        persisted: Option<&Path>,
        persist_min: u64,
        schedule: bool,
    ) -> Result<Arc<Self>, SessionError> {
        let mut file = File::open(path).map_err(|e| SessionError::io(path, e))?;
        let identity = file_stamp(&file).map_err(|e| SessionError::io(path, e))?;
        let (size, mtime_ns) = (identity.len, identity.mtime);
        if let Some(hit) = memory_get(path)
            && hit.size == size
            && hit.mtime_ns == mtime_ns
            && hit.identity == identity
            && hit.matches_blocks(&file).unwrap_or(false)
        {
            // Even an unchanged stat tuple can hide an external rewrite (for
            // example on coarse-ctime filesystems). Loads validate every byte.
            return Ok(hit);
        }
        let cached = memory_take(path);
        let from_memory = cached.is_some();
        let decode_started = std::time::Instant::now();
        let mut disk_validated = false;
        let mut stale = cached.map(Arc::unwrap_or_clone).or_else(|| {
            persisted
                .and_then(|path| {
                    let bytes = fs::read(path).ok()?;
                    if let Some(prefix) = binary::prefix(&bytes) {
                        return std::thread::scope(|scope| {
                            let validation =
                                scope.spawn(|| prefix.matches_blocks(&file).unwrap_or(false));
                            let index = binary::decode(&bytes);
                            disk_validated = validation.join().unwrap_or(false);
                            index
                        });
                    }
                    read_persisted(path)
                })
                .filter(Self::valid_persisted)
        });
        if std::env::var_os("OINO_OPEN_TRACE").is_some() {
            let _ = writeln!(
                io::stderr(),
                "open_phase=sidecar_decode elapsed_us={}",
                decode_started.elapsed().as_micros()
            );
        }
        let owned = owned_growth(path, &identity);
        let trusted_prefix = |index: &Self| {
            // Only a live, in-process index can follow our leased writer's
            // append chain. A deserialized sidecar must always be hashed, even
            // when it carries an identity matching our own most recent write.
            cfg!(unix)
                && from_memory
                && index.size < size
                && owned
                    .as_ref()
                    .is_some_and(|(start, _)| *start == index.identity)
        };
        let reusable = stale.as_ref().is_some_and(|candidate| {
            candidate.size == size
                && candidate.end <= size
                && (disk_validated
                    || trusted_prefix(candidate)
                    || candidate.matches_blocks(&file).unwrap_or(false))
        });
        if let Some(mut index) = stale.take_if(|_| reusable) {
            index.identity = identity;
            index.mtime_ns = mtime_ns;
            let index = Arc::new(index);
            memory_put(path, Arc::clone(&index));
            if schedule && !from_memory {
                schedule_snapshot(path);
            }
            return Ok(index);
        }
        let extendable = stale.filter(|stale| {
            stale.size < size
                && stale.end <= size
                && (disk_validated
                    || trusted_prefix(stale)
                    || stale.matches_blocks(&file).unwrap_or(false))
        });
        let build_started = std::time::Instant::now();
        let index = match extendable {
            Some(mut index) => {
                index.extend(&mut file, path)?;
                index
            }
            None => Self::build(&mut file, path)?,
        };
        if std::env::var_os("OINO_OPEN_TRACE").is_some() {
            let _ = writeln!(
                io::stderr(),
                "open_phase=build elapsed_us={}",
                build_started.elapsed().as_micros()
            );
        }
        let index = Self {
            size,
            mtime_ns,
            identity,
            generation: owned.as_ref().map_or(0, |(_, generation)| *generation),
            ..index
        };
        let index = Arc::new(index);
        memory_put(path, Arc::clone(&index));
        if schedule && size >= persist_min && persisted.is_some() {
            schedule_snapshot(path);
        }
        Ok(index)
    }

    fn valid_persisted(index: &Self) -> bool {
        index.version == FORMAT_VERSION
            && index.end <= index.size
            && index.entries.iter().enumerate().all(|(i, p)| {
                p.offset
                    .checked_add(u64::from(p.len))
                    .is_some_and(|end| end < index.end)
                    && p.parent.is_none_or(|parent| (parent as usize) < i)
                    && p.checkpoint.is_none_or(|anchor| (anchor as usize) <= i)
                    && p.branch_compaction
                        .is_none_or(|anchor| (anchor as usize) <= i)
            })
    }

    /// Extend the exclusively owned live index using only the newly written line.
    pub(super) fn append_line(
        &mut self,
        line: &str,
        path: &Path,
        before: Option<&FileStamp>,
    ) -> Result<(), SessionError> {
        let mut file = File::open(path).map_err(|e| SessionError::io(path, e))?;
        let identity = file_stamp(&file).map_err(|e| SessionError::io(path, e))?;
        let mut before = before.cloned().unwrap_or_default();
        before.tail_digest = 0;
        if !cfg!(unix) || self.identity != before {
            // An external edit before our write invalidates even a manager-private
            // offset index. A rebuild is required if any old prefix hash differs.
            if !self.matches_blocks(&file).unwrap_or(false) {
                *self = Self::build(&mut file, path)?;
                self.size = identity.len;
                self.mtime_ns = identity.mtime;
                self.identity = identity;
                return Ok(());
            }
        }
        let mut ids = self.by_id.take().unwrap_or_default();
        let result = self.line(line.as_bytes(), self.end, &mut ids, path);
        let _ = self.by_id.set(ids);
        result?;
        self.end += line.len() as u64 + 1;
        // This manager-private index is used only for offsets/wake metadata, never
        // published as a validated reader cache. Reader snapshots hash independently.
        self.fingerprint.clear();
        self.last_compaction = self
            .entries
            .last()
            .and_then(|entry| entry.branch_compaction);
        self.size = self.end;
        self.mtime_ns = identity.mtime;
        self.identity = identity;
        Ok(())
    }

    /// Scans `file` from the start (header checked, then every line).
    fn build(file: &mut File, path: &Path) -> Result<Self, SessionError> {
        let mut index = Self {
            version: FORMAT_VERSION,
            size: 0,
            mtime_ns: 0,
            identity: FileStamp::default(),
            generation: 0,
            end: 0,
            fingerprint: Vec::new(),
            entries: Vec::new(),
            last_compaction: None,
            title: None,
            moved_cwd: None,
            by_id: OnceLock::new(),
        };
        index.extend(file, path)?;
        if index.end == 0 {
            // Empty file or a header still being written.
            return Err(SessionError::MissingHeader(path.to_path_buf()));
        }
        Ok(index)
    }

    /// Reads every complete line from `self.end` on.
    fn extend(&mut self, file: &mut File, path: &Path) -> Result<(), SessionError> {
        let old_end = self.end;
        let mut hash = xxhash_rust::xxh3::Xxh3::new();
        let mut hashed = 0usize;
        let io_err = |e| SessionError::io(path, e);
        file.seek(SeekFrom::Start(self.end)).map_err(io_err)?;
        let mut ids: HashMap<String, u32> = self.by_id.take().unwrap_or_else(|| {
            self.entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id.clone(), as_u32(index)))
                .collect()
        });
        let mut carry: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK];
        let mut base = self.end; // file offset of carry[0]
        loop {
            let read = file.read(&mut chunk).map_err(io_err)?;
            if read == 0 {
                break;
            }
            carry.extend_from_slice(&chunk[..read]);
            let mut start = 0;
            while let Some(newline) = memchr::memchr(b'\n', &carry[start..]) {
                let line = &carry[start..start + newline];
                let offset = base + start as u64;
                self.line(line, offset, &mut ids, path)?;
                start += newline + 1;
                self.end = offset + newline as u64 + 1;
            }
            if old_end == 0 {
                let mut bytes = &carry[..start];
                while !bytes.is_empty() {
                    let take = bytes.len().min(HASH_BLOCK_BYTES - hashed);
                    hash.update(&bytes[..take]);
                    hashed += take;
                    bytes = &bytes[take..];
                    if hashed == HASH_BLOCK_BYTES {
                        self.fingerprint.push(hash.digest());
                        hash.reset();
                        hashed = 0;
                    }
                }
            }
            carry.drain(..start);
            base += start as u64;
        }
        self.last_compaction = self
            .entries
            .last()
            .and_then(|entry| entry.branch_compaction);

        self.by_id = OnceLock::new();
        let _ = self.by_id.set(ids);
        if old_end == 0 {
            if hashed != 0 {
                self.fingerprint.push(hash.digest());
            }
        } else {
            self.refresh_blocks(file, old_end).map_err(io_err)?;
        }
        Ok(())
    }

    /// Re-hash only the previous partial block and appended bytes.
    fn refresh_blocks(&mut self, file: &File, old_end: u64) -> io::Result<()> {
        let first = old_end / HASH_BLOCK;
        self.fingerprint
            .truncate(usize::try_from(first).unwrap_or(usize::MAX));
        let mut buffer = vec![0; 1 << 20];
        for block in first..self.end.div_ceil(HASH_BLOCK) {
            self.fingerprint
                .push(block_hash(file, block, self.end, &mut buffer)?);
        }
        Ok(())
    }

    /// Revalidate every byte of the old prefix, including its partial tail.
    /// Sequential large-buffer reads and XXH3 keep untrusted validation inexpensive.
    fn matches_blocks(&self, file: &File) -> io::Result<bool> {
        let count = usize::try_from(self.end.div_ceil(HASH_BLOCK)).unwrap_or(usize::MAX);
        if self.fingerprint.len() != count {
            return Ok(false);
        }
        let started = std::time::Instant::now();
        let validate = |first: usize, last: usize| -> io::Result<bool> {
            let mut buffer = vec![0; READ_CHUNK];
            let mut offset = first as u64 * HASH_BLOCK;
            let end = (last as u64 * HASH_BLOCK).min(self.end);
            let mut block = first;
            while offset < end {
                let len =
                    usize::try_from((end - offset).min(READ_CHUNK as u64)).unwrap_or(READ_CHUNK);
                read_at(file, &mut buffer[..len], offset)?;
                for bytes in buffer[..len].chunks(HASH_BLOCK_BYTES) {
                    if xxhash_rust::xxh3::xxh3_64(bytes) != self.fingerprint[block] {
                        return Ok(false);
                    }
                    block += 1;
                }
                offset += len as u64;
            }
            Ok(true)
        };
        // Two bounded sequential readers for large prefixes, never one thread per block.
        let valid = if self.end >= 32 << 20 {
            std::thread::scope(|scope| {
                let mid = count / 2;
                let other = scope.spawn(move || validate(0, mid));
                let right = validate(mid, count);
                let left = other
                    .join()
                    .map_err(|_| io::Error::other("prefix validator panicked"))?;
                Ok::<_, io::Error>(left? && right?)
            })?
        } else {
            validate(0, count)?
        };
        if !valid {
            return Ok(false);
        }
        if std::env::var_os("OINO_OPEN_TRACE").is_some() {
            let _ = writeln!(
                io::stderr(),
                "open_phase=prefix_validation elapsed_us={} bytes={}",
                started.elapsed().as_micros(),
                self.end
            );
        }
        Ok(true)
    }

    /// Indexes one complete line found at `offset`.
    #[allow(
        clippy::too_many_lines,
        reason = "one entry updates interdependent index metadata"
    )]
    fn line(
        &mut self,
        line: &[u8],
        offset: u64,
        ids: &mut HashMap<String, u32>,
        path: &Path,
    ) -> Result<(), SessionError> {
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            return Ok(());
        }
        if offset == 0 {
            return check_header(trimmed, path);
        }
        let Some(head) = parse_head(trimmed) else {
            // A malformed line: the runtime's open reports or repairs it; skip it here.
            return Ok(());
        };
        let Some(id) = head.id else {
            return Ok(());
        };
        let index = as_u32(self.entries.len());
        let parent = head
            .parent_id
            .and_then(|parent| ids.get(parent.as_ref()).copied());
        let mut flags = 0;
        let role = head.message.as_ref().and_then(|m| m.role.as_deref());
        if head.kind == "message" {
            match role {
                Some("user") => flags |= FLAG_USER,
                Some("assistant") => flags |= FLAG_ASSISTANT,
                _ => {}
            }
        }
        match head.kind.as_ref() {
            "model_change" => flags |= FLAG_MODEL,
            "thinking_level_change" => flags |= FLAG_THINKING,
            _ => {}
        }
        if matches!(
            head.kind.as_ref(),
            "message" | "custom_message" | "branch_summary" | "context_edit"
        ) {
            flags |= FLAG_CONTEXT;
        }
        if flags & FLAG_USER != 0 || head.kind == "custom_message" {
            flags |= FLAG_ANCHOR;
        }
        if head.kind == "compaction" || head.kind == "compaction_state" {
            flags |= FLAG_COMPACTION;
            self.last_compaction = Some(index);
        }
        if head.kind == "session_info" {
            flags |= FLAG_INFO;
            self.title = head
                .name
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty());
        }
        let depth = parent.map_or(0, |p| self.entries[p as usize].depth + 1);
        let mut pending =
            parent.map_or_else(Vec::new, |p| self.entries[p as usize].pending.clone());
        if let Some(message) = &head.message {
            if role == Some("assistant") {
                for block in &message.content {
                    if block.kind == "toolCall"
                        && let Some(id) = &block.id
                        && !pending.contains(id)
                    {
                        pending.push(id.clone());
                    }
                }
            } else if role == Some("toolResult")
                && let Some(id) = &message.tool_call_id
            {
                pending.retain(|p| p != id);
            }
        }
        let checkpoint = if flags & FLAG_COMPACTION != 0 {
            let anchor = head
                .anchor_message
                .as_ref()
                .map(|a| a.id.as_str())
                .or(head.first_kept_entry_id.as_deref());
            if anchor == Some(id.as_ref()) || anchor == Some("") {
                Some(index)
            } else {
                anchor.and_then(|a| ids.get(a).copied()).filter(|a| {
                    let pos = &self.entries[*a as usize];
                    let mut cursor = parent;
                    while let Some(at) = cursor {
                        if at == *a {
                            break;
                        }
                        cursor = self.entries[at as usize].parent;
                    }
                    cursor == Some(*a)
                        && head.anchor_message.as_ref().is_none_or(|anchor| {
                            anchor.index == pos.depth
                                && pos.flags & FLAG_ANCHOR != 0
                                && pos
                                    .parent
                                    .is_none_or(|p| self.entries[p as usize].pending.is_empty())
                        })
                })
            }
        } else {
            None
        };
        if head.custom_type.as_deref() == Some(crate::CWD_CHANGE_CUSTOM_TYPE)
            && let Ok(entry) = serde_json::from_slice::<crate::entry::SessionEntry>(trimmed)
            && let Some(cwd) = entry.kind.cwd_change()
        {
            self.moved_cwd = Some(cwd.to_owned());
        }
        let branch_compaction = if flags & FLAG_COMPACTION != 0 {
            Some(index)
        } else {
            parent.and_then(|p| self.entries[p as usize].branch_compaction)
        };
        ids.insert(id.clone().into_owned(), index);
        self.entries.push(EntryPos {
            offset,
            len: u32::try_from(line.len()).unwrap_or(u32::MAX),
            id: id.into_owned(),
            parent,
            flags,
            depth,
            pending,
            checkpoint,
            compaction_summary_id: head.compaction_summary_id,
            branch_compaction,
            custom_type: if head.kind == "system_snapshot" || role == Some("system") {
                Some("__system_snapshot".into())
            } else {
                head.custom_type
            },
        });
        Ok(())
    }

    /// Context-only branch view. Inert prefix records preserve absolute anchor indexes;
    /// full-history APIs must materialize the original file instead.
    pub(crate) fn wake_entries(
        &self,
        path: &Path,
    ) -> Result<Vec<crate::entry::SessionEntry>, SessionError> {
        use crate::entry::{EntryId, EntryKind, IsoTimestamp, SessionEntry};
        let branch = self.branch();
        // Invalid checkpoints require the full context walk, including its diagnostics.
        let valid = branch.iter().all(|i| {
            let p = &self.entries[*i as usize];
            p.flags & FLAG_COMPACTION == 0 || p.checkpoint.is_some()
        });
        let checkpoint = valid
            .then(|| {
                branch
                    .iter()
                    .rev()
                    .copied()
                    .find(|i| self.entries[*i as usize].checkpoint.is_some())
            })
            .flatten();
        let start = checkpoint
            .and_then(|i| self.entries[i as usize].checkpoint)
            .and_then(|anchor| branch.iter().position(|i| *i == anchor))
            .unwrap_or(0);
        let latest_model = branch[..start]
            .iter()
            .rev()
            .find(|i| self.entries[**i as usize].flags & FLAG_MODEL != 0)
            .copied();
        let latest_assistant = branch[..start]
            .iter()
            .rev()
            .find(|i| self.entries[**i as usize].flags & FLAG_ASSISTANT != 0)
            .copied();
        let latest_thinking = branch[..start]
            .iter()
            .rev()
            .find(|i| self.entries[**i as usize].flags & FLAG_THINKING != 0)
            .copied();
        let selected: Vec<u32> = branch
            .iter()
            .enumerate()
            .filter_map(|(at, i)| {
                let flags = self.entries[*i as usize].flags;
                ((at >= start
                    && flags & (FLAG_CONTEXT | FLAG_COMPACTION | FLAG_MODEL | FLAG_THINKING) != 0)
                    || Some(*i) == latest_model
                    || Some(*i) == latest_assistant
                    || Some(*i) == latest_thinking
                    || self.entries[*i as usize]
                        .custom_type
                        .as_deref()
                        .is_some_and(|kind| kind != "subagent-step"))
                .then_some(*i)
            })
            .collect();
        let mut parsed: HashMap<String, SessionEntry> = self
            .read(path, &selected)?
            .into_iter()
            .filter_map(|value| serde_json::from_value::<SessionEntry>(value).ok())
            .map(|entry| (entry.id.to_string(), entry))
            .collect();
        if let Some(i) = latest_assistant
            && let Some(entry) = parsed.get_mut(&self.entries[i as usize].id)
            && let EntryKind::Message {
                message: oino_ai::AgentMessage::Assistant(assistant),
            } = &mut entry.kind
        {
            // Only settings use this pre-checkpoint assistant. Tool-call
            // validation was already performed by the offset scan.
            assistant.content.clear();
        }
        Ok(branch
            .into_iter()
            .map(|i| {
                let pos = &self.entries[i as usize];
                parsed.remove(&pos.id).unwrap_or_else(|| SessionEntry {
                    id: EntryId::new(&pos.id),
                    parent_id: pos
                        .parent
                        .map(|p| EntryId::new(&self.entries[p as usize].id)),
                    timestamp: IsoTimestamp::default(),
                    compaction_summary_id: pos.compaction_summary_id.clone(),
                    kind: if pos.flags & FLAG_USER != 0 && pos.depth < start {
                        EntryKind::Message {
                            message: oino_ai::AgentMessage::User(oino_ai::UserMessage {
                                author: None,
                                content: oino_ai::UserContentField::Text(String::new()),
                                timestamp: 0,
                            }),
                        }
                    } else {
                        EntryKind::Custom {
                            custom_type: "oino.index-placeholder".into(),
                            data: None,
                        }
                    },
                })
            })
            .collect())
    }

    /// One page of the current branch (see the module docs).
    ///
    /// # Errors
    ///
    /// I/O failures; [`SessionError::UnknownEntry`] when `before` is not on the branch.
    pub fn page(&self, path: &Path, request: &PageRequest) -> Result<EntryPage, SessionError> {
        let branch = self.branch();
        let end = match request.before.as_deref() {
            None => branch.len(),
            Some(before) => branch
                .iter()
                .position(|index| self.entries[*index as usize].id == before)
                .ok_or_else(|| SessionError::UnknownEntry(crate::entry::EntryId::new(before)))?,
        };
        let start = page_start(
            end,
            request.limit,
            request.max_bytes,
            |at| u64::from(self.entries[branch[at] as usize].len),
            |at| self.entries[branch[at] as usize].flags & FLAG_USER != 0,
        );
        let positions: Vec<u32> = branch[start..end].to_vec();
        let entries = self.read(path, &positions)?;
        Ok(EntryPage {
            entries,
            leaf_id: self.leaf_id().map(str::to_owned),
            total: branch.len(),
            has_more: start > 0,
            cursor: branch
                .get(start)
                .filter(|_| start < end)
                .map(|index| self.entries[*index as usize].id.clone()),
        })
    }

    /// The entries with these ids (unknown ids are skipped), in the order asked.
    ///
    /// # Errors
    ///
    /// I/O failures.
    pub fn entries_by_id(
        &self,
        path: &Path,
        ids: &[String],
    ) -> Result<Vec<JsonValue>, SessionError> {
        let positions: Vec<u32> = ids
            .iter()
            .filter_map(|id| self.by_id().get(id.as_str()).copied())
            .collect();
        self.read(path, &positions)
    }

    /// Reads and parses the given entry lines (unparsable ones are skipped).
    pub(crate) fn read(
        &self,
        path: &Path,
        positions: &[u32],
    ) -> Result<Vec<JsonValue>, SessionError> {
        let file = File::open(path).map_err(|e| SessionError::io(path, e))?;
        let mut out = Vec::with_capacity(positions.len());
        let mut buffer = Vec::new();
        for index in positions {
            let entry = &self.entries[*index as usize];
            buffer.resize(entry.len as usize, 0);
            read_at(&file, &mut buffer, entry.offset).map_err(|e| SessionError::io(path, e))?;
            if let Ok(value) = serde_json::from_slice::<JsonValue>(&buffer) {
                out.push(value);
            }
        }
        Ok(out)
    }
}

/// Where a page of a branch starts: the newest entries before `end`, at most `limit` of them
/// and at most `max_bytes` of `size` (≥ 1 entry always), moved forward to the first user
/// message in the page when there is one (turns are never cut when it can be helped; the
/// older part of a cut turn comes whole with the next page). Shared by the file index and
/// the live runtime's `_oino/entries_page`.
#[must_use]
pub fn page_start(
    end: usize,
    limit: usize,
    max_bytes: u64,
    size: impl Fn(usize) -> u64,
    is_user: impl Fn(usize) -> bool,
) -> usize {
    let limit = limit.max(1);
    let mut start = end;
    let mut bytes = 0u64;
    while start > 0 && end - start < limit {
        let next = size(start - 1);
        if end > start && bytes + next > max_bytes {
            break;
        }
        bytes += next;
        start -= 1;
    }
    if start > 0
        && let Some(aligned) = (start..end).find(|at| is_user(*at))
    {
        start = aligned;
    }
    start
}

/// `type` / `id` / `parentId` / `message.role` of one line; everything else is skipped.
#[derive(Deserialize)]
struct Head<'a> {
    #[serde(rename = "type", borrow, default)]
    kind: Cow<'a, str>,
    #[serde(borrow, default)]
    id: Option<Cow<'a, str>>,
    #[serde(rename = "parentId", borrow, default)]
    parent_id: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    message: Option<MessageHead<'a>>,
    #[serde(rename = "firstKeptEntryId", default)]
    first_kept_entry_id: Option<String>,
    #[serde(rename = "anchorMessage", default)]
    anchor_message: Option<crate::entry::CompactionAnchor>,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "customType", default)]
    custom_type: Option<String>,
    #[serde(rename = "compactionSummaryId", default)]
    compaction_summary_id: Option<crate::entry::EntryId>,
}

#[derive(Deserialize)]
struct MessageHead<'a> {
    #[serde(borrow, default)]
    role: Option<Cow<'a, str>>,
    #[serde(rename = "toolCallId", default)]
    tool_call_id: Option<String>,
    #[serde(default, deserialize_with = "tool_blocks")]
    content: Vec<ToolBlock>,
}

#[derive(Deserialize)]
struct ToolBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    id: Option<String>,
}

fn tool_blocks<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ToolBlock>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Content {
        Blocks(Vec<ToolBlock>),
        Text(serde::de::IgnoredAny),
    }
    Ok(match Content::deserialize(deserializer)? {
        Content::Blocks(blocks) => blocks,
        Content::Text(_) => Vec::new(),
    })
}

fn parse_head(line: &[u8]) -> Option<Head<'_>> {
    #[derive(Deserialize)]
    struct Bare<'a> {
        #[serde(rename = "type", borrow, default)]
        kind: Cow<'a, str>,
        #[serde(borrow, default)]
        id: Option<Cow<'a, str>>,
        #[serde(rename = "parentId", borrow, default)]
        parent_id: Option<Cow<'a, str>>,
    }
    if let Ok(head) = serde_json::from_slice::<Head<'_>>(line) {
        return Some(head);
    }
    // `message` of an unexpected shape (a string in a custom entry): retry without it.
    let bare = serde_json::from_slice::<Bare<'_>>(line).ok()?;
    Some(Head {
        kind: bare.kind,
        id: bare.id,
        parent_id: bare.parent_id,
        message: None,
        first_kept_entry_id: None,
        anchor_message: None,
        name: None,
        custom_type: None,
        compaction_summary_id: None,
    })
}

fn check_header(line: &[u8], path: &Path) -> Result<(), SessionError> {
    #[derive(Deserialize)]
    struct Header<'a> {
        #[serde(rename = "type", borrow, default)]
        kind: Cow<'a, str>,
        #[serde(default)]
        version: Option<u32>,
    }
    let header = serde_json::from_slice::<Header<'_>>(line)
        .map_err(|_| SessionError::MissingHeader(path.to_path_buf()))?;
    if header.kind != "session" {
        return Err(SessionError::MissingHeader(path.to_path_buf()));
    }
    match header.version.unwrap_or(1) {
        3 => Ok(()),
        other => Err(SessionError::UnsupportedVersion(other)),
    }
}

fn as_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)
}

#[cfg(not(unix))]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    let mut file = file;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}

/// Process-local append-chain identity, not proof that disk contents are unchanged.
/// Loads validate the prefix even when all metadata fields match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    len: u64,
    mtime: u64,
    inode: u64,
    device: u64,
    changed: (i64, i64),
    #[serde(default)]
    tail_digest: u64,
}

/// Capture before an exclusively leased append; pass to [`record_owned_append`] after success.
///
/// # Errors
/// Returns file open or metadata errors.
pub fn append_stamp(path: &Path) -> io::Result<FileStamp> {
    let file = File::open(path)?;
    let mut stamp = file_stamp(&file)?;
    let start = stamp.len.saturating_sub(4096);
    let mut tail = vec![0; usize::try_from(stamp.len - start).unwrap_or(4096)];
    read_at(&file, &mut tail, start)?;
    stamp.tail_digest = fnv1a(&tail);
    if file_stamp(&file)?
        != (FileStamp {
            tail_digest: 0,
            ..stamp.clone()
        })
    {
        return Err(io::Error::other(
            "file changed while capturing append stamp",
        ));
    }
    Ok(stamp)
}

fn file_stamp(file: &File) -> io::Result<FileStamp> {
    let meta = file.metadata()?;
    let mtime = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FileStamp {
            len: meta.len(),
            mtime: u64::try_from(mtime).unwrap_or(u64::MAX),
            inode: meta.ino(),
            device: meta.dev(),
            changed: (meta.ctime(), meta.ctime_nsec()),
            tail_digest: 0,
        })
    }
    #[cfg(not(unix))]
    Ok(FileStamp {
        len: meta.len(),
        mtime: u64::try_from(mtime).unwrap_or(u64::MAX),
        ..FileStamp::default()
    })
}

type Owned = Mutex<Vec<(PathBuf, FileStamp, FileStamp, u64)>>;
fn owned() -> &'static Owned {
    static OWNED: OnceLock<Owned> = OnceLock::new();
    OWNED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Record identity immediately after our own write. A chain is trusted only when
/// its before stamp matches the previously indexed identity exactly. Callers must
/// call this directly after the write, before allowing any other file mutation.
#[allow(
    clippy::needless_pass_by_value,
    reason = "consumes the before-append ownership token"
)]
pub fn record_owned_append(path: &Path, before: FileStamp) {
    let Ok(file) = File::open(path) else { return };
    let Ok(after) = file_stamp(&file) else { return };
    let start = before.len.saturating_sub(4096);
    let mut tail = vec![0; usize::try_from(before.len - start).unwrap_or(4096)];
    if after.inode != before.inode
        || after.device != before.device
        || after.len <= before.len
        || read_at(&file, &mut tail, start).is_err()
        || fnv1a(&tail) != before.tail_digest
        || file_stamp(&file).ok().as_ref() != Some(&after)
    {
        owned()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(p, _, _, _)| p != path);
        return;
    }
    let before = FileStamp {
        tail_digest: 0,
        ..before
    };
    let mut entries = owned().lock().unwrap_or_else(PoisonError::into_inner);
    let previous = entries
        .iter()
        .position(|(p, _, _, _)| p == path)
        .map(|i| entries.remove(i));
    let (start, generation) = match previous {
        Some((_, start, end, generation)) if end == before => (start, generation + 1),
        _ => (before, 1),
    };
    entries.push((path.to_owned(), start, after, generation));
    if entries.len() > MEMORY_CACHE {
        entries.remove(0);
    }
}

fn owned_growth(path: &Path, stamp: &FileStamp) -> Option<(FileStamp, u64)> {
    owned()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(p, _, end, _)| p == path && end == stamp)
        .map(|(_, start, _, generation)| (start.clone(), *generation))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn block_hash(file: &File, block: u64, end: u64, buffer: &mut [u8]) -> io::Result<u64> {
    let start = block * HASH_BLOCK;
    let len = usize::try_from((end - start).min(HASH_BLOCK)).unwrap_or(buffer.len());
    read_at(file, &mut buffer[..len], start)?;
    Ok(xxhash_rust::xxh3::xxh3_64(&buffer[..len]))
}

#[cfg(test)]
fn fingerprint(file: &mut File, end: u64) -> io::Result<Vec<u64>> {
    let mut buffer = vec![0; 1 << 20];
    (0..end.div_ceil(HASH_BLOCK))
        .map(|block| block_hash(file, block, end, &mut buffer))
        .collect()
}

/// The directory persisted indexes of files under `root` live in:
/// `<parent>/.<root name>.oino-offsets` (`~/.oino/.sessions.oino-offsets`), beside the root
/// like the card index — never inside it, so the sessions tree holds only session files.
#[must_use]
pub fn offsets_dir(root: &Path) -> PathBuf {
    match (root.parent(), root.file_name()) {
        (Some(parent), Some(name)) => {
            parent.join(format!(".{}{OFFSETS_DIR_SUFFIX}", name.to_string_lossy()))
        }
        _ => root.join(OFFSETS_DIR_SUFFIX),
    }
}

fn read_persisted(path: &Path) -> Option<OffsetIndex> {
    let bytes = fs::read(path).ok()?;
    if bytes.starts_with(binary::MAGIC) {
        return binary::decode(&bytes);
    }
    let (checksum, payload): (u64, String) = serde_json::from_slice(&bytes).ok()?;
    if fnv1a(payload.as_bytes()) != checksum {
        return None;
    }
    serde_json::from_str(&payload).ok()
}

fn write_persisted(path: &Path, index: &OffsetIndex) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let bytes = binary::encode(index);
    let tmp = dir.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    Ok(())
}

type Cache = Mutex<Vec<(PathBuf, Arc<OffsetIndex>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn memory_get(path: &Path) -> Option<Arc<OffsetIndex>> {
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    let at = cache.iter().position(|(key, _)| key == path)?;
    let hit = cache.remove(at);
    let index = Arc::clone(&hit.1);
    cache.push(hit);
    Some(index)
}

fn memory_take(path: &Path) -> Option<Arc<OffsetIndex>> {
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    let at = cache.iter().position(|(key, _)| key == path)?;
    Some(cache.remove(at).1)
}

fn memory_put(path: &Path, index: Arc<OffsetIndex>) {
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    cache.retain(|(key, _)| key != path);
    if let Some((_, start, end, _)) = owned()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter_mut()
        .find(|(p, _, _, _)| p == path)
        && *end == index.identity
    {
        *start = end.clone();
    }
    cache.push((path.to_path_buf(), index));
    let excess = cache.len().saturating_sub(MEMORY_CACHE);
    cache.drain(..excess);
}

/// Drops `path` from the in-memory cache (tests; a file rewritten in place).
pub fn forget_offsets(path: &Path) {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|(key, _)| key != path);
}

#[cfg(test)]
mod tests;

mod binary;

/// Bounded, coalescing cache writer. Jobs contain paths only, never a cloned history.
/// Dropping a job on saturation is safe: the next open validates/rebuilds from JSONL.
/// Neither sidecar serialization nor sidecar IO runs on a session-lock holder.
pub(super) fn schedule_snapshot(path: &Path) {
    use std::sync::mpsc::{self, SyncSender};
    static WRITER: OnceLock<Option<SyncSender<PathBuf>>> = OnceLock::new();
    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<PathBuf>(MEMORY_CACHE);
        std::thread::Builder::new()
            .name("session-index-writer".into())
            .spawn(move || {
                while let Ok(path) = receiver.recv() {
                    // One refresh per path per batch, with bounded admission/memory.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let mut paths = vec![path];
                    for path in receiver.try_iter().take(MEMORY_CACHE) {
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                    for path in paths {
                        if let Ok(index) = OffsetIndex::load_deferred(&path) {
                            let _ = write_persisted(&OffsetIndex::sidecar_path(&path), &index);
                        }
                    }
                }
            })
            .ok()
            .map(|_| sender)
    });
    if let Some(sender) = sender {
        let _ = sender.try_send(path.to_path_buf());
    }
}
