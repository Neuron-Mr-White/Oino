//! Small formatting helpers (unipi `utils.ts`, `attach/detect.ts`, `tui-width.ts`).

use std::cmp::Ordering;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};

/// unipi `formatTokens`: `1234 → "1.2k"`, `1500000 → "1.5M"`.
#[must_use]
pub fn format_tokens(n: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "display only")]
    let f = n as f64;
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => format!("{:.1}k", f / 1_000.0),
        10_000..=999_999 => format!("{}k", js_round(f / 1_000.0)),
        1_000_000..=9_999_999 => format!("{:.1}M", f / 1_000_000.0),
        _ => format!("{}M", js_round(f / 1_000_000.0)),
    }
}

/// `Math.round` (half up).
#[allow(clippy::cast_possible_truncation, reason = "display values are small")]
fn js_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// unipi `formatBytes`: `B`, `KB` (rounded), `MB` (one decimal).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "display only")]
    let f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", js_round(f / 1024.0))
    } else {
        format!("{:.1} MB", f / 1024.0 / 1024.0)
    }
}

struct Semver<'a> {
    core: [u64; 3],
    pre: Vec<&'a str>,
}

fn parse_semver(v: &str) -> Semver<'_> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let (core_str, pre_str) = v.split_once('-').unwrap_or((v, ""));
    let mut core = [0u64; 3];
    for (slot, part) in core.iter_mut().zip(core_str.split('.')) {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        *slot = digits.parse().unwrap_or(0);
    }
    Semver {
        core,
        pre: if pre_str.is_empty() {
            Vec::new()
        } else {
            pre_str.split('.').collect()
        },
    }
}

fn compare_pre(a: &[&str], b: &[&str]) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    for i in 0..a.len().max(b.len()) {
        let (Some(x), Some(y)) = (a.get(i), b.get(i)) else {
            return if a.len() < b.len() {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        };
        let xn = !x.is_empty() && x.chars().all(|c| c.is_ascii_digit());
        let yn = !y.is_empty() && y.chars().all(|c| c.is_ascii_digit());
        let ord = match (xn, yn) {
            (true, true) => x
                .parse::<u64>()
                .unwrap_or(0)
                .cmp(&y.parse::<u64>().unwrap_or(0)),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// unipi `compareVersions`: `v` prefix ignored, numeric core, semver §11 prerelease order.
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (l, r) = (parse_semver(a), parse_semver(b));
    l.core
        .cmp(&r.core)
        .then_with(|| compare_pre(&l.pre, &r.pre))
}

/// `latest` is newer than `current`.
#[must_use]
pub fn is_newer_version(latest: &str, current: &str) -> bool {
    compare_versions(latest, current) == Ordering::Greater
}

/// unipi `parseArgs`: matches `(?:[^\s"]+|"[^"]*")+` and strips one leading/trailing quote.
#[must_use]
pub fn parse_args(args: &str) -> Vec<String> {
    let chars: Vec<char> = args.chars().collect();
    // Length of one unit at `i`: a run of non-space non-quote chars, or a closed quote.
    let unit = |i: usize| -> usize {
        match chars.get(i) {
            Some('"') => chars[i + 1..]
                .iter()
                .position(|c| *c == '"')
                .map_or(0, |p| p + 2),
            Some(c) if !c.is_whitespace() => chars[i..]
                .iter()
                .take_while(|c| !c.is_whitespace() && **c != '"')
                .count(),
            _ => 0,
        }
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while let n @ 1.. = unit(i) {
            i += n;
        }
        if i == start {
            i += 1;
            continue;
        }
        out.push(strip_outer_quotes(
            &chars[start..i].iter().collect::<String>(),
        ));
    }
    out
}

fn strip_outer_quotes(t: &str) -> String {
    let t = t.strip_prefix('"').unwrap_or(t);
    t.strip_suffix('"').unwrap_or(t).to_owned()
}

/// Random lowercase hex (WASI randomness through std's `RandomState`).
#[must_use]
pub fn random_hex(len: usize) -> String {
    let mut out = String::with_capacity(len + 16);
    while out.len() < len {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(out.len());
        let _ = write!(out, "{:016x}", h.finish());
    }
    out.truncate(len);
    out
}

/// Display width in terminal columns (one per char; combining marks and wide CJK are not
/// special-cased — frontends do exact layout).
#[must_use]
pub fn visible_width(s: &str) -> usize {
    s.chars().count()
}

/// Truncates to `width` columns, ending with `ellipsis` when cut (pi-tui `truncateToWidth`).
#[must_use]
pub fn truncate_to_width(s: &str, width: usize, ellipsis: &str) -> String {
    if visible_width(s) <= width {
        return s.to_owned();
    }
    let room = width.saturating_sub(visible_width(ellipsis));
    let mut out: String = s.chars().take(room).collect();
    if visible_width(ellipsis) <= width {
        out.push_str(ellipsis);
    }
    out
}

/// ISO-8601 UTC timestamp (`2026-10-26T06:22:37.855Z`) for epoch ms.
#[must_use]
pub fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1234), "1.2k");
        assert_eq!(format_tokens(12_500), "13k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
        assert_eq!(format_tokens(25_000_000), "25M");
    }

    #[test]
    fn bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "2 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn versions() {
        assert_eq!(compare_versions("3.0.0", "v3.0.0"), Ordering::Equal);
        assert_eq!(
            compare_versions("3.0.0-alpha.9", "3.0.0-alpha.10"),
            Ordering::Less
        );
        assert_eq!(compare_versions("3.0.0-alpha.10", "3.0.0"), Ordering::Less);
        assert_eq!(compare_versions("3.0.1", "3.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(
            compare_versions("1.0.0-alpha", "1.0.0-alpha.1"),
            Ordering::Less
        );
        assert_eq!(compare_versions("1.0.0-1", "1.0.0-alpha"), Ordering::Less);
        assert_eq!(
            compare_versions("1.0.0-beta", "1.0.0-alpha"),
            Ordering::Greater
        );
        assert!(is_newer_version("3.0.0", "3.0.0-alpha.25"));
        assert!(!is_newer_version("2.9.9", "3.0.0"));
    }

    #[test]
    fn args() {
        assert_eq!(
            parse_args(r#"next "two words" x"#),
            ["next", "two words", "x"]
        );
        assert_eq!(parse_args("  "), Vec::<String>::new());
        assert_eq!(parse_args(r#"a"b c"d"#), [r#"a"b c"d"#]);
        assert_eq!(
            parse_args(r#""ab cd"#),
            ["ab", "cd"],
            "an unclosed quote is skipped"
        );
    }

    #[test]
    fn widths_and_random() {
        assert_eq!(truncate_to_width("hello world", 8, "…"), "hello w…");
        assert_eq!(truncate_to_width("short", 8, "…"), "short");
        assert_eq!(truncate_to_width("abc", 0, "…"), "");
        let a = random_hex(20);
        assert_eq!(a.len(), 20);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_hex(20));
    }

    #[test]
    fn iso_timestamps() {
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_ms(1_730_000_000_123), "2024-10-27T03:33:20.123Z");
        assert_eq!(iso_from_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }
}
