//! Playwright key names → CDP `Input.dispatchKeyEvent` fields, shared by `browser_press_key`
//! and viewer input (browser-control.md §7.2).

use oino_browser_protocol::view::modifiers;

/// CDP fields of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyDef {
    /// DOM `key`.
    pub(crate) key: String,
    /// DOM `code`.
    pub(crate) code: String,
    /// `windowsVirtualKeyCode`.
    pub(crate) key_code: u32,
    /// Produced text (none for non-printing keys).
    pub(crate) text: Option<String>,
}

const NAMED: &[(&str, &str, u32, Option<&str>)] = &[
    ("Enter", "Enter", 13, Some("\r")),
    ("Tab", "Tab", 9, Some("\t")),
    ("Backspace", "Backspace", 8, None),
    ("Delete", "Delete", 46, None),
    ("Escape", "Escape", 27, None),
    ("ArrowUp", "ArrowUp", 38, None),
    ("ArrowDown", "ArrowDown", 40, None),
    ("ArrowLeft", "ArrowLeft", 37, None),
    ("ArrowRight", "ArrowRight", 39, None),
    ("Home", "Home", 36, None),
    ("End", "End", 35, None),
    ("PageUp", "PageUp", 33, None),
    ("PageDown", "PageDown", 34, None),
    ("Insert", "Insert", 45, None),
    ("Shift", "ShiftLeft", 16, None),
    ("Control", "ControlLeft", 17, None),
    ("Alt", "AltLeft", 18, None),
    ("Meta", "MetaLeft", 91, None),
    ("CapsLock", "CapsLock", 20, None),
    ("ContextMenu", "ContextMenu", 93, None),
];

fn alias(name: &str) -> &str {
    match name {
        "Esc" => "Escape",
        "Return" => "Enter",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "Del" => "Delete",
        "Space" | "Spacebar" => " ",
        "Ctrl" => "Control",
        "Cmd" | "Command" | "Super" => "Meta",
        "Option" => "Alt",
        other => other,
    }
}

/// Looks up one key (no modifiers).
pub(crate) fn key_def(name: &str) -> Option<KeyDef> {
    let name = alias(name);
    if let Some((key, code, kc, text)) = NAMED.iter().find(|(k, ..)| *k == name) {
        return Some(KeyDef {
            key: (*key).to_owned(),
            code: (*code).to_owned(),
            key_code: *kc,
            text: text.map(str::to_owned),
        });
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some(KeyDef {
            key: name.to_owned(),
            code: name.to_owned(),
            key_code: 111 + n,
            text: None,
        });
    }
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(char_def(c))
}

/// Key fields for a printable character.
pub(crate) fn char_def(c: char) -> KeyDef {
    let (code, key_code) = if c.is_ascii_alphabetic() {
        let up = c.to_ascii_uppercase();
        (format!("Key{up}"), u32::from(up))
    } else if c.is_ascii_digit() {
        (format!("Digit{c}"), u32::from(c))
    } else {
        let (code, kc) = match c {
            ' ' => ("Space", 32),
            '-' | '_' => ("Minus", 189),
            '=' | '+' => ("Equal", 187),
            '[' | '{' => ("BracketLeft", 219),
            ']' | '}' => ("BracketRight", 221),
            ';' | ':' => ("Semicolon", 186),
            '\'' | '"' => ("Quote", 222),
            ',' | '<' => ("Comma", 188),
            '.' | '>' => ("Period", 190),
            '/' | '?' => ("Slash", 191),
            '\\' | '|' => ("Backslash", 220),
            '`' | '~' => ("Backquote", 192),
            _ => ("", 0),
        };
        (code.to_owned(), kc)
    };
    KeyDef {
        key: c.to_string(),
        code,
        key_code,
        text: Some(c.to_string()),
    }
}

/// A parsed chord (`Control+Shift+a`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    /// Modifier bits.
    pub(crate) modifiers: u8,
    /// Modifier keys to press first, in order.
    pub(crate) held: Vec<KeyDef>,
    /// The main key.
    pub(crate) key: KeyDef,
}

/// Parses Playwright key syntax; `mac` maps `ControlOrMeta` to Meta.
pub(crate) fn parse_chord(spec: &str, mac: bool) -> Option<Chord> {
    let parts: Vec<&str> = if spec == "+" {
        vec!["+"]
    } else {
        let mut p: Vec<&str> = spec.split('+').collect();
        if spec.ends_with("++") {
            p.pop();
            p.pop();
            p.push("+");
        }
        p
    };
    let (last, mods) = parts.split_last()?;
    let mut bits = 0u8;
    let mut held = Vec::new();
    for m in mods {
        let m = match alias(m) {
            "ControlOrMeta" if mac => "Meta",
            "ControlOrMeta" => "Control",
            other => other,
        };
        bits |= match m {
            "Alt" => modifiers::ALT,
            "Control" => modifiers::CTRL,
            "Meta" => modifiers::META,
            "Shift" => modifiers::SHIFT,
            _ => return None,
        };
        held.push(key_def(m)?);
    }
    let mut key = key_def(last)?;
    if bits & (modifiers::CTRL | modifiers::META | modifiers::ALT) != 0 {
        key.text = None;
    } else if bits & modifiers::SHIFT != 0
        && let Some(t) = key.text.clone()
        && t.len() == 1
    {
        key.key = t.to_uppercase();
        key.text = Some(key.key.clone());
    }
    Some(Chord {
        modifiers: bits,
        held,
        key,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        let c = parse_chord("Control+a", false).unwrap();
        assert_eq!(c.modifiers, modifiers::CTRL);
        assert_eq!(c.key.code, "KeyA");
        assert_eq!(c.key.text, None);
        let c = parse_chord("ControlOrMeta+Shift+Z", true).unwrap();
        assert_eq!(c.modifiers, modifiers::META | modifiers::SHIFT);
        let e = parse_chord("Enter", false).unwrap();
        assert_eq!((e.key.key_code, e.key.text.as_deref()), (13, Some("\r")));
        assert_eq!(parse_chord("Shift+Tab", false).unwrap().held.len(), 1);
        assert_eq!(parse_chord("F5", false).unwrap().key.key_code, 116);
        assert_eq!(parse_chord("Control++", false).unwrap().key.key, "+");
        assert!(parse_chord("Hyper+x", false).is_none());
        assert_eq!(parse_chord("Space", false).unwrap().key.code, "Space");
    }
}
