//! JavaScript semantics the pi tools rely on: number formatting, `toFixed`, UTF-16 lengths and
//! `String.prototype.trimEnd`.

/// JavaScript `Number.prototype.toString()` for the values tools print (counts, offsets,
/// timeouts).
#[must_use]
pub(crate) fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let magnitude = value.abs();
    if (1e-6..1e21).contains(&magnitude) {
        // Rust's shortest round-trip representation matches JavaScript in this range.
        return format!("{value}");
    }
    // Exponent form: `1e+21`, `1.5e-7`.
    let formatted = format!("{value:e}");
    match formatted.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => formatted,
    }
}

/// JavaScript `Number.prototype.toFixed(digits)` for finite non-negative values below `1e21`
/// (ties round up, on the exact binary value).
#[must_use]
pub(crate) fn to_fixed(value: f64, digits: usize) -> String {
    let negative = value < 0.0;
    let exact = format!("{:.60}", value.abs());
    let Some((int_part, frac_part)) = exact.split_once('.') else {
        return exact;
    };
    let mut kept: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes().take(digits))
        .collect();
    let round_up = frac_part
        .as_bytes()
        .get(digits)
        .is_some_and(|digit| *digit >= b'5');
    if round_up {
        let mut index = kept.len();
        loop {
            if index == 0 {
                kept.insert(0, b'1');
                break;
            }
            index -= 1;
            if kept[index] == b'9' {
                kept[index] = b'0';
            } else {
                kept[index] += 1;
                break;
            }
        }
    }
    let int_len = kept.len() - digits;
    let mut out = String::with_capacity(kept.len() + 2);
    if negative && kept.iter().any(|digit| *digit != b'0') {
        out.push('-');
    }
    out.push_str(&String::from_utf8_lossy(&kept[..int_len]));
    if digits > 0 {
        out.push('.');
        out.push_str(&String::from_utf8_lossy(&kept[int_len..]));
    }
    out
}

/// A count as a JavaScript number.
#[must_use]
pub(crate) fn count_f64(value: usize) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "counts are far below 2^53")]
    let value = value as f64;
    value
}

/// JavaScript `string.length` (UTF-16 code units).
#[must_use]
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// JavaScript `string.slice(0, units)` measured in UTF-16 code units. A surrogate pair cut in
/// half leaves a lone surrogate in JavaScript, which becomes U+FFFD once encoded as UTF-8.
#[must_use]
pub(crate) fn utf16_prefix(text: &str, units: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = ch.len_utf16();
        if used + width > units {
            if used < units {
                out.push(char::REPLACEMENT_CHARACTER);
            }
            break;
        }
        out.push(ch);
        used += width;
    }
    out
}

/// Whether JavaScript's `\s` / `trim` treat `ch` as whitespace (`WhiteSpace` + `LineTerminator`).
#[must_use]
pub(crate) fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JavaScript `String.prototype.trimEnd`.
#[must_use]
pub(crate) fn trim_end(text: &str) -> &str {
    text.trim_end_matches(is_js_whitespace)
}

/// JavaScript `String.prototype.trim`.
#[must_use]
pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_javascript() {
        assert_eq!(number_to_string(3.0), "3");
        assert_eq!(number_to_string(0.5), "0.5");
        assert_eq!(number_to_string(1e21), "1e+21");
        assert_eq!(number_to_string(1.5e-7), "1.5e-7");
        assert_eq!(number_to_string(f64::INFINITY), "Infinity");
        assert_eq!(number_to_string(-2.0), "-2");
    }

    #[test]
    fn to_fixed_rounds_ties_up() {
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(1.05, 1), "1.1");
        assert_eq!(to_fixed(50.0, 1), "50.0");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        assert_eq!(to_fixed(1.5, 2), "1.50");
        assert_eq!(to_fixed(0.0, 1), "0.0");
    }

    #[test]
    fn utf16_helpers() {
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(utf16_prefix("a😀b", 2), "a\u{fffd}");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
        assert_eq!(trim_end("x \u{feff}\u{a0}"), "x");
    }
}
