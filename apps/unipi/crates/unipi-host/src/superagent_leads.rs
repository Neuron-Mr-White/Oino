//! Durable lead records (OIN-80): the store at `superagent-leads.json`, the
//! one-time migration from `dispatcher-continuations.json`, the pure error
//! recovery state machine and the `superagent.lead_action` RPC.
//!
//! Owner override (binding): an error NEVER mints a new lead session id. Only
//! `lead:"new"`, a stop or an archive retires a lead; every other path wakes
//! the same chat, stalls it with health, or unloads it.

use crate::superagent::{fail, storage, Delivery, Dispatcher};
use crate::superagent_presets::{Overrides, Preset, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unipi_kanboard::{
    board::Board,
    dispatcher,
    model::{Actor, RunOwner, Status, Task},
    Layout, Project,
};

/// A lead re-probe runs half an hour after a stall; a constant, not a setting.
pub const STALL_REPROBE_MS: i64 = 30 * 60_000;
/// Texts sent to the lead on the state machine's continue paths.
pub const FALLBACK_TEXT: &str =
    "Continue where you stopped. Your model {primary} failed: {error}. You are now on {fallback}.";
pub const PROBE_TEXT: &str = "Continue where you stopped. The last provider error was: {error}.";
pub const RESTART_TEXT: &str = "The host restarted while you were working. Continue where you stopped on {ids}. Check the workspace and git state before you repeat any outside action (push, deploy, message).";
/// The stop note on `in_progress` tickets the retire path releases.
pub const STOP_NOTE: &str =
    "Lead stopped; moved to backlog so Auto-start does not relaunch it. Move to todo to run again.";
/// `blocked:` texts that host plumbing (not the lead) produced. The one-time
/// migration un-blocks tickets parked by them; needsYou filters them (OIN-81).
pub const PLUMBING_PREFIXES: &[&str] = &[
    "Worker was interrupted",
    "Worker ended without a transition",
    "Worker ended while blocked",
    "Worker stopped",
    "Worker could not start",
    "Lead delivery failed",
];
/// The owner override: plumbing-blocked tickets move to BACKLOG (not todo)
/// with a System comment quoting the original block text.
pub const PLUMBING_MIGRATION_NOTE: &str = "Blocked by host plumbing before OIN-80";

/// A lead's life cycle. `Retired` is not a state: retired leads move into
/// `LeadStore::retired` and their record leaves `leads`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadState {
    Working,
    Waiting,
    Idle,
    Unloaded,
    Stalled,
}

impl LeadState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Unloaded => "unloaded",
            Self::Stalled => "stalled",
        }
    }
}

/// Why a stalled lead stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthKind {
    Model,
    NoProgress,
    Launch,
}

impl HealthKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::NoProgress => "no_progress",
            Self::Launch => "launch",
        }
    }
}

/// The stall card the work pane renders (Retry / Switch model / Stop).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub kind: HealthKind,
    pub error: String,
    pub attempts: u32,
    pub since: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_probe_at: Option<i64>,
    /// One stall notification per episode (R4): cleared on the next success.
    #[serde(default)]
    pub notified: bool,
}

/// Delivery mode for a durable `PendingAnswer` outbox entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingAnswerMode {
    Relay,
    Steer,
}

/// Which multi-step operation a `PendingAnswer` outbox entry drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingAnswerKind {
    BlockedAnswer,
    OpenQuestion,
    Timeout,
}

fn default_outbox_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Exponential backoff for outbox retries (F3): 1, 2, 4, 8, 16, 30 min cap.
#[must_use]
pub fn outbox_backoff_ms(attempts: u32) -> i64 {
    let mins: i64 = match attempts {
        0 | 1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        _ => 30,
    };
    mins * 60_000
}

/// Scope selector when flushing a single operation (`strict` RPC path, F2).
#[derive(Clone, Debug)]
pub enum OutboxFilter {
    EntryId(String),
}

/// Durable per-operation answer/timeout outbox (`superagent-leads.json`): stages
/// owner answers and timeout steers BEFORE mutating runtime/dialog/board state
/// and tracks `dialog_closed`, `delivered`, and `board_done` independently so
/// retries and `lead_tick` finish only missing steps without double delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAnswer {
    #[serde(default = "default_outbox_id")]
    pub id: String,
    pub kind: PendingAnswerKind,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Exact encoded `UiAnswer` payload persisted for dialog recovery (F1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog_answer: Option<Value>,
    /// Quoted question excerpt used if the dialog disappears before closure (F1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_excerpt: Option<String>,
    /// Owner's verbatim words (for `BlockedAnswer` / `OpenQuestion`) or the
    /// question excerpt parked on the ticket (for `Timeout`).
    pub text: String,
    /// Formatted message sent to the lead runtime (`""` when delivered via `dialog_answer`).
    #[serde(default)]
    pub delivery_text: String,
    pub mode: PendingAnswerMode,
    #[serde(default)]
    pub dialog_closed: bool,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default)]
    pub board_done: bool,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<i64>,
    #[serde(default)]
    pub runtime_failures: u32,
    pub created_at: i64,
}

impl PendingAnswer {
    /// F4: only entries whose dialog or runtime delivery is unfinished need a live runtime.
    #[must_use]
    pub fn needs_runtime(&self) -> bool {
        !self.dialog_closed || !self.delivered
    }
}

/// OIN-81: a lead's own `ask_user` the owner has not answered yet. Absent in
/// OIN-80; the field exists so records survive the OIN-81 upgrade untouched.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub request_id: String,
    pub task_id: String,
    pub excerpt: String,
    pub first_seen_at: i64,
    /// Durable marker kept until BOTH the board transition to `blocked` AND
    /// dialog dismissal (+ timeout steer) have completed.
    #[serde(default)]
    pub timing_out: bool,
    #[serde(default)]
    pub board_blocked: bool,
    #[serde(default)]
    pub dialog_resolved: bool,
}

/// The continuation budget (moved from the in-memory `Dispatcher` map).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Continuation {
    #[serde(default)]
    pub tickets: Vec<String>,
    #[serde(default)]
    pub sent: bool,
    #[serde(default)]
    pub total: u8,
}

/// One project's lead: the record that survives restarts and unloads.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadRecord {
    pub project_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<PathBuf>,
    /// Resolved preset; survives restart so a resumed lead keeps its model/mode.
    pub preset: Preset,
    pub state: LeadState,
    pub created_at: i64,
    pub last_active_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    #[serde(default)]
    pub continuation: Continuation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<PendingQuestion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_answers: Vec<PendingAnswer>,
    /// The lead runs the fallback model after its primary exhausted retries.
    #[serde(default)]
    pub on_fallback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_sent_at: Option<i64>,
    /// Scratch for `step` (never serialized): the held `in_progress` ids this tick.
    #[serde(skip)]
    pub in_progress_tickets: Vec<String>,
}

impl Default for LeadRecord {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            session_id: String::new(),
            session_file: None,
            preset: Preset {
                id: String::new(),
                label: String::new(),
                description: String::new(),
                model: None,
                thinking: None,
                autonomy: "high".into(),
                mode: "none".into(),
                max_parallel: None,
                review: "none".into(),
                profiles: None,
                priority: false,
            },
            state: LeadState::Working,
            created_at: 0,
            last_active_at: 0,
            health: None,
            continuation: Continuation::default(),
            question: None,
            pending_answers: Vec::new(),
            on_fallback: false,
            resume_sent_at: None,
            in_progress_tickets: Vec::new(),
        }
    }
}

/// A retired lead (owner/Superagent stop, `lead:"new"`, archive). Capped at 20
/// entries so `lead:"reuse"` can reopen the chat while its file still exists.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetiredLead {
    pub project_id: String,
    pub session_id: String,
    pub session_file: Option<PathBuf>,
    pub retired_at: i64,
    pub cause: String,
}

/// `superagent-leads.json` (version 1), written atomically only when a record
/// changes, under the dispatcher transaction.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadStore {
    pub version: u32,
    #[serde(default)]
    pub leads: BTreeMap<String, LeadRecord>,
    #[serde(default)]
    pub retired: Vec<RetiredLead>,
}

/// Settings (`superagent.lead.*`). Owner override: `fallback_model: None`
/// means NO fallback — after the retries the lead goes straight to Stalled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadSettings {
    pub retries: u32,
    pub fallback_model: Option<String>,
    pub warm_minutes: u32,
    pub question_timeout_minutes: u32,
}
impl Default for LeadSettings {
    fn default() -> Self {
        Self {
            retries: 3,
            fallback_model: None,
            warm_minutes: 30,
            question_timeout_minutes: 30,
        }
    }
}
impl LeadSettings {
    /// Clamp on load so a hand-edited config cannot wedge the state machine.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.retries = self.retries.min(10);
        self.warm_minutes = self.warm_minutes.clamp(5, 1440);
        self.question_timeout_minutes = self.question_timeout_minutes.clamp(1, 1440);
        self.fallback_model = self
            .fallback_model
            .map(|model| model.trim().to_owned())
            .filter(|model| !model.is_empty());
        self
    }
}

/// What one tick observed about a lead's runtime.
#[derive(Clone)]
pub enum TurnOutcome {
    /// The runtime is mid-turn (or waiting on the owner); nothing to do.
    Busy,
    /// The turn ended cleanly with this last assistant text.
    Done { last: String },
    /// The turn ended with a provider error (`stop_reason == Error`).
    Errored { message: String },
    /// The runtime is gone while the record is live (restart, crash, unload).
    Gone,
}

/// One state-machine step's side effects, applied by the caller in order.
#[derive(Clone)]
pub enum Effect {
    Continue(String),
    SetModel(String),
    Notify,
}

/// The pure state machine: one row of the spec's table per call.
/// `attempts` counts failed rounds (one round = one turn incl. core retries).
pub fn step(
    rec: &mut LeadRecord,
    obs: TurnOutcome,
    now: i64,
    _settings: &LeadSettings,
    fallback: Option<&str>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    match obs {
        TurnOutcome::Busy => rec.last_active_at = now,
        TurnOutcome::Errored { message } => {
            let attempts = rec.health.as_ref().map_or(0, |h| h.attempts) + 1;
            // The episode's notification survives the switch (finding 8): a
            // probe or a fallback hand-off must not reset the flag, or the
            // next failure re-notifies without an intervening success.
            let notified = rec.health.as_ref().is_some_and(|h| h.notified);
            if !rec.on_fallback
                && fallback.is_some_and(|fb| Some(fb) != rec.preset.model.as_deref())
            {
                // First failure with a fallback that differs from the primary.
                let fb = fallback.unwrap_or_default().to_owned();
                rec.on_fallback = true;
                rec.state = LeadState::Working;
                rec.health = Some(Health {
                    kind: HealthKind::Model,
                    error: message.clone(),
                    attempts,
                    since: now,
                    next_probe_at: None,
                    notified,
                });
                effects.push(Effect::SetModel(fb.clone()));
                effects.push(Effect::Continue(
                    FALLBACK_TEXT
                        .replace(
                            "{primary}",
                            rec.preset.model.as_deref().unwrap_or("your default model"),
                        )
                        .replace("{error}", &message)
                        .replace("{fallback}", &fb),
                ));
            } else {
                let since = rec.health.as_ref().map_or(now, |h| h.since);
                let notified = rec.health.as_ref().is_some_and(|h| h.notified);
                rec.state = LeadState::Stalled;
                rec.health = Some(Health {
                    kind: HealthKind::Model,
                    error: message.clone(),
                    attempts,
                    since,
                    next_probe_at: Some(
                        reset_hint_ms(&message, now).unwrap_or(now + STALL_REPROBE_MS),
                    ),
                    notified,
                });
                if !notified {
                    if let Some(health) = rec.health.as_mut() {
                        health.notified = true;
                    }
                    effects.push(Effect::Notify);
                }
            }
        }
        TurnOutcome::Done { last } => {
            let held = rec.continuation.tickets.clone();
            let in_progress = rec.in_progress_tickets.clone();
            // The first successful `Done` clears `health` (any state it came from).
            rec.health = None;
            if in_progress.is_empty() && !held.is_empty() {
                // Only blocked tickets (or an open question) held: Waiting, healthy.
                rec.health = None;
                rec.state = LeadState::Waiting;
            } else if held.is_empty() {
                rec.health = None;
                rec.continuation = Continuation::default();
                rec.state = LeadState::Idle;
            } else if u32::from(rec.continuation.total) < 2 {
                // The existing nudge; the caller persists before sending.
                rec.continuation.sent = true;
                rec.continuation.total += 1;
                effects.push(Effect::Continue(format!(
                    "You still hold in_progress tickets {}: work them and finish each (in_review with a summary) or block it with a question before ending your turn.",
                    held.join(", ")
                )));
            } else {
                // The nudge budget is out: re-stall (the notify flag survives the
                // health clear above so the episode does not re-notify).
                let notified = rec.health.as_ref().is_some_and(|h| h.notified)
                    || rec.state == LeadState::Stalled;
                rec.state = LeadState::Stalled;
                rec.health = Some(Health {
                    kind: HealthKind::NoProgress,
                    error: format!(
                        "Lead ended its turn without finishing or blocking {}",
                        in_progress.join(", ")
                    ),
                    attempts: u32::from(rec.continuation.total),
                    since: now,
                    next_probe_at: None,
                    notified,
                });
                if !notified {
                    if let Some(health) = rec.health.as_mut() {
                        health.notified = true;
                    }
                    effects.push(Effect::Notify);
                }
            }
            let _ = last; // the last text only fed the legacy block texts
        }
        TurnOutcome::Gone => {
            // Restart resume is `resume_interrupted`; the warm-idle unload and
            // the stalled probe live in `lead_tick`. The pure step stays pure.
        }
    }
    effects
}

/// The first reset hint in the error text, clamped to at least `now + 60 s`.
/// Relative durations ("reset after 79h", "resets in 30 min", "retry-after:
/// 120") are ADDED to `now` (finding 4: a hint is a delay from now, not an
/// epoch). Only known units count (`h`/`m`/`s` families with word endings);
/// an unparsable or unit-less occurrence is skipped and a later valid one is
/// still found. RFC 3339 `resets at <ts>` stays absolute.
#[must_use]
pub fn reset_hint_ms(error: &str, now: i64) -> Option<i64> {
    let clamp = |ms: i64| Some(ms.max(now.saturating_add(60_000)));
    let lower = error.to_lowercase();
    // `h`/`hr`/`hour(s)`, `m`/`min`(ute(s)), `s`/`sec`(ond(s)) — a whitelist,
    // so "reset after 3 w" or "resetA" never parse as a duration.
    let duration_ms = |value: f64, unit: &str| -> Option<i64> {
        // "minutes"→"minute", but "s" stays "s" (no empty singular).
        let singular = unit
            .strip_suffix('s')
            .filter(|stripped| !stripped.is_empty())
            .unwrap_or(unit);
        let ms = match singular {
            "h" | "hr" | "hour" => duration_millis(value, 3_600_000.0),
            "m" | "min" | "minute" => duration_millis(value, 60_000.0),
            "s" | "sec" | "second" => duration_millis(value, 1_000.0),
            _ => return None,
        };
        Some(ms)
    };
    // 1. `resets? (after|in) <n><unit>` — scan every occurrence: an earlier
    //    "reset" that does not carry a parsable duration ("resetA", "reset
    //    later") must not hide a valid hint later in the text.
    let mut search = lower.as_str();
    let mut offset = 0usize;
    while let Some(pos) = search.find("reset") {
        let rest = &lower[offset + pos..];
        let rest = rest
            .strip_prefix("resets")
            .unwrap_or(&rest["reset".len()..]);
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("after")
            .or_else(|| rest.strip_prefix("in"))
            .map(str::trim_start);
        if let Some(rest) = rest {
            let digits: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(value) = digits.parse::<f64>() {
                if !digits.is_empty() {
                    let rest = rest[digits.len()..].trim_start();
                    let unit: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
                    if let Some(ms) = duration_ms(value, &unit) {
                        return clamp(now.saturating_add(ms));
                    }
                }
            }
        }
        offset += pos + "reset".len();
        search = &lower[offset..];
    }
    // 2. `retry[- ]after[: ] <n>` (seconds), same later-occurrence scan.
    for marker in ["retry-after", "retry after"] {
        let mut search = lower.as_str();
        let mut offset = 0usize;
        while let Some(pos) = search.find(marker) {
            let rest = lower[offset + pos + marker.len()..].trim_start_matches([':', ' ']);
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(seconds) = digits.parse::<i64>() {
                if !digits.is_empty() {
                    return clamp(now.saturating_add(seconds.saturating_mul(1_000)));
                }
            }
            offset += pos + marker.len();
            search = &lower[offset..];
        }
    }
    // 3. `resets? at <RFC 3339>` — an absolute timestamp; scan occurrences.
    let mut search = lower.as_str();
    let mut offset = 0usize;
    while let Some(pos) = search.find("reset") {
        let rest = lower[offset + pos..]
            .strip_prefix("resets")
            .or_else(|| lower[offset + pos..].strip_prefix("reset"))
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix("at"))
            .map(str::trim_start);
        if let Some(rest) = rest {
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&rest[..end]) {
                return clamp(t.timestamp_millis());
            }
        }
        offset += pos + "reset".len();
        search = &lower[offset..];
    }
    None
}

#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl LeadStore {
    #[must_use]
    pub fn path_of(layout: &Layout) -> PathBuf {
        layout.home.join("superagent-leads.json")
    }

    /// Load `superagent-leads.json`; when absent, run the one-time migration:
    /// dispatcher claims → Working records (presets rebuilt from `projectLeads`),
    /// plumbing-blocked tickets → backlog with the System note, continuations
    /// folded in, then `dispatcher-continuations.json` removed.
    /// # Errors
    /// Returns an error for invalid lead state or failed persistence, board, and session operations.
    pub fn load_or_migrate(layout: &Layout) -> Result<Self, crate::superagent::DispatchError> {
        let path = Self::path_of(layout);
        // Read errors and parse errors are REAL failures: treating them as
        // "missing" would reconstruct from claims and overwrite the store —
        // silently dropping idle/unloaded leads (whose next task would mint a
        // new chat). Only an absent file migrates (OIN-80 finding 1).
        let Some(bytes) = crate::data_dir::read_optional(&path).map_err(storage)? else {
            return Self::migrate(layout);
        };
        Self::from_json_bytes(&bytes)
            .map_err(|e| storage(format!("reading {}: {e}", path.display())))
    }

    /// Parse `LeadStore` JSON, migrating any legacy single `pendingAnswer` object
    /// on a lead record into `pendingAnswers: Vec<PendingAnswer>` (F2).
    /// # Errors
    /// Returns an error for invalid lead state or failed persistence, board, and session operations.
    pub fn from_json_bytes(bytes: &[u8]) -> std::result::Result<Self, serde_json::Error> {
        let mut raw: Value = serde_json::from_slice(bytes)?;
        if let Some(leads) = raw.get_mut("leads").and_then(Value::as_object_mut) {
            for rec in leads.values_mut() {
                if let Some(obj) = rec.as_object_mut() {
                    if let Some(legacy) = obj.remove("pendingAnswer") {
                        if !legacy.is_null() {
                            let arr = obj
                                .entry("pendingAnswers")
                                .or_insert_with(|| Value::Array(Vec::new()));
                            if let Some(vec) = arr.as_array_mut() {
                                vec.push(legacy);
                            }
                        }
                    }
                }
            }
        }
        serde_json::from_value(raw)
    }

    /// The one-time migration (the store file is absent): fold claims and
    /// `dispatcher-continuations.json` into fresh Working records.
    fn migrate(layout: &Layout) -> Result<Self, crate::superagent::DispatchError> {
        let preferences =
            Store::load(&layout.home.join("superagent-preferences.json")).map_err(storage)?;
        let continuations: BTreeMap<String, Continuation> =
            crate::data_dir::read_json(&layout.home.join("dispatcher-continuations.json"))
                .map_err(storage)?
                .unwrap_or_default();
        let now = now_ms();
        let mut by_session: BTreeMap<String, (Project, Vec<String>)> = BTreeMap::new();
        for (project, task) in claims_of(layout)? {
            let Some(run) = task.run.as_ref() else {
                continue;
            };
            let entry = by_session
                .entry(run.session.clone())
                .or_insert_with(|| (project, Vec::new()));
            entry.1.push(task.id.clone());
        }
        let mut store = Self {
            version: 1,
            leads: BTreeMap::new(),
            retired: Vec::new(),
        };
        for (session, (project, tickets)) in by_session {
            let overrides = preferences
                .project_leads
                .get(&project.slug)
                .map(|lead| Overrides {
                    model: lead.model.clone(),
                    thinking: lead.thinking.clone(),
                    ..Overrides::default()
                })
                .unwrap_or_default();
            let preset = preferences
                .resolve(None, overrides)
                .map_err(|e| fail("E_SUPERAGENT_STORAGE", e))?;
            store.leads.insert(
                project.slug.clone(),
                LeadRecord {
                    project_id: project.slug.clone(),
                    session_id: session.clone(),
                    session_file: None,
                    preset,
                    state: LeadState::Working,
                    created_at: now,
                    last_active_at: now,
                    health: None,
                    continuation: Continuation {
                        tickets,
                        ..continuations.get(&session).cloned().unwrap_or_default()
                    },
                    question: None,
                    pending_answers: Vec::new(),
                    on_fallback: false,
                    resume_sent_at: None,
                    in_progress_tickets: Vec::new(),
                },
            );
        }
        migrate_plumbing_blocks(layout)?;
        crate::data_dir::write_json(&Self::path_of(layout), &store).map_err(storage)?;
        crate::data_dir::remove_optional(&layout.home.join("dispatcher-continuations.json"))
            .map_err(storage)?;
        Ok(store)
    }

    /// # Errors
    /// Returns an error for invalid lead state or failed persistence, board, and session operations.
    pub fn save(&self, path: &Path) -> Result<(), crate::superagent::DispatchError> {
        crate::data_dir::write_json(path, self).map_err(storage)
    }

    #[must_use]
    pub fn for_project(&self, project_id: &str) -> Option<&LeadRecord> {
        self.leads.get(project_id)
    }

    /// The store-side mutation of a retirement (sync; the caller awaited the
    /// runtime dispose outside the lock): drop the record, cap and append to
    /// `retired`. Returns the retired session id.
    pub fn record_retirement(
        &mut self,
        project_id: &str,
        session: &str,
        session_file: Option<PathBuf>,
        cause: &str,
    ) -> Option<String> {
        let record = self.leads.remove(project_id)?;
        self.retired.push(RetiredLead {
            project_id: project_id.to_owned(),
            session_id: session.to_owned(),
            session_file: session_file.or(record.session_file),
            retired_at: now_ms(),
            cause: cause.to_owned(),
        });
        if self.retired.len() > 20 {
            self.retired.remove(0);
        }
        Some(session.to_owned())
    }
}

/// Retire a lead: abort the runtime FIRST (finding 6: no intervening worker
/// writes after the claims are gone), release its board claims (both
/// `in_progress` and `blocked` tickets move to `backlog` with `run` cleared and
/// one `Released incomplete:` activity line), move the record into `retired`,
/// persist. Claim release and save errors propagate: the record stays until the
/// retirement completed, so a failed board write never reports success with
/// claims remaining. Archived projects keep their claims visible (unlike
/// `claims()`), so retiring after a project archive still releases them. The
/// store lock is never held across the dispose await.
///
/// `None` — no record for `project_id`; `Err` — the retirement did NOT
/// complete (claims may remain; the record stays for a later retry).
/// # Errors
/// Returns an error for invalid lead state or failed persistence, board, and session operations.
pub async fn retire_lead(
    dispatcher: &Dispatcher,
    project_id: &str,
    cause: &str,
) -> Result<Option<String>, crate::superagent::DispatchError> {
    let (session, session_file) = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(record) = leads.leads.get(project_id) else {
            return Ok(None);
        };
        (record.session_id.clone(), record.session_file.clone())
    };
    dispatcher.workers.retire(&session).await;
    let layout = &dispatcher.layout;
    let claims = claims_of_all_projects(layout)?;
    for (claimed_project, task) in claims {
        if claimed_project.slug != project_id {
            continue;
        }
        let Some(run) = task.run.as_ref() else {
            continue;
        };
        if run.session != session {
            continue;
        }
        if matches!(task.status, Status::InProgress | Status::Blocked) {
            release_to_backlog(layout, &claimed_project, &task, &session, cause)
                .map_err(storage)?;
        }
    }
    let retired = {
        let mut leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = leads.clone();
        let retired = next.record_retirement(project_id, &session, session_file, cause);
        next.save(&dispatcher.leads_path).map_err(storage)?;
        *leads = next;
        retired
    };
    dispatcher
        .lead_sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&session);
    Ok(retired)
}

/// Owner override: plumbing-blocked tickets (blocked, no run, last blocked line
/// a plumbing text) move to backlog with a System comment quoting the text.
/// Finding 2: every candidate revalidates under the project board lock, so a
/// concurrent writer's newer board state is never overwritten by this pass's
/// snapshot (the migration shares the lock all board CLI writers take).
fn migrate_plumbing_blocks(layout: &Layout) -> Result<(), crate::superagent::DispatchError> {
    for project in layout.list_projects().map_err(storage)? {
        let candidates: Vec<String> = {
            let board = Board::open(layout, project.clone()).map_err(storage)?;
            board
                .tasks()
                .map_err(storage)?
                .iter()
                .filter(|task| {
                    task.status == Status::Blocked
                        && task.run.is_none()
                        && !task
                            .activity
                            .iter()
                            .any(|a| a.text.contains(PLUMBING_MIGRATION_NOTE))
                })
                .filter_map(|task| {
                    task.activity
                        .iter()
                        .rev()
                        .find(|a| a.text.starts_with("blocked: "))
                        .map(|a| a.text["blocked: ".len()..].to_owned())
                        .filter(|text| {
                            PLUMBING_PREFIXES
                                .iter()
                                .any(|prefix| text.starts_with(prefix))
                        })
                        .map(|_| task.id.clone())
                })
                .collect()
        };
        for task_id in candidates {
            migrate_one(layout, &project, &task_id).map_err(storage)?;
        }
    }
    Ok(())
}

/// Move one plumbing-blocked ticket to backlog: under the board lock, reload
/// and revalidate (still blocked, no run, latest block text still plumbing)
/// before writing. A concurrent change (a comment, a run, a real question)
/// between the scan and this call makes it a no-op for that ticket.
fn migrate_one(
    layout: &Layout,
    project: &Project,
    task_id: &str,
) -> Result<(), unipi_kanboard::Error> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project.clone())?;
    let mut current = board.get(task_id)?;
    if current.status != Status::Blocked || current.run.is_some() {
        return Ok(());
    }
    let Some(text) = current
        .activity
        .iter()
        .rev()
        .find(|a| a.text.starts_with("blocked: "))
        .map(|a| a.text["blocked: ".len()..].to_owned())
    else {
        return Ok(());
    };
    if !PLUMBING_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
    {
        return Ok(());
    }
    if current
        .activity
        .iter()
        .any(|a| a.text.contains(PLUMBING_MIGRATION_NOTE))
    {
        return Ok(());
    }
    current.status = Status::Backlog;
    current.run = None;
    current.push_activity(
        chrono::Utc::now(),
        Actor::System,
        format!("{PLUMBING_MIGRATION_NOTE} (blocked: {text}); moved to backlog."),
    );
    board.save(&current)
}

fn retire_reason(cause: &str) -> String {
    let head = match cause {
        "stopped" => "lead stopped".to_owned(),
        "archived" => "lead chat archived".to_owned(),
        "replaced" => "lead replaced".to_owned(),
        other => format!("lead {other}"),
    };
    format!(
        "{head}; moved to backlog so Auto-start does not relaunch it; move to todo to run again"
    )
}

/// Move one claimed `in_progress` or blocked ticket to backlog under the board
/// lock, releasing the claim and recording its `Released incomplete:` note.
fn release_to_backlog(
    layout: &Layout,
    project: &Project,
    task: &Task,
    session: &str,
    cause: &str,
) -> Result<(), unipi_kanboard::Error> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project.clone())?;
    let mut current = board.get(&task.id)?;
    if !dispatcher::matches_claim(&current, session) {
        return Ok(()); // claim already gone; nothing to release
    }
    let was = current.status;
    let note = dispatcher::released_incomplete_line(
        was,
        session,
        &retire_reason(cause),
        dispatcher::last_blocked_question(&current),
    );
    current.status = Status::Backlog;
    current.run = None;
    current.push_activity(chrono::Utc::now(), Actor::System, note);
    board.save(&current)
}

fn claims_of(layout: &Layout) -> Result<Vec<(Project, Task)>, crate::superagent::DispatchError> {
    let mut out = Vec::new();
    for project in layout.list_projects().map_err(storage)? {
        if project.archived {
            continue;
        }
        out.extend(claims_in_project(layout, &project)?);
    }
    Ok(out)
}

/// Every dispatcher claim, INCLUDING archived projects (finding 6: a project
/// archived first must still let the retire path release its lead's claims).
fn claims_of_all_projects(
    layout: &Layout,
) -> Result<Vec<(Project, Task)>, crate::superagent::DispatchError> {
    let mut out = Vec::new();
    for project in layout.list_projects().map_err(storage)? {
        out.extend(claims_in_project(layout, &project)?);
    }
    Ok(out)
}

fn claims_in_project(
    layout: &Layout,
    project: &Project,
) -> Result<Vec<(Project, Task)>, crate::superagent::DispatchError> {
    Ok(Board::open(layout, project.clone())
        .map_err(storage)?
        .tasks()
        .map_err(storage)?
        .into_iter()
        .filter(|task| {
            matches!(task.status, Status::InProgress | Status::Blocked)
                && task
                    .run
                    .as_ref()
                    .is_some_and(|r| r.owner == RunOwner::Dispatcher)
        })
        .map(|task| (project.clone(), task))
        .collect())
}
/// The `Workers::outcome` prefix that marks an errored turn.
pub const ERROR_OUTCOME_PREFIX: &str = "provider error: ";

/// The lead machine's tick body: one pass over every record (spec OIN-80).
/// Runs with the dispatcher transaction already held by `tick`.
/// # Errors
/// Returns an error for invalid lead state or failed persistence, board, and session operations.
#[allow(
    clippy::too_many_lines,
    reason = "Exhaustive lead lifecycle state machine keeps retirement, stall recovery, and wake ordering visible together."
)]
pub async fn lead_tick(
    dispatcher: &Dispatcher,
    now: i64,
) -> Result<(), crate::superagent::DispatchError> {
    let settings = dispatcher.lead_settings();
    let warm_ms = i64::from(settings.warm_minutes) * 60_000;
    let projects = dispatcher.layout.list_projects().map_err(storage)?;
    let session_ids: Vec<String> = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        leads.leads.values().map(|r| r.session_id.clone()).collect()
    };
    for session_id in &session_ids {
        // Item 13a: retire ONLY on positive archive evidence (e.g. file moved
        // into trash / archived by the hub). A missing session file without
        // archive evidence keeps the record and stalls it with 'chat file missing'.
        let missing_info = {
            let leads = dispatcher
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leads
                .leads
                .values()
                .find(|r| r.session_id == *session_id)
                .and_then(|r| {
                    let file = r.session_file.clone()?;
                    (!file.exists()).then(|| {
                        let already_missing_stalled = r.state == LeadState::Stalled
                            && r.health
                                .as_ref()
                                .is_some_and(|h| h.error == "chat file missing" && h.notified);
                        (r.project_id.clone(), file, already_missing_stalled)
                    })
                })
        };
        if let Some((project_id, missing_file, already_missing_stalled)) = missing_info {
            if dispatcher
                .workers
                .is_session_archived(session_id, Some(&missing_file))
                .await
            {
                retire_lead(dispatcher, &project_id, "archived").await?;
                continue;
            }
            if let Some(project) = projects.iter().find(|p| p.slug == project_id) {
                if !already_missing_stalled {
                    enter_stall(
                        dispatcher,
                        project,
                        HealthKind::Launch,
                        "chat file missing".to_owned(),
                        now,
                        None,
                    )
                    .await?;
                }
                // Even while stalled on a missing file, retry any unfinished
                // board/dialog outbox or timeout steps (delivery skips while un-wakeable).
                // Tick isolation: log and skip this lead on error instead of aborting other leads.
                if let Err(e) =
                    flush_pending_answers(dispatcher, project, now, None, false, false).await
                {
                    tracing::warn!(
                        project = %project.slug,
                        session = %session_id,
                        error = %e,
                        "lead_tick outbox flush failed on missing-file lead; skipping lead this tick"
                    );
                    continue;
                }
                if let Err(e) = question_step(dispatcher, project, now).await {
                    tracing::warn!(
                        project = %project.slug,
                        session = %session_id,
                        error = %e,
                        "lead_tick question_step failed on missing-file lead; skipping lead this tick"
                    );
                    continue;
                }
            }
            continue;
        }
        let Some(project) = projects
            .iter()
            .find(|p| {
                dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .leads
                    .values()
                    .any(|r| r.session_id == *session_id && r.project_id == p.slug)
            })
            .cloned()
        else {
            continue;
        };
        let record = {
            let leads = dispatcher
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leads.leads.get(&project.slug).cloned()
        };
        let Some(record) = record else { continue };
        // Observe.
        let (record, observation) = observe(dispatcher, &project, record).await;
        // Finding 7: the configured fallback is RESOLVED (against the lead's
        // own services) before the state machine selects it — an invalid one
        // is dropped, so an unresolvable fallback cannot leave the record
        // claiming onFallback while the switch failed. None → straight Stalled.
        let fallback: Option<&str> = match settings.fallback_model.as_deref() {
            Some(name) if !name.trim().is_empty() => {
                let split = name
                    .split_once('/')
                    .filter(|(p, i)| !p.is_empty() && !i.is_empty());
                let valid = match split {
                    Some((provider, id)) => dispatcher
                        .workers
                        .core_services(&project)
                        .await
                        .is_ok_and(|services| services.models.find(provider, id).is_some()),
                    None => false,
                };
                valid.then_some(name)
            }
            _ => None,
        };
        if !matches!(observation, TurnOutcome::Gone) {
            // Working/Waiting/Idle handled through the pure step below; the
            // state machine (probe/warm-unload) still runs afterwards on the
            // persisted state — a Done that goes Idle must reach its warm
            // unload in the same pass once `warmMinutes` have elapsed.
            let mut next_record = record.clone();
            let effects = step(
                &mut next_record,
                observation.clone(),
                now,
                &settings,
                fallback,
            );
            if let Some(Effect::SetModel(fb)) = effects.first().cloned() {
                // Item 11: apply runtime `set_model` FIRST; commit `on_fallback`
                // only on success. On failure do NOT commit `on_fallback: true`
                // or send continuation — stall with a reason naming the model error.
                match dispatcher.workers.set_model(&record.session_id, &fb).await {
                    Ok(()) => {
                        {
                            let mut leads = dispatcher
                                .leads
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            next_record.session_file = leads
                                .leads
                                .get(&project.slug)
                                .and_then(|r| r.session_file.clone())
                                .or(next_record.session_file);
                            next_record.pending_answers = leads
                                .leads
                                .get(&project.slug)
                                .map(|r| r.pending_answers.clone())
                                .unwrap_or(next_record.pending_answers);
                            if next_record.state == LeadState::Working {
                                next_record.last_active_at = now;
                            }
                            leads
                                .leads
                                .insert(project.slug.clone(), next_record.clone());
                            leads.save(&dispatcher.leads_path).map_err(storage)?;
                        }
                        for effect in effects.into_iter().skip(1) {
                            apply_effect(dispatcher, &project, &next_record, effect).await?;
                        }
                    }
                    Err(model_err) => {
                        let provider_err = match &observation {
                            TurnOutcome::Errored { message } => message.clone(),
                            _ => String::new(),
                        };
                        let reason = if provider_err.is_empty() {
                            format!("Fallback model switch to {fb} failed: {model_err}")
                        } else {
                            format!(
                                "{provider_err}; fallback model switch to {fb} failed: {model_err}"
                            )
                        };
                        let next_probe = Some(
                            reset_hint_ms(&provider_err, now).unwrap_or(now + STALL_REPROBE_MS),
                        );
                        enter_stall(
                            dispatcher,
                            &project,
                            HealthKind::Model,
                            reason,
                            now,
                            next_probe,
                        )
                        .await?;
                    }
                }
            } else {
                let has_notify = effects.iter().any(|e| matches!(e, Effect::Notify));
                {
                    let mut leads = dispatcher
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    next_record.session_file = leads
                        .leads
                        .get(&project.slug)
                        .and_then(|r| r.session_file.clone())
                        .or(next_record.session_file);
                    next_record.pending_answers = leads
                        .leads
                        .get(&project.slug)
                        .map(|r| r.pending_answers.clone())
                        .unwrap_or(next_record.pending_answers);
                    if next_record.state == LeadState::Working {
                        next_record.last_active_at = now;
                    }
                    // Item 8: persist `notified = false` until the notice has actually been sent.
                    if has_notify {
                        if let Some(health) = next_record.health.as_mut() {
                            health.notified = false;
                        }
                    }
                    leads
                        .leads
                        .insert(project.slug.clone(), next_record.clone());
                    leads.save(&dispatcher.leads_path).map_err(storage)?;
                }
                for effect in effects {
                    apply_effect(dispatcher, &project, &next_record, effect).await?;
                }
            }
        }
        // Retry any unfinished durable outbox entries (`pending_answers`) and run
        // `question_step` in ANY lead state including `Stalled` (A & D2).
        // Tick isolation: errors on one lead are logged and skip only that lead.
        if let Err(e) = flush_pending_answers(dispatcher, &project, now, None, false, false).await {
            tracing::warn!(
                project = %project.slug,
                session = %session_id,
                error = %e,
                "lead_tick outbox flush failed; skipping lead this tick"
            );
            continue;
        }
        if let Err(e) = question_step(dispatcher, &project, now).await {
            tracing::warn!(
                project = %project.slug,
                session = %session_id,
                error = %e,
                "lead_tick question_step failed; skipping lead this tick"
            );
            continue;
        }
        let (stored_state, needs_runtime, stored_last_active, stored_health) = {
            let leads = dispatcher
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(r) = leads.leads.get(&project.slug) else {
                continue;
            };
            (
                r.state,
                // F4: suppress warm-unload ONLY while an open question or an outbox
                // entry still needs the runtime (dialog not closed or delivery not done).
                // Bookkeeping-only entries (`delivered == true`, board pending) never retain a runtime.
                r.question.is_some() || r.pending_answers.iter().any(PendingAnswer::needs_runtime),
                r.last_active_at,
                r.health.clone(),
            )
        };
        match stored_state {
            LeadState::Stalled => {
                // Probe only at `next_probe_at`; before that the lead keeps
                // stalling quietly (the reset hint or the 30-minute re-probe).
                if stored_health
                    .as_ref()
                    .and_then(|h| h.next_probe_at)
                    .is_none_or(|at| now < at)
                {
                    continue;
                }
                // Probe: wake if unloaded (a live lead is only continued), back
                // to the record's own model, continue.
                let error = stored_health
                    .as_ref()
                    .map(|h| h.error.clone())
                    .unwrap_or_default();
                let awake = dispatcher.workers.live(&record.session_id).await;
                let launch = async {
                    if awake {
                        return Ok(());
                    }
                    dispatcher.register_lead_session(&record.session_id);
                    dispatcher
                        .workers
                        .launch_lead(&project, &[], &record.session_id, &record.preset)
                        .await
                };
                if let Err(e) = launch.await {
                    enter_stall(
                        dispatcher,
                        &project,
                        HealthKind::Launch,
                        format!("Could not reopen lead: {e}"),
                        now,
                        Some(now + STALL_REPROBE_MS),
                    )
                    .await?;
                    continue;
                }
                let session_file = dispatcher.workers.session_file(&record.session_id).await;
                {
                    let mut leads = dispatcher
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(stored) = leads.leads.get_mut(&project.slug) {
                        stored.state = LeadState::Working;
                        stored.on_fallback = false;
                        stored.session_file = session_file;
                        if let Some(health) = stored.health.as_mut() {
                            health.notified = true; // probes never re-notify
                            health.next_probe_at = None;
                        }
                        leads.save(&dispatcher.leads_path).map_err(storage)?;
                    }
                }
                if let Some(model) = record.preset.model.clone() {
                    let _ = dispatcher
                        .workers
                        .set_model(&record.session_id, &model)
                        .await;
                }
                let _ = dispatcher
                    .workers
                    .continue_turn(&record.session_id, &PROBE_TEXT.replace("{error}", &error))
                    .await;
            }
            LeadState::Idle | LeadState::Waiting => {
                // Warm idle (finding 9 covers Waiting too): past `warmMinutes`
                // with no open question, unload (dispose the runtime; the
                // record and its file survive the restart). Waiting holds only
                // blocked tickets — nothing a wake could continue — so it
                // releases the runtime the same way.
                if now.saturating_sub(stored_last_active) >= warm_ms && !needs_runtime {
                    let session_file = dispatcher.workers.session_file(&record.session_id).await;
                    dispatcher.workers.unload(&record.session_id).await;
                    let mut leads = dispatcher
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(stored) = leads.leads.get_mut(&project.slug) {
                        stored.state = LeadState::Unloaded;
                        stored.session_file = session_file;
                        leads.save(&dispatcher.leads_path).map_err(storage)?;
                    }
                }
            }
            LeadState::Working if matches!(observation, TurnOutcome::Gone) => {
                // The runtime vanished mid-episode (finding 9): resume the
                // SAME session id — never a new chat. A crash between the
                // observe and here (or a killed runtime) resumes on the next
                // tick; failure records launch health (with probe) without
                // dropping the record or its claims, and the retry waits for
                // the recorded backoff (the health's `next_probe_at`) instead
                // of hammering every tick. A live lead (observation not Gone)
                // is mid-turn or awaiting its outcome — untouched.
                if record
                    .health
                    .as_ref()
                    .and_then(|h| h.next_probe_at)
                    .is_some_and(|at| now < at)
                {
                    continue;
                }
                dispatcher.register_lead_session(&record.session_id);
                let launched = dispatcher
                    .workers
                    .launch_lead(&project, &[], &record.session_id, &record.preset)
                    .await;
                match launched {
                    Ok(()) => {
                        let session_file =
                            dispatcher.workers.session_file(&record.session_id).await;
                        {
                            let mut leads = dispatcher
                                .leads
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if let Some(stored) = leads.leads.get_mut(&project.slug) {
                                stored.session_file = session_file.or(stored.session_file.clone());
                                stored.resume_sent_at = Some(now);
                                if stored.health.as_ref().is_some_and(|h| {
                                    h.kind == crate::superagent_leads::HealthKind::Launch
                                }) {
                                    stored.health = None;
                                }
                                leads.save(&dispatcher.leads_path).map_err(storage)?;
                            }
                        }
                        // Persisted before sending (a crash cannot loop the text).
                        let held = record.continuation.tickets.join(", ");
                        let _ = dispatcher
                            .workers
                            .continue_turn(
                                &record.session_id,
                                &RESTART_TEXT.replace("{ids}", &held),
                            )
                            .await;
                    }
                    Err(e) => {
                        enter_stall(
                            dispatcher,
                            &project,
                            HealthKind::Launch,
                            format!("Could not reopen lead: {e}"),
                            now,
                            Some(now + STALL_REPROBE_MS),
                        )
                        .await?;
                    }
                }
            }
            // A live Working lead (observation not Gone): mid-turn, untouched.
            LeadState::Unloaded | LeadState::Working => {}
        }
    }
    Ok(())
}

/// Read one live lead's turn outcome (spec `Workers::outcome`), refreshing the
/// held-ticket sets on the returned record: `continuation.tickets` feeds the
/// nudge, `in_progress_tickets` picks the Done branch (Waiting vs stall).
async fn observe(
    dispatcher: &Dispatcher,
    project: &Project,
    record: LeadRecord,
) -> (LeadRecord, TurnOutcome) {
    if record.state == LeadState::Unloaded || record.state == LeadState::Stalled {
        return (record, TurnOutcome::Gone);
    }
    if !dispatcher.workers.live(&record.session_id).await {
        // Not live: the stalled probe and the warm-idle unload handle it.
        return (record, TurnOutcome::Gone);
    }
    if !dispatcher
        .workers
        .questions(&record.session_id)
        .await
        .is_empty()
    {
        // Waiting on an open ask_user dialog from the owner: never nudge or
        // NoProgress-stall while the dialog is open.
        return (record, TurnOutcome::Busy);
    }
    let Some(text) = dispatcher.workers.outcome(&record.session_id).await else {
        return (record, TurnOutcome::Busy);
    };
    let mut record = record;
    if let Ok(tasks) = held_tickets(dispatcher, project, &record.session_id) {
        let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
        record.in_progress_tickets = tasks
            .iter()
            .filter(|t| t.status == Status::InProgress)
            .map(|t| t.id.clone())
            .collect();
        // A ticket transition (a held id the lead no longer holds) replenishes
        // the nudge budget: the lead made progress since the last tick.
        if record
            .continuation
            .tickets
            .iter()
            .any(|ticket| !ids.contains(ticket))
        {
            record.continuation.sent = false;
            record.continuation.total = 0;
        }
        record.continuation.tickets = ids;
    }
    if let Some(message) = text.strip_prefix(ERROR_OUTCOME_PREFIX) {
        return (
            record,
            TurnOutcome::Errored {
                message: message.to_owned(),
            },
        );
    }
    (record, TurnOutcome::Done { last: text })
}

async fn apply_effect(
    dispatcher: &Dispatcher,
    project: &Project,
    record: &LeadRecord,
    effect: Effect,
) -> Result<(), crate::superagent::DispatchError> {
    match effect {
        Effect::Continue(text) => {
            if dispatcher
                .workers
                .continue_turn(&record.session_id, &text)
                .await
                .is_err()
            {
                // Roll the nudge budget back so the next tick retries the send.
                let mut leads = dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(stored) = leads.leads.get_mut(&record.project_id) {
                    stored.continuation.sent = false;
                    stored.continuation.total = stored.continuation.total.saturating_sub(1);
                    let _ = leads.save(&dispatcher.leads_path);
                }
                return Err(fail("E_RELAY_DELIVERY", "lead continue failed"));
            }
            let _ = project;
        }
        Effect::SetModel(model) => {
            let _ = dispatcher
                .workers
                .set_model(&record.session_id, &model)
                .await;
        }
        Effect::Notify => {
            let error = record
                .health
                .as_ref()
                .map(|h| h.error.clone())
                .unwrap_or_default();
            if stall_entry_notice_for(dispatcher, &record.session_id, &project.name, error).await {
                let mut leads = dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(stored) = leads.leads.get_mut(&record.project_id) {
                    if let Some(health) = stored.health.as_mut() {
                        health.notified = true;
                    }
                    leads.save(&dispatcher.leads_path).map_err(storage)?;
                }
            }
        }
    }
    Ok(())
}

/// Centralized deduplicated stall entry (Item 8): transitions `project`'s lead
/// to `LeadState::Stalled`, preserves `notified` across updates, sends at most
/// one notification per stall episode, and marks `notified = true` ONLY after
/// the notice was actually sent.
/// # Errors
/// Returns an error for invalid lead state or failed persistence, board, and session operations.
pub async fn enter_stall(
    dispatcher: &Dispatcher,
    project: &Project,
    kind: HealthKind,
    error: String,
    now: i64,
    next_probe_at: Option<i64>,
) -> Result<(), crate::superagent::DispatchError> {
    let target = {
        let mut leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stored) = leads.leads.get_mut(&project.slug) else {
            return Ok(());
        };
        let prev_notified = stored.health.as_ref().is_some_and(|h| h.notified);
        let since = stored.health.as_ref().map_or(now, |h| h.since);
        let attempts = stored.health.as_ref().map_or(1, |h| h.attempts + 1);
        let session_id = stored.session_id.clone();
        stored.state = LeadState::Stalled;
        stored.health = Some(Health {
            kind,
            error: error.clone(),
            attempts,
            since,
            next_probe_at,
            notified: prev_notified,
        });
        leads.save(&dispatcher.leads_path).map_err(storage)?;
        (!prev_notified).then_some(session_id)
    };
    if let Some(session_id) = target {
        if stall_entry_notice_for(dispatcher, &session_id, &project.name, error).await {
            let mut leads = dispatcher
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(stored) = leads.leads.get_mut(&project.slug) {
                if let Some(health) = stored.health.as_mut() {
                    health.notified = true;
                }
                leads.save(&dispatcher.leads_path).map_err(storage)?;
            }
        }
    }
    Ok(())
}

/// The one stall-ENTRY push routed through `dispatcher.workers.lead_notice`
/// (returns `true` only when the notice dispatch ran).
async fn stall_entry_notice_for(
    dispatcher: &Dispatcher,
    session_id: &str,
    project_name: &str,
    error: String,
) -> bool {
    dispatcher
        .workers
        .lead_notice(
            session_id,
            &format!("Lead stalled \u{b7} {project_name}"),
            &format!("{error}. Retry, switch model or stop in the work pane."),
        )
        .await
}

/// Direct stall-entry push helper (kept for `session_notify` integration tests).
pub async fn stall_entry_notice(session_id: &str, project_name: &str, error: String) {
    crate::notify::lead_notice(
        session_id,
        &format!("Lead stalled \u{b7} {project_name}"),
        &format!("{error}. Retry, switch model or stop in the work pane."),
    )
    .await;
}

fn held_tickets(
    dispatcher: &Dispatcher,
    project: &Project,
    session: &str,
) -> Result<Vec<Task>, crate::superagent::DispatchError> {
    let board = Board::open(&dispatcher.layout, project.clone()).map_err(storage)?;
    Ok(board
        .tasks()
        .map_err(storage)?
        .into_iter()
        .filter(|task| dispatcher::matches_claim(task, session))
        .collect())
}

/// Drive any unfinished `PendingAnswer` outbox steps (`dialog_closed`, `delivered`,
/// `board_done`) idempotently for `project`'s current lead.
///
/// - `strict = true` (direct RPC call): returns step errors (`E_WORKER_LAUNCH`,
///   `E_RELAY_DELIVERY`, `E_BOARD_RESUME_FAILED`, `E_ALREADY_ANSWERED`) to the caller.
/// - `strict = false` (`lead_tick`): runs board and dialog steps in ANY lead state
///   (including `Stalled`), and runs delivery steps whenever the runtime is live or
///   wakeable (waking the SAME lead session if offline); transient runtime/board
///   failures leave unfinished flags `false` for the next tick without aborting `lead_tick`.
#[must_use]
pub fn ui_answer_to_value(answer: &oino_core::ui::UiAnswer) -> Value {
    match answer {
        oino_core::ui::UiAnswer::Value(v) => v.clone(),
        oino_core::ui::UiAnswer::Cancelled => Value::Null,
        oino_core::ui::UiAnswer::Confirmed(_) => json!({"answers": []}),
    }
}

fn value_to_ui_answer(value: Option<&Value>) -> oino_core::ui::UiAnswer {
    match value {
        Some(Value::Null) => oino_core::ui::UiAnswer::Cancelled,
        Some(v) => oino_core::ui::UiAnswer::Value(v.clone()),
        None => crate::superagent_questions::skipped_answer(),
    }
}

fn record_entry_backoff(
    dispatcher: &Dispatcher,
    project_slug: &str,
    entry_id: &str,
    now: i64,
    is_runtime_step: bool,
) -> Result<u32, crate::superagent::DispatchError> {
    let mut leads = dispatcher
        .leads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut next = leads.clone();
    let mut runtime_failures = 0;
    if let Some(record) = next.leads.get_mut(project_slug) {
        if let Some(stored_pa) = record.pending_answers.iter_mut().find(|p| p.id == entry_id) {
            stored_pa.attempts = stored_pa.attempts.saturating_add(1);
            stored_pa.next_attempt_at = Some(now + outbox_backoff_ms(stored_pa.attempts));
            if is_runtime_step {
                stored_pa.runtime_failures = stored_pa.runtime_failures.saturating_add(1);
            }
            runtime_failures = stored_pa.runtime_failures;
        }
        next.save(&dispatcher.leads_path).map_err(storage)?;
        *leads = next;
    }
    Ok(runtime_failures)
}

/// Drive unfinished `PendingAnswer` outbox entries (`pending_answers`) for `project`:
/// - `filter = Some(EntryId(id))`: touches ONLY that operation (never another ticket's entry, F2).
/// - `strict = true` (direct RPC): returns step errors to the caller.
/// - `strict = false` (`lead_tick` / health `retry`): isolates each entry so one failing
///   entry never blocks sibling entries on the same lead.
/// - `bypass_backoff = true` (explicit owner answer retry or health-card Retry): ignores
///   `next_attempt_at` and stall probe deadlines (F3).
/// # Errors
/// Returns an error for invalid lead state or failed persistence, board, and session operations.
pub async fn flush_pending_answers(
    dispatcher: &Dispatcher,
    project: &Project,
    now: i64,
    filter: Option<OutboxFilter>,
    strict: bool,
    bypass_backoff: bool,
) -> Result<(), crate::superagent::DispatchError> {
    let entry_ids: Vec<String> = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(record) = leads.leads.get(&project.slug) else {
            return Ok(());
        };
        record
            .pending_answers
            .iter()
            .filter(|pa| match &filter {
                Some(OutboxFilter::EntryId(id)) => &pa.id == id,
                None => true,
            })
            .map(|pa| pa.id.clone())
            .collect()
    };

    for entry_id in entry_ids {
        if let Err(e) =
            flush_single_entry(dispatcher, project, &entry_id, now, strict, bypass_backoff).await
        {
            if strict {
                return Err(e);
            }
            tracing::warn!(
                project = %project.slug,
                entry = %entry_id,
                error = %e,
                "outbox entry flush failed; continuing with remaining entries"
            );
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "Durable outbox state machine deliberately orders board, dialog, and steering receipt phases in a single retry operation."
)]
async fn flush_single_entry(
    dispatcher: &Dispatcher,
    project: &Project,
    entry_id: &str,
    now: i64,
    strict: bool,
    bypass_backoff: bool,
) -> Result<(), crate::superagent::DispatchError> {
    let snapshot = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(record) = leads.leads.get(&project.slug) else {
            return Ok(());
        };
        let Some(pa) = record
            .pending_answers
            .iter()
            .find(|p| p.id == entry_id)
            .cloned()
        else {
            return Ok(());
        };
        (
            record.session_id.clone(),
            record.preset.clone(),
            record.session_file.clone(),
            record.state,
            record.health.as_ref().and_then(|h| h.next_probe_at),
            pa,
        )
    };
    let (session, preset, existing_file, initial_state, stall_probe_at, mut pa) = snapshot;

    // F3: automatic tick retries honour per-entry exponential backoff.
    if !bypass_backoff && pa.next_attempt_at.is_some_and(|at| now < at) {
        return Ok(());
    }

    // F3: once an entry has hit >= 3 consecutive runtime failures and entered Stalled,
    // automatic runtime steps honour the stall probe deadline (`next_probe_at`).
    let runtime_gated_by_stall = !bypass_backoff
        && pa.runtime_failures >= 3
        && initial_state == LeadState::Stalled
        && stall_probe_at.is_none_or(|at| now < at);

    // 1. Timeout board step happens BEFORE closing the dialog / steering so the
    //    ticket is durably `blocked` with its claim kept first (Item 3).
    if pa.kind == PendingAnswerKind::Timeout && !pa.board_done {
        if !pa.task_id.is_empty() {
            let already_parked = Board::open(&dispatcher.layout, project.clone())
                .ok()
                .and_then(|b| b.get(&pa.task_id).ok())
                .is_some_and(|t| {
                    t.status == Status::Blocked && dispatcher::matches_claim(&t, &session)
                });
            if !already_parked {
                if let Err(e) = dispatcher::approval_transition(
                    &dispatcher.layout,
                    project.clone(),
                    &pa.task_id,
                    &session,
                    Some(&pa.text),
                ) {
                    let _ = record_entry_backoff(dispatcher, &project.slug, entry_id, now, false);
                    if strict {
                        return Err(storage(e));
                    }
                    tracing::warn!(
                        project = %project.slug,
                        task = %pa.task_id,
                        session = %session,
                        error = %e,
                        "question timeout could not block ticket yet; scheduled backoff retry"
                    );
                    return Ok(());
                }
            }
        }
        pa.board_done = true;
        let mut leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = leads.clone();
        if let Some(record) = next.leads.get_mut(&project.slug) {
            if let Some(stored_pa) = record.pending_answers.iter_mut().find(|p| p.id == entry_id) {
                stored_pa.board_done = true;
                stored_pa.next_attempt_at = None;
            }
            if let Some(q) = record
                .question
                .as_mut()
                .filter(|q| Some(q.request_id.as_str()) == pa.request_id.as_deref())
            {
                q.board_blocked = true;
            }
            next.save(&dispatcher.leads_path).map_err(storage)?;
            *leads = next;
        }
    }

    // 2. Dialog step (`OpenQuestion` or `Timeout`): retries the EXACT persisted
    //    `dialog_answer` when open, or falls back to steer if the dialog vanished
    //    before the owner's words reached the lead through it (F1).
    if !pa.dialog_closed && !runtime_gated_by_stall {
        let mut answered_via_dialog = false;
        if let Some(req_id) = pa.request_id.clone() {
            let open_questions = dispatcher.workers.questions(&session).await;
            if open_questions.iter().any(|q| q.request_id == req_id) {
                let ui_answer = value_to_ui_answer(pa.dialog_answer.as_ref());
                if let Err(e) = dispatcher
                    .workers
                    .answer(&session, &req_id, ui_answer)
                    .await
                {
                    let rf = record_entry_backoff(dispatcher, &project.slug, entry_id, now, true)?;
                    if rf >= 3 {
                        enter_stall(
                            dispatcher,
                            project,
                            HealthKind::Launch,
                            format!("Could not answer lead dialog: {e}"),
                            now,
                            Some(now + STALL_REPROBE_MS),
                        )
                        .await?;
                    }
                    if strict {
                        return Err(fail(
                            "E_ALREADY_ANSWERED",
                            format!("Could not answer dialog: {e}"),
                        ));
                    }
                    tracing::warn!(
                        project = %project.slug,
                        session = %session,
                        request = %req_id,
                        error = %e,
                        "outbox could not resolve dialog yet; scheduled backoff retry"
                    );
                    return Ok(());
                }
                answered_via_dialog = true;
            }
        }
        pa.dialog_closed = true;
        pa.runtime_failures = 0;
        if pa.kind == PendingAnswerKind::OpenQuestion && pa.delivery_text.is_empty() {
            if answered_via_dialog {
                // Selected-option / allowed free-text reached the lead directly via `workers.answer`.
                pa.delivered = true;
            } else {
                // F1: dialog vanished before we answered it; fall back to steering verbatim words!
                let q = pa.question_excerpt.as_deref().unwrap_or("");
                pa.delivery_text = format!("Owner answered your question (\"{q}\"): {}", pa.text);
                pa.mode = PendingAnswerMode::Steer;
                pa.delivered = false;
            }
        }
        let mut leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = leads.clone();
        if let Some(record) = next.leads.get_mut(&project.slug) {
            if let Some(stored_pa) = record.pending_answers.iter_mut().find(|p| p.id == entry_id) {
                stored_pa.dialog_closed = true;
                stored_pa.delivered = pa.delivered;
                stored_pa.delivery_text.clone_from(&pa.delivery_text);
                stored_pa.mode = pa.mode;
                stored_pa.runtime_failures = 0;
                stored_pa.next_attempt_at = None;
            }
            if pa.delivered {
                if record
                    .question
                    .as_ref()
                    .is_some_and(|q| Some(q.request_id.as_str()) == pa.request_id.as_deref())
                {
                    record.question = None;
                }
                if record.state == LeadState::Waiting {
                    record.state = LeadState::Working;
                    record.last_active_at = now;
                }
            } else if let Some(q) = record
                .question
                .as_mut()
                .filter(|q| Some(q.request_id.as_str()) == pa.request_id.as_deref())
            {
                q.dialog_resolved = true;
            }
            next.save(&dispatcher.leads_path).map_err(storage)?;
            *leads = next;
        }
    }

    // 3. Delivery step: wake the SAME lead session when offline & wakeable, deliver
    //    once, and persist `delivered = true` IMMEDIATELY after `relay_delivery` succeeds.
    if !pa.delivered && !runtime_gated_by_stall {
        if pa.delivery_text.is_empty() {
            pa.delivered = true;
        } else {
            let mut is_live = dispatcher.workers.live(&session).await;
            if !is_live {
                if existing_file.as_deref().is_some_and(|f| !f.exists()) {
                    if strict {
                        return Err(fail(
                            "E_WORKER_LAUNCH",
                            "Could not wake the lead: chat file missing",
                        ));
                    }
                } else {
                    dispatcher.register_lead_session(&session);
                    match dispatcher
                        .workers
                        .launch_lead(project, &[], &session, &preset)
                        .await
                    {
                        Ok(()) => {
                            dispatcher.mark_active(&session, &project.slug);
                            is_live = true;
                        }
                        Err(e) => {
                            let rf = record_entry_backoff(
                                dispatcher,
                                &project.slug,
                                entry_id,
                                now,
                                true,
                            )?;
                            if rf >= 3 {
                                enter_stall(
                                    dispatcher,
                                    project,
                                    HealthKind::Launch,
                                    format!("Could not reopen lead: {e}"),
                                    now,
                                    Some(now + STALL_REPROBE_MS),
                                )
                                .await?;
                            }
                            if strict {
                                return Err(fail(
                                    "E_WORKER_LAUNCH",
                                    format!("Could not wake the lead: {e}"),
                                ));
                            }
                            tracing::warn!(
                                project = %project.slug,
                                session = %session,
                                error = %e,
                                attempts = rf,
                                "outbox could not wake offline lead yet; scheduled backoff retry"
                            );
                        }
                    }
                }
            }
            if is_live {
                let delivery = match pa.mode {
                    PendingAnswerMode::Relay => Delivery::Queue,
                    PendingAnswerMode::Steer => Delivery::Steer,
                };
                if let Err(e) = dispatcher
                    .workers
                    .relay_delivery(&session, &pa.delivery_text, delivery)
                    .await
                {
                    let rf = record_entry_backoff(dispatcher, &project.slug, entry_id, now, true)?;
                    if rf >= 3 {
                        enter_stall(
                            dispatcher,
                            project,
                            HealthKind::Launch,
                            format!("Could not deliver to lead: {e}"),
                            now,
                            Some(now + STALL_REPROBE_MS),
                        )
                        .await?;
                    }
                    if strict {
                        return Err(fail(
                            "E_RELAY_DELIVERY",
                            format!("Could not deliver answer to the lead: {e}"),
                        ));
                    }
                    tracing::warn!(
                        project = %project.slug,
                        session = %session,
                        error = %e,
                        attempts = rf,
                        "outbox delivery failed; scheduled backoff retry"
                    );
                } else {
                    pa.delivered = true;
                    pa.runtime_failures = 0;
                    let session_file = dispatcher.workers.session_file(&session).await;
                    let mut leads = dispatcher
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(record) = leads.leads.get_mut(&project.slug) {
                        if let Some(stored_pa) =
                            record.pending_answers.iter_mut().find(|p| p.id == entry_id)
                        {
                            stored_pa.delivered = true;
                            stored_pa.runtime_failures = 0;
                            stored_pa.next_attempt_at = None;
                        }
                        if pa.kind == PendingAnswerKind::BlockedAnswer {
                            record.state = LeadState::Working;
                            record.health = None;
                            record.last_active_at = now;
                            if record
                                .question
                                .as_ref()
                                .is_some_and(|q| q.task_id == pa.task_id)
                            {
                                record.question = None;
                            }
                        } else {
                            if let Some(req_id) = pa.request_id.as_deref() {
                                if record
                                    .question
                                    .as_ref()
                                    .is_some_and(|q| q.request_id == req_id)
                                {
                                    record.question = None;
                                }
                            }
                            if matches!(
                                initial_state,
                                LeadState::Unloaded | LeadState::Waiting | LeadState::Idle
                            ) || (initial_state == LeadState::Stalled
                                && record
                                    .health
                                    .as_ref()
                                    .is_some_and(|h| h.kind == HealthKind::Launch))
                            {
                                record.state = LeadState::Working;
                                record.last_active_at = now;
                                if record
                                    .health
                                    .as_ref()
                                    .is_some_and(|h| h.kind == HealthKind::Launch)
                                {
                                    record.health = None;
                                }
                            }
                        }
                        if session_file.is_some() {
                            record.session_file = session_file;
                        }
                        // Note (F5, accepted): delivery is at-least-once across an abrupt
                        // host crash or save failure in the narrow window after `relay_delivery`
                        // succeeded and before persisting `delivered = true` here, because
                        // runtime message queues do not deduplicate by outbox entry id.
                        leads.save(&dispatcher.leads_path).map_err(storage)?;
                    }
                }
            }
        }
    }

    // 4. Post-delivery board step for `BlockedAnswer` and `OpenQuestion` (F2).
    if !pa.board_done && pa.kind != PendingAnswerKind::Timeout {
        match pa.kind {
            PendingAnswerKind::BlockedAnswer if pa.delivered => {
                enum ResumeResult {
                    Done,
                    Unavailable(unipi_kanboard::Error),
                }
                let outcome = (|| -> ResumeResult {
                    let _lock = match dispatcher.layout.lock_board(&project.slug) {
                        Ok(l) => l,
                        Err(e) => return ResumeResult::Unavailable(e),
                    };
                    let board = match Board::open(&dispatcher.layout, project.clone()) {
                        Ok(b) => b,
                        Err(e) => return ResumeResult::Unavailable(e),
                    };
                    let mut task = match board.get(&pa.task_id) {
                        Ok(t) => t,
                        Err(e) => return ResumeResult::Unavailable(e),
                    };
                    let expected_answer = format!("Owner answer: {}", pa.text);
                    if task.status == Status::InProgress
                        && dispatcher::matches_claim(&task, &session)
                        && task
                            .activity
                            .iter()
                            .any(|a| a.actor == Actor::User && a.text == expected_answer)
                    {
                        return ResumeResult::Done;
                    }
                    // F2: CONFLICT (ticket no longer blocked, or claim no longer belongs
                    // to this lead) is terminal: record a System note and drop the entry.
                    if task.status != Status::Blocked || !dispatcher::matches_claim(&task, &session)
                    {
                        let reason = if task.status == Status::Blocked {
                            "claim no longer belongs to this lead".to_owned()
                        } else {
                            format!("ticket is {}, not blocked", task.status)
                        };
                        let note = format!(
                            "Owner answer was delivered to the lead; board resume skipped: {reason}"
                        );
                        if !task.activity.iter().any(|a| a.text == note) {
                            task.push_activity(chrono::Utc::now(), Actor::System, note);
                            if let Err(e) = board.save(&task) {
                                return ResumeResult::Unavailable(e);
                            }
                        }
                        return ResumeResult::Done;
                    }
                    task.status = Status::InProgress;
                    task.push_activity(chrono::Utc::now(), Actor::User, expected_answer);
                    match board.save(&task) {
                        Ok(()) => ResumeResult::Done,
                        Err(e) => ResumeResult::Unavailable(e),
                    }
                })();
                match outcome {
                    ResumeResult::Done => {
                        pa.board_done = true;
                    }
                    ResumeResult::Unavailable(e) => {
                        let _ =
                            record_entry_backoff(dispatcher, &project.slug, entry_id, now, false);
                        tracing::warn!(
                            project = %project.slug,
                            task = %pa.task_id,
                            session = %session,
                            error = %e,
                            "answer delivered to lead, but moving blocked ticket to in_progress failed; scheduled backoff retry"
                        );
                        if strict {
                            return Err(fail(
                                "E_BOARD_RESUME_FAILED",
                                format!(
                                    "Answer was delivered to lead {session}, but updating {} to in_progress failed: {e}",
                                    pa.task_id
                                ),
                            ));
                        }
                        return Ok(());
                    }
                }
            }
            PendingAnswerKind::OpenQuestion if pa.delivered => {
                if !pa.task_id.is_empty() {
                    let record_activity = || -> Result<(), unipi_kanboard::Error> {
                        let _lock = dispatcher.layout.lock_board(&project.slug)?;
                        let board = Board::open(&dispatcher.layout, project.clone())?;
                        let mut task = board.get(&pa.task_id)?;
                        if dispatcher::matches_claim(&task, &session) {
                            let expected_line = format!("Owner answer: {}", pa.text);
                            if !task
                                .activity
                                .iter()
                                .any(|a| a.actor == Actor::User && a.text == expected_line)
                            {
                                task.push_activity(chrono::Utc::now(), Actor::User, expected_line);
                                board.save(&task)?;
                            }
                        }
                        Ok(())
                    };
                    if let Err(e) = record_activity() {
                        let _ =
                            record_entry_backoff(dispatcher, &project.slug, entry_id, now, false);
                        if strict {
                            return Err(storage(e));
                        }
                        tracing::warn!(
                            project = %project.slug,
                            task = %pa.task_id,
                            session = %session,
                            error = %e,
                            "outbox could not record Owner answer activity yet; scheduled backoff retry"
                        );
                        return Ok(());
                    }
                }
                pa.board_done = true;
            }
            _ => {}
        }
    }

    // 5. Remove this entry once all 3 steps are done!
    if pa.dialog_closed && pa.delivered && pa.board_done {
        let mut leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = leads.clone();
        if let Some(record) = next.leads.get_mut(&project.slug) {
            record.pending_answers.retain(|p| p.id != entry_id);
            if let Some(req_id) = pa.request_id.as_deref() {
                if record
                    .question
                    .as_ref()
                    .is_some_and(|q| q.request_id == req_id)
                {
                    record.question = None;
                }
            } else if record
                .question
                .as_ref()
                .is_some_and(|q| q.task_id == pa.task_id)
            {
                record.question = None;
            }
            if record.state == LeadState::Waiting {
                record.state = LeadState::Working;
            }
            next.save(&dispatcher.leads_path).map_err(storage)?;
            *leads = next;
        }
    }

    Ok(())
}

/// OIN-81: detect the lead's open `ask_user` dialog(s), attribute them to a held
/// ticket and drive the timeout. The ticket stays `in_progress` with its claim;
/// at the timeout `approval_transition(.., Some(question))` blocks it WITH the
/// claim kept, the dialog gets `TIMEOUT_TEXT` and the record's question clears
/// (the lead is released from waiting). A question the owner already answered
/// (dialog gone) clears silently.
#[allow(
    clippy::too_many_lines,
    reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
)]
async fn question_step(
    dispatcher: &Dispatcher,
    project: &Project,
    now: i64,
) -> Result<(), crate::superagent::DispatchError> {
    use crate::superagent_questions::TickAction;
    let (session, pending, pending_answers) = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(record) = leads.leads.get(&project.slug) else {
            return Ok(());
        };
        (
            record.session_id.clone(),
            record.question.clone(),
            record.pending_answers.clone(),
        )
    };
    // While an outbox entry is still closing the dialog or delivering the steer for
    // this question, do not overwrite or prematurely clear `record.question`.
    // Bookkeeping-only entries (`needs_runtime() == false`) never block new questions (F2).
    if let Some(q) = pending.as_ref() {
        if pending_answers
            .iter()
            .any(|pa| pa.needs_runtime() && pa.request_id.as_deref() == Some(q.request_id.as_str()))
        {
            return Ok(());
        }
    }
    let questions = dispatcher.workers.questions(&session).await;
    let held: Vec<String> = held_tickets(dispatcher, project, &session)?
        .iter()
        .filter(|task| task.status == Status::InProgress)
        .map(|task| task.id.clone())
        .collect();
    let timeout_minutes = dispatcher.lead_settings().question_timeout_minutes;
    match crate::superagent_questions::tick_action(
        &questions,
        pending.as_ref(),
        now,
        timeout_minutes,
        &held,
    ) {
        TickAction::None => {
            // No open dialog: clear a stale question.
            let mut leads = dispatcher
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(record) = leads.leads.get_mut(&project.slug) {
                if record.question.take().is_some() {
                    leads.save(&dispatcher.leads_path).map_err(storage)?;
                }
            }
        }
        TickAction::Open(question) => {
            let task_id = crate::superagent_questions::attributed_task(&question.texts, &held)
                .or_else(|| {
                    pending
                        .as_ref()
                        .filter(|q| q.request_id == question.request_id && !q.task_id.is_empty())
                        .map(|q| q.task_id.clone())
                })
                .unwrap_or_default();
            let first_seen_at = pending
                .as_ref()
                .filter(|q| q.request_id == question.request_id)
                .map_or(now, |q| q.first_seen_at);
            let fresh = {
                let mut leads = dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let fresh = leads.leads.get_mut(&project.slug).is_some_and(|record| {
                    let fresh = record
                        .question
                        .as_ref()
                        .is_none_or(|q| q.request_id != question.request_id);
                    record.question = Some(crate::superagent_leads::PendingQuestion {
                        request_id: question.request_id.clone(),
                        task_id: task_id.clone(),
                        excerpt: crate::superagent_questions::excerpt(&question.texts),
                        first_seen_at,
                        timing_out: false,
                        board_blocked: false,
                        dialog_resolved: false,
                    });
                    if !task_id.is_empty() && record.state == LeadState::Working {
                        record.state = LeadState::Waiting;
                    }
                    fresh
                });
                leads.save(&dispatcher.leads_path).map_err(storage)?;
                fresh
            };
            if fresh && !task_id.is_empty() {
                // Exactly ONE notification per question (owner decision): the
                // tap's DialogOpen push is suppressed for leads, so this is the
                // single push; timeout conversion updates the card silently.
                dispatcher
                    .workers
                    .lead_notice(
                        &session,
                        &format!("Lead question \u{b7} {}", project.name),
                        &crate::superagent_questions::excerpt(&question.texts),
                    )
                    .await;
            }
        }
        TickAction::Timeout {
            request_id,
            task_id,
            minutes,
        } => {
            // Stage a durable `PendingAnswerKind::Timeout` outbox entry (alongside
            // `PendingQuestion` markers) and drive board/dialog/steer completion
            // through `flush_pending_answers` (Items 3, N5, D1, D2, F3).
            let same_pending = pending.as_ref().filter(|q| q.request_id == request_id);
            let question_text = same_pending
                .map(|q| q.excerpt.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    questions
                        .first()
                        .map(|q| crate::superagent_questions::excerpt(&q.texts))
                        .unwrap_or_default()
                });
            let first_seen_at = same_pending.map_or(now, |q| q.first_seen_at);
            let board_blocked = task_id.is_empty() || same_pending.is_some_and(|q| q.board_blocked);
            let dialog_resolved = same_pending.is_some_and(|q| q.dialog_resolved);
            let resolution = crate::superagent_questions::timeout_resolution(minutes, &task_id);
            let mut staged_id = None;
            {
                let mut leads = dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut next = leads.clone();
                if let Some(record) = next.leads.get_mut(&project.slug) {
                    record.question = Some(PendingQuestion {
                        request_id: request_id.clone(),
                        task_id: task_id.clone(),
                        excerpt: question_text.clone(),
                        first_seen_at,
                        timing_out: true,
                        board_blocked,
                        dialog_resolved,
                    });
                    if let Some(existing) = record.pending_answers.iter().find(|pa| {
                        pa.kind == PendingAnswerKind::Timeout
                            && pa.request_id.as_deref() == Some(request_id.as_str())
                    }) {
                        staged_id = Some(existing.id.clone());
                    } else {
                        let id = default_outbox_id();
                        staged_id = Some(id.clone());
                        record.pending_answers.push(PendingAnswer {
                            id,
                            kind: PendingAnswerKind::Timeout,
                            task_id: task_id.clone(),
                            request_id: Some(request_id.clone()),
                            dialog_answer: Some(json!({"answers": []})),
                            question_excerpt: Some(question_text.clone()),
                            text: question_text,
                            delivery_text: resolution.steer_message.unwrap_or_default(),
                            mode: PendingAnswerMode::Steer,
                            dialog_closed: dialog_resolved,
                            delivered: false,
                            board_done: board_blocked,
                            attempts: 0,
                            next_attempt_at: None,
                            runtime_failures: 0,
                            created_at: now,
                        });
                    }
                    next.save(&dispatcher.leads_path).map_err(storage)?;
                    *leads = next;
                }
            }
            if let Some(entry_id) = staged_id {
                flush_pending_answers(
                    dispatcher,
                    project,
                    now,
                    Some(OutboxFilter::EntryId(entry_id)),
                    false,
                    false,
                )
                .await?;
            }
            // Item 6: no second notification on timeout; the card updates silently.
        }
    }
    Ok(())
}

/// `status.leads[]` rows from the records (stalled and unloaded leads appear;
/// the OIN-80 fields the UI's `LeadSchema` parses).
/// # Errors
/// Returns an error for invalid lead state or failed persistence, board, and session operations.
pub async fn lead_status_rows<S: std::hash::BuildHasher>(
    dispatcher: &Dispatcher,
    live_session_ids: &std::collections::HashSet<String, S>,
) -> Result<Vec<Value>, crate::superagent::DispatchError> {
    let records: Vec<LeadRecord> = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        leads.leads.values().cloned().collect()
    };
    let mut rows = Vec::new();
    for record in &records {
        let live =
            record.state != LeadState::Unloaded && live_session_ids.contains(&record.session_id);
        let progress = if live {
            dispatcher.workers.progress(&record.session_id).await
        } else {
            json!({"step":null,"tool":null,"cost":null})
        };
        let project = dispatcher
            .layout
            .list_projects()
            .map_err(storage)?
            .into_iter()
            .find(|p| p.slug == record.project_id);
        let ticket_ids: Vec<String> = if live {
            project
                .as_ref()
                .and_then(|project| held_tickets(dispatcher, project, &record.session_id).ok())
                .map(|tasks| tasks.iter().map(|t| t.id.clone()).collect())
                .unwrap_or_default()
        } else {
            record.continuation.tickets.clone()
        };
        let mut row = json!({
            "projectId": record.project_id,
            "sessionId": record.session_id,
            "presetId": record.preset.id,
            "model": progress.get("model").filter(|v| !v.is_null()).cloned()
                .or_else(|| record.preset.model.as_ref().map(|m| json!(m))),
            "thinking": progress.get("thinking").filter(|v| !v.is_null()).cloned()
                .or_else(|| record.preset.thinking.as_ref().map(|t| json!(t))),
            "autonomy": progress.get("autonomy").filter(|v| !v.is_null()).cloned()
                .unwrap_or_else(|| json!(record.preset.autonomy)),
            "mode": record.preset.mode,
            "ticketIds": ticket_ids,
            "step": progress["step"],
            "tool": progress["tool"],
            "startedAt": chrono::DateTime::from_timestamp_millis(record.created_at)
                .map(|t| t.to_rfc3339()),
            "cost": progress["cost"],
            "progress": {"running": ticket_ids.len(), "queued": 0, "done": 0},
            "state": record.state.as_str(),
            "health": record.health.as_ref().map(|h| json!({
                "kind": h.kind.as_str(), "error": h.error, "attempts": h.attempts,
                "since": h.since, "nextProbeAt": h.next_probe_at,
            })),
            "idleReason": (record.state == LeadState::Unloaded).then_some("unloaded"),
            // Finding 10 (UI contract): epoch-ms NUMBERS for every state (the
            // zod schema is `z.number()` — an RFC3339 string parses to null and
            // hides the activity age). A live row's fresher card activity
            // overwrites this in the fleet merge.
            "lastActiveAt": json!(record.last_active_at),
            "question": record.question.as_ref()
                .filter(|q| !q.task_id.is_empty())
                .map(|q| json!({"taskId": q.task_id, "excerpt": q.excerpt})),
        });
        if let (true, Some(project)) = (live, project.as_ref()) {
            if let Some(paused_graph) = dispatcher.workers.agent_dir().and_then(|dir| {
                crate::superagent_workers::paused_graph_for_lead(
                    dir,
                    &project.root,
                    &record.session_id,
                )
            }) {
                row["pausedGraph"] = json!(paused_graph);
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

/// The `superagent.lead_action` RPC (keeps the shape the UI already uses).
/// # Errors
/// Returns an error for invalid lead state or failed persistence, board, and session operations.
#[allow(
    clippy::too_many_lines,
    reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
)]
pub async fn lead_action(
    dispatcher: &Dispatcher,
    session_id: &str,
    action: &str,
    model: Option<&str>,
) -> Result<Value, crate::superagent::DispatchError> {
    let _transaction = dispatcher.transaction.lock().await;
    let record = {
        let leads = dispatcher
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        leads
            .leads
            .values()
            .find(|r| r.session_id == session_id)
            .cloned()
    };
    let Some(mut record) = record else {
        return Err(fail(
            "E_LEAD_UNKNOWN",
            "No lead with this id; refresh the work pane",
        ));
    };
    let Some(project) = dispatcher
        .layout
        .list_projects()
        .map_err(storage)?
        .into_iter()
        .find(|p| p.slug == record.project_id)
    else {
        return Err(fail(
            "E_LEAD_UNKNOWN",
            "No lead with this id; refresh the work pane",
        ));
    };
    let now = now_ms();
    match action {
        "retry" => {
            if record.state != LeadState::Stalled {
                return Err(fail(
                    "E_LEAD_STATE",
                    format!("Lead is {}; nothing to retry", record.state.as_str()),
                ));
            }
            wake_for_continue(dispatcher, &project, &mut record).await?;
            let error = record
                .health
                .as_ref()
                .map(|h| h.error.clone())
                .unwrap_or_default();
            persist_state(dispatcher, &record, now)?;
            // F3: explicit Retry on the health card bypasses outbox retry backoff.
            let _ = flush_pending_answers(dispatcher, &project, now, None, false, true).await;
            let text = PROBE_TEXT.replace("{error}", &error);
            let _ = dispatcher
                .workers
                .continue_turn(&record.session_id, &text)
                .await;
            Ok(json!({"ok": true}))
        }
        "switch_model" => {
            let Some(model) = model.filter(|m: &&str| !m.trim().is_empty()) else {
                return Err(fail("E_SCHEMA", "model is required (provider/id)"));
            };
            let (provider, id) = model
                .split_once('/')
                .filter(|(p, i)| !p.is_empty() && !i.is_empty())
                .ok_or_else(|| fail("E_SCHEMA", "model is required (provider/id)"))?;
            let services = dispatcher
                .workers
                .core_services(&project)
                .await
                .map_err(storage)?;
            if services.models.find(provider, id).is_none() {
                return Err(fail("E_UNKNOWN_MODEL", format!("Unknown model: {model}")));
            }
            wake_for_continue(dispatcher, &project, &mut record).await?;
            // The switch must succeed BEFORE the record claims the new model
            // (finding 7: a swallowed error left the record lying); the
            // contractual continuation goes out after a successful switch.
            dispatcher
                .workers
                .set_model(session_id, model)
                .await
                .map_err(|e| fail("E_WORKER_SET_MODEL", e))?;
            {
                let mut leads = dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(stored) = leads.leads.get_mut(&record.project_id) {
                    // Owner decision (not a defect): persisting the switched model
                    // on the live LeadRecord even when `remember: false` is INTENDED
                    // so the SAME lead chat keeps its model across host restarts.
                    // `remember` in `set_lead_model` only governs the project preset
                    // used for future leads.
                    stored.preset.model = Some(model.to_owned());
                    stored.on_fallback = false;
                    stored.state = LeadState::Working;
                    if stored
                        .health
                        .as_ref()
                        .is_some_and(|h| h.kind == crate::superagent_leads::HealthKind::Model)
                    {
                        stored.health = None;
                    }
                    leads.save(&dispatcher.leads_path).map_err(storage)?;
                }
            }
            // The lead is live but its turn ended (stalled): continue it on the
            // new model. Delivery failure surfaces to the caller.
            dispatcher
                .workers
                .continue_turn(
                    session_id,
                    &format!("Your model was switched to {model}. Continue where you stopped."),
                )
                .await
                .map_err(|e| fail("E_RELAY_DELIVERY", e))?;
            Ok(json!({"ok": true}))
        }
        "stop" => {
            let project_id = record.project_id.clone();
            retire_lead(dispatcher, &project_id, "stopped").await?;
            Ok(json!({"ok": true}))
        }
        "wake" => {
            if !matches!(record.state, LeadState::Idle | LeadState::Unloaded) {
                return Err(fail(
                    "E_LEAD_STATE",
                    format!("Lead is {}; nothing to wake", record.state.as_str()),
                ));
            }
            if record.session_file.as_deref().is_some_and(|f| !f.exists()) {
                return Err(fail("E_WORKER_LAUNCH", "chat file missing"));
            }
            let tasks: Vec<Task> = held_tickets(dispatcher, &project, session_id)?;
            dispatcher.register_lead_session(session_id);
            dispatcher
                .workers
                .launch_lead(&project, &tasks, session_id, &record.preset)
                .await
                .map_err(|e| fail("E_WORKER_LAUNCH", e))?;
            let session_file = dispatcher.workers.session_file(session_id).await;
            {
                let mut leads = dispatcher
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(stored) = leads.leads.get_mut(&record.project_id) {
                    stored.state = LeadState::Working;
                    stored.last_active_at = now;
                    stored.session_file = session_file;
                    leads.save(&dispatcher.leads_path).map_err(storage)?;
                }
            }
            let in_progress: Vec<String> = tasks
                .iter()
                .filter(|t| t.status == Status::InProgress)
                .map(|t| t.id.clone())
                .collect();
            if !in_progress.is_empty() {
                let text = format!(
                    "You still hold in_progress tickets {}: work them and finish each (in_review with a summary) or block it with a question before ending your turn.",
                    in_progress.join(", ")
                );
                let _ = dispatcher
                    .workers
                    .relay_delivery(session_id, &text, Delivery::Queue)
                    .await;
            }
            Ok(json!({"ok": true}))
        }
        _ => Err(fail(
            "E_SCHEMA",
            "action must be retry, switch_model, stop or wake",
        )),
    }
}

/// Launch (attach to) the same session id and mark the record Working.
async fn wake_for_continue(
    dispatcher: &Dispatcher,
    project: &Project,
    record: &mut LeadRecord,
) -> Result<(), crate::superagent::DispatchError> {
    if record.session_file.as_deref().is_some_and(|f| !f.exists()) {
        return Err(fail("E_WORKER_LAUNCH", "chat file missing"));
    }
    dispatcher.register_lead_session(&record.session_id);
    dispatcher
        .workers
        .launch_lead(project, &[], &record.session_id, &record.preset)
        .await
        .map_err(|e| fail("E_WORKER_LAUNCH", e))?;
    record.state = LeadState::Working;
    record.session_file = dispatcher.workers.session_file(&record.session_id).await;
    Ok(())
}

fn persist_state(
    dispatcher: &Dispatcher,
    record: &LeadRecord,
    now: i64,
) -> Result<(), crate::superagent::DispatchError> {
    let mut leads = dispatcher
        .leads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(stored) = leads.leads.get_mut(&record.project_id) {
        stored.clone_from(record);
        stored.last_active_at = now;
        leads.save(&dispatcher.leads_path).map_err(storage)?;
    }
    Ok(())
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Fractional duration units intentionally truncate to saturating whole milliseconds."
)]
fn duration_millis(value: f64, unit: f64) -> i64 {
    (value * unit) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: LeadState) -> LeadRecord {
        LeadRecord {
            project_id: "p".into(),
            session_id: "s".into(),
            session_file: None,
            preset: Store::default()
                .resolve(None, Overrides::default())
                .unwrap(),
            state,
            created_at: 1_000,
            last_active_at: 1_000,
            health: None,
            continuation: Continuation::default(),
            question: None,
            pending_answers: Vec::new(),
            on_fallback: false,
            resume_sent_at: None,
            in_progress_tickets: Vec::new(),
        }
    }

    #[test]
    fn errored_turn_switches_to_fallback_then_stalls() {
        let now = 10_000;
        let settings = LeadSettings {
            retries: 3,
            fallback_model: Some("fb/fall".into()),
            ..Default::default()
        };
        let mut rec = record(LeadState::Working);
        let effects = step(
            &mut rec,
            TurnOutcome::Errored {
                message: "429".into(),
            },
            now,
            &settings,
            Some("fb/fall"),
        );
        assert!(rec.on_fallback);
        assert_eq!(rec.state, LeadState::Working);
        assert_eq!(effects.len(), 2);
        let effects = step(
            &mut rec,
            TurnOutcome::Errored {
                message: "429 again".into(),
            },
            now + 1,
            &settings,
            Some("fb/fall"),
        );
        assert_eq!(rec.state, LeadState::Stalled);
        assert_eq!(rec.health.as_ref().unwrap().attempts, 2);
        assert_eq!(rec.health.as_ref().unwrap().since, now);
        assert_eq!(effects.len(), 1); // one notification per episode
        assert_eq!(
            step(
                &mut rec,
                TurnOutcome::Errored {
                    message: "x".into()
                },
                now + 2,
                &settings,
                Some("fb/fall")
            )
            .len(),
            0
        );
    }

    #[test]
    fn no_fallback_setting_stalls_on_first_error() {
        let settings = LeadSettings::default();
        let mut rec = record(LeadState::Working);
        let effects = step(
            &mut rec,
            TurnOutcome::Errored {
                message: "quota exhausted".into(),
            },
            5_000,
            &settings,
            None,
        );
        assert_eq!(rec.state, LeadState::Stalled);
        assert!(!rec.on_fallback);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn fallback_equal_to_primary_stalls() {
        let settings = LeadSettings {
            fallback_model: Some("same/model".into()),
            ..Default::default()
        };
        let mut rec = record(LeadState::Working);
        rec.preset.model = Some("same/model".into());
        step(
            &mut rec,
            TurnOutcome::Errored {
                message: "e".into(),
            },
            1_000,
            &settings,
            Some("same/model"),
        );
        assert_eq!(rec.state, LeadState::Stalled);
        assert!(!rec.on_fallback);
    }

    #[test]
    fn done_with_held_tickets_nudges_twice_then_stalls() {
        let settings = LeadSettings::default();
        let mut rec = record(LeadState::Working);
        rec.continuation.tickets = vec!["T-1".into()];
        rec.in_progress_tickets = vec!["T-1".into()];
        let effects = step(
            &mut rec,
            TurnOutcome::Done {
                last: String::new(),
            },
            1_000,
            &settings,
            None,
        );
        assert_eq!(rec.state, LeadState::Working);
        assert_eq!(rec.continuation.total, 1);
        assert_eq!(effects.len(), 1);
        step(
            &mut rec,
            TurnOutcome::Done {
                last: String::new(),
            },
            2_000,
            &settings,
            None,
        );
        assert_eq!(rec.continuation.total, 2);
        let effects = step(
            &mut rec,
            TurnOutcome::Done {
                last: String::new(),
            },
            3_000,
            &settings,
            None,
        );
        assert_eq!(rec.state, LeadState::Stalled);
        assert_eq!(rec.health.as_ref().unwrap().kind, HealthKind::NoProgress);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn done_without_tickets_goes_idle_and_clears_health() {
        let settings = LeadSettings::default();
        let mut rec = record(LeadState::Working);
        rec.health = Some(Health {
            kind: HealthKind::Model,
            error: "e".into(),
            attempts: 1,
            since: 1,
            next_probe_at: None,
            notified: true,
        });
        step(
            &mut rec,
            TurnOutcome::Done {
                last: "done".into(),
            },
            1_000,
            &settings,
            None,
        );
        assert_eq!(rec.state, LeadState::Idle);
        assert!(rec.health.is_none());
    }

    #[test]
    fn reset_hint_parses_hours_minutes_and_retry_after() {
        // A contemporary epoch (2026-07-09T05:08:39Z): a relative duration is
        // ADDED to now (finding 4), then clamped to at least now + 60 s.
        let now = 1_794_391_219_566i64;
        assert_eq!(
            reset_hint_ms("quota exhausted, reset after 79h", now),
            Some(now + 79 * 3_600_000)
        );
        assert_eq!(
            reset_hint_ms("Quota resets in 30 min", now),
            Some(now + 30 * 60_000)
        );
        assert_eq!(reset_hint_ms("retry-after: 120", now), Some(now + 120_000));
        // Clamped to at least now + 60 s (30 s < 60 s).
        assert_eq!(reset_hint_ms("retry-after: 30", now), Some(now + 60_000));
        // Whitelisted units only; long forms and plurals parse.
        assert_eq!(
            reset_hint_ms("quota exhausted, reset after 2 hrs", now),
            Some(now + 2 * 3_600_000)
        );
        assert_eq!(
            reset_hint_ms("quota exhausted, resets in 90 seconds", now),
            Some(now + 90_000)
        );
        // "3 w" is not a unit → no hint; a LATER valid occurrence is found.
        assert_eq!(reset_hint_ms("reset after 3 w", now), None);
        assert_eq!(
            reset_hint_ms("reset later; quota resets in 45 mins", now),
            Some(now + 45 * 60_000)
        );
        assert_eq!(
            reset_hint_ms("retry-after header missing; retry-after: 300", now),
            Some(now + 300_000)
        );
        assert_eq!(
            reset_hint_ms("resets at 2026-11-11T12:00:00Z", now),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-11-11T12:00:00Z")
                    .unwrap()
                    .timestamp_millis()
            )
        );
        // Clamped to at least now + 60 s.
        assert_eq!(reset_hint_ms("resets in 1s", now), Some(now + 60_000));
        assert_eq!(reset_hint_ms("no hint here", now), None);
    }

    #[test]
    fn settings_sanitize_clamps_and_clears_empty_fallback() {
        let settings = LeadSettings {
            retries: 99,
            fallback_model: Some("  ".into()),
            warm_minutes: 1,
            question_timeout_minutes: 9_999,
        }
        .sanitized();
        assert_eq!(settings.retries, 10);
        assert_eq!(settings.fallback_model, None);
        assert_eq!(settings.warm_minutes, 5);
        assert_eq!(settings.question_timeout_minutes, 1440);
    }

    #[test]
    fn legacy_single_pending_answer_migrates_and_backoff_caps_at_thirty_minutes() {
        let legacy = serde_json::json!({
            "version": 1,
            "leads": {
                "proj": {
                    "projectId": "proj",
                    "sessionId": "sid-1",
                    "preset": Store::default().resolve(None, Overrides::default()).unwrap(),
                    "state": "working",
                    "createdAt": 1000,
                    "lastActiveAt": 1000,
                    "pendingAnswer": {
                        "kind": "blocked_answer",
                        "taskId": "PRJ-1",
                        "text": "yes",
                        "deliveryText": "Owner answer on PRJ-1: yes",
                        "mode": "relay",
                        "dialogClosed": true,
                        "delivered": true,
                        "boardDone": false,
                        "createdAt": 1000
                    }
                }
            },
            "retired": []
        });
        let store = LeadStore::from_json_bytes(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        let rec = &store.leads["proj"];
        assert_eq!(rec.pending_answers.len(), 1);
        assert_eq!(rec.pending_answers[0].task_id, "PRJ-1");
        assert_ne!(rec.pending_answers[0].id, "");
        assert!(!rec.pending_answers[0].needs_runtime());

        assert_eq!(outbox_backoff_ms(1), 60_000);
        assert_eq!(outbox_backoff_ms(2), 120_000);
        assert_eq!(outbox_backoff_ms(3), 240_000);
        assert_eq!(outbox_backoff_ms(4), 480_000);
        assert_eq!(outbox_backoff_ms(5), 960_000);
        assert_eq!(outbox_backoff_ms(6), 1_800_000);
        assert_eq!(outbox_backoff_ms(99), 1_800_000);
    }
}
