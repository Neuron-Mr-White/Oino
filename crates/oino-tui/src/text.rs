#![forbid(unsafe_code)]

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    for raw in text.split('\n') {
        wrap_raw_line(raw, width, |line| lines.push(line.to_string()));
    }
    lines
}

pub(crate) fn wrapped_line_count(text: &str, width: usize) -> usize {
    let width = width.max(1);
    if text.is_empty() {
        return 1;
    }

    let mut count = 0usize;
    for raw in text.split('\n') {
        wrap_raw_line(raw, width, |_| count = count.saturating_add(1));
    }
    count
}

fn wrap_raw_line<'a>(raw: &'a str, width: usize, mut push: impl FnMut(&'a str)) {
    if raw.is_empty() {
        push("");
        return;
    }

    let mut line_start = 0usize;
    let mut current_width = 0usize;
    for (index, grapheme) in raw.grapheme_indices(true) {
        let grapheme_width = grapheme.width();
        if current_width + grapheme_width > width && current_width != 0 {
            push(&raw[line_start..index]);
            line_start = index;
            current_width = 0;
        }
        current_width += grapheme_width;
        if current_width >= width {
            let end = index + grapheme.len();
            push(&raw[line_start..end]);
            line_start = end;
            current_width = 0;
        }
    }
    if line_start < raw.len() {
        push(&raw[line_start..]);
    }
}

pub(crate) fn truncate_to_width(text: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

pub(crate) fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }

    let ellipsis = '…';
    let ellipsis_width = ellipsis.width().unwrap_or(1);
    if max_width <= ellipsis_width {
        return ellipsis.to_string();
    }

    let target_width = max_width.saturating_sub(ellipsis_width);
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > target_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push(ellipsis);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_line_count_matches_wrapped_lines() {
        let cases = [
            "",
            "short",
            "one two three four five",
            "one\n\nthree",
            "emoji ✅🚀 width",
            "日本語の幅を扱うテキスト",
        ];
        for text in cases {
            for width in 1..12 {
                assert_eq!(
                    wrapped_line_count(text, width),
                    wrap_text(text, width).len()
                );
            }
        }
    }
}
