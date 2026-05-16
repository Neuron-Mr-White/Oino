#![forbid(unsafe_code)]

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub fg: Color,
    pub muted: Color,
    pub focused_border: Color,
    pub panel_border: Color,
    pub user_border: Color,
    pub assistant_border: Color,
    pub tool_border: Color,
    pub error: Style,
    pub warning: Style,
    pub placeholder: Style,
    pub footer: Style,
    pub working: Style,
    pub title: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            fg: Color::Reset,
            muted: Color::DarkGray,
            focused_border: Color::Cyan,
            panel_border: Color::DarkGray,
            user_border: Color::Blue,
            assistant_border: Color::Green,
            tool_border: Color::Yellow,
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            warning: Style::default().fg(Color::Yellow),
            placeholder: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            footer: Style::default().fg(Color::DarkGray),
            working: Style::default().fg(Color::Yellow),
            title: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        }
    }
}

impl Theme {
    #[must_use]
    pub fn bubble_border_for_role(&self, role: &str, is_error: bool) -> Style {
        if is_error {
            return self.error;
        }
        let color = match role {
            "user" => self.user_border,
            "assistant" => self.assistant_border,
            role if role.starts_with("tool:") => self.tool_border,
            _ => self.panel_border,
        };
        Style::default().fg(color)
    }
}
