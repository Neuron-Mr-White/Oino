//! An approximation of `String.prototype.localeCompare` (ICU root collation) for directory
//! listings: punctuation and symbols before digits before letters, accents then case as
//! tie-breakers. Printable ASCII follows ICU's exact order; other characters are ordered by
//! class and code point.

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// ICU root order of printable ASCII (case folded).
const ASCII_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";

/// Primary weight `(class << 32) | value`; `None` for ignorable characters.
fn primary(ch: char) -> Option<u64> {
    if ch.is_control() {
        return None;
    }
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    let ascii_index = ASCII_ORDER.find(lower).filter(|_| lower.is_ascii());
    let (class, value) = if let Some(index) = ascii_index {
        let class = if lower.is_ascii_digit() {
            2
        } else if lower.is_ascii_alphabetic() {
            4
        } else {
            0
        };
        (class, u64::try_from(index).unwrap_or(0))
    } else {
        let class = if lower.is_alphabetic() {
            5
        } else if lower.is_numeric() {
            3
        } else {
            1
        };
        (class, u64::from(u32::from(lower)))
    };
    Some((class << 32) | value)
}

/// Latin letters ICU sorts as expansions or accented base letters.
fn latin_fold(ch: char) -> (String, Option<char>) {
    match ch {
        'ß' => ("ss".to_owned(), Some('\u{1}')),
        'æ' => ("ae".to_owned(), Some('\u{1}')),
        'œ' => ("oe".to_owned(), Some('\u{1}')),
        'ø' => ("o".to_owned(), Some('\u{338}')),
        'đ' => ("d".to_owned(), Some('\u{335}')),
        'ł' => ("l".to_owned(), Some('\u{337}')),
        'ı' => ("i".to_owned(), Some('\u{1}')),
        other => (other.to_string(), None),
    }
}

fn keys(text: &str) -> (Vec<u64>, Vec<char>) {
    let mut primaries = Vec::new();
    let mut accents = Vec::new();
    for ch in text.nfd() {
        if is_combining_mark(ch) {
            accents.push(ch);
            continue;
        }
        let (base, accent) = latin_fold(ch);
        for base in base.chars() {
            if let Some(weight) = primary(base) {
                primaries.push(weight);
                accents.push('\0');
            }
        }
        if let Some(accent) = accent {
            accents.push(accent);
        }
    }
    (primaries, accents)
}

/// Compares like pi's `a.toLowerCase().localeCompare(b.toLowerCase())`.
#[must_use]
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    let (a_primary, a_accents) = keys(a);
    let (b_primary, b_accents) = keys(b);
    let case = |text: &str| text.chars().map(char::is_uppercase).collect::<Vec<_>>();
    a_primary
        .cmp(&b_primary)
        .then_with(|| a_accents.cmp(&b_accents))
        .then_with(|| case(a).cmp(&case(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_digits_letters() {
        // Expected order from Node 24 `localeCompare`.
        let mut items = vec![
            "b", "_a", ".hidden", "10", "2", "a-b", "ab", "é", "e", "f", "z", "ß", "ø", "o", "p",
            "a b", "a.b", "résumé", "resume", "x1", "x10", "x2", "~t", "日本",
        ];
        items.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            items,
            vec![
                "_a", ".hidden", "~t", "10", "2", "a b", "a-b", "a.b", "ab", "b", "e", "é", "f",
                "o", "ø", "p", "resume", "résumé", "ß", "x1", "x10", "x2", "z", "日本",
            ]
        );
    }
}
