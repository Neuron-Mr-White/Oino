//! Card index: fast session listing and id lookup for big session stores.
//!
//! [`super::list::session_info`] parses every line of every file — fine for a handful of
//! sessions, but a store of ~2,000 files / 6 GB (largest 220 MB) took a minute to list.
//! A *card* is what a session list shows: header (id, cwd, parent, created), the latest
//! `session_info` name, the last `oino.cwd_change` folder, the first user text, the last
//! user/assistant activity and a message count. This module reads only that:
//!
//! - **Line classification by prefix.** Entries are written with `"type"` first
//!   (`{"type":"message",…`), so a line's kind, role and timestamp come from its first
//!   bytes; only header, `session_info`, `oino.cwd_change` and the first user message
//!   lines are parsed as JSON. Other lines are skipped at `memchr` speed, never copied
//!   past [`PREFIX_KEEP`] bytes.
//! - **Bounded reads for big files.** Files up to [`FULL_SCAN_MAX`] are scanned whole;
//!   bigger ones only their head ([`HEAD_BYTES`]) and tail ([`TAIL_BYTES`], seek). Their
//!   card is then provisional (`exact: false` in the index: the count is a lower bound, a
//!   rename buried in the middle is missed) until [`CardIndex::refine`] — a background
//!   pass the host runs after a listing — reads the file whole once. Everything appended
//!   later is read incrementally.
//! - **An index keyed by path + size + mtime** (`<dir>/.oino-cards.json`, written
//!   atomically: temp file + rename, so concurrent writers — other processes — never
//!   leave a torn file; a corrupt or foreign index is ignored and rebuilt). An unchanged
//!   file is a map hit; a file that only **grew** (append-only JSONL) is scanned from the
//!   indexed end on, after a fingerprint of the bytes before that point still matches;
//!   anything else is rescanned.
//! - **Lookup by id from file names** (`<ts>_<id>.jsonl`, see
//!   [`super::files::session_file_name`]): a directory walk plus one header read per
//!   candidate — never a content scan — so an unknown id answers in milliseconds.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oino_ai::JsonValue;
use serde::{Deserialize, Serialize};

use super::list::{SessionInfo, jsonl_files};
use crate::entry::IsoTimestamp;
use crate::error::SessionError;
use crate::time::ms_to_system_time;

/// File name of the index inside the indexed directory.
pub const INDEX_FILE_NAME: &str = ".oino-cards.json";
const INDEX_VERSION: u32 = 1;
/// Files up to this size are scanned whole (exact counts).
pub const FULL_SCAN_MAX: u64 = 512 << 10;
/// Bytes read from the start of a bigger file.
pub const HEAD_BYTES: u64 = 256 << 10;
/// Bytes read from the end of a bigger file.
pub const TAIL_BYTES: u64 = 256 << 10;
/// Bytes of every line kept for classification.
const PREFIX_KEEP: usize = 512;
/// Bytes of a line kept when it is parsed (header, name, first user message, …).
const PARSE_KEEP: usize = 1 << 20;
/// Characters of the first user message kept in the index.
const FIRST_MESSAGE_MAX_CHARS: usize = 600;
/// Bytes before the indexed end that must be unchanged for an incremental scan.
const FINGERPRINT_BYTES: u64 = 64;
const READ_CHUNK: usize = 256 << 10;

/// The card fields of one session file (what the index stores).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Card {
    id: String,
    cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    moved_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activity_ms: Option<i64>,
    message_count: usize,
    #[serde(default)]
    first_message: String,
    /// `false` when only head + tail were read (the count is an estimate).
    exact: bool,
}

/// One indexed file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    size: u64,
    mtime_ns: u64,
    /// Offset after the last complete line read (incremental scans resume here).
    scanned_to: u64,
    /// FNV-1a of the [`FINGERPRINT_BYTES`] before `scanned_to`.
    fingerprint: u64,
    /// `None`: not a session file (cached too, so it is not reread).
    card: Option<Card>,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    entries: HashMap<String, Entry>,
}

/// What one directory's index knows, in memory.
#[derive(Default)]
struct State {
    loaded: bool,
    dirty: bool,
    entries: HashMap<PathBuf, Entry>,
}

/// The card index of one directory (a sessions root or a flat directory such as the
/// trash). One instance per index file per process ([`CardIndex::shared`]); other
/// processes share it through the file.
pub struct CardIndex {
    file: PathBuf,
    state: Mutex<State>,
    /// Serialises whole listings so a second concurrent listing finds the first's work.
    scan: Mutex<()>,
    /// A [`CardIndex::refine`] pass is running.
    refining: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The directory layout an index covers.
#[derive(Clone, Copy)]
enum Layout<'a> {
    /// `<root>/<encoded cwd>/*.jsonl` (dot-directories skipped).
    Root(&'a Path),
    /// `<dir>/*.jsonl`.
    Flat(&'a Path),
}

impl Layout<'_> {
    fn base(&self) -> &Path {
        match self {
            Layout::Root(dir) | Layout::Flat(dir) => dir,
        }
    }

    fn dirs(&self) -> Vec<PathBuf> {
        match self {
            Layout::Root(root) => fs::read_dir(root)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                        .map(|entry| entry.path())
                        .collect()
                })
                .unwrap_or_default(),
            Layout::Flat(dir) => vec![dir.to_path_buf()],
        }
    }

    fn files(&self) -> Vec<PathBuf> {
        self.dirs()
            .iter()
            .flat_map(|dir| jsonl_files(dir))
            .collect()
    }

    fn contains(&self, path: &Path) -> bool {
        match self {
            Layout::Root(root) => path
                .parent()
                .and_then(Path::parent)
                .is_some_and(|parent| parent == *root),
            Layout::Flat(dir) => path.parent().is_some_and(|parent| parent == *dir),
        }
    }
}

impl CardIndex {
    /// A standalone index over `file` (tests and tools; the listings use
    /// [`CardIndex::shared`]).
    #[must_use]
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            state: Mutex::new(State::default()),
            scan: Mutex::new(()),
            refining: AtomicBool::new(false),
        }
    }

    /// This process's index for `dir` (`<dir>/.oino-cards.json`).
    #[must_use]
    pub fn shared(dir: &Path) -> Arc<Self> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<CardIndex>>>> = OnceLock::new();
        let file = dir.join(INDEX_FILE_NAME);
        let mut registry = lock(REGISTRY.get_or_init(Mutex::default));
        Arc::clone(
            registry
                .entry(file.clone())
                .or_insert_with(|| Arc::new(Self::new(file))),
        )
    }

    /// The index file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file
    }

    fn ensure_loaded(&self) {
        let mut state = lock(&self.state);
        if state.loaded {
            return;
        }
        state.loaded = true;
        state.entries = read_index(&self.file).unwrap_or_default();
    }

    /// Cards of `files` (with their metadata), in input order; files that are not
    /// sessions are left out. `prune` drops index entries of files not in `files`.
    fn cards(&self, files: &[PathBuf], prune: bool) -> Vec<SessionInfo> {
        let _scan = lock(&self.scan);
        self.ensure_loaded();
        let metas: Vec<Option<(u64, u64)>> =
            files.iter().map(|path| size_and_mtime(path)).collect();
        // Hits and misses (a miss carries the stale entry for an incremental scan).
        let mut results: Vec<Option<Entry>> = vec![None; files.len()];
        let mut misses: Vec<(usize, Option<Entry>)> = Vec::new();
        {
            let state = lock(&self.state);
            for (at, (path, meta)) in files.iter().zip(&metas).enumerate() {
                let Some((size, mtime_ns)) = *meta else {
                    continue;
                };
                match state.entries.get(path) {
                    Some(entry) if entry.size == size && entry.mtime_ns == mtime_ns => {
                        results[at] = Some(entry.clone());
                    }
                    previous => misses.push((at, previous.cloned())),
                }
            }
        }
        if !misses.is_empty() {
            let scanned = scan_many(files, &misses);
            let mut state = lock(&self.state);
            for ((at, _), entry) in misses.iter().zip(scanned) {
                if let Some(entry) = entry {
                    state.entries.insert(files[*at].clone(), entry.clone());
                    results[*at] = Some(entry);
                }
            }
            state.dirty = true;
        }
        if prune {
            let mut state = lock(&self.state);
            let before = state.entries.len();
            let keep: std::collections::HashSet<&PathBuf> = files.iter().collect();
            state.entries.retain(|path, _| keep.contains(path));
            if state.entries.len() != before {
                state.dirty = true;
            }
        }
        self.flush();
        files
            .iter()
            .zip(results)
            .filter_map(|(path, entry)| {
                let entry = entry?;
                to_info(path, &entry)
            })
            .collect()
    }

    /// The card of one file (indexed; the index is saved when it learned something).
    fn card(&self, path: &Path) -> Option<SessionInfo> {
        self.ensure_loaded();
        let (size, mtime_ns) = size_and_mtime(path)?;
        let previous = {
            let state = lock(&self.state);
            match state.entries.get(path) {
                Some(entry) if entry.size == size && entry.mtime_ns == mtime_ns => {
                    return to_info(path, entry);
                }
                previous => previous.cloned(),
            }
        };
        let entry = scan_file(path, previous.as_ref())?;
        let info = to_info(path, &entry);
        {
            let mut state = lock(&self.state);
            state.entries.insert(path.to_path_buf(), entry);
            state.dirty = true;
        }
        self.flush();
        info
    }

    /// Reads every file whose card came from head + tail only (big files) whole, newest
    /// first, and calls `on_changed` with each card that changed (name, folder, count,
    /// …). One pass at a time per index (a second call returns at once); listings and
    /// lookups are not blocked meanwhile. The index is saved every few files, so an
    /// interrupted pass keeps its progress. Answers how many files were read.
    pub fn refine(&self, mut on_changed: impl FnMut(SessionInfo)) -> usize {
        struct Running<'a>(&'a AtomicBool);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        if self.refining.swap(true, Ordering::AcqRel) {
            return 0;
        }
        let _running = Running(&self.refining);
        self.ensure_loaded();
        let mut pending: Vec<(PathBuf, Entry)> = lock(&self.state)
            .entries
            .iter()
            .filter(|(_, entry)| entry.card.as_ref().is_some_and(|card| !card.exact))
            .map(|(path, entry)| (path.clone(), entry.clone()))
            .collect();
        pending.sort_by_key(|(_, entry)| {
            std::cmp::Reverse(entry.card.as_ref().and_then(|card| card.activity_ms))
        });
        let mut read = 0;
        for (path, old) in pending {
            let Some(entry) = scan_file_with(&path, None, true) else {
                continue;
            };
            read += 1;
            let changed = {
                let mut state = lock(&self.state);
                // Someone indexed a newer state of the file meanwhile: theirs stands.
                let current = state.entries.get(&path);
                if current.is_some_and(|current| current != &old) {
                    false
                } else {
                    let changed = !same_card(old.card.as_ref(), entry.card.as_ref());
                    state.entries.insert(path.clone(), entry.clone());
                    state.dirty = true;
                    changed
                }
            };
            if read % 16 == 0 {
                self.flush();
            }
            if changed && let Some(info) = to_info(&path, &entry) {
                on_changed(info);
            }
        }
        self.flush();
        read
    }

    /// The ids the index knows for files whose name does not carry the id (a renamed
    /// file, a foreign name): `(path, id)`.
    fn odd_ids(&self) -> Vec<(PathBuf, String)> {
        self.ensure_loaded();
        lock(&self.state)
            .entries
            .iter()
            .filter_map(|(path, entry)| {
                let id = &entry.card.as_ref()?.id;
                (file_id(path).is_none_or(|named| !id_matches(named, id)))
                    .then(|| (path.clone(), id.clone()))
            })
            .collect()
    }

    /// Writes the index when it changed (atomically; failures only cost the cache).
    fn flush(&self) {
        let snapshot = {
            let mut state = lock(&self.state);
            if !state.dirty {
                return;
            }
            state.dirty = false;
            IndexFile {
                version: INDEX_VERSION,
                entries: state
                    .entries
                    .iter()
                    .filter_map(|(path, entry)| Some((path.to_str()?.to_owned(), entry.clone())))
                    .collect(),
            }
        };
        if let Err(error) = write_index(&self.file, &snapshot) {
            // A read-only store still lists; the next listing retries.
            let _ = error;
            lock(&self.state).dirty = true;
        }
    }
}

/// Same card apart from how it was read.
fn same_card(a: Option<&Card>, b: Option<&Card>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            Card {
                exact: b.exact,
                ..a.clone()
            } == *b
        }
        (None, None) => true,
        _ => false,
    }
}

fn read_index(file: &Path) -> Option<HashMap<PathBuf, Entry>> {
    let bytes = fs::read(file).ok()?;
    let index: IndexFile = serde_json::from_slice(&bytes).ok()?;
    (index.version == INDEX_VERSION).then(|| {
        index
            .entries
            .into_iter()
            .map(|(path, entry)| (PathBuf::from(path), entry))
            .collect()
    })
}

fn write_index(file: &Path, index: &IndexFile) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let bytes = serde_json::to_vec(index).map_err(io::Error::other)?;
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let name = file.file_name().map_or_else(
        || INDEX_FILE_NAME.into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let temp = file.with_file_name(format!("{name}.{}.{nonce}.{nanos}.tmp", std::process::id()));
    let result = (|| {
        let mut out = File::create(&temp)?;
        out.write_all(&bytes)?;
        out.flush()?;
        drop(out);
        fs::rename(&temp, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn size_and_mtime(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    Some((meta.len(), mtime))
}

/// Scans the misses, on a few threads when there are many.
fn scan_many(files: &[PathBuf], misses: &[(usize, Option<Entry>)]) -> Vec<Option<Entry>> {
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .clamp(1, 8);
    if misses.len() < 16 || threads == 1 {
        return misses
            .iter()
            .map(|(at, stale)| scan_file(&files[*at], stale.as_ref()))
            .collect();
    }
    let next = AtomicU64::new(0);
    let out: Mutex<Vec<Option<Entry>>> = Mutex::new(vec![None; misses.len()]);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(i) = usize::try_from(next.fetch_add(1, Ordering::Relaxed))
                    .ok()
                    .filter(|i| *i < misses.len())
                {
                    let (at, previous) = &misses[i];
                    let entry = scan_file(&files[*at], previous.as_ref());
                    lock(&out)[i] = entry;
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(PoisonError::into_inner)
}

fn to_info(path: &Path, entry: &Entry) -> Option<SessionInfo> {
    let card = entry.card.as_ref()?;
    let created = IsoTimestamp::from_text(card.created.clone());
    let mtime = UNIX_EPOCH + Duration::from_nanos(entry.mtime_ns);
    let modified = card
        .activity_ms
        .filter(|ms| *ms > 0)
        .or_else(|| created.to_ms())
        .map_or(mtime, ms_to_system_time);
    Some(SessionInfo {
        path: path.to_path_buf(),
        id: card.id.clone(),
        cwd: card.cwd.clone(),
        moved_cwd: card.moved_cwd.clone(),
        name: card.name.clone(),
        parent_session_path: card.parent.as_ref().map(PathBuf::from),
        created,
        modified,
        message_count: card.message_count,
        first_message: if card.first_message.is_empty() {
            "(no messages)".into()
        } else {
            card.first_message.clone()
        },
        all_messages_text: String::new(),
    })
}

// ---------------------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------------------

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn fingerprint(file: &mut File, end: u64) -> io::Result<u64> {
    let start = end.saturating_sub(FINGERPRINT_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(usize::try_from(end - start).unwrap_or(0));
    file.take(end - start).read_to_end(&mut bytes)?;
    Ok(fnv1a(&bytes))
}

/// Builds (or extends) the index entry of one file. `None` on I/O failure.
fn scan_file(path: &Path, stale: Option<&Entry>) -> Option<Entry> {
    scan_file_with(path, stale, false)
}

/// [`scan_file`]; `whole` reads a big file whole too (exact card).
fn scan_file_with(path: &Path, stale: Option<&Entry>, whole: bool) -> Option<Entry> {
    let mut file = File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let size = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    // Append-only growth: continue from the indexed end.
    if let Some(stale) = stale.filter(|stale| {
        stale.card.is_some()
            && size > stale.scanned_to
            && size - stale.scanned_to <= FULL_SCAN_MAX
            && stale.scanned_to > 0
    }) && fingerprint(&mut file, stale.scanned_to).ok() == Some(stale.fingerprint)
    {
        let mut acc = Acc::resume(stale.card.clone().unwrap_or_default());
        let complete_to =
            scan_range(&mut file, stale.scanned_to, size, false, false, &mut acc).ok()?;
        let scanned_to = complete_to.max(stale.scanned_to);
        return Some(Entry {
            size,
            mtime_ns,
            scanned_to,
            fingerprint: fingerprint(&mut file, scanned_to).ok()?,
            card: acc.finish(),
        });
    }
    let mut acc = Acc::default();
    let scanned_to = if size <= FULL_SCAN_MAX || whole {
        scan_range(&mut file, 0, size, false, false, &mut acc).ok()?
    } else {
        scan_range(&mut file, 0, HEAD_BYTES, false, true, &mut acc).ok()?;
        if acc.invalid || !acc.header_seen {
            0
        } else {
            let tail_start = size - TAIL_BYTES;
            let tail_end = scan_range(&mut file, tail_start, size, true, false, &mut acc).ok()?;
            // The middle is unread: the count is a lower bound and a rename there is
            // missed until [`CardIndex::refine`] reads the file whole.
            acc.card.exact = false;
            tail_end
        }
    };
    Some(Entry {
        size,
        mtime_ns,
        scanned_to,
        fingerprint: fingerprint(&mut file, scanned_to).ok()?,
        card: acc.finish(),
    })
}

/// What one card scan accumulated.
struct Acc {
    card: Card,
    header_seen: bool,
    invalid: bool,
}

impl Default for Acc {
    fn default() -> Self {
        Self {
            card: Card {
                exact: true,
                ..Card::default()
            },
            header_seen: false,
            invalid: false,
        }
    }
}

impl Acc {
    fn resume(card: Card) -> Self {
        Self {
            card,
            header_seen: true,
            invalid: false,
        }
    }

    fn finish(self) -> Option<Card> {
        (self.header_seen && !self.invalid && !self.card.id.is_empty()).then_some(self.card)
    }

    /// Whether a line with this prefix is worth keeping up to [`PARSE_KEEP`] bytes.
    fn wants_full(&self, prefix: &[u8]) -> bool {
        if !self.header_seen {
            return true;
        }
        match line_type(prefix) {
            None | Some(b"session_info" | b"custom_message") => true,
            Some(b"message") => {
                self.card.first_message.is_empty()
                    && prefix_field(prefix, b"\"role\"") == Some(b"user")
            }
            Some(_) => false,
        }
    }

    fn line(&mut self, line: &[u8], truncated: bool) {
        if self.invalid {
            return;
        }
        if !self.header_seen {
            self.header(line, truncated);
            return;
        }
        match line_type(line) {
            Some(b"message") => self.message_prefix(line, truncated),
            Some(b"session_info") if !truncated => {
                if let Some(value) = parse(line) {
                    self.session_info(&value);
                }
            }
            Some(b"custom_message")
                if !truncated
                    && memchr::memmem::find(line, crate::CWD_CHANGE_CUSTOM_TYPE.as_bytes())
                        .is_some() =>
            {
                if let Some(value) = parse(line) {
                    self.custom_message(&value);
                }
            }
            // Not `{"type":"…` first: parse it whole when it fits.
            None if !truncated => {
                if let Some(value) = parse(line) {
                    self.value(&value);
                }
            }
            _ => {}
        }
    }

    fn header(&mut self, line: &[u8], truncated: bool) {
        let value = if truncated { None } else { parse(line) };
        let Some(value) = value else {
            if line.iter().all(u8::is_ascii_whitespace) {
                return;
            }
            self.invalid = true;
            return;
        };
        if value.get("type").and_then(JsonValue::as_str) != Some("session") {
            self.invalid = true;
            return;
        }
        let Some(id) = value.get("id").and_then(JsonValue::as_str) else {
            self.invalid = true;
            return;
        };
        let text = |key: &str| {
            value
                .get(key)
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        };
        id.clone_into(&mut self.card.id);
        self.card.cwd = text("cwd").unwrap_or_default();
        self.card.parent = text("parentSession");
        self.card.created = text("timestamp").unwrap_or_default();
        self.header_seen = true;
    }

    fn session_info(&mut self, value: &JsonValue) {
        self.card.name = value
            .get("name")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
    }

    fn custom_message(&mut self, value: &JsonValue) {
        if value.get("customType").and_then(JsonValue::as_str)
            != Some(crate::CWD_CHANGE_CUSTOM_TYPE)
        {
            return;
        }
        if let Some(cwd) = value
            .get("details")
            .and_then(|details| details.get("cwd"))
            .and_then(JsonValue::as_str)
            .filter(|cwd| !cwd.is_empty())
        {
            self.card.moved_cwd = Some(cwd.to_owned());
        }
    }

    /// A line in an unexpected layout, parsed whole.
    fn value(&mut self, value: &JsonValue) {
        match value.get("type").and_then(JsonValue::as_str) {
            Some("session_info") => self.session_info(value),
            Some("custom_message") => self.custom_message(value),
            Some("message") => {
                self.card.message_count += 1;
                let Some(message) = value.get("message") else {
                    return;
                };
                let role = message.get("role").and_then(JsonValue::as_str);
                if !matches!(role, Some("user" | "assistant")) {
                    return;
                }
                let activity = message
                    .get("timestamp")
                    .and_then(JsonValue::as_i64)
                    .or_else(|| {
                        value
                            .get("timestamp")
                            .and_then(JsonValue::as_str)
                            .and_then(|ts| IsoTimestamp::from_text(ts).to_ms())
                    });
                self.activity(activity);
                if role == Some("user")
                    && self.card.first_message.is_empty()
                    && let Some(text) = message_text(message)
                {
                    self.first_message(&text);
                }
            }
            _ => {}
        }
    }

    /// A `{"type":"message",…` line: role + entry timestamp from the prefix; the text
    /// of the first user message from the (bounded) line.
    fn message_prefix(&mut self, line: &[u8], truncated: bool) {
        self.card.message_count += 1;
        let role = prefix_field(line, b"\"role\"");
        if !matches!(role, Some(b"user" | b"assistant")) {
            return;
        }
        let activity = prefix_field(line, b"\"timestamp\"")
            .and_then(|ts| std::str::from_utf8(ts).ok())
            .and_then(|ts| IsoTimestamp::from_text(ts).to_ms());
        self.activity(activity);
        if role != Some(b"user") || !self.card.first_message.is_empty() {
            return;
        }
        let text = if truncated {
            first_text_of(line)
        } else {
            parse(line).and_then(|value| value.get("message").and_then(message_text))
        };
        if let Some(text) = text {
            self.first_message(&text);
        }
    }

    fn activity(&mut self, activity: Option<i64>) {
        if let Some(ms) = activity {
            self.card.activity_ms = Some(self.card.activity_ms.map_or(ms, |last| last.max(ms)));
        }
    }

    fn first_message(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.card.first_message = text.chars().take(FIRST_MESSAGE_MAX_CHARS).collect();
    }
}

fn parse(line: &[u8]) -> Option<JsonValue> {
    serde_json::from_slice(line).ok()
}

fn message_text(message: &JsonValue) -> Option<String> {
    let text = match message.get("content")? {
        JsonValue::String(text) => text.clone(),
        JsonValue::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(JsonValue::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(JsonValue::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn skip_ws(bytes: &[u8]) -> &[u8] {
    let at = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[at..]
}

/// `: "value"` (any spacing) after a key → the plain (escape-free) `value`.
fn string_value(after_key: &[u8]) -> Option<&[u8]> {
    let rest = skip_ws(after_key).strip_prefix(b":")?;
    let rest = skip_ws(rest).strip_prefix(b"\"")?;
    let end = memchr::memchr(b'"', rest)?;
    let value = &rest[..end];
    (!value.contains(&b'\\')).then_some(value)
}

/// `{"type":"<kind>"` (any spacing: Python writers put a space after `:`) → `kind`.
fn line_type(line: &[u8]) -> Option<&[u8]> {
    let rest = skip_ws(line.strip_prefix(b"{")?).strip_prefix(b"\"type\"")?;
    string_value(rest)
}

/// The plain string value of `key` (given quoted, `"role"`) in the line's first
/// [`PREFIX_KEEP`] bytes.
fn prefix_field<'a>(line: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let prefix = &line[..line.len().min(PREFIX_KEEP)];
    let start = memchr::memmem::find(prefix, key)? + key.len();
    string_value(&line[start..])
}

/// The first `"text": "…"` string of a line that did not fit for parsing.
fn first_text_of(line: &[u8]) -> Option<String> {
    let quoted = memchr::memmem::find_iter(line, b"\"text\"").find_map(|at| {
        let rest = skip_ws(&line[at + b"\"text\"".len()..]).strip_prefix(b":")?;
        let rest = skip_ws(rest);
        rest.starts_with(b"\"").then_some(rest)
    })?;
    // Find the closing quote, honouring escapes.
    let mut escaped = false;
    let mut end = None;
    for (at, byte) in quoted.iter().enumerate().skip(1) {
        match byte {
            _ if escaped => escaped = false,
            b'\\' => escaped = true,
            b'"' => {
                end = Some(at);
                break;
            }
            _ => {}
        }
    }
    let text: String = serde_json::from_slice(&quoted[..=end?]).ok()?;
    (!text.is_empty()).then_some(text)
}

/// Feeds the lines of `[start, end)` to `acc`. `skip_partial` drops everything up to
/// the first newline (a tail read starts mid-line); `flush_partial` hands a line cut by
/// `end` to `acc` as truncated (a head read). Answers the offset after the last complete
/// line.
fn scan_range(
    file: &mut File,
    start: u64,
    end: u64,
    skip_partial: bool,
    flush_partial: bool,
    acc: &mut Acc,
) -> io::Result<u64> {
    file.seek(SeekFrom::Start(start))?;
    let mut reader = file.take(end.saturating_sub(start));
    let mut chunk = vec![0_u8; READ_CHUNK];
    let mut line = LineBuf::default();
    let mut offset = start;
    let mut complete_to = start;
    let mut skipping = skip_partial && start > 0;
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let mut bytes = &chunk[..read];
        while let Some(at) = memchr::memchr(b'\n', bytes) {
            offset += (at + 1) as u64;
            if skipping {
                skipping = false;
            } else {
                line.push(&bytes[..at], acc);
                line.finish(acc);
            }
            complete_to = offset;
            bytes = &bytes[at + 1..];
        }
        offset += bytes.len() as u64;
        if !skipping {
            line.push(bytes, acc);
        }
    }
    if flush_partial && !skipping && line.len > 0 {
        acc.line(&line.kept, true);
    }
    Ok(complete_to)
}

/// The bytes kept of the line being read: its first [`PREFIX_KEEP`] bytes, or up to
/// [`PARSE_KEEP`] when [`Acc::wants_full`] says so.
struct LineBuf {
    kept: Vec<u8>,
    /// Full length so far (`len > kept.len()`: truncated).
    len: usize,
    keep: usize,
    decided: bool,
}

impl Default for LineBuf {
    fn default() -> Self {
        Self {
            kept: Vec::with_capacity(PREFIX_KEEP),
            len: 0,
            keep: PREFIX_KEEP,
            decided: false,
        }
    }
}

impl LineBuf {
    fn push(&mut self, segment: &[u8], acc: &Acc) {
        let mut copied = 0;
        if self.kept.len() < self.keep {
            copied = segment.len().min(self.keep - self.kept.len());
            self.kept.extend_from_slice(&segment[..copied]);
        }
        // The prefix is complete: decide once whether the rest is worth keeping (every
        // earlier segment was copied whole, so this continues seamlessly).
        if !self.decided && self.kept.len() >= PREFIX_KEEP {
            self.decided = true;
            if acc.wants_full(&self.kept) {
                self.keep = PARSE_KEEP;
                let more = (segment.len() - copied).min(self.keep - self.kept.len());
                self.kept.extend_from_slice(&segment[copied..copied + more]);
            }
        }
        self.len += segment.len();
    }

    fn finish(&mut self, acc: &mut Acc) {
        acc.line(&self.kept, self.len > self.kept.len());
        self.kept.clear();
        self.len = 0;
        self.keep = PREFIX_KEEP;
        self.decided = false;
    }
}

// ---------------------------------------------------------------------------------------
// Public listing and lookup
// ---------------------------------------------------------------------------------------

/// The id part of a session file name (`<ts>_<id>.jsonl`, `<ts>-<n>_<id>.jsonl`; an
/// archived copy may carry a `.<n>` before the extension).
fn file_id(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let (_, id) = stem.split_once('_')?;
    (!id.is_empty()).then_some(id)
}

/// Whether a file-name id stands for `id` (exact, or `id.<n>` from a renamed copy).
fn id_matches(named: &str, id: &str) -> bool {
    named == id
        || named.strip_prefix(id).is_some_and(|rest| {
            rest.strip_prefix('.')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}

fn sort_newest(infos: &mut [SessionInfo]) {
    infos.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.path.file_name().cmp(&a.path.file_name()))
    });
}

fn list_layout(layout: Layout<'_>) -> Vec<SessionInfo> {
    let files = layout.files();
    let index = CardIndex::shared(layout.base());
    let mut infos = index.cards(&files, true);
    sort_newest(&mut infos);
    infos
}

/// Cards of every session under a sessions root (`<root>/<encoded cwd>/*.jsonl`), newest
/// activity first — indexed, bounded reads (module docs). `all_messages_text` is empty.
#[must_use]
pub fn list_all_cards(sessions_root: &Path) -> Vec<SessionInfo> {
    list_layout(Layout::Root(sessions_root))
}

/// [`list_all_cards`] for a flat directory of session files (the trash).
#[must_use]
pub fn list_dir_cards(dir: &Path) -> Vec<SessionInfo> {
    list_layout(Layout::Flat(dir))
}

/// Finds a session under a sessions root by file path, exact id or unique id prefix
/// (≥ 4 chars) — from file names and the index, reading one header per candidate.
///
/// # Errors
///
/// [`SessionError::AmbiguousId`] when the target matches several sessions.
pub fn find_card(sessions_root: &Path, target: &str) -> Result<Option<SessionInfo>, SessionError> {
    find_layout(Layout::Root(sessions_root), target)
}

/// [`CardIndex::refine`] for a sessions root's index: reads the big files the listing
/// only sampled, calling `on_changed` per card that changed. Blocking; run it off any
/// request path.
pub fn refine_all_cards(sessions_root: &Path, on_changed: impl FnMut(SessionInfo)) -> usize {
    CardIndex::shared(sessions_root).refine(on_changed)
}

/// [`refine_all_cards`] for a flat directory (the trash).
pub fn refine_dir_cards(dir: &Path, on_changed: impl FnMut(SessionInfo)) -> usize {
    CardIndex::shared(dir).refine(on_changed)
}

/// [`find_card`] in a flat directory (the trash).
///
/// # Errors
///
/// [`SessionError::AmbiguousId`] when the target matches several sessions.
pub fn find_card_in_dir(dir: &Path, target: &str) -> Result<Option<SessionInfo>, SessionError> {
    find_layout(Layout::Flat(dir), target)
}

fn find_layout(layout: Layout<'_>, target: &str) -> Result<Option<SessionInfo>, SessionError> {
    let index = CardIndex::shared(layout.base());
    if target.is_empty() {
        return Ok(None);
    }
    let path = Path::new(target);
    if path.is_absolute() || path.extension().is_some_and(|ext| ext == "jsonl") {
        if path.is_file() && layout.contains(path) {
            return Ok(index.card(path));
        }
        if path.is_absolute() {
            return Ok(None);
        }
    }
    let files = layout.files();
    let card_of = |path: &Path| index.card(path);
    // 1. Exact id: names first, then index entries for oddly named files, then the
    //    headers of files whose name carries no id at all.
    let mut exact: Vec<SessionInfo> = files
        .iter()
        .filter(|path| file_id(path).is_some_and(|named| id_matches(named, target)))
        .filter_map(|path| card_of(path))
        .filter(|info| info.id == target)
        .collect();
    let odd = index.odd_ids();
    for (path, id) in &odd {
        if id == target && path.is_file() && !exact.iter().any(|info| &info.path == path) {
            exact.extend(card_of(path).filter(|info| info.id == target));
        }
    }
    let nameless: Vec<&PathBuf> = files
        .iter()
        .filter(|path| file_id(path).is_none() && !odd.iter().any(|(odd, _)| odd == *path))
        .collect();
    for path in &nameless {
        exact.extend(card_of(path).filter(|info| info.id == target));
    }
    match exact.len() {
        0 => {}
        1 => return Ok(exact.pop()),
        _ => {
            return Err(SessionError::AmbiguousId(
                exact.into_iter().map(|i| i.path).collect(),
            ));
        }
    }
    // 2. Unique prefix (≥ 4 chars).
    if target.chars().count() < 4 {
        return Ok(None);
    }
    let mut candidates: Vec<PathBuf> = files
        .iter()
        .filter(|path| file_id(path).is_some_and(|named| named.starts_with(target)))
        .cloned()
        .collect();
    for (path, id) in index.odd_ids() {
        if id.starts_with(target) && path.is_file() && !candidates.contains(&path) {
            candidates.push(path);
        }
    }
    for path in nameless {
        if !candidates.contains(path) {
            candidates.push(path.clone());
        }
    }
    let mut matches: Vec<SessionInfo> = candidates
        .iter()
        .filter_map(|path| card_of(path))
        .filter(|info| info.id.starts_with(target))
        .collect();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(SessionError::AmbiguousId(
            matches.into_iter().map(|info| info.path).collect(),
        )),
    }
}

#[cfg(test)]
mod tests;
