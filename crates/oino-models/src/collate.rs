//! An approximation of JavaScript's `String.prototype.localeCompare` (ICU root collation) for
//! model ids: punctuation < digits < letters, letters compared case-insensitively first, then
//! lowercase before uppercase, then by code point.

use std::cmp::Ordering;

/// ICU root order of ASCII punctuation and symbols (all sort before digits and letters).
const PUNCTUATION_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

fn primary(c: char) -> (u8, u32) {
    if let Some(index) = PUNCTUATION_ORDER.find(c) {
        return (0, u32::try_from(index).unwrap_or(u32::MAX));
    }
    if c.is_ascii_digit() {
        return (1, u32::from(c));
    }
    if c.is_alphabetic() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        return (2, u32::from(lower));
    }
    if c.is_ascii() {
        // Remaining ASCII controls/symbols: after the known punctuation, before digits.
        return (0, 1000 + u32::from(c));
    }
    (3, u32::from(c))
}

fn tertiary(c: char) -> u8 {
    u8::from(c.is_uppercase())
}

/// Compares like `a.localeCompare(b)` for the identifiers pi sorts.
#[must_use]
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    a.chars()
        .map(primary)
        .cmp(b.chars().map(primary))
        .then_with(|| a.chars().map(tertiary).cmp(b.chars().map(tertiary)))
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_v8_order() {
        let mut ids = vec![
            "a-b", "a_b", "ab", "a.b", "aB", "Ab", "a1", "a-1", "a10", "a9", "A", "a",
        ];
        ids.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            ids,
            vec![
                "a", "A", "a_b", "a-1", "a-b", "a.b", "a1", "a10", "a9", "ab", "aB", "Ab"
            ]
        );
    }
}
