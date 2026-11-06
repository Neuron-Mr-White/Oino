//! `watchdog` settings (unipi `src/config.ts`) plus the Oino session caps (unipi-port §3.10,
//! D22: Oino `budget` moved here).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.watchdog";
/// Namespace.
pub const NAMESPACE: &str = "watchdog";

/// unipi `DEFAULT_WATCHDOG_SETTINGS` + `decisionModel` + Oino `sessionCaps`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "sessionCaps": {"maxTurns": 0, "maxCostUsd": 0, "maxWallMinutes": 0, "maxConsecutiveToolErrors": 0, "summaryTurn": true},
    })
}

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
            Section::new("session-caps", "Session caps")
                .description("Opt-in run caps (0 = off). On a cap the run is aborted; with a summary turn the agent is asked once for a wrap-up summary. CLI flags --max-turns, --max-cost, --max-wall, --max-tool-errors override these.")
                .field(number("sessionCaps.maxTurns", "Max turns", Some(0.0), None, None).description("Abort after this many turns in one run."))
                .field(number("sessionCaps.maxCostUsd", "Max cost", Some(0.0), None, None).description("Abort once the run has spent this many USD."))
                .field(number("sessionCaps.maxWallMinutes", "Max wall time", Some(0.0), None, Some(Unit::Min)).description("Abort once the run has taken this long."))
                .field(number("sessionCaps.maxConsecutiveToolErrors", "Max consecutive tool errors", Some(0.0), None, None).description("Abort after this many failed tool calls in a row."))
                .field(Field::boolean("sessionCaps.summaryTurn", "Summary turn").description("Ask the agent once for a summary of what is done and what remains.")),
        )
}
