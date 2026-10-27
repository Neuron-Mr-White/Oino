//! Sidebar session list (gpui-app §5.1): sessions grouped by workspace (cwd), most recent
//! first, with live status dots for sessions open in this app.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A session as listed in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    /// Session id.
    pub id: String,
    /// Session file (absent for unsaved sessions).
    pub path: Option<PathBuf>,
    /// Working directory.
    pub cwd: String,
    /// Display title (name, else first prompt).
    pub title: String,
    /// Last activity, Unix milliseconds.
    pub modified_ms: i64,
    /// Number of messages.
    pub message_count: usize,
}

/// Live status of a session open in this process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LiveStatus {
    /// Not running.
    #[default]
    Idle,
    /// Streaming / executing tools.
    Running,
    /// A dialog waits for the user.
    Attention,
    /// Finished while not focused (unread).
    Unread,
}

/// One row in a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    /// Summary.
    pub summary: SessionSummary,
    /// Live status.
    pub status: LiveStatus,
    /// Pinned.
    pub pinned: bool,
}

/// Sessions of one project (working directory), shown as a list-column section (A2.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectGroup {
    /// Working directory.
    pub cwd: String,
    /// Short label (last path component).
    pub label: String,
    /// Sessions, most recent first.
    pub sessions: Vec<SessionEntry>,
}

/// Former name of [`ProjectGroup`] (kept for the mobile shell until `gui-fix-mobile`).
pub type WorkspaceGroup = ProjectGroup;

impl LiveStatus {
    /// Attention-first order (A2.8): needs you, working, done-unseen, then the rest.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Attention => 0,
            Self::Running => 1,
            Self::Unread => 2,
            Self::Idle => 3,
        }
    }

    /// The state word of a row (A2.2 state colours: colour + icon + word).
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Attention => "Needs you",
            Self::Running => "Working",
            Self::Unread => "Done",
            Self::Idle => "Idle",
        }
    }
}

/// The session list model.
#[derive(Clone, Debug, Default)]
pub struct SessionList {
    all: Vec<SessionSummary>,
    live: HashMap<String, LiveStatus>,
    pinned: HashSet<String>,
    collapsed: HashSet<String>,
    hidden: HashSet<String>,
    by_name: bool,
    query: String,
}

/// Last path component of `cwd` (`~` for an empty path).
#[must_use]
pub fn workspace_label(cwd: &str) -> String {
    project_label(cwd)
}

/// The project name of `cwd`: its last path component (`~` for an empty path).
#[must_use]
pub fn project_label(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("~")
        .to_owned()
}

/// Compact relative age (`now`, `5m`, `3h`, `2d`, `6w`).
#[must_use]
pub fn age(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..60 => "now".to_owned(),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        86_400..1_209_600 => format!("{}d", secs / 86_400),
        _ => format!("{}w", secs / 604_800),
    }
}

impl SessionList {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the listed sessions (keeps live state and pins).
    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.all = sessions;
    }

    /// Inserts or updates one session.
    pub fn upsert(&mut self, summary: SessionSummary) {
        match self.all.iter_mut().find(|s| s.id == summary.id) {
            Some(existing) => *existing = summary,
            None => self.all.push(summary),
        }
    }

    /// Renames a session id (runtime replaced the session: new / fork).
    pub fn rename_id(&mut self, old: &str, new: &str) {
        if let Some(s) = self.all.iter_mut().find(|s| s.id == old) {
            new.clone_into(&mut s.id);
        }
        if let Some(status) = self.live.remove(old) {
            self.live.insert(new.to_owned(), status);
        }
    }

    /// Sets the live status.
    pub fn set_live(&mut self, id: &str, status: LiveStatus) {
        self.live.insert(id.to_owned(), status);
    }

    /// Live status of `id`.
    #[must_use]
    pub fn live(&self, id: &str) -> LiveStatus {
        self.live.get(id).copied().unwrap_or_default()
    }

    /// Toggles a pin.
    pub fn toggle_pin(&mut self, id: &str) {
        if !self.pinned.remove(id) {
            self.pinned.insert(id.to_owned());
        }
    }

    /// Drops a session from the list (deleted).
    pub fn remove(&mut self, id: &str) {
        self.all.retain(|s| s.id != id);
        self.live.remove(id);
        self.pinned.remove(id);
    }

    /// Hides a workspace group from the sidebar ("Remove"; its sessions stay on disk and the
    /// group comes back when a session there is opened again).
    pub fn hide_group(&mut self, cwd: &str) {
        self.hidden.insert(cwd.to_owned());
    }

    /// Shows a hidden group again.
    pub fn show_group(&mut self, cwd: &str) {
        self.hidden.remove(cwd);
    }

    /// Toggles group order: most recent first ↔ by name (the ⇅ button).
    pub fn toggle_sort(&mut self) {
        self.by_name = !self.by_name;
    }

    /// Whether groups are sorted by name.
    #[must_use]
    pub fn sorted_by_name(&self) -> bool {
        self.by_name
    }

    /// Toggles a workspace group open/closed.
    pub fn toggle_group(&mut self, cwd: &str) {
        if !self.collapsed.remove(cwd) {
            self.collapsed.insert(cwd.to_owned());
        }
    }

    /// Whether a group is collapsed.
    #[must_use]
    pub fn is_collapsed(&self, cwd: &str) -> bool {
        self.collapsed.contains(cwd)
    }

    /// Sets the filter text.
    pub fn set_query(&mut self, query: &str) {
        query.trim().to_lowercase().clone_into(&mut self.query);
    }

    /// Summary by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&SessionSummary> {
        self.all.iter().find(|s| s.id == id)
    }

    /// Number of sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.all.len()
    }

    /// Whether empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    fn matches(&self, s: &SessionSummary) -> bool {
        self.query.is_empty()
            || s.title.to_lowercase().contains(&self.query)
            || s.cwd.to_lowercase().contains(&self.query)
    }

    fn entry(&self, s: &SessionSummary) -> SessionEntry {
        SessionEntry {
            summary: s.clone(),
            status: self.live(&s.id),
            pinned: self.pinned.contains(&s.id),
        }
    }

    /// Pinned sessions (most recent first).
    #[must_use]
    pub fn pinned(&self) -> Vec<SessionEntry> {
        let mut out: Vec<SessionEntry> = self
            .all
            .iter()
            .filter(|s| self.pinned.contains(&s.id) && self.matches(s))
            .map(|s| self.entry(s))
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.summary.modified_ms));
        out
    }

    /// Project sections attention-first (A2.8): sessions inside a section by state rank then
    /// recency; sections by their best state, then recency (or by name when sorted by name).
    #[must_use]
    pub fn sections(&self) -> Vec<ProjectGroup> {
        let mut groups = self.groups();
        for g in &mut groups {
            g.sessions
                .sort_by_key(|e| (e.status.rank(), std::cmp::Reverse(e.summary.modified_ms)));
        }
        if !self.by_name {
            // `groups` is already most-recent-first; a stable sort keeps that within a rank.
            groups.sort_by_key(|g| g.sessions.first().map_or(u8::MAX, |e| e.status.rank()));
        }
        groups
    }

    /// Sessions waiting for the user, most recent first.
    #[must_use]
    pub fn needs_you(&self) -> Vec<SessionEntry> {
        let mut out: Vec<SessionEntry> = self
            .all
            .iter()
            .filter(|s| self.live(&s.id) == LiveStatus::Attention)
            .map(|s| self.entry(s))
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.summary.modified_ms));
        out
    }

    /// Number of sessions in `status`.
    #[must_use]
    pub fn count(&self, status: LiveStatus) -> usize {
        self.all
            .iter()
            .filter(|s| self.live(&s.id) == status)
            .count()
    }

    /// Workspace groups ordered by their most recent session.
    #[must_use]
    pub fn groups(&self) -> Vec<ProjectGroup> {
        let mut groups: Vec<ProjectGroup> = Vec::new();
        let mut sorted: Vec<&SessionSummary> = self
            .all
            .iter()
            .filter(|s| self.matches(s) && !self.hidden.contains(&s.cwd))
            .collect();
        sorted.sort_by_key(|s| std::cmp::Reverse(s.modified_ms));
        for s in sorted {
            let entry = self.entry(s);
            match groups.iter_mut().find(|g| g.cwd == s.cwd) {
                Some(group) => group.sessions.push(entry),
                None => groups.push(ProjectGroup {
                    cwd: s.cwd.clone(),
                    label: workspace_label(&s.cwd),
                    sessions: vec![entry],
                }),
            }
        }
        if self.by_name {
            groups.sort_by_key(|g| g.label.to_lowercase());
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_hide_and_sort() {
        let mut list = SessionList::new();
        list.set_sessions(vec![
            summary("a", "/w/zeta", "A", 3),
            summary("b", "/w/alpha", "B", 2),
            summary("c", "/w/zeta", "C", 1),
        ]);
        assert_eq!(list.groups()[0].label, "zeta");
        list.toggle_sort();
        assert_eq!(list.groups()[0].label, "alpha");
        list.hide_group("/w/alpha");
        assert_eq!(list.groups().len(), 1);
        list.remove("a");
        assert_eq!(list.groups()[0].sessions.len(), 1);
        list.show_group("/w/alpha");
        assert_eq!(list.groups().len(), 2);
    }

    fn summary(id: &str, cwd: &str, title: &str, modified_ms: i64) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            path: None,
            cwd: cwd.into(),
            title: title.into(),
            modified_ms,
            message_count: 1,
        }
    }

    #[test]
    fn groups_by_workspace_most_recent_first() {
        let mut list = SessionList::new();
        list.set_sessions(vec![
            summary("a", "/p/one", "old", 1),
            summary("b", "/p/two/", "mid", 5),
            summary("c", "/p/one", "new", 9),
        ]);
        let groups = list.groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].label, "one");
        assert_eq!(groups[1].label, "two");
        let ids: Vec<&str> = groups[0]
            .sessions
            .iter()
            .map(|e| e.summary.id.as_str())
            .collect();
        assert_eq!(ids, vec!["c", "a"]);
    }

    #[test]
    fn live_status_pins_and_filter() {
        let mut list = SessionList::new();
        list.upsert(summary("a", "/w", "Fix parser", 1));
        list.upsert(summary("b", "/w", "Write docs", 2));
        list.set_live("a", LiveStatus::Attention);
        list.toggle_pin("b");
        assert_eq!(list.pinned().len(), 1);
        list.set_query("PARSER");
        let groups = list.groups();
        assert_eq!(groups[0].sessions.len(), 1);
        assert_eq!(groups[0].sessions[0].status, LiveStatus::Attention);
        list.rename_id("a", "a2");
        assert_eq!(list.live("a2"), LiveStatus::Attention);
        list.upsert(summary("a2", "/w", "Fix parser v2", 3));
        assert_eq!(list.len(), 2);
        list.toggle_group("/w");
        assert!(list.is_collapsed("/w"));
    }

    #[test]
    fn sections_are_attention_first() {
        let mut list = SessionList::new();
        list.set_sessions(vec![
            summary("a", "/p/one", "new idle", 9),
            summary("b", "/p/one", "old waiting", 1),
            summary("c", "/p/two", "working", 5),
            summary("d", "/p/three", "idle", 7),
        ]);
        list.set_live("b", LiveStatus::Attention);
        list.set_live("c", LiveStatus::Running);
        let sections = list.sections();
        let labels: Vec<&str> = sections.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, vec!["one", "two", "three"]);
        assert_eq!(sections[0].sessions[0].summary.id, "b");
        assert_eq!(list.needs_you().len(), 1);
        assert_eq!(list.count(LiveStatus::Running), 1);
        assert_eq!(LiveStatus::Unread.word(), "Done");
    }

    #[test]
    fn ages() {
        assert_eq!(age(10_000, 9_000), "now");
        assert_eq!(age(10 * 60_000, 0), "10m");
        assert_eq!(age(3 * 3_600_000, 0), "3h");
        assert_eq!(age(2 * 86_400_000, 0), "2d");
        assert_eq!(age(30 * 86_400_000, 0), "4w");
        assert_eq!(workspace_label(""), "~");
    }
}
