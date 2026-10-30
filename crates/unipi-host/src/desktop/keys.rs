//! Keysyms (what the phone sends, X11 rules) → what the macOS / Windows
//! input backends inject. Pure and compiled everywhere so it is tested on
//! Linux CI.
//!
//! Rule: printable characters are typed as **Unicode text** (any layout, any
//! script, uppercase without faking Shift) — except while Ctrl/Alt/Super is
//! held by the phone, when they become **key codes** so shortcuts work
//! (Ctrl+C, Cmd+Tab…). Everything else maps to a named key.

/// Keys with no character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    Return,
    Tab,
    Escape,
    BackSpace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Up,
    Right,
    Down,
    /// F1..F12
    F(u8),
    ShiftL,
    ShiftR,
    ControlL,
    ControlR,
    AltL,
    AltR,
    /// Super / Windows / Command.
    SuperL,
    SuperR,
    CapsLock,
}

impl Special {
    pub fn from_keysym(keysym: u32) -> Option<Special> {
        Some(match keysym {
            0xff0d | 0xff8d => Special::Return,
            0xff09 => Special::Tab,
            0xff1b => Special::Escape,
            0xff08 => Special::BackSpace,
            0xffff | 0xff9f => Special::Delete,
            0xff63 => Special::Insert,
            0xff50 => Special::Home,
            0xff57 => Special::End,
            0xff55 => Special::PageUp,
            0xff56 => Special::PageDown,
            0xff51 => Special::Left,
            0xff52 => Special::Up,
            0xff53 => Special::Right,
            0xff54 => Special::Down,
            k @ 0xffbe..=0xffc9 => Special::F((k - 0xffbe + 1) as u8),
            0xffe1 => Special::ShiftL,
            0xffe2 => Special::ShiftR,
            0xffe3 => Special::ControlL,
            0xffe4 => Special::ControlR,
            0xffe9 | 0xffe7 => Special::AltL,
            0xffea | 0xffe8 | 0xfe03 => Special::AltR,
            0xffeb => Special::SuperL,
            0xffec => Special::SuperR,
            0xffe5 => Special::CapsLock,
            _ => return None,
        })
    }

    fn chord_modifier(self) -> bool {
        matches!(self, Special::ControlL | Special::ControlR | Special::AltL | Special::AltR | Special::SuperL | Special::SuperR)
    }
}

/// The character a keysym types (Latin-1 1:1, `0x0100_0000 + code point`).
pub fn keysym_char(keysym: u32) -> Option<char> {
    match keysym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keysym),
        0x0100_0000..=0x0110_ffff => char::from_u32(keysym - 0x0100_0000),
        _ => None,
    }
}

/// One thing to inject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Special { key: Special, down: bool },
    /// Type `ch` as text (a down event only: text has no "up").
    Text { ch: char },
    /// `ch`'s key on the keyboard (a chord with a held modifier).
    Chord { ch: char, down: bool },
}

/// Tracks the chord modifiers the phone holds.
#[derive(Debug, Default, Clone)]
pub struct KeyPlanner {
    held: Vec<Special>,
}

impl KeyPlanner {
    pub fn plan(&mut self, keysym: u32, down: bool) -> Option<KeyAction> {
        if let Some(key) = Special::from_keysym(keysym) {
            if key.chord_modifier() {
                self.held.retain(|k| *k != key);
                if down {
                    self.held.push(key);
                }
            }
            return Some(KeyAction::Special { key, down });
        }
        let ch = keysym_char(keysym)?;
        if !self.held.is_empty() {
            return Some(KeyAction::Chord { ch: ch.to_ascii_lowercase(), down });
        }
        down.then_some(KeyAction::Text { ch })
    }

    /// Modifiers still held (release them when the viewer leaves).
    pub fn held(&self) -> &[Special] {
        &self.held
    }
}

/// macOS virtual key codes (`kVK_*`, Carbon `Events.h`).
pub mod mac {
    use super::Special;

    pub fn special(key: Special) -> u16 {
        match key {
            Special::Return => 36,
            Special::Tab => 48,
            Special::Escape => 53,
            Special::BackSpace => 51,
            Special::Delete => 117,
            Special::Insert => 114, // Help
            Special::Home => 115,
            Special::End => 119,
            Special::PageUp => 116,
            Special::PageDown => 121,
            Special::Left => 123,
            Special::Right => 124,
            Special::Down => 125,
            Special::Up => 126,
            Special::F(n) => [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111][(n.clamp(1, 12) - 1) as usize],
            Special::ShiftL => 56,
            Special::ShiftR => 60,
            Special::ControlL => 59,
            Special::ControlR => 62,
            Special::AltL => 58,
            Special::AltR => 61,
            Special::SuperL | Special::SuperR => 55,
            Special::CapsLock => 57,
        }
    }

    /// The ANSI key for a character (chords only).
    pub fn char_key(ch: char) -> Option<u16> {
        Some(match ch.to_ascii_lowercase() {
            'a' => 0,
            's' => 1,
            'd' => 2,
            'f' => 3,
            'h' => 4,
            'g' => 5,
            'z' => 6,
            'x' => 7,
            'c' => 8,
            'v' => 9,
            'b' => 11,
            'q' => 12,
            'w' => 13,
            'e' => 14,
            'r' => 15,
            'y' => 16,
            't' => 17,
            '1' => 18,
            '2' => 19,
            '3' => 20,
            '4' => 21,
            '6' => 22,
            '5' => 23,
            '=' => 24,
            '9' => 25,
            '7' => 26,
            '-' => 27,
            '8' => 28,
            '0' => 29,
            ']' => 30,
            'o' => 31,
            'u' => 32,
            '[' => 33,
            'i' => 34,
            'p' => 35,
            'l' => 37,
            'j' => 38,
            '\'' => 39,
            'k' => 40,
            ';' => 41,
            '\\' => 42,
            ',' => 43,
            '/' => 44,
            'n' => 45,
            'm' => 46,
            '.' => 47,
            ' ' => 49,
            '`' => 50,
            _ => return None,
        })
    }

    /// `CGEventFlags` bit for a held modifier.
    pub fn flag(key: Special) -> u64 {
        match key {
            Special::ShiftL | Special::ShiftR => 0x0002_0000,
            Special::ControlL | Special::ControlR => 0x0004_0000,
            Special::AltL | Special::AltR => 0x0008_0000,
            Special::SuperL | Special::SuperR => 0x0010_0000,
            _ => 0,
        }
    }
}

/// Windows virtual-key codes (`VK_*`).
pub mod win {
    use super::Special;

    /// `(vk, extended)` — extended keys need `KEYEVENTF_EXTENDEDKEY`.
    pub fn special(key: Special) -> (u16, bool) {
        match key {
            Special::Return => (0x0d, false),
            Special::Tab => (0x09, false),
            Special::Escape => (0x1b, false),
            Special::BackSpace => (0x08, false),
            Special::Delete => (0x2e, true),
            Special::Insert => (0x2d, true),
            Special::Home => (0x24, true),
            Special::End => (0x23, true),
            Special::PageUp => (0x21, true),
            Special::PageDown => (0x22, true),
            Special::Left => (0x25, true),
            Special::Up => (0x26, true),
            Special::Right => (0x27, true),
            Special::Down => (0x28, true),
            Special::F(n) => (0x70 + (n.clamp(1, 12) - 1) as u16, false),
            Special::ShiftL => (0xa0, false),
            Special::ShiftR => (0xa1, false),
            Special::ControlL => (0xa2, false),
            Special::ControlR => (0xa3, true),
            Special::AltL => (0xa4, false),
            Special::AltR => (0xa5, true),
            Special::SuperL => (0x5b, true),
            Special::SuperR => (0x5c, true),
            Special::CapsLock => (0x14, false),
        }
    }

    /// Letters and digits are their own VK; punctuation goes through the
    /// layout (`VkKeyScanW`) in the backend.
    pub fn char_vk(ch: char) -> Option<u16> {
        match ch {
            'a'..='z' => Some(ch.to_ascii_uppercase() as u16),
            'A'..='Z' | '0'..='9' | ' ' => Some(ch as u16),
            _ => None,
        }
    }

    /// Absolute pointer position for `SendInput` (0..=65535 over the
    /// primary screen).
    pub fn absolute(x: f64, y: f64) -> (i32, i32) {
        let f = |v: f64| (v.clamp(0.0, 1.0) * 65535.0).round() as i32;
        (f(x), f(y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_unless_a_chord_modifier_is_held() {
        let mut p = KeyPlanner::default();
        assert_eq!(p.plan(0x41, true), Some(KeyAction::Text { ch: 'A' }), "uppercase as text, no Shift needed");
        assert_eq!(p.plan(0x41, false), None, "text has no up");
        assert_eq!(p.plan(0x0100_20ac, true), Some(KeyAction::Text { ch: '€' }));
        assert_eq!(p.plan(0xe9, true), Some(KeyAction::Text { ch: 'é' }));
        // Ctrl+C → a chord on the C key.
        assert_eq!(p.plan(0xffe3, true), Some(KeyAction::Special { key: Special::ControlL, down: true }));
        assert_eq!(p.plan(0x63, true), Some(KeyAction::Chord { ch: 'c', down: true }));
        assert_eq!(p.plan(0x43, false), Some(KeyAction::Chord { ch: 'c', down: false }));
        assert_eq!(p.held(), &[Special::ControlL]);
        assert_eq!(p.plan(0xffe3, false), Some(KeyAction::Special { key: Special::ControlL, down: false }));
        assert!(p.held().is_empty());
        // Shift alone doesn't make chords (the text already has the case).
        p.plan(0xffe1, true);
        assert_eq!(p.plan(0x21, true), Some(KeyAction::Text { ch: '!' }));
        assert_eq!(p.plan(0xff0d, true), Some(KeyAction::Special { key: Special::Return, down: true }));
        assert_eq!(p.plan(0xffbf, true), Some(KeyAction::Special { key: Special::F(2), down: true }));
        assert_eq!(p.plan(0x12345, true), None, "unknown keysym");
    }

    #[test]
    fn platform_codes() {
        assert_eq!(mac::special(Special::Return), 36);
        assert_eq!(mac::special(Special::F(1)), 122);
        assert_eq!(mac::special(Special::F(12)), 111);
        assert_eq!(mac::char_key('C'), Some(8));
        assert_eq!(mac::char_key('é'), None);
        assert_eq!(mac::flag(Special::SuperL), 0x0010_0000);
        assert_eq!(win::special(Special::F(5)), (0x74, false));
        assert_eq!(win::special(Special::Left), (0x25, true));
        assert_eq!(win::char_vk('c'), Some(0x43));
        assert_eq!(win::char_vk('/'), None);
        assert_eq!(win::absolute(0.0, 1.0), (0, 65535));
        assert_eq!(win::absolute(0.5, 2.0), (32768, 65535));
    }
}
