//! `Badge`, `StatusCapsule`, `Dot`, `Kbd` (design.md §4.2, §4.6).

use gpui_kit::assets::IconName;
use gpui_kit::{
    App, Div, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::model::ous::Tone;

/// Platform key label: `⌘`/`⇧`/`⌥` stay on Apple platforms and become `Ctrl+`/`Shift+`/`Alt+`
/// elsewhere (the primary modifier is never shown as both, design.md §4).
#[must_use]
pub fn platform_keys(keys: &str) -> String {
    if cfg!(target_os = "macos") || !keys.contains(['⌘', '⇧', '⌥', '⌃']) {
        return keys.to_owned();
    }
    let mut out = String::new();
    for c in keys.chars() {
        match c {
            '⌘' | '⌃' => out.push_str("Ctrl+"),
            '⇧' => out.push_str("Shift+"),
            '⌥' => out.push_str("Alt+"),
            other => out.push(other),
        }
    }
    out
}

/// A key hint chip (`Xs` mono on `tag`).
pub fn kbd(t: &OinoTheme, keys: impl Into<SharedString>) -> Div {
    let keys: SharedString = keys.into();
    let keys = SharedString::from(platform_keys(&keys));
    div()
        .flex_shrink_0()
        .px(px(space::S1))
        .rounded(px(radius::R_SM))
        .bg(t.tag())
        .text_color(t.fg_subtle())
        .mono_text(t, TextRole::Xs)
        .child(keys)
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_keys() {
        let k = super::platform_keys("⌘⇧M");
        if cfg!(target_os = "macos") {
            assert_eq!(k, "⌘⇧M");
        } else {
            assert_eq!(k, "Ctrl+Shift+M");
        }
        assert_eq!(super::platform_keys("esc"), "esc");
    }
}

/// A tone badge (`Xs` medium, height 18).
pub fn badge(t: &OinoTheme, tone: Tone, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(size::BADGE))
        .px(px(space::S1_5))
        .rounded(px(radius::R_SM))
        .bg(t.tone_surface(tone))
        .text_color(t.tone_fg(tone))
        .text_role(t, TextRole::Xs)
        .weight_medium()
        .child(text.into())
}

/// A pill badge (counts in group labels, update badge).
pub fn pill(t: &OinoTheme, bg: Hsla, fg: Hsla, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .px(px(space::S1_5))
        .rounded(px(radius::R_FULL))
        .bg(bg)
        .text_color(fg)
        .text_role(t, TextRole::Xs)
        .weight_medium()
        .child(text.into())
}

/// Run/attention state shown as a capsule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capsule {
    /// Agent running.
    Running,
    /// Waiting for the user (approval, question, plan review).
    Waiting,
    /// Failed.
    Error,
    /// Compacting.
    Compacting,
    /// Retrying.
    Retrying,
}

/// A `StatusCapsule` (`R_FULL`, height 22).
pub fn capsule(kind: Capsule, cx: &mut App) -> Div {
    let t = OinoTheme::get(cx);
    let (bg, fg, label) = match kind {
        Capsule::Running => (t.progress_surface(), t.progress_surface_fg(), "Working"),
        Capsule::Waiting => (t.waiting_surface(), t.waiting_fg(), "Needs you"),
        Capsule::Error => (t.danger_surface(), t.danger_surface_fg(), "Error"),
        Capsule::Compacting => (t.progress_surface(), t.progress_surface_fg(), "Compacting"),
        Capsule::Retrying => (t.progress_surface(), t.progress_surface_fg(), "Retrying"),
    };
    let lead = match kind {
        Capsule::Running | Capsule::Compacting | Capsule::Retrying => {
            spinner(Ticker::frame(cx), size::ICON_SM, fg).into_any_element()
        }
        // unipi needs-you: crab eyes `!`, pulsing (A1.2, unipi §1.3).
        Capsule::Waiting => crate::components::unicrab::needs_you_eyes(size::ICON_SM, fg, cx),
        Capsule::Error => icon(IconName::CircleAlert, size::ICON_SM, fg).into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(space::S1))
        .h(px(size::CAPSULE))
        .px(px(space::S2))
        .rounded(px(radius::R_FULL))
        .bg(bg)
        .text_color(fg)
        .text_role(&t, TextRole::Sm)
        .child(lead)
        .child(label)
}

/// Dot state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DotKind {
    /// Working (`claw`).
    Running,
    /// Needs you (`shell`, pulsing).
    Waiting,
    /// Unread finish (`success`).
    Unread,
    /// Error (`danger`).
    Error,
}

/// A 6 px status dot.
#[must_use]
pub fn dot(t: &OinoTheme, kind: DotKind) -> Div {
    let c = match kind {
        DotKind::Running => t.working(),
        DotKind::Waiting => t.brand(),
        DotKind::Unread => t.success(),
        DotKind::Error => t.danger(),
    };
    div()
        .flex_shrink_0()
        .size(px(size::DOT))
        .rounded(px(radius::R_FULL))
        .bg(c)
}

/// A section label: uppercase muted 12/500 (unipi §1.6 `.section-label`, A1.2). Callers
/// write sentence case; the label uppercases it.
pub fn section_label(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    let text: SharedString = text.into();
    div()
        .text_color(t.fg_subtle())
        .text_role(t, TextRole::Sm)
        .weight_medium()
        .child(SharedString::from(text.to_uppercase()))
}

/// A muted metadata text (`Sm` `fg_subtle`).
pub fn meta(t: &OinoTheme, text: impl Into<SharedString>) -> Div {
    div()
        .text_color(t.fg_subtle())
        .text_role(t, TextRole::Sm)
        .child(text.into())
}

/// A divider line.
#[must_use]
pub fn divider(t: &OinoTheme) -> Div {
    div().w_full().h(px(size::HAIRLINE)).bg(t.border())
}

/// An optional tone dot before a label.
#[must_use]
pub fn tone_dot(t: &OinoTheme, tone: Tone) -> Div {
    div()
        .flex_shrink_0()
        .size(px(size::DOT))
        .rounded(px(radius::R_FULL))
        .bg(t.tone_rail(tone))
}

/// Stable categorical color index for a string (workspace folder colors).
#[must_use]
pub fn hash_color(text: &str) -> usize {
    text.bytes().fold(0usize, |h, b| {
        h.wrapping_mul(31).wrapping_add(usize::from(b))
    }) % 12
}
