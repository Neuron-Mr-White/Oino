//! Bulk session ops and the sidebar's session query (owner fix 2026-11-03, `[ux-sessions]`).
//!
//! - `sessions_archive{sessions}` / `sessions_restore{sessions}`: one host op for N chats
//!   (the sidebar's "Archive all" and its Undo), instead of N one-shot requests. The host
//!   resolves every target from ONE listing, moves the files, and announces the removals /
//!   restores on `events_open` in chunks (`sessions_removed` / `sessions_changed`, at most
//!   [`SESSIONS_EVENT_CHUNK`] ids or cards per event), which the app also uses as progress.
//! - `sessions_query{…}`: one page of the sidebar's session search — the host matches the
//!   title and the first user message of every chat (live and archived, cold ones too, from
//!   the card index), applies the list's Properties filters it can evaluate, and answers at
//!   most [`SESSIONS_QUERY_LIMIT_MAX`] cards. The webview never holds the whole store.

use serde::{Deserialize, Serialize};

use crate::session::{SessionCard, SessionTarget};

/// The most targets one `sessions_archive` / `sessions_restore` takes.
pub const SESSIONS_BATCH_MAX: usize = 5_000;
/// Ids (or cards) per `sessions_removed` / `sessions_changed` hub event.
pub const SESSIONS_EVENT_CHUNK: usize = 200;
/// `sessions_query` page size when the request names none.
pub const SESSIONS_QUERY_LIMIT_DEFAULT: u32 = 50;
/// The most cards one `sessions_query` page carries.
pub const SESSIONS_QUERY_LIMIT_MAX: u32 = 100;
/// The longest query the host matches (bytes); longer is cut at a char boundary.
pub const SESSIONS_QUERY_TEXT_MAX: usize = 256;
/// The most values one multi-select filter carries.
pub const SESSIONS_QUERY_FILTER_MAX: usize = 256;

/// One failed target of a batch op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsBatchFailure {
    /// The target as sent.
    pub session: String,
    /// Why it was skipped (running, not found, I/O).
    pub message: String,
}

/// `sessions_archive` / `sessions_restore` → which ids went, which failed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsBatchData {
    /// Session ids that were archived (or restored).
    pub done: Vec<String>,
    /// Targets that were not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<SessionsBatchFailure>,
}

/// Which side of the trash a query covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsQueryScope {
    /// Live (unarchived) chats only — the list's default.
    #[default]
    Live,
    /// Archived chats only.
    Archived,
    /// Both.
    All,
}

/// Row order of a query page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsQuerySort {
    /// Last activity, newest first.
    #[default]
    Updated,
    /// Creation time, newest first.
    Created,
    /// Title, A → Z.
    Name,
}

/// A row's status facet (the list's Status filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsQueryStatus {
    /// A run is in flight.
    Running,
    /// A dialog waits for the user.
    Waiting,
    /// Idle.
    Ready,
    /// Crashed.
    Failed,
    /// A cron run that timed out (the host cannot tell: never matches host-side).
    Timedout,
}

/// The automation facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsQueryAutomation {
    /// A cron run's chat.
    Cron,
    /// Everything else.
    Manual,
}

/// An epoch-ms range `[from, to)`; an absent end is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsQueryRange {
    /// Inclusive lower bound, epoch ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    /// Exclusive upper bound, epoch ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
}

impl SessionsQueryRange {
    /// Whether `ms` falls inside.
    #[must_use]
    pub fn contains(&self, ms: i64) -> bool {
        self.from.is_none_or(|from| ms >= from) && self.to.is_none_or(|to| ms < to)
    }
}

/// The list's Properties filters a query honours (empty lists = any).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsQueryFilters {
    /// Project folders; `""` = No workspace (empty, `/`, a bare home dir or a scratch folder).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    /// Automation facet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<SessionsQueryAutomation>,
    /// Status facet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<SessionsQueryStatus>,
    /// Model ids (`""` = unknown). Cards whose model the host does not know count as `""`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Creation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<SessionsQueryRange>,
    /// Last activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<SessionsQueryRange>,
}

impl SessionsQueryFilters {
    /// Caps every multi-select at [`SESSIONS_QUERY_FILTER_MAX`] values.
    #[must_use]
    pub fn bounded(mut self) -> Self {
        self.workspaces.truncate(SESSIONS_QUERY_FILTER_MAX);
        self.automation.truncate(SESSIONS_QUERY_FILTER_MAX);
        self.status.truncate(SESSIONS_QUERY_FILTER_MAX);
        self.models.truncate(SESSIONS_QUERY_FILTER_MAX);
        self
    }
}

/// One `sessions_query` hit: the card plus whether it is archived and where the text matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsQueryHit {
    /// The card (archived cards are idle).
    pub card: SessionCard,
    /// The chat is in the trash.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// A short excerpt of the first user message around the match, when the title did not match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// `sessions_query` → one page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsQueryData {
    /// The page.
    pub hits: Vec<SessionsQueryHit>,
    /// Every match across pages.
    pub total: usize,
    /// The next page's cursor; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// The query text cut to [`SESSIONS_QUERY_TEXT_MAX`] bytes and split into lowercase words.
#[must_use]
pub fn query_words(query: &str) -> Vec<String> {
    let mut end = query.len().min(SESSIONS_QUERY_TEXT_MAX);
    while !query.is_char_boundary(end) {
        end -= 1;
    }
    query[..end]
        .split_whitespace()
        .map(str::to_lowercase)
        .collect()
}

/// Validates a batch's targets (1..=[`SESSIONS_BATCH_MAX`]).
///
/// # Errors
///
/// A message when the batch is empty or too large.
pub fn check_batch(sessions: &[SessionTarget]) -> Result<(), String> {
    if sessions.is_empty() {
        return Err("sessions must name at least one session".to_owned());
    }
    if sessions.len() > SESSIONS_BATCH_MAX {
        return Err(format!(
            "at most {SESSIONS_BATCH_MAX} sessions per batch, got {}",
            sessions.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn words_are_bounded_and_lowercase() {
        assert_eq!(query_words("  Fix  PARSER "), vec!["fix", "parser"]);
        assert!(query_words(&"é".repeat(400)).concat().len() <= SESSIONS_QUERY_TEXT_MAX);
    }

    #[test]
    fn batch_limits() {
        assert!(check_batch(&[]).is_err());
        let one: SessionTarget = "abcd".to_owned().try_into().unwrap();
        assert!(check_batch(std::slice::from_ref(&one)).is_ok());
        assert!(check_batch(&vec![one; SESSIONS_BATCH_MAX + 1]).is_err());
    }

    #[test]
    fn filters_and_ranges_round_trip() {
        let f: SessionsQueryFilters = serde_json::from_value(json!({
            "workspaces": [""], "status": ["running", "waiting"], "automation": ["cron"],
            "created": {"from": 10}, "updated": {"to": 20}
        }))
        .unwrap();
        assert_eq!(
            f.status,
            vec![SessionsQueryStatus::Running, SessionsQueryStatus::Waiting]
        );
        assert!(f.created.unwrap().contains(10) && !f.created.unwrap().contains(9));
        assert!(f.updated.unwrap().contains(19) && !f.updated.unwrap().contains(20));
        assert!(serde_json::from_value::<SessionsQueryScope>(json!("all")).is_ok());
        let data = SessionsBatchData {
            done: vec!["a".into()],
            failed: vec![],
        };
        assert_eq!(serde_json::to_value(&data).unwrap(), json!({"done": ["a"]}));
    }
}
