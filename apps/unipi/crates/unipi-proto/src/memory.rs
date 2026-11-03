//! Settings › Memory › Browse memories (owner fix 2026-11-03): one page of the
//! memory extension's markdown store, read by the host — never the whole store
//! (the owner's has ~7,000 memories). `memory_browse` → [`MemoryBrowseData`].
use serde::{Deserialize, Serialize};

/// Page size when the request names none.
pub const MEMORY_PAGE_DEFAULT: u32 = 50;
/// The most memories one page carries.
pub const MEMORY_PAGE_MAX: u32 = 100;
/// The longest query the host accepts (bytes); longer is cut.
pub const MEMORY_QUERY_MAX: usize = 256;
/// A memory's body is cut to this many characters in a page (the detail view's text).
pub const MEMORY_BODY_MAX: usize = 4_000;
/// At most this many project names come back with the first page.
pub const MEMORY_PROJECTS_MAX: usize = 1_000;

/// One memory row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryItem {
    /// The memory's id (front matter `id`, else the file stem).
    pub id: String,
    /// Title.
    pub title: String,
    /// `preference` / `decision` / `pattern` / `summary`.
    pub kind: String,
    /// The project (wing) folder it lives in.
    pub project: String,
    /// Last change, epoch milliseconds (the file's modification time).
    pub updated_at: u64,
    /// The markdown body, cut to [`MEMORY_BODY_MAX`] characters.
    pub body: String,
    /// The body was cut.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// One project folder and how many memories it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProject {
    /// Folder name (the extension's sanitized project name).
    pub name: String,
    /// Memory files inside.
    pub count: usize,
}

/// `memory_browse` → one page, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBrowseData {
    /// The page.
    pub items: Vec<MemoryItem>,
    /// Every match in the scope (all pages).
    pub total: usize,
    /// The next page's cursor; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// The store's projects, largest first (first page only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<MemoryProject>,
    /// The store exists on this host (false = the memory extension never wrote one).
    pub store: bool,
}

/// The memory extension's project name for a folder (`files::project_name` in
/// `unipi.memory`): the last path segment, ASCII alphanumeric runs joined by `_`,
/// lowercase; `unknown` when nothing is left.
#[must_use]
pub fn memory_project_name(cwd: &str) -> String {
    let base = cwd
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    let mut out = String::new();
    let mut gap = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_match_the_memory_extension() {
        assert_eq!(memory_project_name("/home/u/Projects/llm/oino"), "oino");
        assert_eq!(memory_project_name("/home/u/My Project-2/"), "my_project_2");
        assert_eq!(memory_project_name("/"), "unknown");
        assert_eq!(memory_project_name("C:\\work\\Tally.App"), "tally_app");
    }

    #[test]
    fn page_round_trips() {
        let page = MemoryBrowseData {
            items: vec![],
            total: 0,
            next_cursor: None,
            projects: vec![],
            store: true,
        };
        let v = serde_json::to_value(&page).unwrap();
        assert!(v.get("next_cursor").is_none());
        assert_eq!(serde_json::from_value::<MemoryBrowseData>(v).unwrap(), page);
    }
}
