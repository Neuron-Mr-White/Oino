//! Search results (unipi `session.ts` `groupSearchHits`, `localSearch`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::files::{list_project_dirs, parse_memory_file, sanitize_project_name, walk_md};

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
        let pi = src.as_deref().is_some_and(|s| {
            pi_roots
                .iter()
                .any(|r| !r.is_empty() && s.starts_with(&format!("{}/", r.trim_end_matches('/'))))
        });
        let doc = h
            .get("content")
            .or_else(|| h.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = if let Some(s) = &src {
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
        let wing = meta
            .get("wing")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
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
    use crate::files::{Record, tmp, write_memory_file};

    #[test]
    fn grouping() {
        let raw = vec![
            json!({"drawer_id": "d1", "content": "---\ntitle: T\n---\nhello world", "score": 0.4, "source_path": "/x/notes/a.md", "wing": "w", "room": "r", "added_by": "devin-cli"}),
            json!({"drawer_id": "d2", "content": "more", "score": 0.9, "source_path": "/x/notes/a.md", "wing": "w"}),
        ];
        let hits = group_hits(&raw, 10, &[], &|_| None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "a");
        assert!((hits[0].score - 0.9).abs() < f64::EPSILON);
        assert_eq!(snippet("---\nt: x\n---\n a  b ", 160), "a b");
        assert_eq!(source_label("devin-cli"), "devin");
    }

    #[test]
    fn local() {
        let root = tmp("search");
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
        write_memory_file(
            &root,
            &Record {
                project: "my_proj".into(),
                ..r
            },
        )
        .unwrap_or_default();
        let hits = local_search(&root, "redis ttl", 10, Some("my_proj"));
        assert_eq!(hits.len(), 1);
        assert!((hits[0].score - 1.0).abs() < f64::EPSILON);
        assert!(local_search(&root, "redis", 10, Some("other")).is_empty());
    }
}
