//! Theme tokens: OUS tones (ui-protocol §6.5) mapped onto gpui-kit theme colors, plus the
//! app's light/dark switch.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Window, hsla};

use crate::host::ThemePref;
use crate::model::ous::Tone;

/// Categorical palette (`c1`…`c12`), tuned to read on both light and dark backgrounds.
const PALETTE: [(f32, f32, f32); 12] = [
    (0.58, 0.65, 0.55),
    (0.08, 0.75, 0.55),
    (0.33, 0.55, 0.45),
    (0.95, 0.65, 0.58),
    (0.75, 0.50, 0.62),
    (0.50, 0.60, 0.45),
    (0.13, 0.80, 0.50),
    (0.68, 0.45, 0.60),
    (0.02, 0.60, 0.55),
    (0.42, 0.50, 0.45),
    (0.88, 0.55, 0.60),
    (0.60, 0.20, 0.55),
];

/// Foreground color for `tone`.
#[must_use]
pub fn tone_color(tone: Tone, cx: &App) -> Hsla {
    let th = cx.theme();
    match tone {
        Tone::Default | Tone::Assistant => th.foreground,
        Tone::Muted | Tone::Thinking => th.muted_foreground,
        Tone::Accent | Tone::User => th.primary,
        Tone::Info | Tone::Tool => th.info,
        Tone::Success => th.success,
        Tone::Warning => th.warning,
        Tone::Error => th.danger,
        Tone::Harness => harness(),
        Tone::Palette(i) => {
            let (h, s, l) = PALETTE[usize::from(i) % PALETTE.len()];
            hsla(h, s, l, 1.0)
        }
    }
}

/// The violet provenance rail color.
#[must_use]
pub fn harness() -> Hsla {
    hsla(0.75, 0.55, 0.62, 1.0)
}

/// Modal backdrop.
#[must_use]
pub fn scrim() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.45)
}

/// Background for added diff lines.
#[must_use]
pub fn diff_added_bg(cx: &App) -> Hsla {
    cx.theme().success.opacity(0.14)
}

/// Background for removed diff lines.
#[must_use]
pub fn diff_removed_bg(cx: &App) -> Hsla {
    cx.theme().danger.opacity(0.14)
}

/// Applies the theme preference.
pub fn apply(pref: ThemePref, window: Option<&mut Window>, cx: &mut App) {
    let mode = match pref {
        ThemePref::Light => ThemeMode::Light,
        ThemePref::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, window, cx);
}
