//! `memory_browse` (owner fix 2026-11-03, Settings › Memory › Browse memories):
//! one page of the memory extension's markdown store, newest first, with an
//! optional word search over title / body / project — read natively by the host
//! so the app never loads the whole store (the owner's holds ~7,000 memories)
//! and Settings needs no chat.
//!
//! The store is `unipi.memory`'s own: `<agent dir>/state/unipi.memory/global/memory/
//! <project>/<type>/<id>.md` (front matter `id` / `title` / `type`, markdown body).
//! Read-only here; forgetting stays the extension's (it also updates MemPalace).
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use unipi_proto::memory::{
    MemoryBrowseData, MemoryItem, MemoryProject, MEMORY_BODY_MAX, MEMORY_PAGE_DEFAULT,
    MEMORY_PAGE_MAX, MEMORY_PROJECTS_MAX, MEMORY_QUERY_MAX,
};

/// The most bytes of one memory file the host reads (title, body, search).
const FILE_READ_MAX: u64 = 256 * 1024;

/// The memory extension's store under an agent dir.
#[must_use]
pub fn store_root(agent_dir: &Path) -> PathBuf {
    agent_dir
        .join("state")
        .join("unipi.memory")
        .join("global")
        .join("memory")
}

/// A browse request (already decoded from the wire).
#[derive(Debug, Clone, Default)]
pub struct Browse<'a> {
    /// One project folder, or every project.
    pub project: Option<&'a str>,
    /// Words that must all match (empty = newest first).
    pub query: &'a str,
    pub limit: Option<u32>,
    pub cursor: Option<&'a str>,
}

/// Why a browse was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum BrowseError {
    /// The project is not a single folder name.
    BadProject,
}

fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Every `.md` under `dir` (hidden entries skipped), with its mtime in ms.
fn walk(dir: &Path, project: &str, out: &mut Vec<(u64, PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if hidden(&name) {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            walk(&entry.path(), project, out);
        } else if kind.is_file()
            && Path::new(name.as_ref())
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        {
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            out.push((mtime, entry.path(), project.to_owned()));
        }
    }
}

fn read_capped(path: &Path) -> Option<(String, bool)> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(FILE_READ_MAX).read_to_end(&mut buf).ok()?;
    let cut = buf.len() as u64 >= FILE_READ_MAX;
    Some((String::from_utf8_lossy(&buf).into_owned(), cut))
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')))
    {
        let inner = &v[1..v.len() - 1];
        return if v.starts_with('\'') {
            inner.replace("''", "'")
        } else {
            inner.replace("\\\"", "\"")
        };
    }
    v.to_owned()
}

/// `(id, title, type, body)` of a memory document; `None` without a front matter title.
fn parse(text: &str) -> Option<(Option<String>, String, String, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    let (mut id, mut title, mut kind) = (None, None, String::new());
    for line in rest[..end].lines() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        match k.trim() {
            "id" => id = Some(unquote(v)).filter(|s| !s.is_empty()),
            "title" => title = Some(unquote(v)).filter(|s| !s.is_empty()),
            "type" => kind = unquote(v),
            _ => {}
        }
    }
    let body = rest[end + 4..].trim();
    if !["preference", "decision", "pattern", "summary"].contains(&kind.as_str()) {
        kind = "summary".into();
    }
    Some((id, title?, kind, body))
}

fn item(mtime: u64, path: &Path, project: &str) -> Option<MemoryItem> {
    let (text, _) = read_capped(path)?;
    let (id, title, kind, body) = parse(&text)?;
    let id = id.unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let truncated = body.chars().count() > MEMORY_BODY_MAX;
    Some(MemoryItem {
        id,
        title,
        kind,
        project: project.to_owned(),
        updated_at: mtime,
        body: body.chars().take(MEMORY_BODY_MAX).collect(),
        truncated,
    })
}

fn matches(path: &Path, project: &str, words: &[String]) -> bool {
    let Some((text, _)) = read_capped(path) else {
        return false;
    };
    let Some((_, title, _, body)) = parse(&text) else {
        return false;
    };
    let hay = format!("{title}\n{project}\n{body}").to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// One page of the store (blocking I/O: run it off the async runtime).
///
/// # Errors
///
/// [`BrowseError::BadProject`] for a project that is not one plain folder name.
pub fn browse(root: &Path, req: &Browse<'_>) -> Result<MemoryBrowseData, BrowseError> {
    if let Some(p) = req.project {
        if p.is_empty() || p.contains(['/', '\\']) || p == "." || p == ".." || hidden(p) {
            return Err(BrowseError::BadProject);
        }
    }
    let store = root.is_dir();
    let first = req.cursor.is_none();
    // Project folders (sorted for a stable walk).
    let mut dirs: Vec<(String, PathBuf)> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .filter(|(n, _)| !hidden(n))
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    let mut files: Vec<(u64, PathBuf, String)> = Vec::new();
    let mut projects: Vec<MemoryProject> = Vec::new();
    for (name, dir) in &dirs {
        let before = files.len();
        if req.project.is_none_or(|p| p == name) || first {
            walk(dir, name, &mut files);
        }
        if first {
            projects.push(MemoryProject {
                name: name.clone(),
                count: files.len() - before,
            });
        }
        if req.project.is_some_and(|p| p != name) {
            files.truncate(before);
        }
    }
    projects.retain(|p| p.count > 0);
    projects.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    projects.truncate(MEMORY_PROJECTS_MAX);

    let mut q = req.query.trim();
    if q.len() > MEMORY_QUERY_MAX {
        let mut end = MEMORY_QUERY_MAX;
        while !q.is_char_boundary(end) {
            end -= 1;
        }
        q = &q[..end];
    }
    let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    // Newest first; the path breaks ties so pages are stable.
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    if !words.is_empty() {
        files.retain(|(_, path, project)| matches(path, project, &words));
    }
    let limit = req
        .limit
        .unwrap_or(MEMORY_PAGE_DEFAULT)
        .clamp(1, MEMORY_PAGE_MAX) as usize;
    let start = req
        .cursor
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(0)
        .min(files.len());
    let total = files.len();
    let end = (start + limit).min(total);
    let items = files[start..end]
        .iter()
        .filter_map(|(mtime, path, project)| item(*mtime, path, project))
        .collect();
    Ok(MemoryBrowseData {
        items,
        total,
        next_cursor: (end < total).then(|| end.to_string()),
        projects: if first { projects } else { Vec::new() },
        store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, project: &str, id: &str, title: &str, body: &str, age_s: u64) {
        let dir = root.join(project).join("decision");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.md"));
        std::fs::write(
            &path,
            format!("---\nid: {id}\ntitle: '{title}'\ntype: decision\ntags: []\n---\n\n{body}\n"),
        )
        .unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age_s);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn pages_newest_first_scoped_and_searched() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for i in 0..120 {
            write(root, if i % 3 == 0 { "oino" } else { "tally" }, &format!("m{i}"), &format!("Memory {i}"), "uses the host", 1000 + i);
        }
        write(root, "oino", "fresh", "Redis cache decision", "prefer refresh tokens", 1);
        std::fs::create_dir_all(root.join(".hidden")).unwrap();

        let page = browse(root, &Browse { limit: Some(50), ..Browse::default() }).unwrap();
        assert!(page.store);
        assert_eq!((page.items.len(), page.total), (50, 121));
        assert_eq!(page.items[0].title, "Redis cache decision");
        assert_eq!(page.next_cursor.as_deref(), Some("50"));
        assert_eq!(page.projects.iter().map(|p| (p.name.as_str(), p.count)).collect::<Vec<_>>(), vec![("tally", 80), ("oino", 41)]);
        let last = browse(root, &Browse { limit: Some(50), cursor: Some("100"), ..Browse::default() }).unwrap();
        assert_eq!(last.items.len(), 21);
        assert!(last.next_cursor.is_none() && last.projects.is_empty());

        let oino = browse(root, &Browse { project: Some("oino"), ..Browse::default() }).unwrap();
        assert_eq!(oino.total, 41);
        assert!(oino.items.iter().all(|m| m.project == "oino"));

        let hits = browse(root, &Browse { query: "REFRESH tokens", ..Browse::default() }).unwrap();
        assert_eq!(hits.total, 1);
        assert_eq!(hits.items[0].id, "fresh");
        assert_eq!(hits.items[0].body, "prefer refresh tokens");
        assert_eq!(browse(root, &Browse { project: Some("tally"), query: "redis", ..Browse::default() }).unwrap().total, 0);

        assert_eq!(browse(root, &Browse { project: Some("../x"), ..Browse::default() }), Err(BrowseError::BadProject));
        let none = browse(&root.join("missing"), &Browse::default()).unwrap();
        assert!(!none.store && none.total == 0);
    }

    /// QA: `MEMORY_STORE=<a COPY of a real store> cargo test -p unipi-host --lib real_store -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs MEMORY_STORE pointing at a copy of a real memory store"]
    fn real_store() {
        let Ok(root) = std::env::var("MEMORY_STORE") else { return };
        let root = Path::new(&root);
        for (label, req) in [
            ("first page", Browse { limit: Some(50), ..Browse::default() }),
            ("search", Browse { query: "release", limit: Some(50), ..Browse::default() }),
            ("project oino", Browse { project: Some("oino"), limit: Some(50), ..Browse::default() }),
        ] {
            let t = std::time::Instant::now();
            let page = browse(root, &req).unwrap();
            let bytes = serde_json::to_vec(&page).unwrap().len();
            println!("{label}: {} items of {} in {:?}, {} projects, {bytes} bytes", page.items.len(), page.total, t.elapsed(), page.projects.len());
            assert!(bytes < unipi_proto::MAX_FRAME);
        }
    }
}
