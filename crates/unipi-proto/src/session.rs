//! Session streams and the hub event channel for `unipi/oino/1` (host.md §7.3–7.5, §4.2;
//! WP-H4).
//!
//! A session stream is one persistent bi-stream per open chat
//! (`session_open` / `session_new` / `session_fork`). Its first frame is
//! `ok{data:SessionSnapshot}`; then `Control::SessionCmd` goes in,
//! `Control::Events` batches come out, and command replies are
//! [`SessionReply`]s correlated by an opaque `id` (host.md §7.3).
//!
//! Payloads that are oino's own JSON — the `ClientEvent` of an event (§7.4),
//! the `RpcSessionState`, entries, commands, topics and the ui-protocol
//! registry snapshot — stay [`serde_json::Value`] here on purpose: the host
//! forwards core's JSON verbatim ("no second vocabulary", §7.4) and the app
//! narrows it by `type` without a second zod pass (ui-protocol D-U14). Only
//! the envelopes are typed, and those are what the shared fixtures pin.
//!
//! Host-only events ride the same envelope with reserved `type`s core never
//! emits: `crashed`, `taken_over`, `reloaded` (§7.4).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppErrorDto;

/// Longest `session` target accepted: an id, a unique prefix (≥ 4 chars) or a session file
/// path (host.md §4.2 `session_open{session: path|id|partial}`).
pub const SESSION_TARGET_MAX: usize = 4096;
/// Longest session name accepted (`session_rename`, `session_new`, `session_fork`).
pub const SESSION_NAME_MAX: usize = 256;
/// Longest session id accepted (ids are timestamps with a suffix today).
pub const SESSION_ID_MAX: usize = 128;
/// The event kinds core never produces (host-only, §7.4).
pub const HOST_EVENT_KINDS: [&str; 3] = ["crashed", "taken_over", "reloaded"];

/// A `session` target: an exact id, a unique prefix or a session file path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SessionTarget(String);

impl SessionTarget {
    /// The target as sent.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionTarget {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > SESSION_TARGET_MAX {
            return Err(format!(
                "session must be 1..={SESSION_TARGET_MAX} characters, got {}",
                trimmed.len()
            ));
        }
        Ok(SessionTarget(trimmed.to_owned()))
    }
}

impl<'de> Deserialize<'de> for SessionTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SessionTarget::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A session id (as the host addresses it in events and cards): 1..=128 characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    /// The id as sent.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > SESSION_ID_MAX {
            return Err(format!(
                "session_id must be 1..={SESSION_ID_MAX} characters, got {}",
                value.len()
            ));
        }
        if value.chars().any(char::is_control) {
            return Err("session_id must not contain control characters".to_string());
        }
        Ok(SessionId(value))
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SessionId::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A session name (`session_rename`, `session_new{name?}`): 1..=256 characters, no NUL or
/// newline (it is written into a session `session_info` entry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SessionName(String);

impl SessionName {
    /// The name as sent.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > SESSION_NAME_MAX {
            return Err(format!(
                "name must be 1..={SESSION_NAME_MAX} characters, got {}",
                trimmed.len()
            ));
        }
        if trimmed.contains(['\0', '\n', '\r']) {
            return Err("name must be a single line".to_string());
        }
        Ok(SessionName(trimmed.to_owned()))
    }
}

impl<'de> Deserialize<'de> for SessionName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SessionName::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// The run state of a card (host.md §4.2 `HubState`, §4.4): the app's Sessions list chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// No run in flight (the record on disk is the truth).
    #[default]
    Idle,
    /// A run (or compaction) is in flight in this host.
    Working,
    /// The session task crashed and was contained; [`SessionCard::error`] says why.
    Crashed,
    /// Another runner took the session over; [`SessionCard::from`] says who.
    TakenOver,
}

/// Line changes of a session (app spec §3.14): the sidebar row's `+N −M`.
/// Unset until a later WP feeds it (the `unipi.file-summary` turns or cron runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CardChanges {
    /// Lines added.
    pub add: u64,
    /// Lines removed.
    pub del: u64,
}

/// One row of `sessions_list` and of `HubEvent::SessionChanged` (host.md §4.2).
///
/// Field names are the wire's (`snake_case`, as everywhere in v2); `run_state` is the
/// enum §4.2 names, and the crash/takeover details travel beside it (`error`, `from`).
/// The §3.14 meta fields (`pinned`, `unread`, `title_locked`, `source`, `changes`,
/// `last_activity`) are stored host-side in `session-meta.json` and merged into every
/// card (WP-A4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCard {
    /// Session id (header).
    pub session_id: String,
    /// Session file (`""` for a session that has not written its first message yet).
    #[serde(default)]
    pub file: String,
    /// Latest name (`session_info` entry), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Project working directory (header).
    #[serde(default)]
    pub cwd: String,
    /// Last activity, epoch milliseconds.
    #[serde(default)]
    pub updated_at: u64,
    /// User/assistant messages in the file.
    #[serde(default)]
    pub message_count: usize,
    /// Live state (§4.2).
    #[serde(default)]
    pub run_state: RunState,
    /// A dialog waits for the user (an extension UI request, §4.2).
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_you: bool,
    /// Model selected in the session, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Panic text, for `run_state = "crashed"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Who took the session over, for `run_state = "taken_over"` (`tui`, `host:<name>`, `cli`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Pinned by the user (sidebar "Pinned" group, Z-14; app spec §3.14).
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// The session ended a run / needed the user while no connection had it focused
    /// (Z-86); cleared when a connection focuses it again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unread: bool,
    /// A manual name locks out auto-rename (Z-15; set by `session_meta_set`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub title_locked: bool,
    /// Who runs the session when it is not the user (`"cron"`; A22 fills it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Line changes (`+N −M` of the row, Z-12) when a summary exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<CardChanges>,
    /// Last activity the host knows (epoch ms); the row's relative time (Z-10).
    #[serde(default)]
    pub last_activity: u64,
}

/// The patch of `session_meta_set` (app spec §3.14): only the fields present change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionMetaPatch {
    /// Pin/unpin (Z-14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    /// Mark (un)read (Z-82/Z-86).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread: Option<bool>,
    /// Lock/unlock the title against auto-rename (Z-15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_locked: Option<bool>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// `sessions_list` → the cards, newest activity first (a bare array, as `term_list` is;
/// §3.14's `sessions_list{archived:true}` is additive).
pub type SessionsListData = Vec<SessionCard>;

/// One numbered event of a session stream (host.md §7.4): `{session, seq, event}`.
///
/// `event` is exactly oino's `ClientEvent` serde JSON (`oino-core/src/events.rs`), or one of
/// [`HOST_EVENT_KINDS`]. `seq` starts at 1 per session and never repeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEventDto {
    /// Session id at the time the event was produced.
    pub session: String,
    /// 1-based, monotonic per session.
    pub seq: u64,
    /// `ClientEvent` JSON (host-only `crashed`/`taken_over`/`reloaded` included).
    pub event: Value,
}

impl SessionEventDto {
    /// A new event.
    #[must_use]
    pub fn new(session: impl Into<String>, seq: u64, event: Value) -> Self {
        Self {
            session: session.into(),
            seq,
            event,
        }
    }

    /// The event's `type` tag (`message_update`, `agent_end`, `oino_ui_patch`, …).
    #[must_use]
    pub fn kind(&self) -> Option<&str> {
        self.event.get("type").and_then(Value::as_str)
    }
}

/// The data of `Control::Events`: one flush tick's batch on a session stream (host.md §7.4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionEvents {
    /// Events in order (delta events already coalesced).
    #[serde(default)]
    pub items: Vec<SessionEventDto>,
}

/// A correlated reply on a session stream: `ok{id?, data}` / `err{id?, error}` (§7.3).
///
/// The generic `Control::Ok` / `Control::Err` stay id-less for every other op; session
/// streams use this envelope so a client can match commands it sent (several may be in
/// flight).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionReply {
    /// The command succeeded; `data` is its RPC result (`null` when it has none).
    Ok {
        /// The `SessionCmd` id, echoed verbatim; absent for an unsolicited reply.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Value>,
        /// The command's `data`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
    /// The command failed.
    Err {
        /// The `SessionCmd` id, echoed verbatim.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Value>,
        /// The error.
        error: AppErrorDto,
    },
}

impl SessionReply {
    /// A successful reply to `id`.
    #[must_use]
    pub fn ok(id: Option<Value>, data: Option<Value>) -> Self {
        Self::Ok { id, data }
    }

    /// A failed reply to `id`.
    #[must_use]
    pub fn failed(id: Option<Value>, error: AppErrorDto) -> Self {
        Self::Err { id, error }
    }
}

/// The first frame of a session stream (host.md §7.3, §7.4).
///
/// `reset: true` is a full snapshot: `state`, `entries` (the current branch), `leaf_id`,
/// `commands`, `ui` (the ui-protocol registry snapshot) and `topics` (sticky values) are
/// present, `replay` is not. `reset: false` resumes: `replay` carries the buffered events
/// after the client's `since_seq` and the snapshot fields are absent — the client applies
/// them in order. Anything the host cannot prove is contiguous is a snapshot (§7.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Session id (it changes after a `fork`; the client follows it).
    pub session_id: String,
    /// Full snapshot (`true`) or resume patch (`false`).
    pub reset: bool,
    /// The last event seq the snapshot covers (or the last one replayed).
    #[serde(default)]
    pub seq: u64,
    /// `RpcSessionState` (`get_state`), full snapshots only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    /// The current branch's entries, full snapshots only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<Value>>,
    /// Leaf entry id of the current branch, full snapshots only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaf_id: Option<Value>,
    /// `CommandInfo` list (`get_commands`), full snapshots only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<Value>>,
    /// ui-protocol registry snapshot (v2 `UiSnapshot`, §9.2), forwarded verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<Value>,
    /// Sticky topic values of the session's subscriptions (`_oino/topics`), full snapshots only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<Value>>,
    /// Resume events after the client's `since_seq`, resume patches only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<Vec<SessionEventDto>>,
}

impl SessionSnapshot {
    /// A full snapshot (all payload fields filled in by the caller).
    #[must_use]
    pub fn full(session_id: impl Into<String>, seq: u64) -> Self {
        Self {
            session_id: session_id.into(),
            reset: true,
            seq,
            state: None,
            entries: None,
            leaf_id: None,
            commands: None,
            ui: None,
            topics: None,
            replay: None,
        }
    }

    /// A resume patch: the buffered events after `since_seq`.
    #[must_use]
    pub fn resume(session_id: impl Into<String>, seq: u64, replay: Vec<SessionEventDto>) -> Self {
        Self {
            session_id: session_id.into(),
            reset: false,
            seq,
            state: None,
            entries: None,
            leaf_id: None,
            commands: None,
            ui: None,
            topics: None,
            replay: Some(replay),
        }
    }

    /// The §7.4 invariant: a full snapshot has no `replay`, a resume patch has one.
    ///
    /// # Errors
    ///
    /// The text of the violated invariant (a client would not know which half it got).
    pub fn validate(&self) -> Result<(), String> {
        match (self.reset, self.replay.is_some()) {
            (true, true) => Err("a full snapshot must not carry replay".to_string()),
            (false, false) => Err("a resume patch must carry replay".to_string()),
            _ => Ok(()),
        }
    }
}

/// Severity of a [`HubEvent::HostNotice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    /// Informational (updater prompt, a session moved).
    Info,
    /// Something the user should act on (safe-mode banner, a crash notice).
    Warning,
}

/// What `events_open` streams, one per connection (host.md §7.4; ui-protocol §9.2 for `Ui`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HubEvent {
    /// A card was created, renamed, changed state or became idle: keeps the Sessions list
    /// live without polling. `run_state` tells created/state/needs-you apart.
    SessionChanged {
        /// The card.
        card: SessionCard,
    },
    /// A session left the list (archived: the file moved to `<agent dir>/trash/`).
    SessionRemoved {
        /// Session id.
        session: String,
    },
    /// A host-level notice (safe-mode banner, updater prompt, crash notice).
    HostNotice {
        /// Severity.
        level: NoticeLevel,
        /// Text to show.
        text: String,
    },
    /// The **host-scope** ui registry changed (ui-protocol §9.2): a snapshot is the first
    /// event after `events_open`, patches follow with `rev = previous + 1`. Forwarded
    /// verbatim; the host never interprets nodes.
    Ui {
        /// Full registry snapshot (`UiSnapshot`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<Value>,
        /// Incremental update (`UiPatch`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        patch: Option<Value>,
    },
    /// App spec §3.14 (WP-A22): a board project's files changed on disk (CLI
    /// or agent writes land here too — the watcher is on the files, not the
    /// writer). The Board re-reads the project; no payload beyond the slug.
    BoardChanged {
        /// Project slug, as `board_cli`'s `--project` spells it.
        project: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_target_and_name_validate() {
        let target: SessionTarget = " 2026-10-31T10-00-00-a1b2 ".to_string().try_into().unwrap();
        assert_eq!(target.as_str(), "2026-10-31T10-00-00-a1b2");
        assert!(SessionTarget::try_from(String::new()).is_err());
        assert!(SessionTarget::try_from("a".repeat(SESSION_TARGET_MAX + 1)).is_err());
        let parsed: SessionTarget = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(parsed.as_str(), "abc");

        assert!(SessionName::try_from("Fix the parser".to_string()).is_ok());
        assert!(SessionName::try_from("  ".to_string()).is_err());
        assert!(SessionName::try_from("two\nlines".to_string()).is_err());
        assert!(SessionName::try_from("a".repeat(SESSION_NAME_MAX + 1)).is_err());
        assert!(SessionId::try_from("x".repeat(SESSION_ID_MAX + 1)).is_err());
        assert!(SessionId::try_from("id\u{7}".to_string()).is_err());
    }

    #[test]
    fn snapshot_snapshot_and_resume_shapes() {
        let mut snapshot = SessionSnapshot::full("s1", 7);
        snapshot.state = Some(json!({"sessionId": "s1", "isStreaming": false}));
        snapshot.entries = Some(vec![json!({"id": "e1", "type": "message"})]);
        snapshot.leaf_id = Some(json!("e1"));
        snapshot.commands = Some(vec![json!({"name": "compact"})]);
        snapshot.ui = Some(json!({"scope": "session", "rev": 3, "entries": []}));
        snapshot.topics = Some(vec![json!({"topic": "plan", "data": {}})]);
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["session_id"], "s1");
        assert_eq!(value["reset"], true);
        assert_eq!(value["seq"], 7);
        assert_eq!(value["leaf_id"], "e1");
        assert!(value.get("replay").is_none(), "{value}");
        let parsed: SessionSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, snapshot);

        let resume = SessionSnapshot::resume(
            "s1",
            9,
            vec![SessionEventDto::new("s1", 8, json!({"type": "agent_end"}))],
        );
        let value = serde_json::to_value(&resume).unwrap();
        assert_eq!(value["reset"], false);
        assert_eq!(value["replay"][0]["seq"], 8);
        assert!(value.get("entries").is_none(), "{value}");
        let parsed: SessionSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, resume);
        assert!(resume.validate().is_ok());

        let mut broken = resume.clone();
        broken.replay = None;
        assert!(broken.validate().is_err());
        let mut broken = snapshot.clone();
        broken.replay = Some(Vec::new());
        assert!(broken.validate().is_err());
    }

    #[test]
    fn replies_and_hub_events_round_trip() {
        let ok = SessionReply::ok(Some(json!("7")), Some(json!({"session_id": "s1"})));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"type": "ok", "id": "7", "data": {"session_id": "s1"}})
        );
        let err = SessionReply::failed(
            Some(json!("8")),
            AppErrorDto::new(crate::error::AppErrorCode::E_INTERNAL, "boom", false),
        );
        let value = serde_json::to_value(&err).unwrap();
        assert_eq!(value["type"], "err");
        assert_eq!(value["id"], "8");
        assert_eq!(value["error"]["message"], "boom");

        let card = SessionCard {
            session_id: "s1".into(),
            file: "/tmp/s1.jsonl".into(),
            name: None,
            cwd: "/tmp/p".into(),
            updated_at: 1_700_000_000_000,
            message_count: 4,
            run_state: RunState::Crashed,
            needs_you: false,
            model: Some("faux-1".into()),
            error: Some("tool body exploded".into()),
            from: None,
            pinned: true,
            unread: false,
            title_locked: true,
            source: None,
            changes: Some(CardChanges { add: 12, del: 4 }),
            last_activity: 1_700_000_000_000,
        };
        let changed = HubEvent::SessionChanged { card };
        let value = serde_json::to_value(&changed).unwrap();
        assert_eq!(value["kind"], "session_changed");
        assert_eq!(value["card"]["run_state"], "crashed");
        assert_eq!(value["card"]["message_count"], 4);
        assert_eq!(value["card"]["pinned"], true);
        assert_eq!(value["card"]["title_locked"], true);
        assert_eq!(value["card"]["changes"], json!({"add": 12, "del": 4}));
        assert_eq!(value["card"]["last_activity"], 1_700_000_000_000u64);
        assert!(value["card"].get("needs_you").is_none(), "false is omitted");
        assert!(value["card"].get("unread").is_none(), "false is omitted");
        assert!(value["card"].get("source").is_none(), "unset is omitted");
        let parsed: HubEvent = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, changed);

        assert_eq!(
            serde_json::to_value(HubEvent::SessionRemoved {
                session: "s1".into()
            })
            .unwrap(),
            json!({"kind": "session_removed", "session": "s1"})
        );
        assert_eq!(
            serde_json::to_value(HubEvent::HostNotice {
                level: NoticeLevel::Warning,
                text: "safe mode".into()
            })
            .unwrap(),
            json!({"kind": "host_notice", "level": "warning", "text": "safe mode"})
        );
        assert_eq!(
            serde_json::to_value(HubEvent::Ui {
                snapshot: Some(json!({"scope": "host", "rev": 1, "entries": []})),
                patch: None,
            })
            .unwrap(),
            json!({"kind": "ui", "snapshot": {"scope": "host", "rev": 1, "entries": []}})
        );
    }

    /// A meta patch round-trips with absent fields omitted (§3.14 `session_meta_set`).
    #[test]
    fn meta_patch_wire_shape() {
        let patch = SessionMetaPatch {
            pinned: Some(true),
            unread: None,
            title_locked: None,
        };
        assert_eq!(
            serde_json::to_value(patch).unwrap(),
            json!({"pinned": true})
        );
        let parsed: SessionMetaPatch = serde_json::from_value(json!({"unread": false})).unwrap();
        assert_eq!(
            parsed,
            SessionMetaPatch {
                unread: Some(false),
                ..SessionMetaPatch::default()
            }
        );
        let empty: SessionMetaPatch = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, SessionMetaPatch::default());
    }

    #[test]
    fn event_dto_kind_and_events_batch() {
        let dto = SessionEventDto::new(
            "s1",
            3,
            json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta"}}),
        );
        assert_eq!(dto.kind(), Some("message_update"));
        let batch = SessionEvents { items: vec![dto] };
        assert_eq!(
            serde_json::to_value(&batch).unwrap(),
            json!({"items": [{"session": "s1", "seq": 3,
                "event": {"type": "message_update",
                          "assistantMessageEvent": {"type": "text_delta"}}}]})
        );
    }
}
