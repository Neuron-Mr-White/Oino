//! Output truncation shared by the tools (pi `tools/truncate.ts`).
//!
//! Two independent limits apply, whichever is hit first: a line limit
//! ([`DEFAULT_MAX_LINES`]) and a byte limit ([`DEFAULT_MAX_BYTES`]). Partial lines are only
//! returned by [`truncate_tail`] when the last line alone exceeds the byte limit.

use serde::{Deserialize, Serialize};

use crate::js;

/// Default line limit (pi `DEFAULT_MAX_LINES`).
pub const DEFAULT_MAX_LINES: u64 = 2000;
/// Default byte limit, 50KB (pi `DEFAULT_MAX_BYTES`).
pub const DEFAULT_MAX_BYTES: u64 = 50 * 1024;
/// Maximum characters per grep match line (pi `GREP_MAX_LINE_LENGTH`).
pub const GREP_MAX_LINE_LENGTH: usize = 500;
/// pi `Number.MAX_SAFE_INTEGER`, used as "no line limit".
pub const NO_LINE_LIMIT: u64 = 9_007_199_254_740_991;

/// Which limit truncated the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TruncatedBy {
    /// The line limit.
    Lines,
    /// The byte limit.
    Bytes,
}

/// pi `TruncationResult` (serialised in tool `details.truncation`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TruncationResult {
    /// The truncated content.
    pub content: String,
    /// Whether truncation occurred.
    pub truncated: bool,
    /// Which limit was hit (`null` when not truncated).
    pub truncated_by: Option<TruncatedBy>,
    /// Lines in the original content.
    pub total_lines: u64,
    /// UTF-8 bytes in the original content.
    pub total_bytes: u64,
    /// Complete lines in the output.
    pub output_lines: u64,
    /// UTF-8 bytes in the output.
    pub output_bytes: u64,
    /// The last line was cut (tail truncation edge case).
    pub last_line_partial: bool,
    /// The first line alone exceeds the byte limit (head truncation).
    pub first_line_exceeds_limit: bool,
    /// Line limit applied.
    pub max_lines: u64,
    /// Byte limit applied.
    pub max_bytes: u64,
}

/// Limits for [`truncate_head`] / [`truncate_tail`] (pi `TruncationOptions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TruncationOptions {
    /// Maximum number of lines.
    pub max_lines: u64,
    /// Maximum number of bytes.
    pub max_bytes: u64,
}

impl Default for TruncationOptions {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_MAX_LINES,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl TruncationOptions {
    /// Default byte limit without a line limit (grep, find, ls).
    #[must_use]
    pub fn bytes_only() -> Self {
        Self {
            max_lines: NO_LINE_LIMIT,
            ..Self::default()
        }
    }
}

fn split_lines_for_counting(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// pi `formatSize`: `512B`, `1.5KB`, `2.0MB`.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "sizes are far below 2^53")]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{}KB", js::to_fixed(value / 1024.0, 1))
    } else {
        format!("{}MB", js::to_fixed(value / (1024.0 * 1024.0), 1))
    }
}

fn untruncated(content: &str, lines: u64, options: TruncationOptions) -> TruncationResult {
    let bytes = count(content.len());
    TruncationResult {
        content: content.to_owned(),
        truncated: false,
        truncated_by: None,
        total_lines: lines,
        total_bytes: bytes,
        output_lines: lines,
        output_bytes: bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines: options.max_lines,
        max_bytes: options.max_bytes,
    }
}

/// pi `truncateHead`: keeps the first lines/bytes, never a partial line. When the first line
/// alone exceeds the byte limit the content is empty and `first_line_exceeds_limit` is set.
#[must_use]
pub fn truncate_head(content: &str, options: TruncationOptions) -> TruncationResult {
    let total_bytes = count(content.len());
    let lines = split_lines_for_counting(content);
    let total_lines = count(lines.len());
    if total_lines <= options.max_lines && total_bytes <= options.max_bytes {
        return untruncated(content, total_lines, options);
    }
    let first_line_bytes = lines.first().map_or(0, |line| count(line.len()));
    if first_line_bytes > options.max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines: options.max_lines,
            max_bytes: options.max_bytes,
        };
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut kept_bytes = 0_u64;
    let mut truncated_by = TruncatedBy::Lines;
    for (index, line) in lines.iter().enumerate() {
        if count(index) >= options.max_lines {
            break;
        }
        let line_bytes = count(line.len()) + u64::from(index > 0);
        if kept_bytes + line_bytes > options.max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }
        kept.push(line);
        kept_bytes += line_bytes;
    }
    if count(kept.len()) >= options.max_lines && kept_bytes <= options.max_bytes {
        truncated_by = TruncatedBy::Lines;
    }
    let output = kept.join("\n");
    TruncationResult {
        output_bytes: count(output.len()),
        output_lines: count(kept.len()),
        content: output,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines: options.max_lines,
        max_bytes: options.max_bytes,
    }
}

/// pi `truncateTail`: keeps the last lines/bytes. May return a partial line when the last line
/// alone exceeds the byte limit (`last_line_partial`).
#[must_use]
pub fn truncate_tail(content: &str, options: TruncationOptions) -> TruncationResult {
    let total_bytes = count(content.len());
    let lines = split_lines_for_counting(content);
    let total_lines = count(lines.len());
    if total_lines <= options.max_lines && total_bytes <= options.max_bytes {
        return untruncated(content, total_lines, options);
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut kept_bytes = 0_u64;
    let mut truncated_by = TruncatedBy::Lines;
    let mut last_line_partial = false;
    for line in lines.iter().rev() {
        if count(kept.len()) >= options.max_lines {
            break;
        }
        let line_bytes = count(line.len()) + u64::from(!kept.is_empty());
        if kept_bytes + line_bytes > options.max_bytes {
            truncated_by = TruncatedBy::Bytes;
            if kept.is_empty() {
                let tail = tail_within_bytes(line, options.max_bytes);
                kept_bytes = count(tail.len());
                kept.push(tail);
                last_line_partial = true;
            }
            break;
        }
        kept.push(line);
        kept_bytes += line_bytes;
    }
    if count(kept.len()) >= options.max_lines && kept_bytes <= options.max_bytes {
        truncated_by = TruncatedBy::Lines;
    }
    kept.reverse();
    let output = kept.join("\n");
    TruncationResult {
        output_bytes: count(output.len()),
        output_lines: count(kept.len()),
        content: output,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        last_line_partial,
        first_line_exceeds_limit: false,
        max_lines: options.max_lines,
        max_bytes: options.max_bytes,
    }
}

/// pi `truncateStringToBytesFromEnd`: the longest suffix of `text` within `max_bytes`, starting
/// at a character boundary.
fn tail_within_bytes(text: &str, max_bytes: u64) -> &str {
    let max = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// pi `truncateLine` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruncatedLine {
    /// The possibly shortened line.
    pub text: String,
    /// Whether the line was shortened.
    pub was_truncated: bool,
}

/// pi `truncateLine`: cuts a line to `max_chars` UTF-16 units and appends `... [truncated]`.
#[must_use]
pub fn truncate_line(line: &str, max_chars: usize) -> TruncatedLine {
    if js::utf16_len(line) <= max_chars {
        return TruncatedLine {
            text: line.to_owned(),
            was_truncated: false,
        };
    }
    TruncatedLine {
        text: format!("{}... [truncated]", js::utf16_prefix(line, max_chars)),
        was_truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_matches_pi() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(1023), "1023B");
        assert_eq!(format_size(1024), "1.0KB");
        assert_eq!(format_size(1280), "1.3KB");
        assert_eq!(format_size(51_200), "50.0KB");
        assert_eq!(format_size(1024 * 1024 * 3 / 2), "1.5MB");
    }

    #[test]
    fn head_keeps_complete_lines() {
        let result = truncate_head(
            "aaa\nbbb\nccc",
            TruncationOptions {
                max_lines: 10,
                max_bytes: 8,
            },
        );
        assert_eq!(result.content, "aaa\nbbb");
        assert_eq!(result.truncated_by, Some(TruncatedBy::Bytes));
        assert_eq!(result.output_lines, 2);
    }

    #[test]
    fn head_first_line_too_long() {
        let result = truncate_head(
            "abcdef\nx",
            TruncationOptions {
                max_lines: 10,
                max_bytes: 3,
            },
        );
        assert!(result.first_line_exceeds_limit);
        assert_eq!(result.content, "");
    }

    #[test]
    fn tail_partial_line() {
        let result = truncate_tail(
            "a\nbéééé",
            TruncationOptions {
                max_lines: 10,
                max_bytes: 4,
            },
        );
        assert!(result.last_line_partial);
        assert_eq!(result.content, "éé");
    }

    #[test]
    fn line_truncation_counts_utf16() {
        assert_eq!(truncate_line("abc", 2).text, "ab... [truncated]");
        assert!(!truncate_line("ab", 2).was_truncated);
    }

    #[test]
    fn untruncated_counts_trailing_newline() {
        let result = truncate_head("a\nb\n", TruncationOptions::default());
        assert_eq!(result.total_lines, 2);
        assert!(!result.truncated);
    }
}
