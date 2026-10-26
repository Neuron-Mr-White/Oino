//! Colour theme: pi's built-in `dark` and `light` themes (resolved from their okhsl values)
//! and the OUS tone mapping (ui-protocol §6.5).

use oino_ai::ThinkingLevel;
use oino_view::ous::Tone;
use ratatui::style::{Color, Modifier, Style};

/// Resolved theme colours (pi theme tokens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    missing_docs,
    reason = "fields are pi theme token names (theme-schema.json)"
)]
pub struct Theme {
    pub name: &'static str,
    pub accent: Color,
    pub border: Color,
    pub border_accent: Color,
    pub border_muted: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub muted: Color,
    pub dim: Color,
    pub text: Color,
    pub thinking_text: Color,
    pub selected_bg: Color,
    pub user_message_bg: Color,
    pub user_message_text: Color,
    pub custom_message_bg: Color,
    pub custom_message_text: Color,
    pub custom_message_label: Color,
    pub tool_pending_bg: Color,
    pub tool_success_bg: Color,
    pub tool_error_bg: Color,
    pub tool_title: Color,
    pub tool_output: Color,
    pub md_heading: Color,
    pub md_link: Color,
    pub md_link_url: Color,
    pub md_code: Color,
    pub md_code_block: Color,
    pub md_code_block_border: Color,
    pub md_quote: Color,
    pub md_quote_border: Color,
    pub md_hr: Color,
    pub md_list_bullet: Color,
    pub diff_added: Color,
    pub diff_removed: Color,
    pub diff_context: Color,
    pub syntax_comment: Color,
    pub syntax_keyword: Color,
    pub syntax_function: Color,
    pub syntax_variable: Color,
    pub syntax_string: Color,
    pub syntax_number: Color,
    pub syntax_type: Color,
    pub syntax_operator: Color,
    pub syntax_punctuation: Color,
    pub thinking_off: Color,
    pub thinking_minimal: Color,
    pub thinking_low: Color,
    pub thinking_medium: Color,
    pub thinking_high: Color,
    pub thinking_xhigh: Color,
    pub thinking_max: Color,
    pub bash_mode: Color,
}

/// pi `dark` theme.
pub const DARK: Theme = Theme {
    name: "dark",
    accent: Color::Rgb(0xa7, 0x98, 0xd7),
    border: Color::Rgb(0x5f, 0xa8, 0xcc),
    border_accent: Color::Rgb(0xa0, 0x8e, 0xd5),
    border_muted: Color::Rgb(0x76, 0x81, 0x86),
    success: Color::Rgb(0x68, 0xb7, 0x8d),
    error: Color::Rgb(0xea, 0x7f, 0x81),
    warning: Color::Rgb(0xcd, 0x9a, 0x22),
    muted: Color::Rgb(0x9d, 0xa5, 0xa9),
    dim: Color::Rgb(0x7e, 0x88, 0x8e),
    text: Color::Rgb(0xde, 0xe0, 0xe1),
    thinking_text: Color::Rgb(0x96, 0xa0, 0xa4),
    selected_bg: Color::Rgb(0x21, 0x3b, 0x49),
    user_message_bg: Color::Rgb(0x21, 0x3b, 0x49),
    user_message_text: Color::Rgb(0xde, 0xe0, 0xe1),
    custom_message_bg: Color::Rgb(0x3a, 0x30, 0x55),
    custom_message_text: Color::Rgb(0x9d, 0xa5, 0xa9),
    custom_message_label: Color::Rgb(0xa7, 0x98, 0xd7),
    tool_pending_bg: Color::Rgb(0x34, 0x38, 0x3a),
    tool_success_bg: Color::Rgb(0x25, 0x41, 0x31),
    tool_error_bg: Color::Rgb(0x5b, 0x28, 0x2a),
    tool_title: Color::Rgb(0xde, 0xe0, 0xe1),
    tool_output: Color::Rgb(0x9d, 0xa5, 0xa9),
    md_heading: Color::Rgb(0xcd, 0x9a, 0x22),
    md_link: Color::Rgb(0x69, 0xad, 0xd0),
    md_link_url: Color::Rgb(0x9d, 0xa5, 0xa9),
    md_code: Color::Rgb(0xa7, 0x98, 0xd7),
    md_code_block: Color::Rgb(0x68, 0xb7, 0x8d),
    md_code_block_border: Color::Rgb(0x9d, 0xa5, 0xa9),
    md_quote: Color::Rgb(0x9d, 0xa5, 0xa9),
    md_quote_border: Color::Rgb(0x9d, 0xa5, 0xa9),
    md_hr: Color::Rgb(0x9d, 0xa5, 0xa9),
    md_list_bullet: Color::Rgb(0xa7, 0x98, 0xd7),
    diff_added: Color::Rgb(0x68, 0xb7, 0x8d),
    diff_removed: Color::Rgb(0xea, 0x7f, 0x81),
    diff_context: Color::Rgb(0x9d, 0xa5, 0xa9),
    syntax_comment: Color::Rgb(0x9d, 0xa5, 0xa9),
    syntax_keyword: Color::Rgb(0x69, 0xad, 0xd0),
    syntax_function: Color::Rgb(0xcd, 0x9a, 0x22),
    syntax_variable: Color::Rgb(0x5d, 0xb3, 0xba),
    syntax_string: Color::Rgb(0xde, 0x8d, 0x5a),
    syntax_number: Color::Rgb(0x68, 0xb7, 0x8d),
    syntax_type: Color::Rgb(0xa7, 0x98, 0xd7),
    syntax_operator: Color::Rgb(0x9d, 0xa5, 0xa9),
    syntax_punctuation: Color::Rgb(0x9d, 0xa5, 0xa9),
    thinking_off: Color::Rgb(0x6c, 0x76, 0x7b),
    thinking_minimal: Color::Rgb(0x68, 0x80, 0x8d),
    thinking_low: Color::Rgb(0x54, 0x89, 0xa4),
    thinking_medium: Color::Rgb(0x61, 0x85, 0xcc),
    thinking_high: Color::Rgb(0x97, 0x76, 0xe5),
    thinking_xhigh: Color::Rgb(0xde, 0x54, 0xc1),
    thinking_max: Color::Rgb(0xfe, 0x54, 0x62),
    bash_mode: Color::Rgb(0x5e, 0xb2, 0x86),
};

/// pi `light` theme.
pub const LIGHT: Theme = Theme {
    name: "light",
    accent: Color::Rgb(0x74, 0x59, 0xb4),
    border: Color::Rgb(0x3d, 0x8e, 0xb3),
    border_accent: Color::Rgb(0x8a, 0x72, 0xcb),
    border_muted: Color::Rgb(0x9a, 0xa2, 0xa7),
    success: Color::Rgb(0x33, 0x7e, 0x58),
    error: Color::Rgb(0xc8, 0x25, 0x3d),
    warning: Color::Rgb(0x8f, 0x68, 0x02),
    muted: Color::Rgb(0x67, 0x71, 0x76),
    dim: Color::Rgb(0x87, 0x90, 0x95),
    text: Color::Rgb(0x3b, 0x3f, 0x41),
    thinking_text: Color::Rgb(0x7c, 0x86, 0x8c),
    selected_bg: Color::Rgb(0xdf, 0xe7, 0xec),
    user_message_bg: Color::Rgb(0xdf, 0xe7, 0xec),
    user_message_text: Color::Rgb(0x3b, 0x3f, 0x41),
    custom_message_bg: Color::Rgb(0xe6, 0xe4, 0xee),
    custom_message_text: Color::Rgb(0x67, 0x71, 0x76),
    custom_message_label: Color::Rgb(0x74, 0x59, 0xb4),
    tool_pending_bg: Color::Rgb(0xe4, 0xe5, 0xe6),
    tool_success_bg: Color::Rgb(0xde, 0xe9, 0xe1),
    tool_error_bg: Color::Rgb(0xee, 0xe2, 0xe1),
    tool_title: Color::Rgb(0x3b, 0x3f, 0x41),
    tool_output: Color::Rgb(0x67, 0x71, 0x76),
    md_heading: Color::Rgb(0x8f, 0x68, 0x02),
    md_link: Color::Rgb(0x2f, 0x78, 0x99),
    md_link_url: Color::Rgb(0x67, 0x71, 0x76),
    md_code: Color::Rgb(0x74, 0x59, 0xb4),
    md_code_block: Color::Rgb(0x33, 0x7e, 0x58),
    md_code_block_border: Color::Rgb(0x67, 0x71, 0x76),
    md_quote: Color::Rgb(0x67, 0x71, 0x76),
    md_quote_border: Color::Rgb(0x67, 0x71, 0x76),
    md_hr: Color::Rgb(0x67, 0x71, 0x76),
    md_list_bullet: Color::Rgb(0x74, 0x59, 0xb4),
    diff_added: Color::Rgb(0x33, 0x7e, 0x58),
    diff_removed: Color::Rgb(0xc8, 0x25, 0x3d),
    diff_context: Color::Rgb(0x67, 0x71, 0x76),
    syntax_comment: Color::Rgb(0x67, 0x71, 0x76),
    syntax_keyword: Color::Rgb(0x2f, 0x78, 0x99),
    syntax_function: Color::Rgb(0x8f, 0x68, 0x02),
    syntax_variable: Color::Rgb(0x28, 0x7a, 0x81),
    syntax_string: Color::Rgb(0xa4, 0x54, 0x17),
    syntax_number: Color::Rgb(0x33, 0x7e, 0x58),
    syntax_type: Color::Rgb(0x74, 0x59, 0xb4),
    syntax_operator: Color::Rgb(0x67, 0x71, 0x76),
    syntax_punctuation: Color::Rgb(0x67, 0x71, 0x76),
    thinking_off: Color::Rgb(0xc2, 0xc8, 0xca),
    thinking_minimal: Color::Rgb(0xb5, 0xc4, 0xcb),
    thinking_low: Color::Rgb(0x9f, 0xc2, 0xd5),
    thinking_medium: Color::Rgb(0xa2, 0xb7, 0xe0),
    thinking_high: Color::Rgb(0xb5, 0xa5, 0xe8),
    thinking_xhigh: Color::Rgb(0xe5, 0x85, 0xcd),
    thinking_max: Color::Rgb(0xfe, 0x74, 0x79),
    bash_mode: Color::Rgb(0x40, 0x97, 0x6c),
};

/// Categorical palette for `c1`..`c12` tones.
const CATEGORY: [Color; 12] = [
    Color::Rgb(0x69, 0xad, 0xd0),
    Color::Rgb(0x68, 0xb7, 0x8d),
    Color::Rgb(0xcd, 0x9a, 0x22),
    Color::Rgb(0xa7, 0x98, 0xd7),
    Color::Rgb(0xea, 0x7f, 0x81),
    Color::Rgb(0x5d, 0xb3, 0xba),
    Color::Rgb(0xde, 0x8d, 0x5a),
    Color::Rgb(0x97, 0x76, 0xe5),
    Color::Rgb(0xde, 0x54, 0xc1),
    Color::Rgb(0x61, 0x85, 0xcc),
    Color::Rgb(0x5e, 0xb2, 0x86),
    Color::Rgb(0xfe, 0x54, 0x62),
];

impl Default for Theme {
    fn default() -> Self {
        DARK
    }
}

impl Theme {
    /// Theme by name (`dark`, `light`); unknown names fall back to dark.
    #[must_use]
    pub fn named(name: &str) -> Self {
        match name {
            "light" => LIGHT,
            _ => DARK,
        }
    }

    /// Foreground style.
    #[must_use]
    pub fn fg(color: Color) -> Style {
        Style::default().fg(color)
    }

    /// Editor border colour for a thinking level (pi `getThinkingBorderColor`).
    #[must_use]
    pub const fn thinking_border(&self, level: ThinkingLevel) -> Color {
        match level {
            ThinkingLevel::Off => self.thinking_off,
            ThinkingLevel::Minimal => self.thinking_minimal,
            ThinkingLevel::Low => self.thinking_low,
            ThinkingLevel::Medium => self.thinking_medium,
            ThinkingLevel::High => self.thinking_high,
            ThinkingLevel::Xhigh => self.thinking_xhigh,
            ThinkingLevel::Max => self.thinking_max,
        }
    }

    /// Colour of an OUS tone.
    #[must_use]
    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Default | Tone::User | Tone::Assistant => self.text,
            Tone::Muted => self.muted,
            Tone::Accent => self.accent,
            Tone::Info => self.md_link,
            Tone::Success => self.success,
            Tone::Warning => self.warning,
            Tone::Error => self.error,
            Tone::Harness => self.border_accent,
            Tone::Thinking => self.thinking_text,
            Tone::Tool => self.tool_title,
            Tone::Category(n) => CATEGORY[usize::from(n.clamp(1, 12)) - 1],
        }
    }

    /// Bold style helper.
    #[must_use]
    pub fn bold(color: Color) -> Style {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }
}
