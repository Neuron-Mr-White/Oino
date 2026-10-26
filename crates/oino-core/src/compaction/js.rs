//! JavaScript string semantics the pi algorithms depend on: UTF-16 lengths, UTF-16 slicing,
//! default `Array#sort` order and `JSON.stringify` output.

use std::cmp::Ordering;
use std::fmt::Write as _;

use oino_ai::JsonValue;

/// `string.length` (UTF-16 code units).
pub(crate) fn len16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `string.slice(0, units)` rounded down to a character boundary (Rust strings cannot hold a
/// lone surrogate).
pub(crate) fn prefix16(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, ch) in text.char_indices() {
        used += ch.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

/// Default `Array#sort` comparator for strings (UTF-16 code unit order).
pub(crate) fn cmp16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `JSON.stringify(value)` for JSON data (no indentation).
pub(crate) fn stringify(value: &JsonValue) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &JsonValue) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        JsonValue::Number(number) => write_number(out, number),
        JsonValue::String(text) => write_string(out, text),
        JsonValue::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        JsonValue::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, item);
            }
            out.push('}');
        }
    }
}

fn write_number(out: &mut String, number: &serde_json::Number) {
    if number.is_i64() || number.is_u64() {
        let _ = write!(out, "{number}");
        return;
    }
    let Some(float) = number.as_f64() else {
        out.push_str("null");
        return;
    };
    if !float.is_finite() {
        out.push_str("null");
    } else if float.fract() == 0.0 && float.abs() < 1e21 {
        // JS prints integral doubles without a fraction (`1.0` → `1`, `-0` → `0`).
        let _ = write!(out, "{float:.0}");
        if out.ends_with("-0") {
            out.truncate(out.len() - 2);
            out.push('0');
        }
    } else {
        // serde_json uses the shortest round-trip form; JS writes `e+` for positive exponents.
        let text = number.to_string();
        match text.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with(['-', '+']) => {
                let _ = write!(out, "{mantissa}e+{exponent}");
            }
            _ => out.push_str(&text),
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if u32::from(ch) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(ch));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn utf16_helpers() {
        assert_eq!(len16("a😀é"), 4);
        assert_eq!(prefix16("a😀b", 2), "a");
        assert_eq!(prefix16("a😀b", 3), "a😀");
        assert_eq!(prefix16("ab", 9), "ab");
        let mut list = vec!["b", "B", "a", "é", "😀", "\u{ffff}"];
        list.sort_by(|a, b| cmp16(a, b));
        assert_eq!(list, ["B", "a", "b", "é", "😀", "\u{ffff}"]);
    }

    #[test]
    fn stringify_matches_js() {
        let value = json!({"s": "q\"\n\u{1}ü", "n": 1.0, "f": 1.5, "i": -3, "z": null,
            "a": [true, false], "big": 1e21, "small": 1e-7, "neg": -0.0});
        assert_eq!(
            stringify(&value),
            r#"{"s":"q\"\n\u0001ü","n":1,"f":1.5,"i":-3,"z":null,"a":[true,false],"big":1e+21,"small":1e-7,"neg":0}"#
        );
    }
}
