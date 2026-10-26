//! Namespace `input-shortcuts` (unipi `src/settings.ts`): stored keys, shown in the info
//! screen; like unipi the shortcuts stay registered on their default keys (users rebind them
//! through `keybindings.json` `extensionShortcuts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, plain};

/// Namespace.
pub const NAMESPACE: &str = "input-shortcuts";
/// Default chord key.
pub const CHORD_KEY: &str = "alt+s";
/// Default tab-insert key.
pub const TAB_INSERT_KEY: &str = "alt+i";

const KEYBINDING_HINT: &str = "format mod+key — mods: ctrl alt shift super (combine: ctrl+shift+x) · keys: a-z 0-9 f1-f12 space tab enter escape up down left right home end pageup pagedown";

const MODS: &[&str] = &["ctrl", "alt", "shift", "super"];
const SYMBOLS: &str = "`-=[]\\;',./!@#$%^&*()_+|~{}:<>?";
const NAMED: &[&str] = &[
    "escape",
    "esc",
    "enter",
    "return",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "clear",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
];

/// Defaults.
#[must_use]
pub fn defaults() -> Value {
    json!({"chordKey": CHORD_KEY, "tabInsertKey": TAB_INSERT_KEY})
}

/// unipi `validateKeybinding`: `None` when `raw` is a valid pi-tui key id (`mod+key`).
#[must_use]
pub fn validate_keybinding(raw: &str) -> Option<String> {
    let lower = raw.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    let (key, mods) = parts.split_last().map_or(("", &[][..]), |(k, m)| (*k, m));
    if mods.is_empty() || key.is_empty() {
        return Some(format!(
            "invalid key \"{raw}\" — expected mod+key like alt+s"
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for m in mods {
        if !MODS.contains(m) || seen.contains(m) {
            return Some(format!(
                "invalid key \"{raw}\" — mods: ctrl alt shift super"
            ));
        }
        seen.push(m);
    }
    let single = key.len() == 1
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let function = key
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n))
        && !key.starts_with("f0");
    let symbol = key.chars().count() == 1 && SYMBOLS.contains(key);
    if !single && !function && !symbol && !NAMED.contains(&key) {
        return Some(format!(
            "invalid key \"{raw}\" — keys: a-z 0-9 f1-f12 or a named key (space, enter, escape, …)"
        ));
    }
    None
}

/// A stored key, or `default` when missing/invalid.
#[must_use]
pub fn key_or_default(value: Option<&Value>, default: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|k| validate_keybinding(k).is_none())
        .unwrap_or(default)
        .to_owned()
}

fn key_field(key: &str, label: &str, options: &[&str], description: &str) -> Field {
    Field::enumeration(key, label, options.iter().map(|o| plain(o)).collect())
        .description(description)
        .hint(KEYBINDING_HINT)
        .with_kind(|k| {
            if let Kind::Enum { allow_custom, .. } = k {
                *allow_custom = true;
            }
        })
}

/// Schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Input Shortcuts", crate::EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("keys", "Keys")
                .description("Key names as pi keybinding ids (e.g. alt+s)")
                .field(key_field(
                    "chordKey",
                    "Chord key",
                    &["alt+s", "alt+d", "alt+x"],
                    "Key that opens the shortcuts chord overlay.",
                ))
                .field(key_field(
                    "tabInsertKey",
                    "Tab-insert key",
                    &["alt+i", "alt+o", "alt+p"],
                    "Key that inserts a tab in the editor.",
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keybinding_grammar() {
        for ok in [
            "alt+s",
            "ctrl+shift+x",
            "ALT+F5",
            "super+space",
            "alt+/",
            "ctrl+0",
            "shift+pageup",
        ] {
            assert_eq!(validate_keybinding(ok), None, "{ok}");
        }
        assert!(validate_keybinding("s").is_some_and(|e| e.contains("expected mod+key")));
        assert!(validate_keybinding("alt+").is_some());
        assert!(validate_keybinding("meta+s").is_some_and(|e| e.contains("mods:")));
        assert!(validate_keybinding("alt+alt+s").is_some());
        assert!(validate_keybinding("alt+f13").is_some_and(|e| e.contains("keys:")));
        assert!(validate_keybinding("alt+foo").is_some());
        assert_eq!(key_or_default(Some(&json!("bogus")), CHORD_KEY), "alt+s");
        assert_eq!(key_or_default(Some(&json!("alt+d")), CHORD_KEY), "alt+d");
    }

    #[test]
    fn schema_matches_unipi() {
        let v = namespace().to_json();
        assert_eq!(v["title"], "Input Shortcuts");
        let s = &v["sections"][0];
        assert_eq!(s["title"], "Keys");
        assert_eq!(s["fields"][0]["key"], "chordKey");
        assert_eq!(s["fields"][0]["type"], "enum");
        assert_eq!(s["fields"][0]["allowCustom"], true);
        assert_eq!(s["fields"][0]["default"], "alt+s");
        assert_eq!(s["fields"][1]["options"][2]["value"], "alt+p");
        assert_eq!(s["fields"][1]["hint"], KEYBINDING_HINT);
    }
}
