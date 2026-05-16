#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub kind: CommandKind,
}

pub const COMMANDS: &[CommandSpec] = &[CommandSpec {
    name: "/settings",
    summary: "Open model and thinking settings",
    kind: CommandKind::Settings,
}];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandSuggestionsState {
    pub selected: usize,
    dismissed_input: Option<String>,
}

impl CommandSuggestionsState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_selection(&mut self, delta: isize, len: usize) {
        self.selected = move_index(self.selected, len, delta);
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len.saturating_sub(1));
        }
    }

    pub fn dismiss_for(&mut self, input: &str) {
        self.dismissed_input = Some(input.to_string());
    }

    #[must_use]
    pub fn is_dismissed_for(&self, input: &str) -> bool {
        self.dismissed_input.as_deref() == Some(input)
    }

    pub fn clear_dismissal_if_input_changed(&mut self, input: &str) {
        if self
            .dismissed_input
            .as_deref()
            .is_some_and(|dismissed| dismissed != input)
        {
            self.dismissed_input = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSuggestionsView {
    pub query: String,
    pub items: Vec<CommandSpec>,
    pub selected: usize,
}

impl CommandSuggestionsView {
    #[must_use]
    pub fn selected_command(&self) -> Option<CommandSpec> {
        self.items.get(self.selected).copied()
    }
}

#[must_use]
pub fn command_suggestions_for(input: &str, cursor: usize) -> Option<CommandSuggestionsView> {
    let query = command_query(input, cursor)?;
    let items = COMMANDS
        .iter()
        .copied()
        .filter(|command| command.name.starts_with(query.as_str()))
        .collect::<Vec<_>>();
    Some(CommandSuggestionsView {
        query,
        items,
        selected: 0,
    })
}

#[must_use]
pub fn command_query(input: &str, cursor: usize) -> Option<String> {
    let cursor_byte = byte_index_at_char(input, cursor);
    let before_cursor = &input[..cursor_byte.min(input.len())];
    if before_cursor.contains('\n') || before_cursor.contains(char::is_whitespace) {
        return None;
    }
    if !before_cursor.starts_with('/') {
        return None;
    }
    let after_cursor = &input[cursor_byte.min(input.len())..];
    if after_cursor.contains('\n') || after_cursor.contains(char::is_whitespace) {
        return None;
    }
    Some(before_cursor.to_string())
}

#[must_use]
pub fn parse_command(input: &str) -> Option<CommandSpec> {
    let trimmed = input.trim();
    COMMANDS
        .iter()
        .find(|command| command.name == trimmed)
        .copied()
}

fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len.saturating_sub(1);
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs()).min(last)
    } else {
        current.saturating_add(delta as usize).min(last)
    }
}

fn byte_index_at_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_appear_only_for_first_slash_token() {
        assert!(command_suggestions_for("/", 1).is_some());
        assert!(command_suggestions_for("/set", 4).is_some());
        assert!(command_suggestions_for("hello /", 7).is_none());
        assert!(command_suggestions_for("/settings now", 9).is_none());
    }

    #[test]
    fn filters_commands_by_prefix() {
        let view = command_suggestions_for("/set", 4).unwrap_or_else(|| panic!("missing view"));
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].name, "/settings");
        let view = command_suggestions_for("/nope", 5).unwrap_or_else(|| panic!("missing view"));
        assert!(view.items.is_empty());
    }

    #[test]
    fn parses_known_commands_exactly() {
        assert_eq!(
            parse_command("/settings").map(|command| command.kind),
            Some(CommandKind::Settings)
        );
        assert!(parse_command("/set").is_none());
    }
}
