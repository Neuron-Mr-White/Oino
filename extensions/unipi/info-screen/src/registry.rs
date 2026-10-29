//! Page registry and cache (unipi `registry.ts`, `palette.ts`).
//!
//! unipi kept a `globalThis` registry that every package filled with `registerGroup`; Oino
//! collects groups from the `info-screen.groups` service fan-out (each provider answers one
//! group or a list, with its data inline — see [`Group::from_provider`]). Data is cached in
//! memory (5 s TTL) and persisted cache-first: global pages in `/cache/info-screen.json`,
//! project pages per workspace in `/cache/info-screen/<wsid>.json`, session pages never.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Snapshot format version.
pub const SNAPSHOT_VERSION: u32 = 2;
/// Memory cache TTL.
pub const CACHE_TTL_MS: i64 = 5_000;

/// One stat definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatInfo {
    /// Stat id.
    pub id: String,
    /// Label.
    pub label: String,
    /// Shown by default.
    #[serde(default = "yes")]
    pub show: bool,
}

fn yes() -> bool {
    true
}

/// Group identity for settings (id, name, stats).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupInfo {
    /// Group id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Stats.
    pub stats: Vec<StatInfo>,
}

/// Page identity: tab label, accent tone, priority (unipi `PAGE_STYLES`).
pub const PAGE_STYLES: &[(&str, &str, &str, i32)] = &[
    ("session", "Session", "warning", 10),
    ("usage", "Usage", "c3", 20),
    ("tools", "Tools", "info", 30),
    ("skills", "Skills", "c8", 40),
    ("extensions", "Modules", "c7", 50),
    ("mcp", "MCP", "c5", 60),
    ("compactor", "Compactor", "c6", 70),
    ("memory", "Memory", "c3", 80),
    ("web-api", "Web", "c4", 90),
    ("updater", "Updates", "c2", 100),
];

/// Snapshot scope of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageScope {
    /// Never persisted.
    Session,
    /// Per workspace.
    Project,
    /// Once per machine.
    Global,
}

/// unipi `PAGE_SCOPE`.
#[must_use]
pub fn scope_of(id: &str) -> PageScope {
    match id {
        "session" => PageScope::Session,
        "memory" | "mcp" | "compactor" => PageScope::Project,
        _ => PageScope::Global,
    }
}

/// Stable categorical tone for an unknown page id (hash → palette).
#[must_use]
pub fn accent_for(id: &str) -> String {
    if let Some((_, _, tone, _)) = PAGE_STYLES.iter().find(|(k, ..)| *k == id) {
        return (*tone).to_owned();
    }
    let h = id
        .chars()
        .fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    format!("c{}", h % 12 + 1)
}

/// One dashboard page.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Tab label.
    pub short: String,
    /// Accent tone.
    pub accent: String,
    /// Tab order (lower first).
    pub priority: i32,
    /// Stats.
    pub stats: Vec<StatInfo>,
    /// Owning extension (`None` = built-in).
    pub owner: Option<String>,
    /// Provider-rendered OUS body (overrides the built-in renderer).
    pub render: Option<Value>,
}

impl Group {
    /// A built-in group.
    #[must_use]
    pub fn builtin(id: &str, name: &str, priority: i32, stats: &[(&str, &str)]) -> Self {
        Self::styled(
            id,
            name,
            None,
            priority,
            stats
                .iter()
                .map(|(i, l)| StatInfo {
                    id: (*i).into(),
                    label: (*l).into(),
                    show: true,
                })
                .collect(),
            None,
            None,
        )
    }

    fn styled(
        id: &str,
        name: &str,
        short: Option<&str>,
        priority: i32,
        stats: Vec<StatInfo>,
        owner: Option<String>,
        render: Option<Value>,
    ) -> Self {
        let style = PAGE_STYLES.iter().find(|(k, ..)| *k == id);
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            short: style.map(|s| s.1).or(short).unwrap_or(name).to_owned(),
            accent: accent_for(id),
            priority: style.map_or(priority, |s| s.3),
            stats,
            owner,
            render,
        }
    }

    /// A provider's group (`{id, name, icon?, short?, priority, config: {stats}, data, render?}`)
    /// with its data.
    #[must_use]
    pub fn from_provider(owner: &str, v: &Value) -> Option<(Self, Map<String, Value>)> {
        let id = v.get("id")?.as_str()?;
        let name = v.get("name").and_then(Value::as_str).unwrap_or(id);
        #[allow(clippy::cast_possible_truncation, reason = "priorities are small")]
        let priority = v.get("priority").and_then(Value::as_i64).unwrap_or(500) as i32;
        let stats = v
            .pointer("/config/stats")
            .and_then(|s| serde_json::from_value::<Vec<StatInfo>>(s.clone()).ok())
            .unwrap_or_default();
        let data = v
            .get("data")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let g = Self::styled(
            id,
            name,
            v.get("short").and_then(Value::as_str),
            priority,
            stats,
            Some(owner.to_owned()),
            v.get("render").filter(|r| r.is_object()).cloned(),
        );
        Some((g, data))
    }

    /// Identity for settings.
    #[must_use]
    pub fn info(&self) -> GroupInfo {
        GroupInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            stats: self.stats.clone(),
        }
    }
}

/// Decodes a fan-out reply (one group or a list).
#[must_use]
pub fn decode_reply(owner: &str, v: &Value) -> Vec<(Group, Map<String, Value>)> {
    match v {
        Value::Array(items) => items
            .iter()
            .filter_map(|g| Group::from_provider(owner, g))
            .collect(),
        other => Group::from_provider(owner, other).into_iter().collect(),
    }
}

/// Sorted by `groupOrder` first (listed ids in that order), then priority, then id.
#[must_use]
pub fn ordered(mut groups: Vec<Group>, order: &[String]) -> Vec<Group> {
    let rank = |g: &Group| order.iter().position(|o| *o == g.id).unwrap_or(usize::MAX);
    groups.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id))
    });
    groups
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SnapshotFile {
    version: u32,
    #[serde(default)]
    pages: BTreeMap<String, Page>,
}

/// One cached page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// Fetch time (epoch ms).
    pub at: i64,
    /// Group data.
    pub data: Map<String, Value>,
}

/// Memory + disk cache.
#[derive(Debug, Default)]
pub struct Cache {
    mem: BTreeMap<String, Page>,
    global: Option<BTreeMap<String, Page>>,
    project: Option<BTreeMap<String, Page>>,
    /// Snapshot directory (`/cache`); `None` disables persistence.
    pub dir: Option<PathBuf>,
    /// Workspace id for project snapshots.
    pub workspace: String,
}

impl Cache {
    fn path(&self, scope: PageScope) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        match scope {
            PageScope::Global => Some(dir.join("info-screen.json")),
            PageScope::Project => {
                let id = if self.workspace.is_empty() {
                    "default".to_owned()
                } else {
                    unipi_kit::paths::sanitize(&self.workspace)
                };
                Some(dir.join("info-screen").join(format!("{id}.json")))
            }
            PageScope::Session => None,
        }
    }

    fn bucket(&mut self, scope: PageScope) -> &mut BTreeMap<String, Page> {
        let path = self.path(scope);
        let load = move || -> BTreeMap<String, Page> {
            let file = path
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice::<SnapshotFile>(&b).ok())
                .filter(|f| f.version == SNAPSHOT_VERSION)
                .map(|f| f.pages)
                .unwrap_or_default();
            file.into_iter()
                .filter(|(id, _)| scope_of(id) == scope)
                .collect()
        };
        let slot = if scope == PageScope::Project {
            &mut self.project
        } else {
            &mut self.global
        };
        slot.get_or_insert_with(load)
    }

    /// Points project snapshots at another workspace (drops project pages in memory).
    pub fn set_workspace(&mut self, wsid: &str) {
        if self.workspace == wsid {
            return;
        }
        wsid.clone_into(&mut self.workspace);
        self.project = None;
        self.mem.retain(|id, _| scope_of(id) == PageScope::Global);
    }

    /// Memory, else the last snapshot.
    pub fn cached(&mut self, id: &str) -> Option<Page> {
        if let Some(p) = self.mem.get(id) {
            return Some(p.clone());
        }
        match scope_of(id) {
            PageScope::Session => None,
            scope => self.bucket(scope).get(id).cloned(),
        }
    }

    /// Fresh memory data (within the TTL).
    #[must_use]
    pub fn fresh(&self, id: &str, now: i64) -> Option<&Page> {
        self.mem.get(id).filter(|p| now - p.at < CACHE_TTL_MS)
    }

    /// Stores fetched data.
    pub fn put(&mut self, id: &str, data: Map<String, Value>, now: i64) {
        self.mem.insert(id.to_owned(), Page { at: now, data });
    }

    /// Drops the memory entry (the snapshot stays as a stale fallback).
    pub fn invalidate(&mut self, id: &str) {
        self.mem.remove(id);
    }

    /// Writes memory pages over both snapshots (atomic rename).
    pub fn write(&mut self) {
        for scope in [PageScope::Global, PageScope::Project] {
            let fresh: Vec<(String, Page)> = self
                .mem
                .iter()
                .filter(|(id, p)| scope_of(id) == scope && !p.data.is_empty())
                .map(|(id, p)| (id.clone(), p.clone()))
                .collect();
            if fresh.is_empty() {
                continue;
            }
            let pages = {
                let b = self.bucket(scope);
                b.extend(fresh);
                b.clone()
            };
            let Some(path) = self.path(scope) else {
                continue;
            };
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let body = json!({"version": SNAPSHOT_VERSION, "pages": pages});
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, body.to_string()).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn data(v: &str) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("v".into(), json!({"value": v}));
        m
    }

    #[test]
    fn styles_scopes_and_order() {
        let g = Group::builtin("extensions", "Modules", 999, &[]);
        assert_eq!((g.short.as_str(), g.priority), ("Modules", 50));
        assert_eq!(scope_of("memory"), PageScope::Project);
        assert_eq!(scope_of("session"), PageScope::Session);
        assert_eq!(accent_for("usage"), "c3");
        assert!(accent_for("zzz").starts_with('c'));
        let groups = vec![
            Group::builtin("usage", "U", 20, &[]),
            Group::builtin("session", "S", 10, &[]),
            Group::builtin("tools", "T", 30, &[]),
        ];
        let ids: Vec<String> = ordered(groups.clone(), &[])
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, ["session", "usage", "tools"]);
        let ids: Vec<String> = ordered(groups, &["tools".into()])
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, ["tools", "session", "usage"]);
    }

    #[test]
    fn provider_groups() {
        let v = json!({"id": "compactor", "name": "Compactor", "icon": "🗜️", "priority": 115,
            "config": {"showByDefault": true, "stats": [{"id": "ratio", "label": "Ratio", "show": true}]},
            "data": {"ratio": {"value": "39k→13k"}}});
        let out = decode_reply("unipi.compactor", &json!([v.clone(), {"bad": 1}]));
        assert_eq!(out.len(), 1);
        let (g, d) = &out[0];
        assert_eq!(
            (g.short.as_str(), g.owner.as_deref()),
            ("Compactor", Some("unipi.compactor"))
        );
        assert_eq!(d["ratio"]["value"], "39k→13k");
        assert_eq!(decode_reply("x", &v).len(), 1);
    }

    #[test]
    fn snapshot_scopes_per_workspace() {
        let dir = std::env::temp_dir().join(format!("unipi-info-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Cache {
            dir: Some(dir.clone()),
            workspace: "wsA".into(),
            ..Cache::default()
        };
        a.put("memory", data("A-memories"), 1);
        a.put("usage", data("machine-usage"), 1);
        a.put("session", data("live"), 1);
        a.write();
        let mut b = Cache {
            dir: Some(dir.clone()),
            workspace: "wsB".into(),
            ..Cache::default()
        };
        assert_eq!(
            b.cached("memory"),
            None,
            "project A memory must not leak into B"
        );
        assert_eq!(
            b.cached("usage").unwrap().data["v"]["value"],
            "machine-usage"
        );
        assert_eq!(b.cached("session"), None, "session never persists");
        let mut a2 = Cache {
            dir: Some(dir.clone()),
            workspace: "wsA".into(),
            ..Cache::default()
        };
        assert_eq!(
            a2.cached("memory").unwrap().data["v"]["value"],
            "A-memories"
        );
        assert!(dir.join("info-screen.json").exists());
        assert_eq!(
            std::fs::read_dir(dir.join("info-screen")).unwrap().count(),
            1
        );
        a2.put("memory", data("x"), 1);
        a2.set_workspace("wsC");
        assert_eq!(
            a2.cached("memory"),
            None,
            "switching workspace drops project data in memory"
        );
        assert!(a2.fresh("usage", 2).is_none());
    }
}
