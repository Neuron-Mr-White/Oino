//! The dream schedule as a host cron job (v3 J2): the always-on `unipi-oino-host` runs the
//! cron scheduler, so a dream fires at the chosen local time even when no chat is open.
//!
//! The extension owns one job file, `~/.oino/cron/jobs/unipi-dream.json` (the same v2 job
//! schema the `unipi.cron` extension writes; written through the `/cron-jobs` preopen under
//! the shared `.lock`). Each fire runs `oino -p "/dream run --scheduled"` in the agent
//! directory: the command applies the gates (`minSessions`, `minGapHours`, already running)
//! and waits for the dream agent, so the headless process lives exactly as long as the dream.
//! `tz: "local"` makes the host resolve the machine's zone (a wasm guest can't).
//!
//! `enabled = false` or `schedule.mode = manual` removes the job. Without a running host
//! nothing fires; an interactive session then catches up on an overdue dream instead
//! ([`overdue`]).

use serde_json::{Value, json};

use crate::settings::{DreamSettings, Mode};

/// The job id (= file stem).
pub const JOB_ID: &str = "unipi-dream";
/// The prompt every fire sends.
pub const PROMPT: &str = "/dream run --scheduled";
/// The writable jobs preopen.
pub const JOBS_DIR: &str = "/cron-jobs";
/// The job-file lock shared with the host library and `unipi.cron`.
pub const LOCK: &str = "/cron-jobs/.lock";
/// Extra hours before a missed scheduled dream counts as overdue.
pub const GRACE_HOURS: f64 = 2.0;

/// The job file path.
#[must_use]
pub fn job_path() -> String {
    format!("{JOBS_DIR}/{JOB_ID}.json")
}

/// The 6-field cron expression (seconds first) for the schedule; `None` for manual.
///
/// `interval` below a day steps the hour field (`*/N`, so a non-divisor of 24 has one short
/// gap at midnight); a day or more steps the day of month (`*/D`, restarting each month).
#[must_use]
pub fn cron_expr(cfg: &DreamSettings) -> Option<String> {
    let (h, m) = cfg.schedule.time;
    match cfg.schedule.mode {
        Mode::Manual => None,
        Mode::Daily => Some(format!("0 {m} {h} * * *")),
        Mode::Interval => {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 1..=168"
            )]
            let n = cfg.schedule.interval_hours.round().clamp(1.0, 168.0) as u32;
            Some(match n {
                0..24 => format!("0 {m} */{n} * * *"),
                24 => format!("0 {m} {h} * * *"),
                _ => {
                    let days = (f64::from(n) / 24.0).round().max(1.0);
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "1..=7"
                    )]
                    let d = days as u32;
                    format!("0 {m} {h} */{d} * *")
                }
            })
        }
    }
}

/// Hours between scheduled dreams (`None` for manual).
#[must_use]
pub fn period_hours(cfg: &DreamSettings) -> Option<f64> {
    match cfg.schedule.mode {
        Mode::Manual => None,
        Mode::Daily => Some(24.0),
        Mode::Interval => Some(cfg.schedule.interval_hours.round().clamp(1.0, 168.0)),
    }
}

/// A scheduled dream was missed: the last one is older than a period plus [`GRACE_HOURS`].
#[must_use]
pub fn overdue(cfg: &DreamSettings, last_run_at: i64, now: i64) -> bool {
    let Some(period) = period_hours(cfg) else {
        return false;
    };
    if !cfg.enabled {
        return false;
    }
    #[allow(clippy::cast_precision_loss, reason = "ms spans fit f64")]
    let since_h = (now - last_run_at) as f64 / 3_600_000.0;
    last_run_at <= 0 || since_h >= period + GRACE_HOURS
}

/// The job object the host scheduler reads (`None` = no job should exist).
///
/// `created` is kept from an existing job file so rewrites don't look like new jobs.
#[must_use]
pub fn job_value(cfg: &DreamSettings, cwd: &str, now: u64, created: Option<u64>) -> Option<Value> {
    if !cfg.enabled || cwd.is_empty() {
        return None;
    }
    let schedule = cron_expr(cfg)?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "minutes, bounded"
    )]
    let limit = (cfg.max_runtime_min * 60.0) as u64 + 300;
    Some(json!({
        "v": 2,
        "id": JOB_ID,
        "name": "Dream (memory consolidation)",
        "prompt": PROMPT,
        "schedule": schedule,
        "tz": "local",
        "cwd": cwd,
        "enabled": true,
        "timeLimitS": limit,
        "overlap": "skip",
        "missed": "run-latest",
        "created": created.unwrap_or(now),
        "updated": now,
    }))
}

/// The job fields that matter for a rewrite (everything but `created`/`updated`).
#[must_use]
pub fn same_job(a: &Value, b: &Value) -> bool {
    [
        "id",
        "name",
        "prompt",
        "schedule",
        "tz",
        "cwd",
        "enabled",
        "timeLimitS",
        "overlap",
        "missed",
    ]
    .iter()
    .all(|k| a.get(k) == b.get(k))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;
    use crate::settings::{Schedule, normalize};

    fn cfg(mode: Mode, time: (u32, u32), hours: f64) -> DreamSettings {
        DreamSettings {
            enabled: true,
            schedule: Schedule {
                mode,
                time,
                interval_hours: hours,
            },
            ..DreamSettings::default()
        }
    }

    #[test]
    fn expressions_parse_in_the_host_scheduler() {
        let cases = [
            (cfg(Mode::Daily, (3, 0), 24.0), "0 0 3 * * *"),
            (cfg(Mode::Daily, (22, 45), 24.0), "0 45 22 * * *"),
            (cfg(Mode::Interval, (3, 15), 6.0), "0 15 */6 * * *"),
            (cfg(Mode::Interval, (3, 0), 24.0), "0 0 3 * * *"),
            (cfg(Mode::Interval, (4, 30), 48.0), "0 30 4 */2 * *"),
            (cfg(Mode::Interval, (0, 0), 1.0), "0 0 */1 * * *"),
        ];
        for (c, want) in cases {
            let e = cron_expr(&c).unwrap();
            assert_eq!(e, want);
            unipi_cron_schedule::Schedule::parse(&e).unwrap_or_else(|err| panic!("{e}: {err}"));
        }
        assert_eq!(cron_expr(&cfg(Mode::Manual, (3, 0), 24.0)), None);
    }

    #[test]
    fn job_file_shape() {
        let c = cfg(Mode::Daily, (3, 0), 24.0);
        let j = job_value(&c, "/home/u/.oino", 1000, None).unwrap();
        assert_eq!(j["id"], JOB_ID);
        assert_eq!(j["prompt"], PROMPT);
        assert_eq!(j["tz"], "local");
        assert_eq!(j["created"], 1000);
        assert_eq!(j["timeLimitS"], 40 * 60 + 300);
        let k = job_value(&c, "/home/u/.oino", 2000, Some(1000)).unwrap();
        assert_eq!(k["created"], 1000);
        assert!(same_job(&j, &k));
        let later =
            job_value(&cfg(Mode::Daily, (4, 0), 24.0), "/home/u/.oino", 3000, None).unwrap();
        assert!(!same_job(&j, &later));
        assert!(
            job_value(&DreamSettings::default(), "/x", 1, None).is_none(),
            "off by default"
        );
        assert!(job_value(&cfg(Mode::Manual, (3, 0), 24.0), "/x", 1, None).is_none());
        assert!(job_value(&c, "", 1, None).is_none());
        assert_eq!(job_path(), "/cron-jobs/unipi-dream.json");
    }

    #[test]
    fn overdue_catch_up() {
        const H: i64 = 3_600_000;
        let c = cfg(Mode::Daily, (3, 0), 24.0);
        assert!(overdue(&c, 0, 100 * H), "never dreamed");
        assert!(!overdue(&c, 100 * H - 25 * H, 100 * H));
        assert!(overdue(&c, 100 * H - 27 * H, 100 * H));
        assert!(!overdue(&cfg(Mode::Manual, (3, 0), 24.0), 0, 100 * H));
        assert!(!overdue(&normalize(&Value::Null), 0, 100 * H), "disabled");
        assert_eq!(period_hours(&cfg(Mode::Interval, (0, 0), 6.4)), Some(6.0));
    }
}
