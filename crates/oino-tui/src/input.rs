//! Terminal key events → pi [`KeyId`]s.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use oino_view::keys::{Key, KeyId, Modifiers};

/// Converts a crossterm key press; `None` for releases and keys pi has no id for.
#[must_use]
pub fn key_id(event: &KeyEvent) -> Option<KeyId> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    let mut mods = Modifiers {
        ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
        shift: event.modifiers.contains(KeyModifiers::SHIFT),
        alt: event.modifiers.contains(KeyModifiers::ALT),
        super_key: event.modifiers.contains(KeyModifiers::SUPER),
    };
    let key = match event.code {
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Char(c) => {
            if c.is_ascii_uppercase() {
                mods.shift = true;
                Key::Char(c.to_ascii_lowercase())
            } else {
                if !c.is_ascii_alphanumeric() && !mods.ctrl && !mods.alt {
                    // Shifted symbols (`!`, `?`) are their own keys in pi.
                    mods.shift = false;
                }
                Key::Char(c)
            }
        }
        KeyCode::Esc => Key::Escape,
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => {
            mods.shift = true;
            Key::Tab
        }
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Insert => Key::Insert,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::F(n) if (1..=12).contains(&n) => Key::F(n),
        _ => return None,
    };
    Some(KeyId::new(key, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_shift_and_symbols() {
        let upper = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
        assert_eq!(key_id(&upper), Some("shift+l".parse().unwrap()));
        let bang = KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT);
        assert_eq!(key_id(&bang), Some("!".parse().unwrap()));
        let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(key_id(&backtab), Some("shift+tab".parse().unwrap()));
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(key_id(&ctrl_p), Some("ctrl+p".parse().unwrap()));
    }
}
