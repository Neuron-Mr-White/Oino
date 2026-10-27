//! GPUI input → [`BrowserInput`] (browser-control.md §9.2): keystroke names to DOM keys,
//! modifier bits, clipboard chords. Pure helpers; the view dispatches the results.

use oino_extension_api::browser::protocol::view::modifiers as bits;
use oino_extension_api::browser::protocol::{BrowserInput, KeyEventType};

/// GPUI modifier flags (decoupled from GPUI for tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors GPUI's modifier flags"
)]
pub struct Mods {
    /// Control.
    pub control: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// ⌘ / Super.
    pub platform: bool,
}

impl Mods {
    /// CDP modifier bits.
    #[must_use]
    pub fn bits(self) -> u8 {
        let mut b = 0;
        if self.alt {
            b |= bits::ALT;
        }
        if self.control {
            b |= bits::CTRL;
        }
        if self.platform {
            b |= bits::META;
        }
        if self.shift {
            b |= bits::SHIFT;
        }
        b
    }

    /// The platform's "secondary" modifier (⌘ on macOS, Ctrl elsewhere).
    #[must_use]
    pub fn secondary(self) -> bool {
        if cfg!(target_os = "macos") {
            self.platform
        } else {
            self.control
        }
    }
}

/// GPUI key name → DOM `key`.
#[must_use]
pub fn dom_key(gpui: &str, key_char: Option<&str>) -> String {
    let named = match gpui {
        "enter" => "Enter",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "escape" => "Escape",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "insert" => "Insert",
        "space" => " ",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    if let Some(n) = gpui.strip_prefix('f').filter(|n| n.parse::<u8>().is_ok()) {
        return format!("F{n}");
    }
    key_char.map_or_else(|| gpui.to_owned(), str::to_owned)
}

/// What a key-down should do.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyAction {
    /// Forward these events.
    Send(Vec<BrowserInput>),
    /// Paste the OS clipboard.
    Paste,
    /// Copy the page selection to the OS clipboard (`cut`: then delete it).
    Copy {
        /// Also delete.
        cut: bool,
    },
    /// Not for the page (app shortcut).
    Ignore,
}

/// Maps a key-down.
#[must_use]
pub fn key_down(gpui: &str, key_char: Option<&str>, m: Mods) -> KeyAction {
    if m.secondary() && !m.alt {
        match gpui {
            "v" => return KeyAction::Paste,
            "c" => return KeyAction::Copy { cut: false },
            "x" => return KeyAction::Copy { cut: true },
            _ => {}
        }
    }
    // App shortcuts (⌘… on macOS, except editing chords) stay with the app.
    if m.platform && cfg!(target_os = "macos") && !matches!(gpui, "a" | "z" | "y") {
        return KeyAction::Ignore;
    }
    let key = dom_key(gpui, key_char);
    let printable = !m.control && !m.platform && key_char.is_some_and(|c| !c.is_empty());
    let text = if printable {
        key_char.map(str::to_owned)
    } else if key == "Enter" && !m.control && !m.platform {
        Some("\r".to_owned())
    } else {
        None
    };
    KeyAction::Send(vec![BrowserInput::Key {
        event: KeyEventType::Down,
        key: key.clone(),
        text,
        modifiers: m.bits(),
    }])
}

/// Maps a key-up.
#[must_use]
pub fn key_up(gpui: &str, key_char: Option<&str>, m: Mods) -> BrowserInput {
    BrowserInput::Key {
        event: KeyEventType::Up,
        key: dom_key(gpui, key_char),
        text: None,
        modifiers: m.bits(),
    }
}

/// CSS `cursor` → whether to show a pointer / text cursor.
#[must_use]
pub fn cursor_kind(css: &str) -> CursorKind {
    match css {
        "pointer" => CursorKind::Pointer,
        "text" | "vertical-text" => CursorKind::Text,
        "grab" | "grabbing" | "move" => CursorKind::Grab,
        _ => CursorKind::Default,
    }
}

/// GPUI cursor styles we map to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    /// Arrow.
    Default,
    /// Hand.
    Pointer,
    /// I-beam.
    Text,
    /// Open hand.
    Grab,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(dom_key("enter", None), "Enter");
        assert_eq!(dom_key("left", None), "ArrowLeft");
        assert_eq!(dom_key("f5", None), "F5");
        assert_eq!(dom_key("a", Some("A")), "A");
        let none = Mods::default();
        assert_eq!(
            key_down("a", Some("a"), none),
            KeyAction::Send(vec![BrowserInput::Key {
                event: KeyEventType::Down,
                key: "a".into(),
                text: Some("a".into()),
                modifiers: 0,
            }])
        );
        let ctrl = Mods {
            control: true,
            ..Mods::default()
        };
        let secondary = if cfg!(target_os = "macos") {
            Mods {
                platform: true,
                ..Mods::default()
            }
        } else {
            ctrl
        };
        assert_eq!(key_down("v", Some("v"), secondary), KeyAction::Paste);
        assert_eq!(
            key_down("c", Some("c"), secondary),
            KeyAction::Copy { cut: false }
        );
        match key_down("a", Some("a"), ctrl) {
            KeyAction::Send(v) => assert!(matches!(
                &v[0],
                BrowserInput::Key {
                    text: None,
                    modifiers: 2,
                    ..
                }
            )),
            other => panic!("{other:?}"),
        }
        assert_eq!(cursor_kind("pointer"), CursorKind::Pointer);
        assert_eq!(cursor_kind("auto"), CursorKind::Default);
    }
}
