//! Settings namespace `kanboard` (unipi `settings.ts`, hub section "Kanboard").

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, Values, choice};

/// Settings namespace.
pub const NAMESPACE: &str = "kanboard";
/// Hidden command behind the hub's "Open board…" action.
pub const ACTION_OPEN: &str = "unipi:kanboard-open";
/// Hidden command behind "Stop daemon…".
pub const ACTION_STOP_DAEMON: &str = "unipi:kanboard-stop-daemon";
/// Hidden command behind "Rotate access token…".
pub const ACTION_ROTATE_TOKEN: &str = "unipi:kanboard-rotate-token";

/// Which dependency status counts as "the chain reached this task".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainGate {
    /// `in_review` or `done`.
    InReview,
    /// `done` only.
    Done,
}

impl ChainGate {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InReview => "in_review",
            Self::Done => "done",
        }
    }
}

/// Effective kanboard settings (validated like unipi `readKanboardSettings`).
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi's settings object field for field"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KanboardSettings {
    /// Chain gate.
    pub chain_gate: ChainGate,
    /// `serve --idle-min`.
    pub idle_min: u64,
    /// `serve --host`.
    pub host: String,
    /// `serve --port` (0 = OS-assigned).
    pub port: u16,
    /// Auto-archive done/cancelled after N days (0 = off).
    pub archive_after_days: u64,
    /// Cold storage after N days (0 = off).
    pub retention_days: u64,
    /// Open the board on `open` (unipi: browser; Oino: the board panel always opens).
    pub open_browser: bool,
    /// Token-gate loopback binds too.
    pub require_auth: bool,
    /// Reuse `<home>/token` across restarts.
    pub keep_token: bool,
    /// Distinct sessions running tasks per project (min 1).
    pub max_sessions: u64,
    /// `add` calls per turn (0 = unlimited).
    pub turn_add_limit: u64,
    /// Task slots a `-do` grants.
    pub do_tasks: u64,
    /// Board writes a `-do` grants.
    pub do_writes: u64,
    /// Progress reminders.
    pub reminders: bool,
}

/// unipi `DEFAULT_SETTINGS`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "chainGate": "in_review",
        "idleMin": 10,
        "host": "127.0.0.1",
        "port": 0,
        "archiveAfterDays": 0,
        "retentionDays": 90,
        "openBrowser": false,
        "requireAuth": false,
        "keepToken": false,
        "maxSessions": 2,
        "turnAddLimit": 20,
        "doTasks": 5,
        "doWrites": 10,
        "reminders": true,
    })
}

fn num(raw: &Value, key: &str) -> Option<f64> {
    raw.get(key).and_then(Value::as_f64)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated non-negative settings numbers; fractions truncate like unipi's integer use"
)]
fn whole(v: f64) -> u64 {
    v as u64
}

impl KanboardSettings {
    /// Reads and validates `raw` (the merged namespace object). `stored` is the namespace as
    /// the user stored it (no defaults), used for the `doCredits` → `doWrites` migration.
    #[must_use]
    pub fn from_raw(raw: &Value, stored: Option<&Value>) -> Self {
        let get_bool = |key: &str| raw.get(key).and_then(Value::as_bool);
        let stored_do_writes =
            stored.is_some_and(|s| s.get("doWrites").is_some_and(Value::is_number));
        let legacy_credits = stored
            .and_then(|s| num(s, "doCredits"))
            .filter(|v| *v >= 0.0)
            .map(whole);
        let host = raw
            .get("host")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or("127.0.0.1")
            .to_owned();
        Self {
            chain_gate: if raw.get("chainGate").and_then(Value::as_str) == Some("done") {
                ChainGate::Done
            } else {
                ChainGate::InReview
            },
            idle_min: num(raw, "idleMin").filter(|v| *v >= 1.0).map_or(10, whole),
            host,
            port: num(raw, "port")
                .filter(|v| (0.0..=65535.0).contains(v))
                .map_or(0, |v| u16::try_from(whole(v)).unwrap_or(0)),
            archive_after_days: num(raw, "archiveAfterDays")
                .filter(|v| *v > 0.0)
                .map_or(0, whole),
            retention_days: num(raw, "retentionDays")
                .filter(|v| *v >= 0.0)
                .map_or(90, whole),
            open_browser: get_bool("openBrowser") == Some(true),
            require_auth: get_bool("requireAuth") == Some(true),
            keep_token: get_bool("keepToken") == Some(true),
            max_sessions: num(raw, "maxSessions")
                .filter(|v| *v >= 1.0)
                .map_or(2, whole),
            turn_add_limit: num(raw, "turnAddLimit")
                .filter(|v| *v >= 0.0)
                .map_or(20, whole),
            do_tasks: num(raw, "doTasks").filter(|v| *v >= 0.0).map_or(5, whole),
            do_writes: match num(raw, "doWrites").filter(|v| *v >= 0.0) {
                Some(v) if stored_do_writes => whole(v),
                _ => legacy_credits.unwrap_or(10),
            },
            reminders: get_bool("reminders") != Some(false),
        }
    }

    /// From the settings store values.
    #[must_use]
    pub fn from_values(values: &Values, stored: Option<&Value>) -> Self {
        Self::from_raw(&values.0, stored)
    }
}

/// A number field with unipi's `min`/`max`/`unit`/`zeroLabel`.
fn number(
    key: &str,
    label: &str,
    min: f64,
    max: Option<f64>,
    unit: Option<Unit>,
    zero: Option<&str>,
) -> Field {
    Field::number(key, label).with_kind(|k| {
        if let Kind::Number {
            min: lo,
            max: hi,
            unit: u,
            zero_label,
            ..
        } = k
        {
            *lo = Some(min);
            *hi = max;
            *u = unit;
            *zero_label = zero.map(ToOwned::to_owned);
        }
    })
}

/// The hub schema (unipi `registerKanboardSettings`).
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Kanboard", "unipi.kanboard")
        .defaults(defaults())
        .section(
            Section::new("agent", "Agent")
                .description("Task flow and budgets for sessions working the board")
                .field(
                    Field::enumeration(
                        "chainGate",
                        "Chain gate",
                        vec![
                            choice("in_review", "in review", "next task may start once this one is in review"),
                            choice("done", "done", "next task waits for a completed dependency"),
                        ],
                    )
                    .description("When a dependency counts as satisfied and the next task may start."),
                )
                .field(
                    number("maxSessions", "Max sessions", 1.0, None, None, None)
                        .description("Distinct sessions that may hold in_progress tasks, per project."),
                )
                .field(
                    number("turnAddLimit", "Adds per turn", 0.0, None, None, Some("unlimited"))
                        .description("`add` calls one turn may make."),
                )
                .field(
                    Field::boolean("reminders", "Progress reminders")
                        .description("Remind the agent to `start` a mentioned task before editing; text only, off in child sessions."),
                )
                .field(
                    number("doTasks", "-do task slots", 0.0, None, None, Some("off"))
                        .description("Task slots a /unipi:kanboard-do grants; each `start` uses one."),
                )
                .field(
                    number("doWrites", "-do write budget", 0.0, None, None, Some("off"))
                        .description("Board writes a /unipi:kanboard-do grants (add, edit, link, order, move, notes)."),
                ),
        )
        .section(
            Section::new("housekeeping", "Housekeeping")
                .description("What happens to finished and cancelled tasks")
                .field(
                    number("archiveAfterDays", "Archive after", 0.0, None, Some(Unit::Days), Some("off"))
                        .description("Auto-archive done/cancelled tasks on session start."),
                )
                .field(
                    number("retentionDays", "Cold storage after", 0.0, None, Some(Unit::Days), Some("off"))
                        .description("Move archived tasks to cold storage — readable files, gone from the board."),
                ),
        )
        .section(
            Section::new("server", "Board server")
                .description("Daemon binding, lifetime and access token")
                .advanced(true)
                .field(
                    Field::string("host", "Bind address")
                        .empty_label("127.0.0.1 (local only)")
                        .hint("127.0.0.1 · 0.0.0.0 (LAN, token-gated) · tailscale (tailnet IP)")
                        .description("Address the board server binds; anything but a loopback requires the access token."),
                )
                .field(
                    number("port", "Daemon port", 0.0, Some(65535.0), None, Some("auto"))
                        .description("Port the board server listens on; 0 lets the OS pick a free one."),
                )
                .field(
                    number("idleMin", "Daemon idle", 1.0, None, Some(Unit::Min), None)
                        .description("Shut the daemon down after this long with no board open."),
                )
                .field(
                    Field::boolean("openBrowser", "Open board panel")
                        .description("Open the board panel when the daemon starts from the hub action."),
                )
                .field(
                    Field::boolean("requireAuth", "Token on localhost")
                        .description("Require the access token on 127.0.0.1 too; remote binds always need it. Next daemon start."),
                )
                .field(
                    Field::boolean("keepToken", "Keep token")
                        .description("Reuse <home>/token across restarts so board links stay valid; rotate below."),
                )
                .field(Field::action("open", "Open board…", ACTION_OPEN).description("Start the daemon and open the board."))
                .field(
                    Field::action("stopDaemon", "Stop daemon…", ACTION_STOP_DAEMON)
                        .description("Shut the board server down now."),
                )
                .field(
                    Field::action("rotateToken", "Rotate access token…", ACTION_ROTATE_TOKEN)
                        .description("Issue a new token; old board links stop working."),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_like_unipi() {
        let s = KanboardSettings::from_raw(&defaults(), None);
        assert_eq!(s.chain_gate, ChainGate::InReview);
        assert_eq!(
            (s.idle_min, s.port, s.max_sessions, s.turn_add_limit),
            (10, 0, 2, 20)
        );
        assert_eq!(
            (
                s.do_tasks,
                s.do_writes,
                s.retention_days,
                s.archive_after_days
            ),
            (5, 10, 90, 0)
        );
        assert!(s.reminders && !s.open_browser && !s.require_auth && !s.keep_token);
        assert_eq!(s.host, "127.0.0.1");
        let bad = KanboardSettings::from_raw(
            &json!({"chainGate": "done", "idleMin": 0, "host": "  ", "port": 70000, "maxSessions": 0, "turnAddLimit": -1, "reminders": "no", "archiveAfterDays": -3}),
            None,
        );
        assert_eq!(bad.chain_gate, ChainGate::Done);
        assert_eq!(
            (bad.idle_min, bad.port, bad.max_sessions, bad.turn_add_limit),
            (10, 0, 2, 20)
        );
        assert_eq!(bad.host, "127.0.0.1");
        assert!(bad.reminders, "only an explicit false turns reminders off");
        assert!(!KanboardSettings::from_raw(&json!({"reminders": false}), None).reminders);
    }

    #[test]
    fn do_credits_migrate_into_do_writes() {
        let merged = json!({"doWrites": 10});
        let stored = json!({"doCredits": 3});
        assert_eq!(
            KanboardSettings::from_raw(&merged, Some(&stored)).do_writes,
            3
        );
        let stored = json!({"doCredits": 3, "doWrites": 7});
        assert_eq!(
            KanboardSettings::from_raw(&json!({"doWrites": 7}), Some(&stored)).do_writes,
            7
        );
        assert_eq!(
            KanboardSettings::from_raw(&merged, Some(&json!({}))).do_writes,
            10
        );
    }

    #[test]
    fn schema_has_every_unipi_field_and_action() {
        let ns = namespace();
        for key in [
            "chainGate",
            "maxSessions",
            "turnAddLimit",
            "reminders",
            "doTasks",
            "doWrites",
            "archiveAfterDays",
            "retentionDays",
            "host",
            "port",
            "idleMin",
            "openBrowser",
            "requireAuth",
            "keepToken",
            "open",
            "stopDaemon",
            "rotateToken",
        ] {
            assert!(ns.field(key).is_some(), "{key}");
        }
        let j = ns.to_json();
        assert_eq!(j["namespace"], "kanboard");
        assert_eq!(j["extensionId"], "unipi.kanboard");
    }
}
