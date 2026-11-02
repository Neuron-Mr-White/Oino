//! Entry offset index: open a big session at its tail without parsing the whole file.
//!
//! [`SessionManager::open`](super::SessionManager::open) parses every line into a typed
//! entry — right for a runtime, far too slow to *show* a 200 MB session (seconds of parse and
//! hundreds of MB of memory). A viewer needs only the last page of the current branch, and
//! older pages on scroll. This index records, per entry line, its byte range plus the few
//! fields the tree needs:
//!
//! - **What is stored.** For every complete line after the header: offset, length, `id`,
//!   the index of its parent, and flags (user message, compaction). The leaf is the last
//!   entry (pi semantics: a reopened session continues from its last line), so the current
//!   branch is the parent chain from the last entry.
//! - **How it is built.** One sequential pass that parses only `type`/`id`/`parentId`/
//!   `message.role` per line (serde skips everything else without allocating). Only lines
//!   ending in `\n` count: a torn tail is left for the runtime's repair and re-read once it
//!   is complete.
//! - **Append-only growth.** An index remembers where it stopped (`end`) and a fingerprint
//!   of the bytes before that point; a file that only grew is scanned from `end` on.
//! - **Persistence.** Indexes of files over [`PERSIST_MIN`] are written (atomic temp +
//!   rename, corrupt or foreign files ignored) to `<parent>/.<root>.oino-offsets/` beside
//!   the sessions root — next to the card index — keyed by a hash of the file path, so a
//!   cold host opens a known big session without a scan. A small process-wide cache keeps
//!   the hot ones in memory.
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
const FORMAT_VERSION: u32 = 1;
const FINGERPRINT_BYTES: u64 = 64;
const READ_CHUNK: usize = 4 << 20;
/// Process-wide in-memory cache size (indexes, not entries).
const MEMORY_CACHE: usize = 32;

/// The entry is a `message` with `role: "user"`.
pub const FLAG_USER: u8 = 1;
/// The entry is a `compaction`.
pub const FLAG_COMPACTION: u8 = 2;

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
    /// Offset after the last complete line read.
    end: u64,
    /// FNV-1a of the bytes before `end`.
    fingerprint: u64,
    /// Every entry line, in file order.
    entries: Vec<EntryPos>,
    /// Index of the last compaction entry in the file, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_compaction: Option<u32>,
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
        let mut file = File::open(path).map_err(|e| SessionError::io(path, e))?;
        let (size, mtime_ns) = stat(&file).map_err(|e| SessionError::io(path, e))?;
        if let Some(hit) = memory_get(path) {
            if hit.size == size && hit.mtime_ns == mtime_ns {
                return Ok(hit);
            }
        }
        let persisted = offsets_dir.map(|dir| persisted_path(dir, path));
        let stale = memory_get(path).map(|hit| (*hit).clone()).or_else(|| {
            persisted
                .as_deref()
                .and_then(read_persisted)
                .filter(|index| index.version == FORMAT_VERSION)
        });
        if let Some(stale) = stale.as_ref() {
            if stale.size == size && stale.mtime_ns == mtime_ns {
                let index = Arc::new(stale.clone());
                memory_put(path, Arc::clone(&index));
                return Ok(index);
            }
        }
        let extendable = stale.filter(|stale| {
            stale.end <= size
                && fingerprint(&mut file, stale.end).ok() == Some(stale.fingerprint)
        });
        let index = match extendable {
            Some(mut index) => {
                index.extend(&mut file, path)?;
                index
            }
            None => Self::build(&mut file, path)?,
        };
        let index = Self {
            size,
            mtime_ns,
            ..index
        };
        if size >= PERSIST_MIN {
            if let Some(persisted) = persisted.as_deref() {
                // A cache: failing to write it only costs the next cold open a scan.
                let _ = write_persisted(persisted, &index);
            }
        }
        let index = Arc::new(index);
        memory_put(path, Arc::clone(&index));
        Ok(index)
    }

    /// Scans `file` from the start (header checked, then every line).
    fn build(file: &mut File, path: &Path) -> Result<Self, SessionError> {
        let mut index = Self {
            version: FORMAT_VERSION,
            size: 0,
            mtime_ns: 0,
            end: 0,
            fingerprint: 0,
            entries: Vec::new(),
            last_compaction: None,
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
            carry.drain(..start);
            base += start as u64;
        }
        self.fingerprint = fingerprint(file, self.end).map_err(io_err)?;
        self.by_id = OnceLock::new();
        let _ = self.by_id.set(ids);
        Ok(())
    }

    /// Indexes one complete line found at `offset`.
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
        let parent = head.parent_id.and_then(|parent| ids.get(parent.as_ref()).copied());
        let mut flags = 0;
        if head.kind == "message" && head.message.and_then(|m| m.role).as_deref() == Some("user") {
            flags |= FLAG_USER;
        }
        if head.kind == "compaction" {
            flags |= FLAG_COMPACTION;
            self.last_compaction = Some(index);
        }
        ids.insert(id.clone().into_owned(), index);
        self.entries.push(EntryPos {
            offset,
            len: u32::try_from(line.len()).unwrap_or(u32::MAX),
            id: id.into_owned(),
            parent,
            flags,
        });
        Ok(())
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
    pub fn entries_by_id(&self, path: &Path, ids: &[String]) -> Result<Vec<JsonValue>, SessionError> {
        let positions: Vec<u32> = ids
            .iter()
            .filter_map(|id| self.by_id().get(id.as_str()).copied())
            .collect();
        self.read(path, &positions)
    }

    /// Reads and parses the given entry lines (unparsable ones are skipped).
    fn read(&self, path: &Path, positions: &[u32]) -> Result<Vec<JsonValue>, SessionError> {
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
    if start > 0 {
        if let Some(aligned) = (start..end).find(|at| is_user(*at)) {
            start = aligned;
        }
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
    message: Option<Role<'a>>,
}

#[derive(Deserialize)]
struct Role<'a> {
    #[serde(borrow, default)]
    role: Option<Cow<'a, str>>,
}

fn parse_head(line: &[u8]) -> Option<Head<'_>> {
    if let Ok(head) = serde_json::from_slice::<Head<'_>>(line) {
        return Some(head);
    }
    // `message` of an unexpected shape (a string in a custom entry): retry without it.
    #[derive(Deserialize)]
    struct Bare<'a> {
        #[serde(rename = "type", borrow, default)]
        kind: Cow<'a, str>,
        #[serde(borrow, default)]
        id: Option<Cow<'a, str>>,
        #[serde(rename = "parentId", borrow, default)]
        parent_id: Option<Cow<'a, str>>,
    }
    let bare = serde_json::from_slice::<Bare<'_>>(line).ok()?;
    Some(Head {
        kind: bare.kind,
        id: bare.id,
        parent_id: bare.parent_id,
        message: None,
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

fn stat(file: &File) -> io::Result<(u64, u64)> {
    let meta = file.metadata()?;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| u64::try_from(since.as_nanos()).unwrap_or(u64::MAX));
    Ok((meta.len(), mtime_ns))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn fingerprint(file: &mut File, end: u64) -> io::Result<u64> {
    let start = end.saturating_sub(FINGERPRINT_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(usize::try_from(end - start).unwrap_or(0));
    (&mut *file).take(end - start).read_to_end(&mut bytes)?;
    Ok(fnv1a(&bytes))
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

fn persisted_path(dir: &Path, file: &Path) -> PathBuf {
    let key = fnv1a(file.to_string_lossy().as_bytes());
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().chars().take(48).collect::<String>())
        .unwrap_or_default();
    dir.join(format!("{key:016x}-{stem}.json"))
}

fn read_persisted(path: &Path) -> Option<OffsetIndex> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_persisted(path: &Path, index: &OffsetIndex) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec(index).map_err(io::Error::other)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id()
    ));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
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

fn memory_put(path: &Path, index: Arc<OffsetIndex>) {
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    cache.retain(|(key, _)| key != path);
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
