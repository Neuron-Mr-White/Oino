//! Session cards and project grouping (spec-host §4.2): the pure data the Sessions list and
//! `SessionChanged` hub events carry. No I/O, no core types — [`crate::sessions::hub`] maps
//! `oino_core` records onto these.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the host knows about one session (spec-host §4.2 `HubState`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HubState {
    /// No run in flight (the session file exists; the runtime may be live or still a record).
    Idle,
    /// A run (or compaction) is in flight in this host.
    Working,
    /// The session task crashed and was contained (§4.4): listed `crashed` until re-opened.
    Crashed {
        /// Panic message (or a generic text when the crash was detected without one).
        error: String,
    },
    /// Another runner took the session over (D-TAKEOVER, §8): openable again from here.
    TakenOver {
        /// Who took it (`tui`, `host:<name>`, `cli`; [`oino_session::LeaseInfo::describe`]).
        from: String,
    },
}

impl HubState {
    /// Is the session running in this host?
    #[must_use]
    pub fn is_working(&self) -> bool {
        matches!(self, Self::Working)
    }

    /// The generic crash text when the panic message itself was not observed.
    pub const UNKNOWN_CRASH: &'static str = "the session task crashed (see the host log)";
}

/// Line changes of one session (app spec §3.14): the sidebar row's `+N −M`.
/// Unset until a later WP feeds it.
pub use unipi_proto::session::CardChanges;

/// One row of the Sessions list (§4.2): the record's header facts plus the host's live state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCard {
    /// Session id (header).
    pub session_id: String,
    /// Session file.
    pub file: PathBuf,
    /// Latest name (`session_info` entry), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Project working directory (header).
    pub cwd: String,
    /// Last activity, epoch milliseconds.
    pub updated_at: u64,
    /// User/assistant messages in the file.
    pub message_count: usize,
    /// Live state (§4.2 `HubState`).
    pub state: HubState,
    /// A dialog waits for the user (extension UI requests; H4 fills it in).
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_you: bool,
    /// Model selected in the session, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Pinned by the user (app spec §3.14, Z-14).
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// Ended a run / needed the user while unfocused (Z-86).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unread: bool,
    /// Manual name beats auto-rename (Z-15).
    #[serde(default, skip_serializing_if = "is_false")]
    pub title_locked: bool,
    /// Who runs the session when it is not the user (`"cron"`; A22 fills it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Line changes (`+N −M`) when a summary exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<CardChanges>,
    /// Last activity the host knows (epoch ms; the row's relative time).
    #[serde(default)]
    pub last_activity: u64,
}

impl SessionCard {
    /// The display name: the session name, else the first user line, else the id.
    #[must_use]
    pub fn display_name(&self, first_message: &str) -> String {
        if let Some(name) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            return name.to_owned();
        }
        let first = first_message.trim();
        if !first.is_empty() && first != "(no messages)" {
            return first.chars().take(80).collect();
        }
        self.session_id.chars().take(8).collect()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One project section of the Sessions list (§4.2 project grouping).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGroup {
    /// Canonical cwd the sessions share.
    pub cwd: String,
    /// Display name (the cwd's last path segment).
    pub name: String,
    /// Cards, newest activity first.
    pub cards: Vec<SessionCard>,
}

impl ProjectGroup {
    /// Groups `cards` by `cwd`, newest group first, cards newest first inside a group
    /// (stable: equal timestamps keep the input order).
    #[must_use]
    pub fn group(cards: Vec<SessionCard>) -> Vec<Self> {
        let mut groups: Vec<Self> = Vec::new();
        for card in cards {
            match groups.iter_mut().find(|g| g.cwd == card.cwd) {
                Some(group) => group.cards.push(card),
                None => groups.push(Self {
                    name: project_name(&card.cwd),
                    cwd: card.cwd.clone(),
                    cards: vec![card],
                }),
            }
        }
        for group in &mut groups {
            group
                .cards
                .sort_by_key(|card| std::cmp::Reverse(card.updated_at));
        }
        groups.sort_by(|a, b| {
            let a_newest = a.cards.first().map_or(0, |c| c.updated_at);
            let b_newest = b.cards.first().map_or(0, |c| c.updated_at);
            b_newest.cmp(&a_newest)
        });
        groups
    }
}

/// The display name of a project cwd: its last non-empty path segment (`~` for home).
#[must_use]
pub fn project_name(cwd: &str) -> String {
    if cwd.is_empty() {
        return "Sessions".to_owned();
    }
    let path = Path::new(cwd);
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => cwd.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, cwd: &str, updated_at: u64) -> SessionCard {
        SessionCard {
            session_id: id.to_owned(),
            file: PathBuf::from(format!("/sessions/{id}.jsonl")),
            name: None,
            cwd: cwd.to_owned(),
            updated_at,
            message_count: 2,
            state: HubState::Idle,
            needs_you: false,
            model: None,
            pinned: false,
            unread: false,
            title_locked: false,
            source: None,
            changes: None,
            last_activity: updated_at,
        }
    }

    #[test]
    fn groups_by_cwd_newest_first() {
        let groups = ProjectGroup::group(vec![
            card("a", "/p/one", 10),
            card("b", "/p/two", 40),
            card("c", "/p/one", 30),
            card("d", "/p/two", 20),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].cwd, "/p/two");
        assert_eq!(groups[0].name, "two");
        assert_eq!(
            groups[0]
                .cards
                .iter()
                .map(|c| c.session_id.as_str())
                .collect::<Vec<_>>(),
            ["b", "d"]
        );
        assert_eq!(groups[1].cwd, "/p/one");
        assert_eq!(
            groups[1]
                .cards
                .iter()
                .map(|c| c.session_id.as_str())
                .collect::<Vec<_>>(),
            ["c", "a"]
        );
    }

    #[test]
    fn equal_timestamps_keep_the_input_order() {
        let groups = ProjectGroup::group(vec![card("a", "/p", 7), card("b", "/p", 7)]);
        assert_eq!(
            groups[0]
                .cards
                .iter()
                .map(|c| c.session_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(project_name(""), "Sessions");
        assert_eq!(project_name("/"), "/");
    }

    #[test]
    fn state_and_card_wire_shape() {
        assert_eq!(
            serde_json::to_value(HubState::Crashed {
                error: "boom".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "crashed", "error": "boom"})
        );
        assert_eq!(
            serde_json::to_value(HubState::TakenOver { from: "tui".into() }).unwrap(),
            serde_json::json!({"kind": "takenOver", "from": "tui"})
        );
        let value = serde_json::to_value(card("a", "/p", 10)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "sessionId": "a",
                "file": "/sessions/a.jsonl",
                "cwd": "/p",
                "updatedAt": 10,
                "messageCount": 2,
                "state": {"kind": "idle"},
            })
        );
    }

    #[test]
    fn display_name_prefers_the_session_name() {
        let mut card = card("abcdef12", "/p", 1);
        assert_eq!(card.display_name("(no messages)"), "abcdef12");
        assert_eq!(card.display_name("first question"), "first question");
        card.name = Some("Triage".into());
        assert_eq!(card.display_name("first question"), "Triage");
    }
}
