//! Type scale (design.md §2.4 as amended by A2.3): unipi CSS sizes as offsets from one
//! `ui_font_size` (14 by default).

/// Default UI font size on desktop (px).
pub const UI_FONT_SIZE: f32 = 14.0;
/// Default UI font size on mobile (px).
pub const UI_FONT_SIZE_MOBILE: f32 = 15.0;
/// Allowed UI font sizes.
pub const UI_FONT_RANGE: (f32, f32) = (12.0, 18.0);
/// Default code / diff / log font size (px).
pub const CODE_FONT_SIZE: f32 = 12.0;
/// Allowed code font sizes.
pub const CODE_FONT_RANGE: (f32, f32) = (10.0, 20.0);

/// UI font family (bundled Inter).
pub const UI_FONT: &str = "Inter";
/// Monospace family (bundled `JetBrains` Mono).
pub const MONO_FONT: &str = "JetBrains Mono";

/// Text roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextRole {
    /// 30/36: page titles (Sessions, Board, Extensions, Settings).
    Display,
    /// 20/26: dialog / sheet titles, empty-state titles.
    Title,
    /// 18/23: markdown h1.
    Xl,
    /// 16/21: detail header title, markdown h2–h4, compact empty title.
    Lg,
    /// 15/24: chat prose, user bubble, palette input.
    Prose,
    /// 14/20: body, buttons, rows.
    Base,
    /// 13/18: compact supporting copy.
    Caption,
    /// 12/16: metadata, tooltips, inline code, meta row.
    Sm,
    /// 11/15: turn end, kbd, counters, mono stats.
    Xs,
}

/// `(font size, line height)` of `role` for UI size `ui`.
#[must_use]
pub const fn size(role: TextRole, ui: f32) -> (f32, f32) {
    match role {
        TextRole::Display => (ui + 16.0, ui + 22.0),
        TextRole::Title => (ui + 6.0, ui + 12.0),
        TextRole::Xl => (ui + 4.0, ui + 9.0),
        TextRole::Lg => (ui + 2.0, ui + 7.0),
        TextRole::Prose => (ui + 1.0, ui + 10.0),
        TextRole::Base => (ui, ui + 6.0),
        TextRole::Caption => (ui - 1.0, ui + 4.0),
        TextRole::Sm => (ui - 2.0, ui + 2.0),
        TextRole::Xs => (ui - 3.0, ui + 1.0),
    }
}

/// Clamps a stored UI font size into [`UI_FONT_RANGE`].
#[must_use]
pub fn clamp_ui(size: f32) -> f32 {
    size.clamp(UI_FONT_RANGE.0, UI_FONT_RANGE.1)
}

/// Clamps a stored code font size into [`CODE_FONT_RANGE`].
#[must_use]
pub fn clamp_code(size: f32) -> f32 {
    size.clamp(CODE_FONT_RANGE.0, CODE_FONT_RANGE.1)
}

/// ZCode's absolute text scale (round5 §1.1): px sizes that do not track `ui_font_size` —
/// greeting heroes, page furniture. Where a role should follow the user's font setting,
/// use [`TextRole`] instead.
pub mod zcode {
    /// 18: greeting hero.
    pub const XL: f32 = 18.0;
    /// 16.
    pub const LG: f32 = 16.0;
    /// 14.
    pub const BASE: f32 = 14.0;
    /// 13.
    pub const CAPTION: f32 = 13.0;
    /// 12.
    pub const SM: f32 = 12.0;
    /// 10.
    pub const XS: f32 = 10.0;
    /// Settings page title `(size, line height)` 24/30.
    pub const SETTINGS_TITLE: (f32, f32) = (24.0, 30.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unipi_sizes_at_14() {
        assert_eq!(size(TextRole::Display, 14.0), (30.0, 36.0));
        assert_eq!(size(TextRole::Title, 14.0), (20.0, 26.0));
        assert_eq!(size(TextRole::Xl, 14.0), (18.0, 23.0));
        assert_eq!(size(TextRole::Lg, 14.0), (16.0, 21.0));
        assert_eq!(size(TextRole::Prose, 14.0), (15.0, 24.0));
        assert_eq!(size(TextRole::Base, 14.0), (14.0, 20.0));
        assert_eq!(size(TextRole::Caption, 14.0), (13.0, 18.0));
        assert_eq!(size(TextRole::Sm, 14.0), (12.0, 16.0));
        assert_eq!(size(TextRole::Xs, 14.0), (11.0, 15.0));
        assert_eq!(clamp_ui(30.0), 18.0);
        assert_eq!(clamp_code(5.0), 10.0);
    }

    #[test]
    fn zcode_scale_is_absolute() {
        use super::zcode;
        assert_eq!((zcode::XL, zcode::LG, zcode::BASE), (18.0, 16.0, 14.0));
        assert_eq!((zcode::CAPTION, zcode::SM, zcode::XS), (13.0, 12.0, 10.0));
        assert_eq!(zcode::SETTINGS_TITLE, (24.0, 30.0));
    }
}
