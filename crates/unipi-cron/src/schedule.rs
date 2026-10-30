//! Parsing and evaluating a job's `schedule` + `tz`.
//!
//! `schedule` is either a 6-field cron (seconds first — `s m h dom mon dow`) or
//! `at:<RFC3339>` for a one-time run. Plain-language forms ("every day at 9:00",
//! "in 2 hours", "tomorrow at 9") are accepted by [`parse_human`] as a convenience on
//! top, never a replacement. `croner` (with its `chrono` backend) does the cron math;
//! `chrono-tz` resolves the IANA zone name.

use chrono::{DateTime, Days, Duration, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use croner::parser::{CronParser, Seconds};

/// A parsed schedule: either a recurring cron pattern or a single instant.
#[derive(Debug, Clone)]
pub enum Schedule {
    /// Recurring 6-field cron.
    Cron(Box<Cron>),
    /// One-time instant.
    At(DateTime<Utc>),
}

/// An example shown in validation errors.
pub const CRON_EXAMPLE: &str =
    "0 30 9 * * MON-FRI (9:30 every weekday, seconds first) or at:2025-12-01T09:00:00Z";

/// A validation error (message meant to be shown).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ScheduleError(pub String);

impl Schedule {
    /// Parses a machine-form schedule (6-field cron or `at:<RFC3339>`).
    ///
    /// # Errors
    /// [`ScheduleError`] with a clear message and examples.
    pub fn parse(schedule: &str) -> Result<Self, ScheduleError> {
        if let Some(rest) = schedule.strip_prefix("at:") {
            let at = DateTime::parse_from_rfc3339(rest).map_err(|e| {
                ScheduleError(format!(
                    "bad `at:` time {rest:?}: {e} — example: at:2025-12-01T09:00:00Z"
                ))
            })?;
            return Ok(Self::At(at.with_timezone(&Utc)));
        }
        let fields = schedule.split_whitespace().count();
        if fields != 6 {
            return Err(ScheduleError(format!(
                "cron schedules need exactly 6 space-separated fields (seconds first), got {fields} in {schedule:?}. Example: {CRON_EXAMPLE}"
            )));
        }
        let parser = CronParser::builder().seconds(Seconds::Required).build();
        let cron = parser.parse(schedule).map_err(|e| {
            ScheduleError(format!(
                "bad cron schedule {schedule:?}: {e}. Example: {CRON_EXAMPLE}"
            ))
        })?;
        Ok(Self::Cron(Box::new(cron)))
    }

    /// Is this a one-time `at:` schedule?
    #[must_use]
    pub fn is_one_time(&self) -> bool {
        matches!(self, Self::At(_))
    }

    /// The next fire time strictly after `after` (exclusive), or the `at:` time itself if
    /// it is still in the future (or equal to `after`, when `inclusive`).
    #[must_use]
    pub fn next_after(
        &self,
        tz: Tz,
        after: DateTime<Utc>,
        inclusive: bool,
    ) -> Option<DateTime<Utc>> {
        match self {
            Self::At(at) => {
                let ok = if inclusive { *at >= after } else { *at > after };
                ok.then_some(*at)
            }
            Self::Cron(cron) => {
                let local = after.with_timezone(&tz);
                cron.find_next_occurrence(&local, inclusive)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }
        }
    }

    /// The latest fire time at or before `at` (inclusive). Finds what's due right now in
    /// one bounded step, however far in the past the job's last known run was.
    #[must_use]
    pub fn last_at_or_before(&self, tz: Tz, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::At(scheduled) => (*scheduled <= at).then_some(*scheduled),
            Self::Cron(cron) => {
                let local = at.with_timezone(&tz);
                cron.find_previous_occurrence(&local, true)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }
        }
    }
}

/// Turns a plain-language schedule into the 6-field cron (or `at:`) form
/// [`Schedule::parse`] understands, resolved against `tz`/`now` for the relative and
/// one-time forms. Returns the machine form unchanged when `input` already looks like
/// one (starts with `at:` or has 6 whitespace-separated fields).
///
/// Recognized forms (case-insensitive):
/// - `hourly` → `0 0 * * * *`
/// - `daily` → `0 0 0 * * *`
/// - `weekly` → `0 0 0 * * SUN`
/// - `every N minutes|hours|seconds`
/// - `every day at H[:MM]`
/// - `weekdays at H[:MM]`
/// - `every <weekday> H[:MM]` (also `every weekday at H[:MM]`)
/// - `in N minutes|hours|days` → one-time `at:`
/// - `tomorrow at H[:MM]` → one-time `at:`
///
/// # Errors
/// [`ScheduleError`] with examples when nothing matches.
#[allow(
    clippy::too_many_lines,
    reason = "a flat table of accepted plain-language forms; splitting it would hide the grammar"
)]
pub fn parse_human(input: &str, tz: Tz, now: DateTime<Utc>) -> Result<String, ScheduleError> {
    let trimmed = input.trim();
    if trimmed.starts_with("at:") || trimmed.split_whitespace().count() == 6 {
        return Ok(trimmed.to_owned());
    }
    let lower = trimmed.to_ascii_lowercase();

    if lower == "hourly" {
        return Ok("0 0 * * * *".to_owned());
    }
    if lower == "daily" {
        return Ok("0 0 0 * * *".to_owned());
    }
    if lower == "weekly" {
        return Ok("0 0 0 * * SUN".to_owned());
    }

    // "every N minutes|hours|seconds"
    if let Some(rest) = lower.strip_prefix("every ") {
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.len() == 2
            && let Ok(n) = words[0].parse::<u32>()
        {
            let unit = words[1].trim_end_matches('s');
            match unit {
                "second" | "sec" => return Ok(format!("*/{n} * * * * *")),
                "minute" | "min" => return Ok(format!("0 */{n} * * * *")),
                "hour" => return Ok(format!("0 0 */{n} * * *")),
                _ => {}
            }
        }
        // "every day at H[:MM]"
        if words.first() == Some(&"day")
            && words.get(1) == Some(&"at")
            && let Some((h, m)) = parse_time(words.get(2).copied().unwrap_or(""))
        {
            return Ok(format!("0 {m} {h} * * *"));
        }
        // "every <weekday> H[:MM]" / "every <weekday> at H[:MM]"
        if let Some(day) = words.first().and_then(|w| weekday_abbrev(w)) {
            let time_word = if words.get(1) == Some(&"at") {
                words.get(2)
            } else {
                words.get(1)
            };
            if let Some((h, m)) = time_word.and_then(|w| parse_time(w)) {
                return Ok(format!("0 {m} {h} * * {day}"));
            }
        }
        // "every weekday at H[:MM]" (Mon-Fri), singular "weekday".
        if words.first() == Some(&"weekday") {
            let time_word = if words.get(1) == Some(&"at") {
                words.get(2)
            } else {
                words.get(1)
            };
            if let Some((h, m)) = time_word.and_then(|w| parse_time(w)) {
                return Ok(format!("0 {m} {h} * * MON-FRI"));
            }
        }
        return Err(human_error(trimmed));
    }

    // "weekdays at H[:MM]"
    if let Some(rest) = lower.strip_prefix("weekdays at ") {
        if let Some((h, m)) = parse_time(rest.trim()) {
            return Ok(format!("0 {m} {h} * * MON-FRI"));
        }
        return Err(human_error(trimmed));
    }

    // "in N minutes|hours|days" → one-time.
    if let Some(rest) = lower.strip_prefix("in ") {
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.len() == 2
            && let Ok(n) = words[0].parse::<i64>()
        {
            let unit = words[1].trim_end_matches('s');
            let delta = match unit {
                "second" | "sec" => Some(Duration::seconds(n)),
                "minute" | "min" => Some(Duration::minutes(n)),
                "hour" => Some(Duration::hours(n)),
                "day" => Some(Duration::days(n)),
                _ => None,
            };
            if let Some(delta) = delta {
                let at = now + delta;
                return Ok(format!(
                    "at:{}",
                    at.to_rfc3339_opts(SecondsFormat::Secs, true)
                ));
            }
        }
        return Err(human_error(trimmed));
    }

    // "tomorrow at H[:MM]" → one-time, resolved in `tz`.
    if let Some(rest) = lower.strip_prefix("tomorrow at ") {
        if let Some((h, m)) = parse_time(rest.trim()) {
            let local_now = now.with_timezone(&tz);
            let tomorrow = local_now.date_naive() + Days::new(1);
            let naive = tomorrow
                .and_hms_opt(u32::from(h), u32::from(m), 0)
                .ok_or_else(|| human_error(trimmed))?;
            let local_at = tz.from_local_datetime(&naive).single().ok_or_else(|| {
                ScheduleError(format!(
                    "{naive} doesn't exist or is ambiguous in {tz} (a DST transition) — try a different time"
                ))
            })?;
            return Ok(format!(
                "at:{}",
                local_at
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            ));
        }
        return Err(human_error(trimmed));
    }

    Err(human_error(trimmed))
}

fn human_error(input: &str) -> ScheduleError {
    ScheduleError(format!(
        "didn't understand the schedule {input:?}. Try: \"every 5 minutes\", \"every day at 9:00\", \"weekdays at 18:30\", \"every monday 08:00\", \"hourly\", \"daily\", \"in 2 hours\", \"tomorrow at 9\" — or a 6-field cron / at:<RFC3339>. Example: {CRON_EXAMPLE}"
    ))
}

/// `"9"`, `"9:30"`, `"09:00"` → `(hour, minute)`.
fn parse_time(text: &str) -> Option<(u8, u8)> {
    let text = text.trim();
    if let Some((h, m)) = text.split_once(':') {
        let h: u8 = h.parse().ok()?;
        let m: u8 = m.parse().ok()?;
        (h < 24 && m < 60).then_some((h, m))
    } else {
        let h: u8 = text.parse().ok()?;
        (h < 24).then_some((h, 0))
    }
}

/// A weekday word (`monday`, `mon`, …) → its croner 3-letter form.
fn weekday_abbrev(word: &str) -> Option<&'static str> {
    match word {
        "monday" | "mon" => Some("MON"),
        "tuesday" | "tue" | "tues" => Some("TUE"),
        "wednesday" | "wed" => Some("WED"),
        "thursday" | "thu" | "thur" | "thurs" => Some("THU"),
        "friday" | "fri" => Some("FRI"),
        "saturday" | "sat" => Some("SAT"),
        "sunday" | "sun" => Some("SUN"),
        _ => None,
    }
}

/// The next `count` fire times (UTC) for display. Empty for an `at:` schedule once it has
/// fired (there is no "next" after that).
#[must_use]
pub fn next_fires(
    schedule: &Schedule,
    tz: Tz,
    after: DateTime<Utc>,
    count: usize,
) -> Vec<DateTime<Utc>> {
    let mut out = Vec::with_capacity(count);
    let mut cursor = after;
    let mut inclusive = true;
    for _ in 0..count {
        let Some(next) = schedule.next_after(tz, cursor, inclusive) else {
            break;
        };
        out.push(next);
        cursor = next;
        inclusive = false;
    }
    out
}

/// `Utc::now()` (re-exported so callers without a chrono dependency get one call site).
#[must_use]
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Resolves a job's `tz` field. Empty/`"local"`/`"system"` means the system zone.
///
/// # Errors
/// [`ScheduleError`] naming the zone when it isn't an IANA zone.
pub fn resolve_tz(tz: &str) -> Result<Tz, ScheduleError> {
    let name = if tz.trim().is_empty()
        || tz.eq_ignore_ascii_case("local")
        || tz.eq_ignore_ascii_case("system")
    {
        system_tz_name()
    } else {
        tz.to_owned()
    };
    name.parse::<Tz>().map_err(|_| {
        ScheduleError(format!(
            "unknown IANA time zone {name:?} — example: Europe/Berlin, America/New_York, UTC"
        ))
    })
}

/// The system's IANA zone name, or `"UTC"` if it can't be determined.
#[must_use]
pub fn system_tz_name() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_owned())
}

/// Validates that `cwd` exists and is a directory.
///
/// # Errors
/// [`ScheduleError`] describing the problem.
pub fn validate_cwd(cwd: &str) -> Result<(), ScheduleError> {
    let path = std::path::Path::new(cwd);
    if !path.is_absolute() {
        return Err(ScheduleError(format!(
            "the working directory must be an absolute path, got {cwd:?}"
        )));
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(ScheduleError(format!(
            "the working directory {cwd:?} exists but is not a directory"
        ))),
        Err(e) => Err(ScheduleError(format!(
            "the working directory {cwd:?} doesn't exist or can't be read: {e}. Create the folder first."
        ))),
    }
}

/// Full validation of a job's schedule/tz/cwd; returns the resolved zone.
///
/// # Errors
/// The first failing check's [`ScheduleError`].
pub fn validate(schedule: &str, tz: &str, cwd: &str) -> Result<Tz, ScheduleError> {
    Schedule::parse(schedule)?;
    let zone = resolve_tz(tz)?;
    validate_cwd(cwd)?;
    Ok(zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_field_cron_with_tz_and_dst() {
        // Europe/Berlin: DST starts 2024-03-31 at 02:00 → 03:00 (clocks skip
        // forward). A job at 02:30 every day has no 02:30 on that date.
        let sched = Schedule::parse("0 30 2 * * *").unwrap();
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let before = Utc.with_ymd_and_hms(2024, 3, 30, 1, 0, 0).unwrap();
        let next = sched.next_after(tz, before, false).unwrap();
        // 2024-03-30 02:30 CET == 01:30 UTC.
        assert_eq!(next, Utc.with_ymd_and_hms(2024, 3, 30, 1, 30, 0).unwrap());
        let next2 = sched.next_after(tz, next, false).unwrap();
        // 2024-03-31 02:30 doesn't exist (clocks jump 02:00 -> 03:00): the
        // resolution rounds the missing wall clock forward to 03:00 CEST.
        assert!(next2 > next);
        assert_eq!(next2, Utc.with_ymd_and_hms(2024, 3, 31, 1, 0, 0).unwrap());
        let next3 = sched.next_after(tz, next2, false).unwrap();
        // Back to normal the next day.
        assert_eq!(next3, Utc.with_ymd_and_hms(2024, 4, 1, 0, 30, 0).unwrap());
    }

    #[test]
    fn at_schedule_is_one_time() {
        let sched = Schedule::parse("at:2030-01-01T00:00:00Z").unwrap();
        assert!(sched.is_one_time());
        let tz: Tz = "UTC".parse().unwrap();
        let before = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let next = sched.next_after(tz, before, false).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap());
        // After it fires, there is no next occurrence.
        assert!(sched.next_after(tz, next, false).is_none());
    }

    #[test]
    fn bad_cron_or_tz_or_cwd_give_clear_errors() {
        assert!(Schedule::parse("not a cron").is_err());
        assert!(Schedule::parse("* * * * *").is_err(), "needs 6 fields");
        assert!(resolve_tz("Not/AZone").is_err());
        assert!(validate_cwd("relative/path").is_err());
        let tmp = tempfile::tempdir().unwrap();
        assert!(validate_cwd(&tmp.path().join("missing").to_string_lossy()).is_err());
        assert!(validate_cwd(&tmp.path().to_string_lossy()).is_ok());
    }

    #[test]
    fn local_tz_falls_back_to_system() {
        assert!(resolve_tz("local").is_ok());
        assert!(resolve_tz("").is_ok());
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 10, 12, 0, 0).unwrap() // a Tuesday
    }

    #[test]
    fn human_every_n_units() {
        let utc: Tz = "UTC".parse().unwrap();
        assert_eq!(
            parse_human("every 5 minutes", utc, now()).unwrap(),
            "0 */5 * * * *"
        );
        assert_eq!(
            parse_human("every 30 seconds", utc, now()).unwrap(),
            "*/30 * * * * *"
        );
        assert_eq!(
            parse_human("every 2 hours", utc, now()).unwrap(),
            "0 0 */2 * * *"
        );
    }

    #[test]
    fn human_hourly_daily_weekly() {
        let utc: Tz = "UTC".parse().unwrap();
        assert_eq!(parse_human("hourly", utc, now()).unwrap(), "0 0 * * * *");
        assert_eq!(parse_human("Daily", utc, now()).unwrap(), "0 0 0 * * *");
        assert_eq!(parse_human("weekly", utc, now()).unwrap(), "0 0 0 * * SUN");
    }

    #[test]
    fn human_every_day_at_and_weekdays_at() {
        let utc: Tz = "UTC".parse().unwrap();
        assert_eq!(
            parse_human("every day at 9:00", utc, now()).unwrap(),
            "0 0 9 * * *"
        );
        assert_eq!(
            parse_human("weekdays at 18:30", utc, now()).unwrap(),
            "0 30 18 * * MON-FRI"
        );
    }

    #[test]
    fn human_every_weekday_name() {
        let utc: Tz = "UTC".parse().unwrap();
        assert_eq!(
            parse_human("every monday 08:00", utc, now()).unwrap(),
            "0 0 8 * * MON"
        );
        assert_eq!(
            parse_human("every friday at 17:45", utc, now()).unwrap(),
            "0 45 17 * * FRI"
        );
    }

    #[test]
    fn human_in_n_units_is_one_time() {
        let utc: Tz = "UTC".parse().unwrap();
        let sched = parse_human("in 2 hours", utc, now()).unwrap();
        assert_eq!(sched, "at:2025-06-10T14:00:00Z");
        assert!(Schedule::parse(&sched).unwrap().is_one_time());
    }

    #[test]
    fn human_tomorrow_at() {
        let utc: Tz = "UTC".parse().unwrap();
        let sched = parse_human("tomorrow at 9", utc, now()).unwrap();
        assert_eq!(sched, "at:2025-06-11T09:00:00Z");
    }

    #[test]
    fn human_machine_forms_pass_through_unchanged() {
        let utc: Tz = "UTC".parse().unwrap();
        assert_eq!(
            parse_human("0 30 9 * * MON-FRI", utc, now()).unwrap(),
            "0 30 9 * * MON-FRI"
        );
        assert_eq!(
            parse_human("at:2030-01-01T00:00:00Z", utc, now()).unwrap(),
            "at:2030-01-01T00:00:00Z"
        );
    }

    #[test]
    fn human_unrecognized_gives_a_clear_error_with_examples() {
        let utc: Tz = "UTC".parse().unwrap();
        let err = parse_human("whenever", utc, now()).unwrap_err();
        assert!(err.0.contains("every 5 minutes"), "{}", err.0);
        assert!(err.0.contains("didn't understand"), "{}", err.0);
    }

    #[test]
    fn next_fires_lists_three_upcoming() {
        let sched = Schedule::parse("0 0 * * * *").unwrap(); // hourly
        let utc: Tz = "UTC".parse().unwrap();
        let after = Utc.with_ymd_and_hms(2025, 1, 1, 0, 30, 0).unwrap();
        let fires = next_fires(&sched, utc, after, 3);
        assert_eq!(fires.len(), 3);
        assert_eq!(fires[0], Utc.with_ymd_and_hms(2025, 1, 1, 1, 0, 0).unwrap());
        assert_eq!(fires[1], Utc.with_ymd_and_hms(2025, 1, 1, 2, 0, 0).unwrap());
        assert_eq!(fires[2], Utc.with_ymd_and_hms(2025, 1, 1, 3, 0, 0).unwrap());
    }

    #[test]
    fn last_at_or_before_collapses_to_the_latest_occurrence() {
        let sched = Schedule::parse("0 0 0 * * *").unwrap(); // daily
        let tz = chrono_tz::UTC;
        let at = Utc.with_ymd_and_hms(2023, 12, 3, 2, 0, 0).unwrap();
        assert_eq!(
            sched.last_at_or_before(tz, at),
            Some(Utc.with_ymd_and_hms(2023, 12, 3, 0, 0, 0).unwrap())
        );
        // An `at:` schedule only counts before its instant.
        let one = Schedule::parse("at:2024-01-01T00:00:00Z").unwrap();
        assert_eq!(
            one.last_at_or_before(tz, at),
            None,
            "2024-01-01 is after 2023-12-03"
        );
    }
}
