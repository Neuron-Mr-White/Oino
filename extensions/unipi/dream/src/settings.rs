//! `dream` settings (unipi `src/settings.ts`, v3 J2/J3: schedule, model picker, staleness).
//!
//! | key | default | meaning |
//! |---|---|---|
//! | `enabled` | `false` | scheduled dreaming on (`/dream run` works either way) |
//! | `schedule.mode` | `daily` | `daily` (at `schedule.time`) · `interval` (every `schedule.intervalHours`) · `manual` |
//! | `schedule.time` | `03:00` | local `HH:MM` for `daily` (also the minute for `interval`) |
//! | `schedule.intervalHours` | `24` | hours between dreams for `interval` |
//! | `minSessions` | `1` | new sessions required since the last dream (`0` = always dream) |
//! | `minGapHours` | `6` | minimum hours between two dreams |
//! | `staleDays` | `30` | a memory unused this long is a prune candidate (`0` = never prune) |
//! | `model` | `""` | dream agent model (model picker; `""` = session default) |
//! | `thinking` | `medium` | dream agent thinking level |
//! | `skillsTarget` | `.agents/skills` | where approved skills land (project-relative) |
//! | `maxRuntimeMin` | `40` | hard cap for one run |

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, choice};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.dream";
/// Namespace.
pub const NAMESPACE: &str = "dream";

/// When dreams run on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Once a day at `time`.
    Daily,
    /// Every `interval_hours`.
    Interval,
    /// Only `/dream run` / "Dream now".
    Manual,
}

impl Mode {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Interval => "interval",
            Self::Manual => "manual",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "daily" => Some(Self::Daily),
            "interval" => Some(Self::Interval),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

/// `schedule.*`.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    /// Mode.
    pub mode: Mode,
    /// Local `(hour, minute)`.
    pub time: (u32, u32),
    /// Hours between interval dreams (`1..=168`).
    pub interval_hours: f64,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            mode: Mode::Daily,
            time: (3, 0),
            interval_hours: 24.0,
        }
    }
}

impl Schedule {
    /// `HH:MM`.
    #[must_use]
    pub fn time_str(&self) -> String {
        format!("{:02}:{:02}", self.time.0, self.time.1)
    }
}

/// `HH:MM` (also `H:MM`) → `(hour, minute)`.
#[must_use]
pub fn parse_time(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// Resolved settings.
#[derive(Clone, Debug, PartialEq)]
pub struct DreamSettings {
    /// Scheduled dreaming on.
    pub enabled: bool,
    /// When.
    pub schedule: Schedule,
    /// New sessions required (`0` = no gate).
    pub min_sessions: f64,
    /// Minimum hours between dreams.
    pub min_gap_hours: f64,
    /// Prune window in days (`0` = never).
    pub stale_days: f64,
    /// `provider/model` for the dream agent (`""` = session default).
    pub model: String,
    /// Thinking level.
    pub thinking: String,
    /// Where approved skills land (project-relative).
    pub skills_target: String,
    /// Hard cap for one run (minutes).
    pub max_runtime_min: f64,
}

impl Default for DreamSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            schedule: Schedule::default(),
            min_sessions: 1.0,
            min_gap_hours: 6.0,
            stale_days: 30.0,
            model: String::new(),
            thinking: "medium".into(),
            skills_target: ".agents/skills".into(),
            max_runtime_min: 40.0,
        }
    }
}

/// Defaults JSON.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "enabled": false,
        "schedule": {"mode": "daily", "time": "03:00", "intervalHours": 24},
        "minSessions": 1,
        "minGapHours": 6,
        "staleDays": 30,
        "model": "",
        "thinking": "medium",
        "skillsTarget": ".agents/skills",
        "maxRuntimeMin": 40
    })
}

/// unipi `normalizeDream` + the v3 keys.
#[must_use]
pub fn normalize(raw: &Value) -> DreamSettings {
    let d = DreamSettings::default();
    let num_at = |v: Option<&Value>, dv: f64| {
        v.and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(dv)
    };
    let num = |k: &str, dv: f64| num_at(raw.get(k), dv);
    let s = |k: &str, dv: &str| {
        raw.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(dv)
            .to_owned()
    };
    let sched = raw.get("schedule");
    let schedule = Schedule {
        mode: sched
            .and_then(|s| s.get("mode"))
            .and_then(Value::as_str)
            .and_then(Mode::parse)
            .unwrap_or(d.schedule.mode),
        time: sched
            .and_then(|s| s.get("time"))
            .and_then(Value::as_str)
            .and_then(parse_time)
            .unwrap_or(d.schedule.time),
        interval_hours: num_at(
            sched.and_then(|s| s.get("intervalHours")),
            d.schedule.interval_hours,
        )
        .clamp(1.0, 168.0),
    };
    DreamSettings {
        enabled: raw
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(d.enabled),
        schedule,
        min_sessions: num("minSessions", d.min_sessions),
        min_gap_hours: num("minGapHours", d.min_gap_hours),
        stale_days: num("staleDays", d.stale_days),
        model: raw
            .get("model")
            .and_then(Value::as_str)
            .map_or_else(String::new, |m| m.trim().to_owned()),
        thinking: s("thinking", &d.thinking),
        skills_target: s("skillsTarget", &d.skills_target),
        max_runtime_min: num("maxRuntimeMin", d.max_runtime_min).max(5.0),
    }
}

fn number(
    min_v: f64,
    max_v: Option<f64>,
    unit: Option<Unit>,
    zero: Option<&str>,
) -> impl FnOnce(&mut Kind) {
    move |k| {
        if let Kind::Number {
            min,
            max,
            unit: u,
            zero_label,
            ..
        } = k
        {
            *min = Some(min_v);
            *max = max_v;
            *u = unit;
            *zero_label = zero.map(ToOwned::to_owned);
        }
    }
}

/// The hub schema (every field renders in the generic settings form).
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Dream", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("schedule", "Schedule")
                .description("A daily background pass over your recent sessions and memories: merges overlapping memories, drops transient details, adds lessons the sessions missed, prunes memories nothing used, and proposes skills or checks for your approval")
                .field(Field::boolean("enabled", "Dream on a schedule").description("Off by default. On: the Oino host runs a dream at the scheduled time, even when no chat is open. \"Dream now\" and /dream run work either way."))
                .field(
                    Field::enumeration(
                        "schedule.mode",
                        "Frequency",
                        vec![
                            choice("daily", "Daily", "Once a day at the chosen time."),
                            choice("interval", "Every N hours", "Repeats every N hours."),
                            choice("manual", "Manual only", "Only when you start one."),
                        ],
                    )
                    .description("How often a scheduled dream runs."),
                )
                .field(
                    Field::string("schedule.time", "Time of day")
                        .description("Local time (HH:MM, 24-hour) for the daily dream; also the minute an interval dream starts on.")
                        .with_kind(|k| {
                            if let Kind::String { pattern, .. } = k {
                                *pattern = Some("^([01]?[0-9]|2[0-3]):[0-5][0-9]$".into());
                            }
                        }),
                )
                .field(
                    Field::number("schedule.intervalHours", "Every (hours)")
                        .description("Hours between dreams when the frequency is \"Every N hours\".")
                        .with_kind(number(1.0, Some(168.0), None, None)),
                )
                .field(
                    Field::number("minSessions", "Min new sessions")
                        .description("Skip a scheduled dream unless this many sessions ran since the last one (0 = always dream).")
                        .with_kind(number(0.0, None, None, Some("always dream"))),
                )
                .field(
                    Field::number("minGapHours", "Min gap (hours)")
                        .description("Skip a scheduled dream when the last one started less than this many hours ago.")
                        .with_kind(number(0.0, None, None, Some("no gap"))),
                ),
        )
        .section(
            Section::new("agent", "Dream agent")
                .description("The background agent that does the dreaming")
                .field(
                    Field::model("model", "Dream model")
                        .with_kind(|k| {
                            if let Kind::Model {
                                filter,
                                empty_option,
                                ..
                            } = k
                            {
                                *filter = Some(json!({"capability": "text"}));
                                *empty_option = Some("session default".into());
                            }
                        })
                        .empty_label("session default")
                        .description("Model for the dream agent; empty uses the default model."),
                )
                .field(
                    Field::enumeration(
                        "thinking",
                        "Thinking level",
                        vec![
                            choice("off", "off", "No thinking."),
                            choice("low", "low", "Brief thinking."),
                            choice("medium", "medium", "Default balance."),
                            choice("high", "high", "Deeper reasoning."),
                        ],
                    )
                    .description("The dream agent's thinking level."),
                )
                .field(
                    Field::number("maxRuntimeMin", "Max runtime")
                        .description("Hard cap for one dream run.")
                        .with_kind(number(5.0, None, Some(Unit::Min), None)),
                ),
        )
        .section(
            Section::new("upkeep", "Memory upkeep")
                .description("What the dream may remove, and where approved proposals land")
                .field(
                    Field::number("staleDays", "Prune unused after")
                        .description("A memory no session searched, read or was shown for this long becomes a prune candidate. Preferences are never pruned (0 = never prune).")
                        .with_kind(number(0.0, None, Some(Unit::Days), Some("never prune"))),
                )
                .field(Field::string("skillsTarget", "Skills target dir").description("Where approved skills are copied (project-relative).")),
        )
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn normalizing() {
        assert_eq!(normalize(&Value::Null), DreamSettings::default());
        assert_eq!(normalize(&defaults()), DreamSettings::default());
        let s = normalize(
            &json!({"enabled": true, "minSessions": 0, "maxRuntimeMin": 1, "thinking": " ", "model": " a/b ",
                    "schedule": {"mode": "interval", "time": "7:05", "intervalHours": 500}, "staleDays": 0}),
        );
        assert!(s.enabled);
        assert_eq!(s.min_sessions, 0.0);
        assert_eq!(s.max_runtime_min, 5.0);
        assert_eq!(s.thinking, "medium");
        assert_eq!(s.model, "a/b");
        assert_eq!(s.schedule.mode, Mode::Interval);
        assert_eq!(s.schedule.time, (7, 5));
        assert_eq!(s.schedule.time_str(), "07:05");
        assert_eq!(s.schedule.interval_hours, 168.0);
        assert_eq!(s.stale_days, 0.0);
        let bad = normalize(&json!({"schedule": {"mode": "weekly", "time": "25:00"}}));
        assert_eq!(bad.schedule, Schedule::default());
        assert_eq!(parse_time("23:59"), Some((23, 59)));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("x"), None);
    }

    #[test]
    fn schema_has_every_key_and_a_model_picker() {
        let ns = namespace().to_json();
        assert_eq!(ns["namespace"], "dream");
        let keys: Vec<String> = namespace()
            .all_fields()
            .iter()
            .map(|f| f.key.clone())
            .collect();
        for k in [
            "enabled",
            "schedule.mode",
            "schedule.time",
            "schedule.intervalHours",
            "minSessions",
            "minGapHours",
            "model",
            "thinking",
            "maxRuntimeMin",
            "staleDays",
            "skillsTarget",
        ] {
            assert!(keys.iter().any(|x| x == k), "missing {k}: {keys:?}");
        }
        let model = namespace().field("model").cloned().unwrap();
        assert!(matches!(model.kind, Kind::Model { .. }), "{model:?}");
        // Every default the form reads exists in the defaults object.
        for f in namespace().all_fields() {
            let mut v = &defaults();
            for part in f.key.split('.') {
                v = &v[part];
            }
            assert!(!v.is_null(), "no default for {}", f.key);
        }
    }
}
