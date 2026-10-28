//! Building the real platform events the driver injects (docs/dev/v1/gui-driver.md §3).
//!
//! Every event goes through `Window::dispatch_event`, the same entry point the X11 / Wayland /
//! macOS / Windows backends call, so hit testing, focus transfer, click synthesis, keymap
//! dispatch and IME all behave exactly as they do for a person at the machine.

use gpui_kit::{
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    OperatingSystem, PlatformInput, Point, ScrollDelta, ScrollEvent, ScrollSource, TouchPhase,
    point, px,
};

/// A keyboard event for `spec` (`enter`, `secondary-k`, `shift-tab`, …), with GPUI's
/// platform-aware `secondary-` modifier resolved from `os`.
///
/// # Errors
///
/// `Err` carries GPUI's parse message for a spec it cannot read.
pub fn keystroke(spec: &str, os: OperatingSystem) -> Result<Keystroke, String> {
    // Scenarios and the docs spell chords with `+` (`alt+h`, `shift-secondary-p`); GPUI's
    // parser wants `-` between modifiers. Accept both, and leave a lone `+` key alone.
    let normalized = if spec == "+" {
        spec.to_owned()
    } else {
        spec.replace('+', "-")
    };
    Keystroke::parse_with_os(&normalized, os).map_err(|e| e.to_string())
}

/// The keystrokes that type `text`: one per character, the way a keyboard produces them
/// (`key` is the unshifted key, `key_char` what it types, `shift` for capitals).
#[must_use]
pub fn keystrokes_for(text: &str) -> Vec<Keystroke> {
    text.chars()
        .map(|c| {
            let (key, shift) = match c {
                ' ' => ("space".to_owned(), false),
                '\n' => ("enter".to_owned(), false),
                '\t' => ("tab".to_owned(), false),
                c if c.is_ascii_uppercase() => (c.to_lowercase().to_string(), true),
                c => (c.to_string(), false),
            };
            Keystroke {
                modifiers: Modifiers {
                    shift,
                    ..Modifiers::default()
                },
                key,
                key_char: Some(c.to_string()),
            }
        })
        .collect()
}

/// A pointer move to `(x, y)` (window px), with no button held.
#[must_use]
pub fn mouse_move(x: f32, y: f32) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position: point(px(x), px(y)),
        pressed_button: None,
        modifiers: Modifiers::default(),
    })
}

/// A pointer press at `(x, y)`; `click_count` is 2 for the second press of a double click.
#[must_use]
pub fn mouse_down(x: f32, y: f32, button: MouseButton, click_count: usize) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        button,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count,
        first_mouse: false,
    })
}

/// A pointer release at `(x, y)`.
#[must_use]
pub fn mouse_up(x: f32, y: f32, button: MouseButton, click_count: usize) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent {
        button,
        position: point(px(x), px(y)),
        modifiers: Modifiers::default(),
        click_count,
    })
}

/// A scroll of `dy` lines (positive = down) at `(x, y)`.
#[must_use]
pub fn scroll(x: f32, y: f32, dx: f32, dy: f32) -> PlatformInput {
    PlatformInput::Scroll(ScrollEvent {
        position: Point { x: px(x), y: px(y) },
        delta: ScrollDelta::Lines(Point { x: dx, y: dy }),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
        source: ScrollSource::Pointer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_resolves_per_os() {
        let mac = keystroke("secondary-k", OperatingSystem::Mac).expect("parse");
        assert!(mac.modifiers.platform && !mac.modifiers.control);
        let linux = keystroke("secondary-k", OperatingSystem::Linux).expect("parse");
        assert!(linux.modifiers.control && !linux.modifiers.platform);
        assert_eq!(linux.key, "k");
    }

    #[test]
    fn typing_produces_one_keystroke_per_char() {
        let keys = keystrokes_for("Hi there\n");
        assert_eq!(keys.len(), 9);
        assert_eq!(keys[0].key, "h");
        assert!(keys[0].modifiers.shift);
        assert_eq!(keys[0].key_char.as_deref(), Some("H"));
        assert_eq!(keys[1].key, "i");
        assert_eq!(keys[2].key, "space");
        assert_eq!(keys[2].key_char.as_deref(), Some(" "));
        assert_eq!(keys[8].key, "enter");
    }
}

#[cfg(test)]
mod plus_tests {
    use super::*;

    #[test]
    fn chord_specs_accept_plus_and_dash() {
        let os = OperatingSystem::Linux;
        let plus = keystroke("alt+h", os).unwrap();
        let dash = keystroke("alt-h", os).unwrap();
        assert_eq!(plus, dash);
        assert!(plus.modifiers.alt);
        assert_eq!(plus.key, "h");
    }
}
