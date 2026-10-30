//! Unified diff of a snapshot → the file as it is now (the `diff` service, app.md §3.13).

use similar::{ChangeTag, TextDiff};

use crate::store::MAX_DIFF_BYTES;

/// Unified diff of `before` → `after`, 3 lines of context, capped at [`MAX_DIFF_BYTES`].
/// Returns the text and whether it was truncated.
#[must_use]
pub fn unified(path: &str, before: &str, after: &str) -> (String, bool) {
    let text = TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    clamp(&text)
}

/// Cuts `text` to [`MAX_DIFF_BYTES`] at a char boundary.
#[must_use]
pub fn clamp(text: &str) -> (String, bool) {
    if text.len() <= MAX_DIFF_BYTES {
        return (text.to_owned(), false);
    }
    let mut end = MAX_DIFF_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text.get(..end).unwrap_or_default().to_owned();
    out.push_str("\n… truncated\n");
    (out, true)
}

/// Lines added and removed between two texts (`similar`, line granularity).
#[must_use]
pub fn line_counts(before: &str, after: &str) -> (u64, u64) {
    let diff = TextDiff::from_lines(before, after);
    let mut add = 0_u64;
    let mut del = 0_u64;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => add += 1,
            ChangeTag::Delete => del += 1,
            ChangeTag::Equal => {}
        }
    }
    (add, del)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests assert by unwrapping")]
    use super::*;

    #[test]
    fn counts_changed_lines() {
        assert_eq!(line_counts("a\nb\nc\n", "a\nB\nc\nd\n"), (2, 1));
        assert_eq!(line_counts("", "x\ny\n"), (2, 0));
        assert_eq!(line_counts("x\ny\n", ""), (0, 2));
        assert_eq!(line_counts("same\n", "same\n"), (0, 0));
    }

    #[test]
    fn unified_has_headers_and_hunk() {
        let (text, truncated) = unified("src/a.rs", "one\ntwo\n", "one\nTWO\n");
        assert!(!truncated);
        assert!(
            text.starts_with("--- a/src/a.rs\n+++ b/src/a.rs\n"),
            "{text}"
        );
        assert!(text.contains("-two\n+TWO\n"), "{text}");
    }

    #[test]
    fn clamp_cuts_at_a_char_boundary() {
        let text = "é".repeat(MAX_DIFF_BYTES);
        let (out, truncated) = clamp(&text);
        assert!(truncated);
        assert!(out.len() <= MAX_DIFF_BYTES + "\n… truncated\n".len());
        assert!(out.ends_with("… truncated\n"));
    }
}
