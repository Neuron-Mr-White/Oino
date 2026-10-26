//! Markdown memory files, the durable tier (unipi `files.ts`, `paths.ts`): front matter,
//! naming rules, project scans, title matching. Paths are guest paths under the memory root
//! (`/ext-global/memory/<project>/<type>/<id>.md`).

use std::path::{Path, PathBuf};

/// The four memory types.
pub const MEMORY_TYPES: [&str; 4] = ["preference", "decision", "pattern", "summary"];

/// One memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// Id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Body.
    pub content: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Project (wing).
    pub project: String,
    /// Type.
    pub kind: String,
    /// ISO created.
    pub created: String,
    /// ISO updated.
    pub updated: String,
    /// File it came from.
    pub file: Option<PathBuf>,
}

fn sanitize_run(s: &str, lower_only: bool) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        let keep = if lower_only {
            c.is_ascii_lowercase() || c.is_ascii_digit()
        } else {
            c.is_ascii_alphanumeric()
        };
        if keep {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// `[^A-Za-z0-9]+ → _`, trimmed, lowercased, `unknown` when empty.
#[must_use]
pub fn sanitize_project_name(name: &str) -> String {
    let s = sanitize_run(name, false).to_lowercase();
    if s.is_empty() { "unknown".into() } else { s }
}

/// Project name of a cwd.
#[must_use]
pub fn project_name(cwd: &str) -> String {
    let base = cwd
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    sanitize_project_name(base)
}

/// Id from a title (`[^a-z0-9]+ → _` on the lowercased title, untrimmed).
#[must_use]
pub fn id_from_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut out = String::new();
    let mut gap = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    if gap {
        out.push('_');
    }
    out
}

/// The filename-stem fallback id.
#[must_use]
pub fn safe_id_part(stem: &str) -> String {
    sanitize_project_name(stem)
}

/// `<root>/<project>`.
#[must_use]
pub fn project_dir(root: &Path, project: &str) -> PathBuf {
    root.join(project)
}

/// `<root>/<project>/<type>/<id>.md`.
#[must_use]
pub fn memory_file_path(root: &Path, project: &str, kind: &str, id: &str) -> PathBuf {
    root.join(project).join(kind).join(format!("{id}.md"))
}

/// The per-project `mempalace.yaml`.
#[must_use]
pub fn mempalace_yaml(project: &str) -> String {
    let room = |n: &str, d: &str, k: &[&str]| {
        format!(
            "  - name: {n}\n    description: {d}\n    keywords: [{}]",
            k.join(", ")
        )
    };
    [
        format!("wing: {project}"),
        "rooms:".into(),
        room("preference", "user preferences", &["preference", "prefer"]),
        room("decision", "decisions", &["decision", "decided"]),
        room("pattern", "patterns", &["pattern"]),
        room("summary", "summaries", &["summary", "summarize"]),
        room("general", "catch-all", &["general"]),
        String::new(),
    ]
    .join("\n")
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].replace("''", "'");
    }
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        return v[1..v.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\\\", "\\");
    }
    v.to_owned()
}

fn flow_list(v: &str) -> Vec<String> {
    let inner = v.trim().trim_start_matches('[').trim_end_matches(']');
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(unquote)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parses a memory document (front matter subset of YAML used by memory files).
#[must_use]
pub fn parse_memory(text: &str, file: Option<&Path>) -> Option<Record> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    let fm = &rest[..end];
    let body = rest[end + 4..]
        .trim_start_matches(['\r'])
        .strip_prefix('\n')
        .unwrap_or(&rest[end + 4..]);
    let mut rec = Record::default();
    let mut title = None;
    let mut id = None;
    let mut lines = fm.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let k = k.trim();
        let v = v.trim();
        match k {
            "id" => id = Some(unquote(v)),
            "title" => title = Some(unquote(v)),
            "project" => rec.project = unquote(v),
            "created" => rec.created = unquote(v),
            "updated" => rec.updated = unquote(v),
            "type" => rec.kind = unquote(v),
            "tags" if v.starts_with('[') => rec.tags = flow_list(v),
            "tags" if v.is_empty() => {
                while let Some(next) = lines.peek() {
                    let t = next.trim_start();
                    let Some(item) = t.strip_prefix("- ").or_else(|| (t == "-").then_some(""))
                    else {
                        break;
                    };
                    rec.tags.push(unquote(item));
                    lines.next();
                }
            }
            _ => {}
        }
    }
    let title = title.filter(|t| !t.is_empty())?;
    let file_id = file
        .and_then(|f| f.file_stem())
        .map(|s| safe_id_part(&s.to_string_lossy()))
        .unwrap_or_default();
    rec.id = id.filter(|i| !i.is_empty()).unwrap_or(file_id);
    rec.title = title;
    body.trim().clone_into(&mut rec.content);
    if !MEMORY_TYPES.contains(&rec.kind.as_str()) {
        rec.kind = "summary".into();
    }
    rec.file = file.map(Path::to_path_buf);
    Some(rec)
}

fn needs_quote(s: &str) -> bool {
    if s.is_empty() || s != s.trim() {
        return true;
    }
    let first = s.chars().next().unwrap_or(' ');
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        return true;
    }
    if s.contains(": ") || s.contains(" #") || s.ends_with(':') || s.contains('\n') {
        return true;
    }
    let l = s.to_lowercase();
    matches!(
        l.as_str(),
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off"
    ) || s.parse::<f64>().is_ok()
        || (s.len() >= 10
            && s.as_bytes()[4] == b'-'
            && s.as_bytes()[7] == b'-'
            && s[..4].chars().all(|c| c.is_ascii_digit()))
}

fn yaml_str(s: &str) -> String {
    if needs_quote(s) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        s.to_owned()
    }
}

/// Serializes a record (js-yaml style front matter + body).
#[must_use]
pub fn memory_document(r: &Record) -> String {
    let tags = if r.tags.is_empty() {
        "tags: []".to_owned()
    } else {
        format!(
            "tags:\n{}",
            r.tags
                .iter()
                .map(|t| format!("  - {}", yaml_str(t)))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "---\nid: {}\ntitle: {}\n{tags}\nproject: {}\ncreated: {}\nupdated: {}\ntype: {}\n---\n\n{}\n",
        yaml_str(&r.id),
        yaml_str(&r.title),
        yaml_str(&r.project),
        yaml_str(&r.created),
        yaml_str(&r.updated),
        yaml_str(&r.kind),
        r.content
    )
}

/// Reads a memory file.
#[must_use]
pub fn parse_memory_file(path: &Path) -> Option<Record> {
    parse_memory(&std::fs::read_to_string(path).ok()?, Some(path))
}

/// Writes the record's file; returns its path.
///
/// # Errors
///
/// I/O failures.
pub fn write_memory_file(root: &Path, r: &Record) -> std::io::Result<PathBuf> {
    let p = memory_file_path(root, &r.project, &r.kind, &r.id);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, memory_document(r))?;
    Ok(p)
}

/// Writes `mempalace.yaml` and the `/*.md` `.gitignore` when missing.
pub fn ensure_mempalace_yaml(root: &Path, project: &str) {
    let dir = project_dir(root, project);
    let _ = std::fs::create_dir_all(&dir);
    let y = dir.join("mempalace.yaml");
    if !y.exists() {
        let _ = std::fs::write(&y, mempalace_yaml(project));
    }
    let g = dir.join(".gitignore");
    if !g.exists() {
        let _ = std::fs::write(&g, "/*.md\n");
    }
}

/// Project directories under the root `(name, path)`.
#[must_use]
pub fn list_project_dirs(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

/// Every `.md` under `dir` (hidden entries skipped).
pub fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            walk_md(&e.path(), out);
        } else if Path::new(&name)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        {
            out.push(e.path());
        }
    }
}

/// All records of a project (case-variant dirs included, any depth).
#[must_use]
pub fn scan_project(root: &Path, project: &str) -> Vec<Record> {
    let mut files = Vec::new();
    for (name, dir) in list_project_dirs(root) {
        if name == project || sanitize_project_name(&name) == project {
            walk_md(&dir, &mut files);
        }
    }
    files.iter().filter_map(|f| parse_memory_file(f)).collect()
}

/// Jaccard over > 2-char words.
#[must_use]
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let norm = |s: &str| -> std::collections::BTreeSet<String> {
        sanitize_run(&s.to_lowercase(), true)
            .split('_')
            .filter(|w| w.len() > 2)
            .map(ToOwned::to_owned)
            .collect()
    };
    let (wa, wb) = (norm(a), norm(b));
    let union = wa.union(&wb).count();
    if union == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "word counts are small")]
    let r = wa.intersection(&wb).count() as f64 / union as f64;
    r
}

/// Exact (case-insensitive) title match.
#[must_use]
pub fn find_by_title(root: &Path, project: &str, title: &str) -> Option<Record> {
    let lowered = title.trim().to_lowercase();
    scan_project(root, project)
        .into_iter()
        .find(|r| r.title == title || r.title.to_lowercase() == lowered)
}

/// Similar titles (≥ threshold), best first.
#[must_use]
pub fn find_similar(root: &Path, project: &str, title: &str, threshold: f64) -> Vec<(Record, f64)> {
    let mut v: Vec<(Record, f64)> = scan_project(root, project)
        .into_iter()
        .map(|r| {
            let s = title_similarity(title, &r.title);
            (r, s)
        })
        .filter(|(_, s)| *s >= threshold)
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

#[cfg(test)]
pub(crate) fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("unipi-memory-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    fn rec(title: &str) -> Record {
        Record {
            id: id_from_title(title),
            title: title.into(),
            content: "Body text".into(),
            tags: vec!["a".into(), "b c".into()],
            project: "proj".into(),
            kind: "decision".into(),
            created: "2026-01-01T00:00:00.000Z".into(),
            updated: "2026-01-01T00:00:00.000Z".into(),
            file: None,
        }
    }

    #[test]
    fn naming() {
        assert_eq!(project_name("/home/u/My Project!"), "my_project");
        assert_eq!(project_name("/"), "unknown");
        assert_eq!(sanitize_project_name("EnvStripper"), "envstripper");
        assert_eq!(id_from_title("Auth: JWT refresh!"), "auth_jwt_refresh_");
        assert_eq!(safe_id_part("--Foo.Bar--"), "foo_bar");
        assert!(mempalace_yaml("p").starts_with("wing: p\nrooms:\n  - name: preference"));
    }

    #[test]
    fn documents() {
        let r = rec("auth_jwt: prefer refresh");
        let doc = memory_document(&r);
        assert!(doc.contains("title: 'auth_jwt: prefer refresh'"), "{doc}");
        assert!(doc.contains("created: '2026-01-01T00:00:00.000Z'"));
        let back = parse_memory(&doc, None).unwrap_or_default();
        assert_eq!(back.title, r.title);
        assert_eq!(back.tags, r.tags);
        assert_eq!(back.kind, "decision");
        assert_eq!(back.content, "Body text");
        let legacy = parse_memory(
            "---\ntitle: \"x\"\ntags: [a, 'b']\ntype: weird\n---\nbody",
            Some(Path::new("/m/Old File.md")),
        )
        .unwrap_or_default();
        assert_eq!(
            (legacy.id.as_str(), legacy.kind.as_str()),
            ("old_file", "summary")
        );
        assert_eq!(legacy.tags, ["a", "b"]);
        assert!(parse_memory("no front matter", None).is_none());
        assert!(parse_memory("---\nid: x\n---\nbody", None).is_none());
    }

    #[test]
    fn scanning() {
        let root = tmp("scan");
        let r = rec("redis cache decision");
        write_memory_file(&root, &r).unwrap_or_default();
        let mut legacy = rec("Old flat note");
        legacy.project = "Proj".into();
        std::fs::create_dir_all(root.join("Proj")).unwrap_or_default();
        std::fs::write(root.join("Proj/flat.md"), memory_document(&legacy)).unwrap_or_default();
        ensure_mempalace_yaml(&root, "proj");
        assert!(root.join("proj/mempalace.yaml").exists() && root.join("proj/.gitignore").exists());
        assert_eq!(scan_project(&root, "proj").len(), 2);
        assert!(find_by_title(&root, "proj", "REDIS cache decision").is_some());
        assert_eq!(
            find_similar(&root, "proj", "redis cache decision notes", 0.6).len(),
            1
        );
        assert!((title_similarity("a b", "c") - 0.0).abs() < f64::EPSILON);
        assert!((title_similarity("redis cache", "redis cache") - 1.0).abs() < f64::EPSILON);
    }
}
