//! Phone layouts (feature `mobile`, gpui-app §4; owned by gpui-mobile).
//!
//! The mobile shell reuses the desktop building blocks — [`crate::model`] view models,
//! [`crate::views::session_pane::SessionPane`] (conversation, composer, dialogs, panels) and
//! the theme — and replaces the window chrome with a stacked, touch-first layout:
//!
//! ```text
//! ┌ status bar inset ─────────────┐   Sessions ─tap─▶ Conversation ─⋯─▶ panel / diff sheet
//! │ ‹  title · model ▾      ■  ⋯ │        │                 │
//! │ conversation (virtualised)    │        └──── ⚙ ──▶ Settings
//! │ composer                      │   Back (Android back = `escape`) pops sheet → screen.
//! └ keyboard / nav-bar inset ─────┘
//! ```
//!
//! The platform crate (`oino-mobile`) installs a [`MobilePlatform`] global with the soft
//! keyboard and safe-area hooks so this crate stays free of platform dependencies and keeps
//! building (and testing) on desktop hosts.

mod shell;

use gpui_kit::{App, Global};

pub use shell::MobileShell;

/// Safe-area insets in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    /// Status bar / notch.
    pub top: f32,
    /// Home indicator / navigation bar.
    pub bottom: f32,
    /// Left cut-out.
    pub left: f32,
    /// Right cut-out.
    pub right: f32,
}

/// Platform hooks the shell calls (installed by the app crate; no-ops by default).
#[derive(Clone, Copy, Debug)]
pub struct MobilePlatform {
    /// Shows the soft keyboard (a text field gained focus).
    pub show_keyboard: fn(),
    /// Hides the soft keyboard.
    pub hide_keyboard: fn(),
    /// Current safe-area insets.
    pub safe_area: fn() -> Insets,
    /// Current soft keyboard height (0 when hidden), logical pixels.
    pub keyboard_height: fn() -> f32,
}

fn no_op() {}

fn no_insets() -> Insets {
    Insets::default()
}

fn no_keyboard() -> f32 {
    0.0
}

impl Default for MobilePlatform {
    fn default() -> Self {
        Self {
            show_keyboard: no_op,
            hide_keyboard: no_op,
            safe_area: no_insets,
            keyboard_height: no_keyboard,
        }
    }
}

impl Global for MobilePlatform {}

impl MobilePlatform {
    /// Installs the hooks.
    pub fn init(self, cx: &mut App) {
        cx.set_global(self);
    }

    /// The installed hooks (defaults when none were installed).
    #[must_use]
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hooks_are_inert() {
        let hooks = MobilePlatform::default();
        (hooks.show_keyboard)();
        (hooks.hide_keyboard)();
        assert_eq!((hooks.safe_area)(), Insets::default());
        assert!((hooks.keyboard_height)().abs() < f32::EPSILON);
    }
}
