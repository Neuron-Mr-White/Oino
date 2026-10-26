//! OUS tones → tokens (design.md §2.2) and OUS icon names → Lucide (§2.9). Toolkit-free.

use crate::design::tokens::Palette;
use crate::model::ous::Tone;

/// Text / icon color of `tone` (the `_fg` column keeps 4.5:1 on surfaces).
#[must_use]
pub fn fg(tone: Tone, p: &Palette) -> u32 {
    match tone {
        Tone::Default | Tone::Assistant | Tone::Accent | Tone::User => p.fg,
        Tone::Muted => p.fg_subtle,
        Tone::Thinking => p.thinking,
        Tone::Info | Tone::Tool => p.progress_surface_fg,
        Tone::Success => p.success_surface_fg,
        Tone::Warning => p.warning_surface_fg,
        Tone::Error => p.danger_surface_fg,
        Tone::Harness => p.harness,
        Tone::Palette(i) => p.categorical[usize::from(i) % 12],
    }
}

/// Rail / bar / dot / meter color of `tone` (saturated column).
#[must_use]
pub fn rail(tone: Tone, p: &Palette) -> u32 {
    match tone {
        Tone::Default | Tone::Assistant => p.border_hover,
        Tone::Muted | Tone::Thinking => p.border,
        Tone::Accent => p.brand,
        Tone::User => p.fg_subtlest,
        Tone::Info | Tone::Tool => p.ansi[12],
        Tone::Success => p.success,
        Tone::Warning => p.warning,
        Tone::Error => p.danger,
        Tone::Harness => p.harness,
        Tone::Palette(i) => p.categorical[usize::from(i) % 12],
    }
}

/// Badge background of `tone`.
#[must_use]
pub fn surface(tone: Tone, p: &Palette) -> u32 {
    match tone {
        Tone::Default | Tone::Assistant => p.tag,
        Tone::Muted | Tone::Thinking | Tone::User => p.surface,
        Tone::Accent => p.accent,
        Tone::Info | Tone::Tool => p.progress_surface,
        Tone::Success => p.success_surface,
        Tone::Warning => p.warning_surface,
        Tone::Error => p.danger_surface,
        Tone::Harness => p.harness_surface,
        Tone::Palette(i) => (p.categorical[usize::from(i) % 12] & 0xFFFF_FF00) | 0x1F,
    }
}

/// Whether text in `tone` is rendered at medium weight (`accent`).
#[must_use]
pub const fn emphasised(tone: Tone) -> bool {
    matches!(tone, Tone::Accent)
}

/// Lucide icon file stem for an OUS icon name (ui-protocol §6.5); unknown → `None`.
#[must_use]
pub fn ous_icon(name: &str) -> Option<&'static str> {
    Some(match name {
        "check" => "check",
        "x" => "x",
        "dot" => "dot",
        "warn" => "triangle-alert",
        "info" => "info",
        "clock" => "clock",
        "gear" => "settings",
        "branch" => "git-branch",
        "bolt" => "zap",
        "bot" => "bot",
        "folder" => "folder",
        "file" => "file",
        "search" => "search",
        "cloud" => "cloud",
        "lock" => "lock",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::tokens::{DARK, LIGHT};

    #[test]
    fn tones_follow_the_table() {
        assert_eq!(fg(Tone::Error, &LIGHT), LIGHT.danger_surface_fg);
        assert_eq!(rail(Tone::Error, &LIGHT), LIGHT.danger);
        assert_eq!(surface(Tone::Info, &DARK), DARK.progress_surface);
        assert_eq!(rail(Tone::Accent, &DARK), DARK.brand);
        assert_eq!(surface(Tone::Palette(0), &LIGHT), 0x0B7F_FF1F);
        assert_eq!(fg(Tone::Palette(13), &LIGHT), LIGHT.categorical[1]);
        assert_eq!(ous_icon("warn"), Some("triangle-alert"));
        assert_eq!(ous_icon("nope"), None);
    }
}
