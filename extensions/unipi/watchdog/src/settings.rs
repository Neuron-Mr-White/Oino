//! `watchdog` settings (unipi `src/config.ts`) plus the Oino session caps (unipi-port §3.10,
//! D22: Oino `budget` moved here).

use serde_json::{Value, json};
use unipi_kit::jev;
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, Values, choice};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.watchdog";
/// Namespace.
pub const NAMESPACE: &str = "watchdog";

/// unipi `WatchdogAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Hand a stuck bash call to a background task.
    Background,
    /// Kill it.
    Kill,
    /// Warn only.
    Warn,
}

/// unipi `OtherToolsMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherTools {
    /// Unwatched.
    Off,
    /// Warn.
    Warn,
    /// Abort the turn.
    AbortTurn,
}

/// unipi `WatchdogSettings`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi WatchdogSettings"
)]
pub struct Settings {
    /// Master switch.
    pub enabled: bool,
    /// Minutes between checks.
    pub interval_min: f64,
    /// Minutes until the first check.
    pub first_check_min: f64,
    /// Stop score / status confidence.
    pub confidence: f64,
    /// Agreeing checks.
    pub agree_checks: u32,
    /// Action.
    pub action: Action,
    /// Watch bash.
    pub watch_bash: bool,
    /// Watch background tasks.
    pub watch_bg_tasks: bool,
    /// Other tools.
    pub other_tools: OtherTools,
}

/// unipi `DEFAULT_WATCHDOG_SETTINGS` + `decisionModel` + Oino `sessionCaps`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "enabled": true,
        "intervalMin": 3,
        "firstCheckMin": 2,
        "confidence": 0.5,
        "agreeChecks": 2,
        "action": "background",
        "watchBash": true,
        "watchBgTasks": false,
        "otherTools": "off",
        "debug": false,
        "decisionModel": jev::consumer_defaults(),
        "sessionCaps": {"maxTurns": 0, "maxCostUsd": 0, "maxWallMinutes": 0, "maxConsecutiveToolErrors": 0, "summaryTurn": true},
    })
}

fn num(v: &Values, key: &str, fallback: f64, min: f64, max: Option<f64>) -> f64 {
    match v.get(key).and_then(Value::as_f64) {
        Some(n) if n.is_finite() && n >= min && max.is_none_or(|m| n <= m) => n,
        _ => fallback,
    }
}

impl Settings {
    /// unipi `loadWatchdogSettings`: invalid values fall back to defaults.
    #[must_use]
    pub fn load(v: &Values) -> Self {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked ≥ 1"
        )]
        let agree = num(v, "agreeChecks", 2.0, 1.0, None) as u32;
        Self {
            enabled: v.bool("enabled", true),
            interval_min: num(v, "intervalMin", 3.0, 0.1, None),
            first_check_min: num(v, "firstCheckMin", 2.0, 0.0, None),
            confidence: num(v, "confidence", 0.5, 0.0, Some(1.0)),
            agree_checks: agree,
            action: match v.str("action", "") {
                "warn" => Action::Warn,
                "kill" => Action::Kill,
                _ => Action::Background,
            },
            watch_bash: v.bool("watchBash", true),
            watch_bg_tasks: v.bool("watchBgTasks", false),
            other_tools: match v.str("otherTools", "") {
                "warn" => OtherTools::Warn,
                "abort-turn" => OtherTools::AbortTurn,
                _ => OtherTools::Off,
            },
        }
    }

    /// unipi `startTimer` period: `min(15 s, firstCheckMin, intervalMin)`, at least 1 s.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive millis"
    )]
    pub fn heartbeat_ms(&self) -> u64 {
        let period = 15_000.0_f64
            .min(self.first_check_min * 60_000.0)
            .min(self.interval_min * 60_000.0);
        (period.max(1_000.0)) as u64
    }
}

/// unipi `HEARTBEAT_MS`.
pub const HEARTBEAT_MS: u64 = 15_000;

fn number(key: &str, label: &str, min: Option<f64>, max: Option<f64>, unit: Option<Unit>) -> Field {
    Field::number(key, label).with_kind(|k| {
        if let Kind::Number {
            min: m,
            max: x,
            unit: u,
            ..
        } = k
        {
            *m = min;
            *x = max;
            *u = unit;
        }
    })
}

/// Namespace schema (unipi `registerWatchdogSettings` + Oino session caps).
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Watchdog", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("watchdog", "Watchdog")
                .description("On by default for bash calls; hands stuck-looking calls to background tasks")
                .field(Field::boolean("enabled", "Enabled").description("On by default for bash calls; off runs no timers."))
                .field(number("intervalMin", "Check interval", Some(0.1), None, Some(Unit::Min)).description("Every watched item is judged once per interval."))
                .field(number("firstCheckMin", "First check after", Some(0.0), None, Some(Unit::Min)).description("Delay before the first check; 0 checks right away."))
                .field(number("confidence", "Stop score", Some(0.0), Some(1.0), None).description("Minimum jev score that a bash call will not finish on its own (bash calls); background tasks keep using it as the status confidence."))
                .field(number("agreeChecks", "Agreeing checks", Some(1.0), None, None).description("Consecutive agreeing checks before the action fires."))
                .field(
                    Field::enumeration(
                        "action",
                        "Action",
                        vec![
                            choice("background", "background", "hand a stuck bash call to a background task"),
                            choice("kill", "kill", "kill the stuck item"),
                            choice("warn", "warn", "notify you and leave it running"),
                        ],
                    )
                    .description("What happens when the watchdog is confident something is stuck."),
                )
                .field(Field::boolean("watchBash", "Watch bash").description("Watch bash tool calls; on by default."))
                .field(Field::boolean("watchBgTasks", "Watch background tasks").description("Off by default. Servers (no completion triggers) are never checked."))
                .field(
                    Field::enumeration(
                        "otherTools",
                        "Other tools",
                        vec![
                            choice("off", "off", "leave other tools unwatched"),
                            choice("warn", "warn", "notify and inject a message"),
                            choice("abort-turn", "abort turn", "abort the running turn"),
                        ],
                    )
                    .description("Off by default; what happens to tools without a kill handle when they look stuck."),
                )
                .field(Field::boolean("debug", "Debug log").advanced().description("Log every heartbeat and decision to the extension log.")),
        )
        .section(jev::consumer_section(Some("Watchdog — Decision model"), false))
        .section(
            Section::new("session-caps", "Session caps")
                .description("Opt-in run caps (0 = off). On a cap the run is aborted; with a summary turn the agent is asked once for a wrap-up summary. CLI flags --max-turns, --max-cost, --max-wall, --max-tool-errors override these.")
                .field(number("sessionCaps.maxTurns", "Max turns", Some(0.0), None, None).description("Abort after this many turns in one run."))
                .field(number("sessionCaps.maxCostUsd", "Max cost", Some(0.0), None, None).description("Abort once the run has spent this many USD."))
                .field(number("sessionCaps.maxWallMinutes", "Max wall time", Some(0.0), None, Some(Unit::Min)).description("Abort once the run has taken this long."))
                .field(number("sessionCaps.maxConsecutiveToolErrors", "Max consecutive tool errors", Some(0.0), None, None).description("Abort after this many failed tool calls in a row."))
                .field(Field::boolean("sessionCaps.summaryTurn", "Summary turn").description("Ask the agent once for a summary of what is done and what remains.")),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_with_fallbacks() {
        let s = Settings::load(&Values(defaults()));
        assert!(s.enabled && s.watch_bash && !s.watch_bg_tasks);
        assert_eq!(
            (s.action, s.other_tools, s.agree_checks),
            (Action::Background, OtherTools::Off, 2)
        );
        assert_eq!(s.heartbeat_ms(), 15_000);
        let bad = Settings::load(&Values(
            json!({"intervalMin": 0.01, "confidence": 2, "action": "x", "otherTools": "abort-turn", "firstCheckMin": 0}),
        ));
        assert!((bad.interval_min - 3.0).abs() < 1e-9);
        assert!((bad.confidence - 0.5).abs() < 1e-9);
        assert_eq!(
            (bad.action, bad.other_tools),
            (Action::Background, OtherTools::AbortTurn)
        );
        assert_eq!(bad.heartbeat_ms(), 1_000);
        let v = namespace().to_json();
        assert_eq!(v["sections"].as_array().map(Vec::len), Some(3));
        assert_eq!(v["sections"][0]["fields"][1]["unit"], "min");
    }
}
