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
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut current_width = 0usize;
        for grapheme in raw.graphemes(true) {
            let grapheme_width = grapheme.width();
            if current_width + grapheme_width > width && current_width != 0 {
                lines.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push_str(grapheme);
            current_width += grapheme_width;
            if current_width >= width {
                lines.push(current);
                current = String::new();
                current_width = 0;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
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
