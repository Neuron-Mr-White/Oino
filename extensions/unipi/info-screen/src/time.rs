//! ISO-8601 timestamps (pi `toISOString()` form) → epoch ms.

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`; `None` when malformed (JS `Date.parse` → NaN).
#[must_use]
#[allow(clippy::many_single_char_names, reason = "date fields y/m/d/h/mi/sec")]
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = s.get(from..from + len)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if b.len() < 19 || b.get(4) != Some(&b'-') || b.get(10).is_none_or(|c| *c != b'T' && *c != b' ')
    {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let mut i = 19;
    let mut ms = 0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        let mut end = start;
        while b.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        let frac = s.get(start..end)?;
        let padded: String = frac.chars().chain("000".chars()).take(3).collect();
        ms = padded.parse().ok()?;
        i = end;
    }
    let offset_min = match b.get(i) {
        None | Some(b'Z' | b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let oh = num(i + 1, 2)?;
            let om = num(i + 4, 2).or_else(|| num(i + 3, 2)).unwrap_or(0);
            let total = oh * 60 + om;
            if *sign == b'+' { total } else { -total }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 86_400 + h * 3_600 + mi * 60 + sec - offset_min * 60) * 1_000) + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pi_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2026-01-01T10:00:01.500Z"),
            Some(1_767_261_601_500)
        );
        assert_eq!(
            parse_iso_ms("2026-01-01T12:00:01.500+02:00"),
            Some(1_767_261_601_500)
        );
        assert_eq!(
            parse_iso_ms("2026-01-01T10:00:01Z"),
            Some(1_767_261_601_000)
        );
        assert_eq!(parse_iso_ms("nope"), None);
        assert_eq!(parse_iso_ms(""), None);
    }
}
