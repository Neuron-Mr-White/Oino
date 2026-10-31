//! Filesystem browser/editor wire types (UNI-155 "files — browser +
//! editor"). Mirrors `packages/protocol/src/fs.ts`.
//!
//! All `fs_*` requests carry a path under the host's allowed scope
//! (canonicalized, under `$HOME` by default; `outside:true` opts into
//! anywhere else — the app asks the user first). Errors use the shared
//! `AppErrorDto` codes, plus `E_SCOPE` (path outside the allowed scope),
//! `E_BINARY` (fs_read on a non-UTF-8 file: route to the previewer/
//! blob_get instead) and `E_CONFLICT` (fs_write's `base_hash` is stale).
//!
//! `fs_read` streams its text the same way `blob_get` streams bytes:
//! `ok{data:{size,hash,mtime,truncated}}` then the UTF-8 bytes in kind-1
//! frames, then `exit{code:0}`.

use serde::{Deserialize, Serialize};

/// A filesystem path: 1..=4096 bytes, no NUL (same shape as `BlobPath`,
/// named separately so each type can grow its own rules later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct FsPath(String);

impl FsPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FsPath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 4096 {
            return Err(format!(
                "path must be 1..=4096 characters, got {}",
                value.len()
            ));
        }
        if value.bytes().any(|b| b == 0) {
            return Err("path must not contain NUL bytes".to_string());
        }
        Ok(FsPath(value))
    }
}

impl<'de> Deserialize<'de> for FsPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        FsPath::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A `fs_search` query: 1..=512 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SearchQuery(String);

impl SearchQuery {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SearchQuery {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if (1..=512).contains(&len) {
            Ok(SearchQuery(value))
        } else {
            Err(format!("query must be 1..=512 characters, got {len}"))
        }
    }
}

impl<'de> Deserialize<'de> for SearchQuery {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SearchQuery::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// What kind of filesystem entry a listing row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

/// A `fs_search` mode: by file name, or by file content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    Name,
    Content,
}

/// Highest number of entries a `fs_list` reply carries before setting
/// `more`.
pub const FS_LIST_CAP: usize = 2000;
/// Highest number of `fs_search` matches returned.
pub const FS_SEARCH_CAP: usize = 200;
/// `fs_read` size cap before the host switches to head/tail + `truncated`.
pub const FS_READ_CAP: u64 = 2 * 1024 * 1024;

/// One `fs_list` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// ms since epoch.
    pub mtime: i64,
    /// Git porcelain status letter (`M`/`A`/`D`/`?`), when inside a repo.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub git: Option<String>,
    /// Whether `.gitignore` (or similar) excludes this entry.
    #[serde(default)]
    pub ignored: bool,
}

/// `ok.data` for `fs_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsListData {
    pub entries: Vec<FsEntry>,
    pub more: bool,
}

/// `ok.data` for `fs_read` (text follows in kind-1 frames, see module docs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsReadData {
    pub size: u64,
    /// sha256 hex of the bytes actually returned (after any truncation).
    pub hash: String,
    /// ms since epoch.
    pub mtime: i64,
    /// Set when the file was over `FS_READ_CAP`: only the head is sent.
    #[serde(default)]
    pub truncated: bool,
}

/// `ok.data` for `fs_stat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsStatData {
    pub kind: EntryKind,
    pub size: u64,
    pub mtime: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub git: Option<String>,
}

/// `ok.data` for a successful `fs_write`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsWriteData {
    pub hash: String,
    pub mtime: i64,
}

/// One `fs_search` match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsSearchResult {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub preview: Option<String>,
}

/// `ok.data` for `fs_search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsSearchData {
    pub results: Vec<FsSearchResult>,
    pub more: bool,
}

/// `ok.data` for `fs_roots`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsRootsData {
    pub roots: Vec<String>,
}

/// `detail` payload for an `E_CONFLICT` reply to `fs_write`: the file's
/// current hash so the app can offer "load theirs"/"show diff".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsConflictDetail {
    pub hash: String,
    pub mtime: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_path_validates() {
        assert!(FsPath::try_from("/home/u/f.txt".to_string()).is_ok());
        assert!(FsPath::try_from(String::new()).is_err());
        assert!(FsPath::try_from("a\0b".to_string()).is_err());
    }

    #[test]
    fn search_query_bounds() {
        assert!(SearchQuery::try_from("x".to_string()).is_ok());
        assert!(SearchQuery::try_from(String::new()).is_err());
        assert!(SearchQuery::try_from("x".repeat(513)).is_err());
    }
}
