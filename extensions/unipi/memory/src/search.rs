//! Search results (unipi `session.ts` `groupSearchHits`, `localSearch`) and the legacy import
//! that replaces unipi's in-place v2→v3 conversion (unipi `convert.ts`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::files::{
    self, list_project_dirs, parse_memory, parse_memory_file, sanitize_project_name, walk_md,
};

/// One hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// Title.
    pub title: String,
    /// Project.
    pub wing: String,
    /// Room (type).
    pub room: String,
    /// Score.
    pub score: f64,
    /// Snippet.
    pub snippet: String,
    /// Source path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// `added_by`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_by: Option<String>,
    /// Display source.
    pub source_label: String,
    /// Written by Oino/unipi memory.
    pub is_pi_memory: bool,
}

/// Body snippet without front matter.
#[must_use]
pub fn snippet(text: &str, max: usize) -> String {
    let body = text
        .strip_prefix("---")
        .and_then(|r| r.find("\n---").map(|i| &r[i + 4..]))
        .unwrap_or(text);
    let clean = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() > max {
        format!("{}…", clean.chars().take(max - 1).collect::<String>())
    } else {
        clean
    }
}

/// Display label of a source.
#[must_use]
pub fn source_label(raw: &str) -> String {
    match raw {
        "unipi" | "unipi-memory-bridge" => "pi".into(),
        "devin-cli" => "devin".into(),
        "zcode" => "zcode".into(),
        "" => "mempalace".into(),
        other => other.to_owned(),
    }
}

fn decode_uri_part(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn legacy_source(src: &str) -> Option<(String, String)> {
    let rest = src.strip_prefix("unipi://memory/")?;
    let (p, id) = rest.split_once('/')?;
    (!id.contains('/')).then(|| (decode_uri_part(p), decode_uri_part(id)))
}

/// Groups raw reader hits by source (best score), maps titles and labels.
/// `pi_roots` = host memory roots counted as Oino/unipi memory; `guest_of` maps a host path to
/// a readable guest path.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one mapping pass over raw hits")]
pub fn group_hits(
    raw: &[Value],
    limit: usize,
    pi_roots: &[String],
    guest_of: &dyn Fn(&str) -> Option<String>,
) -> Vec<Hit> {
    let mut grouped: BTreeMap<String, Hit> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for h in raw {
        let meta = h.get("metadata").cloned().unwrap_or(Value::Null);
        let src = h
            .get("source_path")
            .or_else(|| h.get("source_file"))
            .or_else(|| meta.get("source_file"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let key = src.clone().unwrap_or_else(|| {
            h.get("drawer_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        });
        let score = h
            .get("score")
            .or_else(|| h.get("similarity"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let legacy = src.as_deref().and_then(legacy_source);
        let pi = src.as_deref().is_some_and(|s| {
            s.starts_with("unipi://memory/")
                || pi_roots.iter().any(|r| {
                    !r.is_empty() && s.starts_with(&format!("{}/", r.trim_end_matches('/')))
                })
        });
        let doc = h
            .get("content")
            .or_else(|| h.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = if let Some((_, id)) = &legacy {
            parse_memory(doc, None).map_or_else(
                || {
                    if id.is_empty() {
                        key.clone()
                    } else {
                        id.clone()
                    }
                },
                |r| r.title,
            )
        } else if let Some(s) = &src {
            let from_file = if pi {
                guest_of(s)
                    .and_then(|g| parse_memory_file(Path::new(&g)))
                    .map(|r| r.title)
            } else {
                None
            };
            from_file.unwrap_or_else(|| {
                let base = s.rsplit('/').next().unwrap_or(s);
                let t = base
                    .strip_suffix(".md")
                    .or_else(|| base.strip_suffix(".MD"))
                    .unwrap_or(base);
                if t.is_empty() {
                    key.clone()
                } else {
                    t.to_owned()
                }
            })
        } else {
            key.clone()
        };
        let room = meta
            .get("room")
            .and_then(Value::as_str)
            .or_else(|| h.get("room").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        let wing = legacy
            .as_ref()
            .map(|(p, _)| p.clone())
            .or_else(|| {
                meta.get("wing")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .or_else(|| h.get("wing").and_then(Value::as_str).map(ToOwned::to_owned))
            .unwrap_or_default();
        let added_by = meta
            .get("added_by")
            .and_then(Value::as_str)
            .or_else(|| h.get("added_by").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        let label = if pi {
            "pi".to_owned()
        } else {
            source_label(&added_by)
        };
        let hit = Hit {
            title,
            wing,
            room,
            score,
            snippet: snippet(doc, 160),
            source_file: src,
            added_by: Some(added_by).filter(|a| !a.is_empty()),
            source_label: label,
            is_pi_memory: pi,
        };
        match grouped.get(&key) {
            Some(e) if e.score >= hit.score => {}
            Some(_) => {
                grouped.insert(key, hit);
            }
            None => {
                order.push(key.clone());
                grouped.insert(key, hit);
            }
        }
    }
    let mut out: Vec<Hit> = order
        .into_iter()
        .filter_map(|k| grouped.remove(&k))
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(limit);
    out
}

/// Term-overlap search over local md files (markdown-only mode).
#[must_use]
pub fn local_search(
    root: &Path,
    query: &str,
    limit: usize,
    project_only: Option<&str>,
) -> Vec<Hit> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|t| t.chars().count() > 1)
        .map(ToOwned::to_owned)
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut scored = Vec::new();
    for (name, dir) in list_project_dirs(root) {
        if let Some(p) = project_only
            && name != p
            && sanitize_project_name(&name) != p
        {
            continue;
        }
        let mut files = Vec::new();
        walk_md(&dir, &mut files);
        for f in files {
            let Some(r) = parse_memory_file(&f) else {
                continue;
            };
            let hay = format!("{} {} {}", r.title, r.tags.join(" "), r.content).to_lowercase();
            let hits = terms.iter().filter(|t| hay.contains(t.as_str())).count();
            if hits == 0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "term counts are small")]
            let score = hits as f64 / terms.len() as f64;
            scored.push(Hit {
                title: r.title,
                wing: name.clone(),
                room: r.kind,
                score,
                snippet: snippet(&r.content, 160),
                source_file: Some(f.display().to_string()),
                added_by: None,
                source_label: "local".into(),
                is_pi_memory: true,
            });
        }
    }
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    scored.truncate(limit);
    scored
}

/// Import state (`<root>/.import.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportState {
    /// `pending | importing | filing | done | failed`.
    pub phase: String,
    /// Done.
    pub done: usize,
    /// Total.
    pub total: usize,
    /// Failed units.
    pub failed_units: usize,
    /// Errors.
    pub errors: Vec<String>,
}

/// Reads the import state.
#[must_use]
pub fn read_import(root: &Path) -> Option<ImportState> {
    serde_json::from_slice(&std::fs::read(root.join(".import.json")).ok()?).ok()
}

/// Writes the import state.
pub fn write_import(root: &Path, s: &ImportState) {
    let _ = std::fs::create_dir_all(root);
    let _ = std::fs::write(
        root.join(".import.json"),
        serde_json::to_vec_pretty(s).unwrap_or_default(),
    );
}

/// Legacy inventory `(md files, non-sanitized dirs, markers)`.
#[must_use]
pub fn legacy_inventory(legacy: &Path) -> (usize, Vec<String>, Vec<String>) {
    let markers: Vec<String> = [".mempalace-ledger.json", ".mempalace-migrated"]
        .iter()
        .filter(|m| legacy.join(m).exists())
        .map(|m| (*m).to_owned())
        .collect();
    let mut n = 0;
    let mut odd = Vec::new();
    for (name, dir) in list_project_dirs(legacy) {
        if name != sanitize_project_name(&name) {
            odd.push(name);
        }
        let mut f = Vec::new();
        walk_md(&dir, &mut f);
        n += f.len();
    }
    (n, odd, markers)
}

/// `/unipi:memory migrate` is needed: legacy memories exist and no import finished.
#[must_use]
pub fn needs_migration(root: &Path, legacy: Option<&Path>) -> bool {
    if read_import(root).is_some_and(|s| s.phase == "done") {
        return false;
    }
    legacy.is_some_and(|l| legacy_inventory(l).0 > 0)
}

/// Copies every legacy memory into the typed layout under `root`; returns the projects
/// touched and the written files per project.
#[must_use]
pub fn import_legacy(
    root: &Path,
    legacy: &Path,
    state: &mut ImportState,
) -> BTreeMap<String, Vec<std::path::PathBuf>> {
    let mut written: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    for (name, dir) in list_project_dirs(legacy) {
        let mut f = Vec::new();
        walk_md(&dir, &mut f);
        files.extend(f.into_iter().map(|p| (sanitize_project_name(&name), p)));
    }
    state.total = files.len();
    state.phase = "importing".into();
    for (project, f) in files {
        let Some(mut r) = parse_memory_file(&f) else {
            state.failed_units += 1;
            state.errors.push(format!("unparsable: {}", f.display()));
            continue;
        };
        r.project.clone_from(&project);
        if r.id.is_empty() {
            r.id = files::id_from_title(&r.title);
        }
        let target = files::memory_file_path(root, &project, &r.kind, &r.id);
        if !target.exists() {
            match files::write_memory_file(root, &r) {
                Ok(p) => written.entry(project.clone()).or_default().push(p),
                Err(e) => {
                    state.failed_units += 1;
                    state.errors.push(format!("{}: {e}", f.display()));
                    continue;
                }
            }
        }
        state.done += 1;
    }
    written
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
    use serde_json::json;

    use super::*;
    use crate::files::{Record, memory_document, tmp};

    #[test]
    fn grouping() {
        let raw = vec![
            json!({"drawer_id": "d1", "content": "---\ntitle: T\n---\nhello world", "score": 0.4, "source_path": "/x/notes/a.md", "wing": "w", "room": "r", "added_by": "devin-cli"}),
            json!({"drawer_id": "d2", "content": "more", "score": 0.9, "source_path": "/x/notes/a.md", "wing": "w"}),
            json!({"drawer_id": "d3", "content": "---\ntitle: Legacy\n---\nbody", "similarity": 0.5, "metadata": {"source_file": "unipi://memory/my%20proj/legacy_id", "room": "decision"}}),
        ];
        let hits = group_hits(&raw, 10, &[], &|_| None);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "a");
        assert!((hits[0].score - 0.9).abs() < f64::EPSILON);
        assert_eq!(hits[1].title, "Legacy");
        assert_eq!(hits[1].wing, "my proj");
        assert_eq!(hits[1].source_label, "pi");
        assert_eq!(snippet("---\nt: x\n---\n a  b ", 160), "a b");
        assert_eq!(source_label("devin-cli"), "devin");
    }

    #[test]
    fn local_and_import() {
        let root = tmp("search");
        let legacy = tmp("legacy");
        let r = Record {
            id: "redis".into(),
            title: "redis cache".into(),
            content: "use ttl 60".into(),
            tags: vec![],
            project: "P".into(),
            kind: "decision".into(),
            created: String::new(),
            updated: String::new(),
            file: None,
        };
        std::fs::create_dir_all(legacy.join("My Proj")).unwrap_or_default();
        std::fs::write(legacy.join("My Proj/redis.md"), memory_document(&r)).unwrap_or_default();
        assert!(needs_migration(&root, Some(&legacy)));
        assert_eq!(
            legacy_inventory(&legacy),
            (1, vec!["My Proj".to_owned()], vec![])
        );
        let mut st = ImportState::default();
        let w = import_legacy(&root, &legacy, &mut st);
        assert_eq!((st.done, st.total), (1, 1));
        assert_eq!(w["my_proj"].len(), 1);
        st.phase = "done".into();
        write_import(&root, &st);
        assert!(!needs_migration(&root, Some(&legacy)));
        let hits = local_search(&root, "redis ttl", 10, Some("my_proj"));
        assert_eq!(hits.len(), 1);
        assert!((hits[0].score - 1.0).abs() < f64::EPSILON);
        assert!(local_search(&root, "redis", 10, Some("other")).is_empty());
    }
}
