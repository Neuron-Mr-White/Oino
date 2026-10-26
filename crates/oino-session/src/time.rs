//! ISO-8601 timestamps exactly like JavaScript `Date#toISOString` (UTC, milliseconds, `Z`).

use std::time::{SystemTime, UNIX_EPOCH};

use oino_ai::TimestampMs;

const MS_PER_DAY: i64 = 86_400_000;

/// Current Unix time in milliseconds.
#[must_use]
pub fn now_ms() -> TimestampMs {
    system_time_ms(SystemTime::now())
}

/// Converts a [`SystemTime`] to Unix milliseconds (negative before 1970).
#[must_use]
pub fn system_time_ms(time: SystemTime) -> TimestampMs {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// Converts Unix milliseconds to a [`SystemTime`].
#[must_use]
pub fn ms_to_system_time(ms: TimestampMs) -> SystemTime {
    let magnitude = std::time::Duration::from_millis(ms.unsigned_abs());
    if ms >= 0 {
        UNIX_EPOCH + magnitude
    } else {
        UNIX_EPOCH - magnitude
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Formats Unix milliseconds like `new Date(ms).toISOString()`.
#[must_use]
pub fn format_iso(ms: TimestampMs) -> String {
    let days = ms.div_euclid(MS_PER_DAY);
    let rem = ms.rem_euclid(MS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (rem / 3_600_000, rem / 60_000 % 60);
    let (second, milli) = (rem / 1000 % 60, rem % 1000);
    if (0..=9999).contains(&year) {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
    } else {
        let sign = if year < 0 { '-' } else { '+' };
        let abs = year.abs();
        format!("{sign}{abs:06}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
    }
}

fn digits(text: &str) -> Option<i64> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// Parses an ISO-8601 date-time (`YYYY-MM-DD[THH:MM[:SS[.fff…]]][Z|±HH:MM]`) to Unix ms.
/// Strings without an offset are read as UTC.
#[must_use]
pub fn parse_iso(text: &str) -> Option<TimestampMs> {
    let text = text.trim();
    let (date, time) = match text.find(['T', ' ']) {
        Some(index) => (&text[..index], &text[index + 1..]),
        None => (text, ""),
    };
    let mut date_parts = date.splitn(3, '-');
    let year = digits(date_parts.next()?)?;
    let month = digits(date_parts.next()?)?;
    let day = digits(date_parts.next()?)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (clock, offset_minutes) = split_offset(time)?;
    let mut clock_parts = clock.splitn(3, ':');
    let hour = clock_parts
        .next()
        .filter(|s| !s.is_empty())
        .map_or(Some(0), digits)?;
    let minute = clock_parts.next().map_or(Some(0), digits)?;
    let (second, millis) = match clock_parts.next() {
        None => (0, 0),
        Some(sec) => match sec.split_once('.') {
            None => (digits(sec)?, 0),
            Some((whole, fraction)) => {
                let padded: String = fraction.chars().chain("000".chars()).take(3).collect();
                (digits(whole)?, digits(&padded)?)
            }
        },
    };
    if hour > 24 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(
        days * MS_PER_DAY + hour * 3_600_000 + minute * 60_000 + second * 1000 + millis
            - offset_minutes * 60_000,
    )
}

fn split_offset(time: &str) -> Option<(&str, i64)> {
    if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        return Some((clock, 0));
    }
    match time.rfind(['+', '-']) {
        Some(index) => {
            let (clock, offset) = time.split_at(index);
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let offset = offset[1..].replace(':', "");
            let hours = digits(offset.get(..2)?)?;
            let minutes = offset
                .get(2..)
                .filter(|s| !s.is_empty())
                .map_or(Some(0), digits)?;
            Some((clock, sign * (hours * 60 + minutes)))
        }
        None => Some((time, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn formats_like_to_iso_string() {
        assert_eq!(format_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_iso(1_733_234_400_000), "2024-12-03T14:00:00.000Z");
        assert_eq!(format_iso(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn parses_offsets_and_fractions() {
        assert_eq!(
            parse_iso("2024-12-03T14:00:00.000Z"),
            Some(1_733_234_400_000)
        );
        assert_eq!(
            parse_iso("2024-12-03T16:00:00+02:00"),
            Some(1_733_234_400_000)
        );
        assert_eq!(parse_iso("2024-12-03T14:00:00.5Z"), Some(1_733_234_400_500));
        assert_eq!(parse_iso("2024-12-03"), Some(1_733_184_000_000));
        assert_eq!(parse_iso("nope"), None);
    }

    proptest! {
        #[test]
        fn iso_round_trip(ms in -62_135_596_800_000_i64..253_402_300_799_999_i64) {
            prop_assert_eq!(parse_iso(&format_iso(ms)), Some(ms));
        }
    }
}
