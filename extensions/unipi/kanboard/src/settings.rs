//! Settings namespace `kanboard` (unipi `settings.ts`, hub section "Kanboard").

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, Values, choice};

/// Settings namespace.
pub const NAMESPACE: &str = "kanboard";
/// Hidden command behind the hub's "Open board…" action.
pub const ACTION_OPEN: &str = "kanboard-open";

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
    /// Auto-archive done/cancelled after N days (0 = off).
    pub archive_after_days: u64,
    /// Cold storage after N days (0 = off).
    pub retention_days: u64,
    /// Progress reminders.
    pub reminders: bool,
}

/// unipi `DEFAULT_SETTINGS`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "chainGate": "in_review",
        "archiveAfterDays": 0,
        "retentionDays": 90,
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
    /// Reads and validates `raw` (the merged namespace object).
    #[must_use]
    pub fn from_raw(raw: &Value) -> Self {
        Self {
            chain_gate: if raw.get("chainGate").and_then(Value::as_str) == Some("done") {
                ChainGate::Done
            } else {
                ChainGate::InReview
            },
            archive_after_days: num(raw, "archiveAfterDays")
                .filter(|v| *v > 0.0)
                .map_or(0, whole),
            retention_days: num(raw, "retentionDays")
                .filter(|v| *v >= 0.0)
                .map_or(90, whole),
            reminders: raw.get("reminders").and_then(Value::as_bool) != Some(false),
        }
    }

    /// From the settings store values.
    #[must_use]
    pub fn from_values(values: &Values) -> Self {
        Self::from_raw(&values.0)
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
                .description("Task flow for sessions working the board")
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
                    Field::boolean("reminders", "Progress reminders")
                        .description("Remind the agent to `start` a mentioned task before editing; text only, off in child sessions."),
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
            Section::new("board", "Board")
                .field(Field::action("open", "Open board…", ACTION_OPEN)
                    .description("Open the board tab — it renders in Oino itself (no daemon, no browser).")),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_like_unipi() {
        let s = KanboardSettings::from_raw(&defaults());
        assert_eq!(s.chain_gate, ChainGate::InReview);
        assert_eq!((s.retention_days, s.archive_after_days), (90, 0));
        assert!(s.reminders);
        let bad = KanboardSettings::from_raw(
            &json!({"chainGate": "done", "reminders": "no", "archiveAfterDays": -3}),
        );
        assert_eq!(bad.chain_gate, ChainGate::Done);
        assert_eq!((bad.retention_days, bad.archive_after_days), (90, 0));
        assert!(bad.reminders, "only an explicit false turns reminders off");
        assert!(!KanboardSettings::from_raw(&json!({"reminders": false})).reminders);
    }

    #[test]
    fn removed_budget_and_limit_settings_are_ignored() {
        // An old settings.json may still carry them; they parse away without an error and
        // leave every live setting at its default.
        let s = KanboardSettings::from_raw(&json!({
            "doTasks": 7,
            "doWrites": 3,
            "maxSessions": 5,
            "turnAddLimit": 10,
        }));
        assert_eq!((s.retention_days, s.archive_after_days), (90, 0));
        assert!(s.reminders);
        // They are gone from the defaults and the schema too.
        assert_eq!(defaults().get("maxSessions"), None);
        assert_eq!(defaults().get("turnAddLimit"), None);
    }

    #[test]
    fn schema_has_every_unipi_field_and_action() {
        let ns = namespace();
        for key in [
            "chainGate",
            "reminders",
            "archiveAfterDays",
            "retentionDays",
            "open",
        ] {
            assert!(ns.field(key).is_some(), "{key}");
        }
        for gone in ["doTasks", "doWrites", "maxSessions", "turnAddLimit"] {
            assert!(ns.field(gone).is_none(), "{gone} is no longer a setting");
        }
        let j = ns.to_json();
        assert_eq!(j["namespace"], "kanboard");
        assert_eq!(j["extensionId"], "unipi.kanboard");
    }
}
