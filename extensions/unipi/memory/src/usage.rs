//! Per-memory usage tracking (v3 J2, Devin "Memory and Dreaming" parity): a sidecar
//! `/ext-global/memory/.usage.json` next to the markdown memories, so the dream pass can
//! prune memories no session used and keep the ones that earn their place.
//!
//! The store stays `MemPalace` + markdown; this file only adds bookkeeping:
//!
//! | event | recorded as |
//! |---|---|
//! | `memory_search` / `global_memory_search` returned the memory | a use (`search`) |
//! | the agent `read` the memory's file | a use (`read`) |
//! | the `MemPalace` wake-up injected the memory (`(<id>.md)` in the text) | a use (`wake-up`) |
//! | `memory_store` created / updated it | a source session (`sources`) |
//! | `memory_store` with `merge_from` | the merged memories' usage and sources fold in, `mergedFrom` lists them |
//! | the session-start recall reminder listed the project's titles | a project-level `recalled` counter (a title index is not a use) |
//!
//! Uses by the dream agent and the end-of-task save pass are not counted: they read
//! memories to maintain them, not to work with them.
//!
//! Staleness ([`is_stale`]): a non-preference memory unused for `stale_days`, where the
//! clock starts at `max(updated, trackingSince, lastUsed)` — memories written before
//! tracking began are never stale on day one.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::files::{MEMORY_TYPES, Record};

/// The sidecar file name (inside the memory root).
pub const FILE: &str = ".usage.json";
/// The lock file serialising read-modify-write cycles across processes.
pub const LOCK: &str = ".usage.lock";
/// Sessions kept per memory (newest last).
pub const MAX_SESSIONS: usize = 8;
/// Source sessions kept per memory (newest last).
pub const MAX_SOURCES: usize = 12;
/// Default staleness window (days).
pub const DEFAULT_STALE_DAYS: f64 = 30.0;
const DAY_MS: f64 = 86_400_000.0;

/// One memory's usage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Last use (unix ms, `0` = never).
    #[serde(default)]
    pub last_used: i64,
    /// Uses counted.
    #[serde(default)]
    pub use_count: u64,
    /// How the last use happened (`search | read | wake-up`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_kind: Option<String>,
    /// Sessions that used it (newest last, capped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<String>,
    /// Sessions that created or updated it (newest last, capped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// Keys (`project/id`) merged into this memory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_from: Vec<String>,
}

/// Project-level recall bookkeeping.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    /// Last session-start recall (unix ms).
    #[serde(default)]
    pub last_recalled: i64,
    /// Recalls counted.
    #[serde(default)]
    pub recall_count: u64,
}

/// `.usage.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageFile {
    /// Schema version.
    #[serde(default)]
    pub v: u32,
    /// When tracking began (unix ms).
    #[serde(default)]
    pub tracking_since: i64,
    /// `project/id` → usage.
    #[serde(default)]
    pub memories: BTreeMap<String, Usage>,
    /// Project → recall bookkeeping.
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectUsage>,
}

/// `project/id`.
#[must_use]
pub fn key(project: &str, id: &str) -> String {
    format!("{project}/{id}")
}

/// `(project, id)` of a memory file path (`…/<project>/<type>/<id>.md`, host or guest).
#[must_use]
pub fn key_from_path(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    let n = parts.len();
    if n < 3 {
        return None;
    }
    let id = parts[n - 1].strip_suffix(".md")?;
    if id.is_empty() || !MEMORY_TYPES.contains(&parts[n - 2]) {
        return None;
    }
    Some((parts[n - 3].to_owned(), id.to_owned()))
}

/// Memory ids named in a `MemPalace` wake-up text (`(<id>.md)`), first occurrence order.
#[must_use]
pub fn ids_in_wake_up(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices(".md)") {
        let head = &text[..i];
        let Some(open) = head.rfind('(') else {
            continue;
        };
        let id = &head[open + 1..];
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        if !out.iter().any(|o| o == id) {
            out.push(id.to_owned());
        }
    }
    out
}

fn push_capped(v: &mut Vec<String>, item: &str, cap: usize) {
    if item.is_empty() {
        return;
    }
    v.retain(|s| s != item);
    v.push(item.to_owned());
    let over = v.len().saturating_sub(cap);
    v.drain(..over);
}

impl UsageFile {
    /// Reads the sidecar (defaults on any problem).
    #[must_use]
    pub fn read(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes the sidecar (temp + rename).
    ///
    /// # Errors
    ///
    /// I/O failures.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(&tmp, path)
    }

    /// Starts the tracking clock once.
    pub fn ensure_since(&mut self, now: i64) {
        self.v = 1;
        if self.tracking_since <= 0 {
            self.tracking_since = now;
        }
    }

    /// One use of `key` in `session`.
    pub fn record_use(&mut self, key: &str, session: &str, now: i64, kind: &str) {
        self.ensure_since(now);
        let u = self.memories.entry(key.to_owned()).or_default();
        u.last_used = u.last_used.max(now);
        u.use_count += 1;
        u.last_kind = Some(kind.to_owned());
        push_capped(&mut u.sessions, session, MAX_SESSIONS);
    }

    /// `session` created or updated `key`.
    pub fn record_source(&mut self, key: &str, session: &str, now: i64) {
        self.ensure_since(now);
        let u = self.memories.entry(key.to_owned()).or_default();
        push_capped(&mut u.sources, session, MAX_SOURCES);
    }

    /// Folds `from` (deleted by a merge) into `into`.
    pub fn merge(&mut self, into: &str, from: &[String], now: i64) {
        self.ensure_since(now);
        let mut folded = Vec::new();
        for f in from.iter().filter(|f| f.as_str() != into) {
            if let Some(old) = self.memories.remove(f) {
                folded.push(old);
            }
        }
        let u = self.memories.entry(into.to_owned()).or_default();
        for old in folded {
            u.last_used = u.last_used.max(old.last_used);
            u.use_count += old.use_count;
            for s in &old.sessions {
                push_capped(&mut u.sessions, s, MAX_SESSIONS);
            }
            for s in &old.sources {
                push_capped(&mut u.sources, s, MAX_SOURCES);
            }
            for m in old.merged_from {
                push_capped(&mut u.merged_from, &m, 64);
            }
        }
        for f in from.iter().filter(|f| f.as_str() != into) {
            push_capped(&mut u.merged_from, f, 64);
        }
    }

    /// Drops `key` (deleted).
    pub fn forget(&mut self, key: &str) {
        self.memories.remove(key);
    }

    /// The session-start recall listed `project`'s titles.
    pub fn record_recall(&mut self, project: &str, now: i64) {
        self.ensure_since(now);
        let p = self.projects.entry(project.to_owned()).or_default();
        p.last_recalled = now;
        p.recall_count += 1;
    }
}

/// Unix ms of an ISO-8601 UTC timestamp (`2026-11-06T06:10:41.759Z`); `None` when unparsable.
#[must_use]
pub fn ms_from_iso(iso: &str) -> Option<i64> {
    let text = iso.trim();
    let num = |r: std::ops::Range<usize>| text.get(r).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = if text.len() >= 19 {
        (num(11..13)?, num(14..16)?, num(17..19)?)
    } else {
        (0, 0, 0)
    };
    let ms = if text.get(19..20) == Some(".") {
        let frac: String = text[20..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let f3: String = format!("{frac:0<3}").chars().take(3).collect();
        f3.parse::<i64>().unwrap_or(0)
    } else {
        0
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let m = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + ms)
}

/// A memory is stale when it is not a preference and nothing used it for `stale_days`,
/// counted from the newest of its update, its last use and the start of tracking.
#[must_use]
pub fn is_stale(
    kind: &str,
    updated_ms: Option<i64>,
    usage: Option<&Usage>,
    tracking_since: i64,
    now: i64,
    stale_days: f64,
) -> bool {
    if kind == "preference" || tracking_since <= 0 || stale_days <= 0.0 {
        return false;
    }
    let anchor = updated_ms
        .unwrap_or(0)
        .max(tracking_since)
        .max(usage.map_or(0, |u| u.last_used));
    #[allow(clippy::cast_precision_loss, reason = "ms spans fit f64")]
    let idle_days = (now - anchor) as f64 / DAY_MS;
    idle_days >= stale_days
}

/// One `memory.inventory` row.
#[must_use]
pub fn inventory_entry(
    rec: &Record,
    file_host: Option<&str>,
    usage: &UsageFile,
    now: i64,
    stale_days: f64,
) -> Value {
    let u = usage.memories.get(&key(&rec.project, &rec.id));
    let updated = ms_from_iso(&rec.updated);
    let snippet: String = rec
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    json!({
        "project": rec.project,
        "id": rec.id,
        "title": rec.title,
        "type": rec.kind,
        "tags": rec.tags,
        "created": rec.created,
        "updated": rec.updated,
        "file": file_host,
        "snippet": snippet,
        "lastUsed": u.map_or(0, |u| u.last_used),
        "useCount": u.map_or(0, |u| u.use_count),
        "sessions": u.map(|u| u.sessions.clone()).unwrap_or_default(),
        "sources": u.map(|u| u.sources.clone()).unwrap_or_default(),
        "mergedFrom": u.map(|u| u.merged_from.clone()).unwrap_or_default(),
        "stale": is_stale(&rec.kind, updated, u, usage.tracking_since, now, stale_days),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, reason = "test assertions")]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    #[test]
    fn keys_from_paths() {
        assert_eq!(
            key_from_path("/ext-global/memory/oino/decision/a_b.md"),
            Some(("oino".into(), "a_b".into()))
        );
        assert_eq!(
            key_from_path("/home/u/.oino/state/unipi.memory/global/memory/memory/pattern/x.md"),
            Some(("memory".into(), "x".into()))
        );
        assert_eq!(key_from_path("/x/oino/notes/a.md"), None);
        assert_eq!(key_from_path("a.md"), None);
        assert_eq!(key_from_path("/x/oino/decision/a.txt"), None);
    }

    #[test]
    fn wake_up_ids() {
        let t = "- foo bar ...  (oino_droid_round3.md)\n- x (oino_droid_round3.md) (other-id.md) (bad id.md)";
        assert_eq!(ids_in_wake_up(t), ["oino_droid_round3", "other-id"]);
    }

    #[test]
    fn iso_parsing() {
        assert_eq!(ms_from_iso("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(ms_from_iso("1970-01-02T00:00:01.5Z"), Some(DAY + 1500));
        assert_eq!(
            ms_from_iso("2026-11-06T06:10:41.759Z"),
            Some(1_793_945_441_759)
        );
        assert_eq!(
            ms_from_iso("2026-11-06"),
            ms_from_iso("2026-11-06T00:00:00Z")
        );
        assert_eq!(ms_from_iso("nope"), None);
    }

    #[test]
    fn uses_sources_merge_and_forget() {
        let mut f = UsageFile::default();
        f.record_use("p/a", "s1", 10, "search");
        f.record_use("p/a", "s2", 20, "read");
        f.record_use("p/a", "s1", 30, "search");
        let a = &f.memories["p/a"];
        assert_eq!(a.use_count, 3);
        assert_eq!(a.last_used, 30);
        assert_eq!(a.sessions, ["s2", "s1"]);
        assert_eq!(a.last_kind.as_deref(), Some("search"));
        assert_eq!(f.tracking_since, 10);
        f.record_source("p/b", "s9", 40);
        f.record_use("p/b", "s3", 50, "wake-up");
        f.merge("p/c", &["p/a".into(), "p/b".into(), "p/c".into()], 60);
        assert!(!f.memories.contains_key("p/a") && !f.memories.contains_key("p/b"));
        let c = &f.memories["p/c"];
        assert_eq!(c.use_count, 4);
        assert_eq!(c.last_used, 50);
        assert_eq!(c.sources, ["s9"]);
        assert_eq!(c.merged_from, ["p/a", "p/b"]);
        f.forget("p/c");
        assert!(f.memories.is_empty());
        f.record_recall("p", 70);
        f.record_recall("p", 80);
        assert_eq!(f.projects["p"].recall_count, 2);
        let mut caps = UsageFile::default();
        for i in 0..20 {
            caps.record_use("k", &format!("s{i}"), i, "search");
        }
        assert_eq!(caps.memories["k"].sessions.len(), MAX_SESSIONS);
        assert_eq!(caps.memories["k"].sessions.last().unwrap(), "s19");
    }

    #[test]
    fn staleness() {
        let now = 100 * DAY;
        let used = Usage {
            last_used: now - 2 * DAY,
            ..Usage::default()
        };
        // Tracking just began: nothing is stale, however old.
        assert!(!is_stale("decision", Some(0), None, now - DAY, now, 30.0));
        // Tracking for 40 days, never used, updated long ago: stale.
        assert!(is_stale(
            "decision",
            Some(0),
            None,
            now - 40 * DAY,
            now,
            30.0
        ));
        // Preferences never go stale.
        assert!(!is_stale(
            "preference",
            Some(0),
            None,
            now - 40 * DAY,
            now,
            30.0
        ));
        // Used recently: fresh.
        assert!(!is_stale(
            "decision",
            Some(0),
            Some(&used),
            now - 40 * DAY,
            now,
            30.0
        ));
        // Updated recently: fresh.
        assert!(!is_stale(
            "pattern",
            Some(now - DAY),
            None,
            now - 40 * DAY,
            now,
            30.0
        ));
        // No tracking / disabled window: never stale.
        assert!(!is_stale("decision", Some(0), None, 0, now, 30.0));
        assert!(!is_stale(
            "decision",
            Some(0),
            None,
            now - 40 * DAY,
            now,
            0.0
        ));
    }

    #[test]
    fn roundtrip_and_entry() {
        let dir = std::env::temp_dir().join(format!("mem-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join(FILE);
        assert_eq!(UsageFile::read(&p), UsageFile::default());
        let mut f = UsageFile::default();
        f.record_use("oino/a", "s1", 5, "search");
        f.write(&p).unwrap();
        assert_eq!(UsageFile::read(&p), f);
        let rec = Record {
            id: "a".into(),
            title: "A".into(),
            content: "body  text".into(),
            project: "oino".into(),
            kind: "decision".into(),
            updated: "1970-01-01T00:00:00Z".into(),
            ..Record::default()
        };
        let e = inventory_entry(&rec, Some("/h/a.md"), &f, 5 + 40 * DAY, 30.0);
        assert_eq!(e["useCount"], 1);
        assert_eq!(e["lastUsed"], 5);
        assert_eq!(e["snippet"], "body text");
        assert_eq!(e["stale"], true);
        assert_eq!(e["file"], "/h/a.md");
    }
}
