//! `background-tasks` settings namespace (unipi `config.ts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, Values};

use crate::snapshot::EXTENSION_ID;

/// Namespace.
pub const NAMESPACE: &str = "background-tasks";
/// unipi `maxOutputBytes` default (20 MiB).
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 20 * 1024 * 1024;

/// unipi `DEFAULT_CONFIG`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "enabled": true,
        "notifyOnCompletion": true,
        "triggerOnCompletion": true,
        "maxFinishedTasks": 30,
        "maxOutputBytes": DEFAULT_MAX_OUTPUT_BYTES,
    })
}

fn min_unit(min: f64, unit: Option<Unit>) -> impl FnOnce(&mut Kind) {
    move |k| {
        if let Kind::Number {
            min: m, unit: u, ..
        } = k
        {
            *m = Some(min);
            *u = unit;
        }
    }
}

/// Schema (unipi `registerSettings({namespace: "background-tasks"})`).
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Background Tasks", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("tasks", "Tasks")
                .field(
                    Field::boolean("enabled", "Enabled").description(
                        "Master toggle — off registers no tools, commands, hooks or UI.",
                    ),
                )
                .field(
                    Field::boolean("notifyOnCompletion", "Notify on completion").description(
                        "Default for tool-launched tasks; bg_run can override per call.",
                    ),
                )
                .field(
                    Field::boolean("triggerOnCompletion", "Follow-up wake").description(
                        "A finished task's terminal state wakes the agent for a follow-up turn.",
                    ),
                )
                .field(
                    Field::number("maxFinishedTasks", "Max finished kept")
                        .with_kind(min_unit(1.0, None))
                        .description("Finished tasks kept in memory before the oldest is dropped."),
                )
                .field(
                    Field::number("maxOutputBytes", "Output cap")
                        .with_kind(min_unit(1024.0, Some(Unit::Bytes)))
                        .description("Kill and fail a task that outputs more than this."),
                ),
        )
}

/// Sanitised config (unipi `loadBackgroundTasksConfig`: invalid values fall back to defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Master toggle.
    pub enabled: bool,
    /// Default notify.
    pub notify_on_completion: bool,
    /// Default wake.
    pub trigger_on_completion: bool,
    /// Finished tasks shown.
    pub max_finished_tasks: usize,
    /// Output cap.
    pub max_output_bytes: u64,
}

impl Config {
    /// From resolved namespace values.
    #[must_use]
    pub fn from_values(v: &Values) -> Self {
        let int_at_least = |key: &str, min: u64, default: u64| {
            v.get(key)
                .and_then(Value::as_u64)
                .filter(|n| *n >= min)
                .unwrap_or(default)
        };
        Self {
            enabled: v.bool("enabled", true),
            notify_on_completion: v.bool("notifyOnCompletion", true),
            trigger_on_completion: v.bool("triggerOnCompletion", true),
            max_finished_tasks: usize::try_from(int_at_least("maxFinishedTasks", 1, 30))
                .unwrap_or(30),
            max_output_bytes: int_at_least("maxOutputBytes", 1024, DEFAULT_MAX_OUTPUT_BYTES),
        }
    }
}

/// unipi `validateBackgroundTasksConfig`.
#[must_use]
pub fn validate(config: &Value) -> Vec<String> {
    let Some(c) = config.as_object() else {
        return vec!["config must be an object".into()];
    };
    let mut problems = Vec::new();
    for key in ["enabled", "notifyOnCompletion", "triggerOnCompletion"] {
        if c.get(key).is_some_and(|v| !v.is_boolean()) {
            problems.push(format!("{key} must be a boolean"));
        }
    }
    if c.get("maxFinishedTasks")
        .is_some_and(|v| v.as_u64().is_none_or(|n| n < 1))
    {
        problems.push("maxFinishedTasks must be a positive integer".into());
    }
    if c.get("maxOutputBytes")
        .is_some_and(|v| v.as_u64().is_none_or(|n| n < 1024))
    {
        problems.push("maxOutputBytes must be an integer >= 1024".into());
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_defaults_and_validation() {
        let v = namespace().to_json();
        assert_eq!(v["namespace"], NAMESPACE);
        let f = &v["sections"][0]["fields"];
        assert_eq!(f.as_array().map(Vec::len), Some(5));
        assert_eq!(f[4]["unit"], "bytes");
        assert_eq!(f[4]["min"], 1024.0);
        assert_eq!(f[4]["default"], DEFAULT_MAX_OUTPUT_BYTES);
        assert_eq!(validate(&defaults()), Vec::<String>::new());
        assert_eq!(
            validate(&json!({"enabled": "yes", "maxFinishedTasks": 0, "maxOutputBytes": 10})),
            [
                "enabled must be a boolean",
                "maxFinishedTasks must be a positive integer",
                "maxOutputBytes must be an integer >= 1024"
            ]
        );
        assert_eq!(validate(&json!(1)), ["config must be an object"]);
        let c = Config::from_values(&Values(
            json!({"maxOutputBytes": 10, "maxFinishedTasks": 5}),
        ));
        assert_eq!(
            (c.max_output_bytes, c.max_finished_tasks),
            (DEFAULT_MAX_OUTPUT_BYTES, 5)
        );
    }
}
