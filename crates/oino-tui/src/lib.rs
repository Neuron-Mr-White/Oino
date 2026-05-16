#![doc = "Minimal Ratatui UI primitives for Oino."]
#![forbid(unsafe_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oino_types::{ContentBlock, Message};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    pub role: String,
    pub content: String,
    pub is_error: bool,
}

impl MessageView {
    #[must_use]
    pub fn line(&self) -> String {
        format!("{}: {}", self.role, self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    None,
    SubmitPrompt(String),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiState {
    pub messages: Vec<MessageView>,
    pub input: String,
    pub status: String,
    pub working: bool,
    pub error: Option<String>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            status: "Enter send • Esc/Ctrl-C quit".into(),
            working: false,
            error: None,
        }
    }
}

impl TuiState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_messages_from_oino(&mut self, messages: &[Message]) {
        self.messages = project_messages(messages);
    }

    pub fn set_working(&mut self, working: bool) {
        self.working = working;
        self.status = if working {
            "Working…".into()
        } else {
            "Enter send • Esc/Ctrl-C quit".into()
        };
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.working = false;
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Esc => TuiAction::Quit,
            KeyCode::Char('c') | KeyCode::Char('C')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                TuiAction::Quit
            }
            KeyCode::Enter => {
                let prompt = self.input.trim().to_string();
                if prompt.is_empty() {
                    TuiAction::None
                } else {
                    self.input.clear();
                    self.clear_error();
                    TuiAction::SubmitPrompt(prompt)
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                TuiAction::None
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.input.push(ch);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }
}

#[must_use]
pub fn project_messages(messages: &[Message]) -> Vec<MessageView> {
    messages.iter().map(project_message).collect()
}

#[must_use]
pub fn project_message(message: &Message) -> MessageView {
    match message {
        Message::User { content, .. } => MessageView {
            role: "user".into(),
            content: summarize_content(content),
            is_error: false,
        },
        Message::Assistant { content, .. } => MessageView {
            role: "assistant".into(),
            content: summarize_content(content),
            is_error: false,
        },
        Message::ToolResult {
            tool_name,
            content,
            is_error,
            ..
        } => MessageView {
            role: format!("tool:{tool_name}"),
            content: summarize_content(content),
            is_error: *is_error,
        },
        Message::Custom { name, .. } => MessageView {
            role: format!("custom:{name}"),
            content: "<custom>".into(),
            is_error: false,
        },
        Message::CompactionSummary { summary, .. } => MessageView {
            role: "compaction".into(),
            content: summary.clone(),
            is_error: false,
        },
        Message::BranchSummary { summary, .. } => MessageView {
            role: "branch".into(),
            content: summary.clone(),
            is_error: false,
        },
    }
}

fn summarize_content(content: &[ContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in content {
        match block {
            ContentBlock::Text { text } => parts.push(text.clone()),
            ContentBlock::Image { media_type, .. } => parts.push(format!("<image:{media_type}>")),
            ContentBlock::Thinking { text, .. } => parts.push(format!("<thinking:{text}>")),
            ContentBlock::ToolCall { name, .. } => parts.push(format!("<tool-call:{name}>")),
        }
    }
    if parts.is_empty() {
        "<empty>".into()
    } else {
        parts.join(" ")
    }
}

pub fn render(frame: &mut Frame<'_>, state: &TuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let mut lines: Vec<Line<'_>> = state
        .messages
        .iter()
        .rev()
        .take(chunks[0].height.saturating_sub(2) as usize)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| {
            let role_style = if message.is_error {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            };
            Line::from(vec![
                Span::styled(format!("{}: ", message.role), role_style),
                Span::raw(message.content.clone()),
            ])
        })
        .collect();
    if let Some(error) = &state.error {
        lines.push(Line::from(vec![Span::styled(
            format!("error: {error}"),
            Style::default().fg(Color::Red),
        )]));
    }
    let messages = Paragraph::new(lines)
        .block(Block::default().title("Oino").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(messages, chunks[0]);

    let input_title = if state.working {
        "Input (working)"
    } else {
        "Input"
    };
    let input = Paragraph::new(format!("> {}", state.input))
        .block(Block::default().title(input_title).borders(Borders::ALL));
    frame.render_widget(input, chunks[1]);

    let status_style = if state.error.is_some() {
        Style::default().fg(Color::Red)
    } else if state.working {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let status = Paragraph::new(state.status.clone()).style(status_style);
    frame.render_widget(status, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_and_submits_input() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('h'))), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Char('i'))), TuiAction::None);
        assert_eq!(state.input, "hi");
        assert_eq!(state.handle_key(key(KeyCode::Backspace)), TuiAction::None);
        assert_eq!(state.input, "h");
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SubmitPrompt("h".into())
        );
        assert!(state.input.is_empty());
    }

    #[test]
    fn quit_keys_exit() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::Quit);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            TuiAction::Quit
        );
    }

    #[test]
    fn projects_messages() {
        let messages = vec![
            Message::user_text("hello"),
            Message::assistant_text("hi", oino_types::StopReason::EndTurn),
        ];
        let views = project_messages(&messages);
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].role, "user");
        assert_eq!(views[0].content, "hello");
        assert_eq!(views[1].role, "assistant");
    }

    #[test]
    fn render_smoke_test() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(err) => panic!("terminal init failed: {err}"),
        };
        let mut state = TuiState::new();
        state.messages.push(MessageView {
            role: "user".into(),
            content: "hello".into(),
            is_error: false,
        });
        if let Err(err) = terminal.draw(|frame| render(frame, &state)) {
            panic!("draw failed: {err}");
        }
    }
}
