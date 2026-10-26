//! Display formatting shared by frontends (pi `footer.ts`, `session-selector.ts`, tool
//! renderers).

use std::path::{Component, Path};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// pi `formatTokens`: `999`, `1.2k`, `12k`, `1.2M`, `12M`.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "token counts are far below 2^52; display rounding only"
)]
pub fn format_tokens(count: u64) -> String {
    let n = count as f64;
    if count < 1000 {
        count.to_string()
    } else if count < 10_000 {
        format!("{:.1}k", n / 1000.0)
    } else if count < 1_000_000 {
        format!("{}k", (n / 1000.0).round())
    } else if count < 10_000_000 {
        format!("{:.1}M", n / 1_000_000.0)
    } else {
        format!("{}M", (n / 1_000_000.0).round())
    }
}

/// pi `formatCwdForFooter`: `~` for home, `~/rel` inside home, else the path unchanged.
#[must_use]
pub fn format_cwd(cwd: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return cwd.display().to_string();
    };
    match cwd.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) if !rest.components().any(|c| matches!(c, Component::ParentDir)) => {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        }
        _ => cwd.display().to_string(),
    }
}

/// Replaces a leading home directory with `~` (pi `shortenPath`).
#[must_use]
pub fn shorten_path(path: &str, home: Option<&str>) -> String {
    match home {
        Some(home) if !home.is_empty() && path.starts_with(home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

/// pi `formatSessionDate`: `now`, `5m`, `3h`, `2d`, `4w`, `6mo`, `1y` relative to `now_ms`.
#[must_use]
pub fn format_age(modified_ms: i64, now_ms: i64) -> String {
    let seconds = (now_ms - modified_ms).max(0) / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 1 {
        "now".to_owned()
    } else if hours < 1 {
        format!("{minutes}m")
    } else if days < 1 {
        format!("{hours}h")
    } else if days < 7 {
        format!("{days}d")
    } else if days < 30 {
        format!("{}w", days / 7)
    } else if days < 365 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

/// Display width of `text` in terminal cells.
#[must_use]
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Truncates `text` to `max` cells, appending `ellipsis` when cut (pi `truncateToWidth`).
#[must_use]
pub fn truncate(text: &str, max: usize, ellipsis: &str) -> String {
    if width(text) <= max {
        return text.to_owned();
    }
    let budget = max.saturating_sub(width(ellipsis));
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    if max >= width(ellipsis) {
        out.push_str(ellipsis);
    }
    out
}

/// Collapses newlines/tabs/control characters to single spaces (pi `sanitizeStatusText`).
#[must_use]
pub fn single_line(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// pi `formatReadLineRange`: `:offset` / `:offset-end` for read calls.
#[must_use]
pub fn read_line_range(offset: Option<u64>, limit: Option<u64>) -> String {
    match (offset, limit) {
        (None, None) => String::new(),
        (offset, limit) => {
            let start = offset.unwrap_or(1);
            match limit {
                Some(limit) => format!(":{start}-{}", start + limit.saturating_sub(1)),
                None => format!(":{start}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_pi() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1234), "1.2k");
        assert_eq!(format_tokens(12_345), "12k");
        assert_eq!(format_tokens(1_234_567), "1.2M");
        assert_eq!(format_tokens(12_345_678), "12M");
    }

    #[test]
    fn cwd_and_truncation() {
        let home = Path::new("/home/u");
        assert_eq!(format_cwd(Path::new("/home/u"), Some(home)), "~");
        assert_eq!(format_cwd(Path::new("/home/u/p"), Some(home)), "~/p");
        assert_eq!(format_cwd(Path::new("/srv"), Some(home)), "/srv");
        assert_eq!(truncate("hello world", 8, "..."), "hello...");
        assert_eq!(truncate("hi", 8, "..."), "hi");
        assert_eq!(single_line("a\n\tb  c"), "a b c");
        assert_eq!(read_line_range(Some(10), Some(5)), ":10-14");
        assert_eq!(format_age(0, 3_600_000 * 5), "5h");
    }
}
