//! Colour tokens (§5), modifiers and [`Style`] plus the ratatui-mirroring [`Stylize`] trait.
//!
//! Colours are theme role tokens only — no hex, no RGB, no ANSI indices (§4.2).
//! Unknown token strings parse tolerantly into [`Color::Other`]; renderers drop them
//! (with a diagnostic, §4.8) instead of rejecting the tree.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Theme role token (ui-protocol v2 §5). The closed set of tokens plus [`Color::Other`]
/// for tolerant parsing of unknown names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// Clear inherited colour / terminal default.
    Reset,
    /// Body text.
    Text,
    /// Secondary text.
    Muted,
    /// Hints, timestamps.
    Dim,
    /// Highlights, active marks (claw orange in the app).
    Accent,
    /// Primary action, focus (shell red; never for errors).
    Primary,
    /// Information, links.
    Info,
    /// Alias of [`Color::Info`] (links).
    Link,
    /// Done, ok.
    Success,
    /// Warnings.
    Warning,
    /// Errors.
    Error,
    /// Harness / provenance rails.
    Harness,
    /// Thinking text.
    Thinking,
    /// Tool titles.
    Tool,
    /// Message-role tint (user).
    User,
    /// Message-role tint (assistant).
    Assistant,
    /// Inline code.
    Code,
    /// Borders.
    Border,
    /// Focused / accent borders.
    BorderAccent,
    /// Quiet borders.
    BorderMuted,
    /// Categorical palette slots 1..12.
    C1,
    /// Categorical palette slots 1..12.
    C2,
    /// Categorical palette slots 1..12.
    C3,
    /// Categorical palette slots 1..12.
    C4,
    /// Categorical palette slots 1..12.
    C5,
    /// Categorical palette slots 1..12.
    C6,
    /// Categorical palette slots 1..12.
    C7,
    /// Categorical palette slots 1..12.
    C8,
    /// Categorical palette slots 1..12.
    C9,
    /// Categorical palette slots 1..12.
    C10,
    /// Categorical palette slots 1..12.
    C11,
    /// Categorical palette slots 1..12.
    C12,
    /// Panels, inputs.
    Surface1,
    /// Panels, inputs.
    Surface2,
    /// Selection.
    Surface3,
    /// Selected rows.
    SelectedBg,
    /// Tinted rows (success).
    SuccessBg,
    /// Tinted rows (error).
    ErrorBg,
    /// Tinted rows (warning).
    WarningBg,
    /// Tinted rows (info).
    InfoBg,
    /// Tinted rows (accent).
    AccentBg,
    /// Diff-like added text.
    DiffAdded,
    /// Diff-like removed text.
    DiffRemoved,
    /// Diff-like context text.
    DiffContext,
    /// Unknown token: renderers drop it (with a diagnostic) instead of rejecting (§4.8).
    Other(String),
}

/// The 44 normative token names, in Appendix A order.
pub const COLOR_TOKENS: [&str; 44] = [
    "reset",
    "text",
    "muted",
    "dim",
    "accent",
    "primary",
    "info",
    "link",
    "success",
    "warning",
    "error",
    "harness",
    "thinking",
    "tool",
    "user",
    "assistant",
    "code",
    "border",
    "borderAccent",
    "borderMuted",
    "c1",
    "c2",
    "c3",
    "c4",
    "c5",
    "c6",
    "c7",
    "c8",
    "c9",
    "c10",
    "c11",
    "c12",
    "surface1",
    "surface2",
    "surface3",
    "selectedBg",
    "successBg",
    "errorBg",
    "warningBg",
    "infoBg",
    "accentBg",
    "diffAdded",
    "diffRemoved",
    "diffContext",
];

impl Color {
    /// The wire name of a known token, or `None` for [`Color::Other`].
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        let name = match self {
            Self::Reset => "reset",
            Self::Text => "text",
            Self::Muted => "muted",
            Self::Dim => "dim",
            Self::Accent => "accent",
            Self::Primary => "primary",
            Self::Info => "info",
            Self::Link => "link",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Harness => "harness",
            Self::Thinking => "thinking",
            Self::Tool => "tool",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Code => "code",
            Self::Border => "border",
            Self::BorderAccent => "borderAccent",
            Self::BorderMuted => "borderMuted",
            Self::C1 => "c1",
            Self::C2 => "c2",
            Self::C3 => "c3",
            Self::C4 => "c4",
            Self::C5 => "c5",
            Self::C6 => "c6",
            Self::C7 => "c7",
            Self::C8 => "c8",
            Self::C9 => "c9",
            Self::C10 => "c10",
            Self::C11 => "c11",
            Self::C12 => "c12",
            Self::Surface1 => "surface1",
            Self::Surface2 => "surface2",
            Self::Surface3 => "surface3",
            Self::SelectedBg => "selectedBg",
            Self::SuccessBg => "successBg",
            Self::ErrorBg => "errorBg",
            Self::WarningBg => "warningBg",
            Self::InfoBg => "infoBg",
            Self::AccentBg => "accentBg",
            Self::DiffAdded => "diffAdded",
            Self::DiffRemoved => "diffRemoved",
            Self::DiffContext => "diffContext",
            Self::Other(_) => return None,
        };
        Some(name)
    }

    /// Parses a known token; unknown names become [`Color::Other`].
    #[must_use]
    pub fn parse(name: &str) -> Self {
        Self::parse_known(name).unwrap_or_else(|| Self::Other(name.to_owned()))
    }

    fn parse_known(name: &str) -> Option<Self> {
        Some(match name {
            "reset" => Self::Reset,
            "text" => Self::Text,
            "muted" => Self::Muted,
            "dim" => Self::Dim,
            "accent" => Self::Accent,
            "primary" => Self::Primary,
            "info" => Self::Info,
            "link" => Self::Link,
            "success" => Self::Success,
            "warning" => Self::Warning,
            "error" => Self::Error,
            "harness" => Self::Harness,
            "thinking" => Self::Thinking,
            "tool" => Self::Tool,
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "code" => Self::Code,
            "border" => Self::Border,
            "borderAccent" => Self::BorderAccent,
            "borderMuted" => Self::BorderMuted,
            "c1" => Self::C1,
            "c2" => Self::C2,
            "c3" => Self::C3,
            "c4" => Self::C4,
            "c5" => Self::C5,
            "c6" => Self::C6,
            "c7" => Self::C7,
            "c8" => Self::C8,
            "c9" => Self::C9,
            "c10" => Self::C10,
            "c11" => Self::C11,
            "c12" => Self::C12,
            "surface1" => Self::Surface1,
            "surface2" => Self::Surface2,
            "surface3" => Self::Surface3,
            "selectedBg" => Self::SelectedBg,
            "successBg" => Self::SuccessBg,
            "errorBg" => Self::ErrorBg,
            "warningBg" => Self::WarningBg,
            "infoBg" => Self::InfoBg,
            "accentBg" => Self::AccentBg,
            "diffAdded" => Self::DiffAdded,
            "diffRemoved" => Self::DiffRemoved,
            "diffContext" => Self::DiffContext,
            _ => return None,
        })
    }

    /// True when this is [`Color::Other`] (not a known token).
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Other(_))
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Self::Other(name) = self {
            return serializer.serialize_str(name);
        }
        serializer.serialize_str(self.as_str().unwrap_or_default())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Self::parse(&name))
    }
}

/// A text modifier (ratatui constant names, §4.2).
#[allow(
    non_camel_case_types,
    reason = "wire names are ratatui's constant names (§4.2)"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Modifier {
    /// Bold.
    BOLD,
    /// Dim / half bright.
    DIM,
    /// Italic.
    ITALIC,
    /// Underlined.
    UNDERLINED,
    /// Slow blink.
    SLOW_BLINK,
    /// Rapid blink.
    RAPID_BLINK,
    /// Reversed (swap fg/bg).
    REVERSED,
    /// Hidden.
    HIDDEN,
    /// Crossed out.
    CROSSED_OUT,
}

/// All modifiers, in canonical (wire) order.
pub const ALL_MODIFIERS: [Modifier; 9] = [
    Modifier::BOLD,
    Modifier::DIM,
    Modifier::ITALIC,
    Modifier::UNDERLINED,
    Modifier::SLOW_BLINK,
    Modifier::RAPID_BLINK,
    Modifier::REVERSED,
    Modifier::HIDDEN,
    Modifier::CROSSED_OUT,
];

/// A bitflags-like set of [`Modifier`]s. Serializes as an array of modifier names
/// in canonical order; an absent/empty set serializes to nothing.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct Modifiers(u16);

impl Modifiers {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self(0)
    }

    /// Adds a modifier.
    pub fn insert(&mut self, m: Modifier) {
        self.0 |= 1 << (m as u16);
    }

    /// Removes a modifier.
    pub fn remove(&mut self, m: Modifier) {
        self.0 &= !(1 << (m as u16));
    }

    /// True when the modifier is set.
    #[must_use]
    pub fn contains(&self, m: Modifier) -> bool {
        self.0 & (1 << (m as u16)) != 0
    }

    /// True when no modifier is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Union of both sets.
    #[must_use]
    pub fn union(mut self, other: Self) -> Self {
        self.0 |= other.0;
        self
    }

    /// Iterates the set in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = Modifier> + '_ {
        ALL_MODIFIERS.into_iter().filter(|m| self.contains(*m))
    }

    /// Builds a set from an iterator.
    #[must_use]
    pub fn from_iter_const<I: IntoIterator<Item = Modifier>>(mods: I) -> Self {
        let mut set = Self::new();
        for m in mods {
            set.insert(m);
        }
        set
    }
}

impl Serialize for Modifiers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Modifiers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mods = Vec::<Modifier>::deserialize(deserializer)?;
        Ok(Self::from_iter_const(mods))
    }
}

/// ratatui `Style` with theme-token colours (§4.2). Composition follows ratatui
/// `Style::patch`: colours override when set, modifiers accumulate, `subModifier`
/// removes inherited modifiers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Style {
    /// Foreground token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<Color>,
    /// Background token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<Color>,
    /// Underline colour token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<Color>,
    /// Modifiers to add on top of inherited ones.
    #[serde(skip_serializing_if = "Modifiers::is_empty")]
    pub add_modifier: Modifiers,
    /// Modifiers to remove from inherited ones.
    #[serde(skip_serializing_if = "Modifiers::is_empty")]
    pub sub_modifier: Modifiers,
}

impl Style {
    /// Empty style.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            underline_color: None,
            add_modifier: Modifiers::new(),
            sub_modifier: Modifiers::new(),
        }
    }

    /// Sets the foreground token.
    #[must_use]
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// Sets the background token.
    #[must_use]
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Sets the underline colour token.
    #[must_use]
    pub fn underline_color(mut self, color: Color) -> Self {
        self.underline_color = Some(color);
        self
    }

    /// Adds modifiers.
    #[must_use]
    pub fn add_modifier(mut self, m: impl IntoIterator<Item = Modifier>) -> Self {
        self.add_modifier = self.add_modifier.union(Modifiers::from_iter_const(m));
        self
    }

    /// Removes inherited modifiers.
    #[must_use]
    pub fn sub_modifier(mut self, m: impl IntoIterator<Item = Modifier>) -> Self {
        self.sub_modifier = self.sub_modifier.union(Modifiers::from_iter_const(m));
        self
    }

    /// ratatui `Style::patch`: `other` wins for colours, modifiers accumulate.
    #[must_use]
    pub fn patch(mut self, other: Self) -> Self {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        if other.bg.is_some() {
            self.bg = other.bg;
        }
        if other.underline_color.is_some() {
            self.underline_color = other.underline_color;
        }
        self.add_modifier = self.add_modifier.union(other.add_modifier);
        self.sub_modifier = self.sub_modifier.union(other.sub_modifier);
        self
    }

    /// True when nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && self.underline_color.is_none()
            && self.add_modifier.is_empty()
            && self.sub_modifier.is_empty()
    }
}

/// ratatui-mirroring `Stylize`: chainable style helpers on [`Span`], [`Line`] and
/// [`Text`], and on `&str` / `String` (converting to a [`Span`]).
///
/// ```
/// use oino_ui::prelude::*;
/// let span = "Board ".bold().fg(Color::Accent);
/// let line = Span::from("hi").dim();
/// ```
pub trait Stylize<T>: Sized {
    /// Applies a style.
    fn style(self, style: Style) -> T;

    /// Sets the foreground token.
    fn fg(self, color: Color) -> T {
        self.style(Style::new().fg(color))
    }

    /// Sets the background token.
    fn bg(self, color: Color) -> T {
        self.style(Style::new().bg(color))
    }

    /// Sets the underline colour token.
    fn underline_color(self, color: Color) -> T {
        self.style(Style::new().underline_color(color))
    }

    /// Adds the `BOLD` modifier.
    fn bold(self) -> T {
        self.style(Style::new().add_modifier([Modifier::BOLD]))
    }

    /// Adds the `DIM` modifier.
    fn dim(self) -> T {
        self.style(Style::new().add_modifier([Modifier::DIM]))
    }

    /// Adds the `ITALIC` modifier.
    fn italic(self) -> T {
        self.style(Style::new().add_modifier([Modifier::ITALIC]))
    }

    /// Adds the `UNDERLINED` modifier.
    fn underlined(self) -> T {
        self.style(Style::new().add_modifier([Modifier::UNDERLINED]))
    }

    /// Adds the `SLOW_BLINK` modifier.
    fn slow_blink(self) -> T {
        self.style(Style::new().add_modifier([Modifier::SLOW_BLINK]))
    }

    /// Adds the `RAPID_BLINK` modifier.
    fn rapid_blink(self) -> T {
        self.style(Style::new().add_modifier([Modifier::RAPID_BLINK]))
    }

    /// Adds the `REVERSED` modifier.
    fn reversed(self) -> T {
        self.style(Style::new().add_modifier([Modifier::REVERSED]))
    }

    /// Adds the `HIDDEN` modifier.
    fn hidden(self) -> T {
        self.style(Style::new().add_modifier([Modifier::HIDDEN]))
    }

    /// Adds the `CROSSED_OUT` modifier.
    fn crossed_out(self) -> T {
        self.style(Style::new().add_modifier([Modifier::CROSSED_OUT]))
    }
}
