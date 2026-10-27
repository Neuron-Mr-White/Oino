//! Settings namespace `cron` (hub section "Cron"): a mirror of the scheduler's own
//! settings block, pushed through to it on every write (unipi's `settings.ts` pattern).
//! `~/.oino/cron/settings.json` stays the source of truth; `oino cron settings` and the
//! desktop Automations page read it directly.

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Unit, Values};

/// Settings namespace.
pub const NAMESPACE: &str = "cron";

/// Scheduler defaults (must mirror `oino_cron::model::CronSettings::default`).
#[must_use]
pub fn defaults() -> Value {
    json!({
        "concurrency": 2,
        "missedWindowS": 21600,
        "retentionRuns": 50,
        "retentionDays": 30,
        "defaultTimeLimitS": 1800,
        "paused": false,
        "notifyDefaultEvents": [],
    })
}

/// Builds the hub schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Cron", "unipi.cron")
        .defaults(defaults())
        .section(
            Section::new("scheduler", "Scheduled runs").fields(vec![
                Field::number("concurrency", "Concurrency")
                    .description("How many runs execute at once."),
                Field::number("missedWindowS", "Missed window")
                    .with_kind(|k| {
                        if let Kind::Number { unit, .. } = k {
                            *unit = Some(Unit::S);
                        }
                    })
                    .description(
                        "Seconds after its scheduled time within which a missed fire still runs late.",
                    ),
                Field::number("retentionRuns", "Keep runs")
                    .description("How many past runs to keep per job."),
                Field::number("retentionDays", "Keep days")
                    .with_kind(|k| {
                        if let Kind::Number { unit, .. } = k {
                            *unit = Some(Unit::Days);
                        }
                    })
                    .description("How many days of past runs to keep."),
                Field::number("defaultTimeLimitS", "Default time limit")
                    .with_kind(|k| {
                        if let Kind::Number { unit, .. } = k {
                            *unit = Some(Unit::S);
                        }
                    })
                    .description("Seconds a run gets when its job sets no own limit."),
                Field::boolean("paused", "Paused")
                    .description("Pause every job; running ones finish."),
            ]),
        )
        .section(Section::new("notify", "Notifications").fields(vec![Field::multiselect(
            "notifyDefaultEvents",
            "Notify on",
            vec![
                unipi_kit::settings::choice("done", "Done", "a run finishes successfully"),
                unipi_kit::settings::choice("failed", "Failed", "a run fails or times out"),
                unipi_kit::settings::choice("skipped", "Skipped", "a fire is skipped (overlap)"),
            ],
        )
        .description(
            "Which run outcomes notify by default (jobs can override). Channels come from \
             the notify component's settings.",
        )]))
}

/// Maps a hub key to the scheduler op payload key.
fn op_key(key: &str) -> Option<&'static str> {
    match key {
        "concurrency" => Some("concurrency"),
        "missedWindowS" => Some("missedWindowS"),
        "retentionRuns" => Some("retentionRuns"),
        "retentionDays" => Some("retentionDays"),
        "defaultTimeLimitS" => Some("defaultTimeLimitS"),
        "paused" => Some("paused"),
        "notifyDefaultEvents" => Some("notifyDefaultEvents"),
        _ => None,
    }
}

/// Pushes one hub-layer field write through to the scheduler.
pub async fn push(values: &Values) {
    for (key, value) in [
        ("concurrency", values.get("concurrency").cloned()),
        ("missedWindowS", values.get("missedWindowS").cloned()),
        ("retentionRuns", values.get("retentionRuns").cloned()),
        ("retentionDays", values.get("retentionDays").cloned()),
        (
            "defaultTimeLimitS",
            values.get("defaultTimeLimitS").cloned(),
        ),
        ("paused", values.get("paused").cloned()),
        (
            "notifyDefaultEvents",
            values.get("notifyDefaultEvents").cloned(),
        ),
    ] {
        let Some(value) = value else { continue };
        let Some(payload_key) = op_key(key) else {
            continue;
        };
        // Fire-and-forget like unipi: a failure means the scheduler didn't get the change;
        // the hub layer still shows the intent, and `oino cron settings` shows the truth.
        let _ = oino_extension::cron::call("settings-set", &json!({ payload_key: value })).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_key_maps_to_a_scheduler_key() {
        let ns = namespace();
        for field in ns.sections.iter().flat_map(|s| s.fields.iter()) {
            assert!(
                op_key(&field.key).is_some(),
                "field {} has no scheduler mapping",
                field.key
            );
        }
    }

    #[test]
    fn defaults_mirror_the_scheduler_block() {
        assert_eq!(defaults()["concurrency"], 2);
        assert_eq!(defaults()["missedWindowS"], 21600);
        assert_eq!(defaults()["retentionRuns"], 50);
        assert_eq!(defaults()["retentionDays"], 30);
        assert_eq!(defaults()["defaultTimeLimitS"], 1800);
        assert_eq!(defaults()["paused"], false);
    }
}
