#![forbid(unsafe_code)]

use crate::settings::{
    chat_style_value as settings_chat_style_value, parse_chat_style as settings_parse_chat_style,
    ChatStyle, CollapseMode, CollapseTarget, ModelOption,
};
use oino_types::{Model, ThinkingLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Session,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub kind: CommandKind,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/settings",
        summary: "Open or change settings",
        kind: CommandKind::Settings,
    },
    CommandSpec {
        name: "/model",
        summary: "Open or change model",
        kind: CommandKind::Settings,
    },
    CommandSpec {
        name: "/thinking",
        summary: "Open or change thinking level",
        kind: CommandKind::Settings,
    },
    CommandSpec {
        name: "/new",
        summary: "Start a new session",
        kind: CommandKind::Session,
    },
    CommandSpec {
        name: "/sessions",
        summary: "Browse saved sessions",
        kind: CommandKind::Session,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    NewSession,
    Sessions,
    Settings(SettingsCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsCommand {
    Open,
    OpenModelSelection,
    OpenThinkingLevel,
    OpenChatStyle,
    SetModel(Model),
    SetThinkingLevel(ThinkingLevel),
    SetCollapseMode {
        target: CollapseTarget,
        mode: CollapseMode,
    },
    SetChatStyle(ChatStyle),
}

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
pub struct CommandSuggestionItem {
    pub label: String,
    pub summary: String,
    pub replacement: String,
    pub replace_start: usize,
    pub replace_end: usize,
    pub complete_on_enter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSuggestionsView {
    pub query: String,
    pub title: String,
    pub items: Vec<CommandSuggestionItem>,
    pub selected: usize,
}

impl CommandSuggestionsView {
    #[must_use]
    pub fn selected_item(&self) -> Option<&CommandSuggestionItem> {
        self.items.get(self.selected)
    }
}

#[must_use]
pub fn command_suggestions_for(
    input: &str,
    cursor: usize,
    models: &[ModelOption],
) -> Option<CommandSuggestionsView> {
    let context = suggestion_context(input, cursor)?;
    match context.completed.as_slice() {
        [] => root_suggestions(context),
        [settings] if settings == "/settings" => settings_subject_suggestions(context),
        [settings, subject] if settings == "/settings" && subject == "model" => {
            model_suggestions(context, models)
        }
        [model] if model == "/model" => model_suggestions(context, models),
        [settings, subject] if settings == "/settings" && subject == "thinking" => {
            thinking_suggestions(context)
        }
        [thinking] if thinking == "/thinking" => thinking_suggestions(context),
        [settings, subject] if settings == "/settings" && subject == "collapse" => {
            collapse_target_suggestions(context)
        }
        [settings, subject, target]
            if settings == "/settings"
                && subject == "collapse"
                && parse_collapse_target(target).is_some() =>
        {
            collapse_mode_suggestions(context)
        }
        [settings, subject]
            if settings == "/settings" && (subject == "chat-style" || subject == "chat_style") =>
        {
            chat_style_suggestions(context)
        }
        _ => None,
    }
}

#[must_use]
pub fn file_suggestions_for(
    input: &str,
    cursor: usize,
    files: &[String],
) -> Option<CommandSuggestionsView> {
    let context = file_suggestion_context(input, cursor)?;
    let query = context.query.to_lowercase();
    let mut scored = files
        .iter()
        .filter_map(|file| fuzzy_file_score(file, &query).map(|score| (score, file)))
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.len().cmp(&right.len()))
            .then_with(|| left.cmp(right))
    });
    let items = scored
        .into_iter()
        .take(10)
        .map(|(_, file)| CommandSuggestionItem {
            label: file.clone(),
            summary: "file".into(),
            replacement: format!("@{file}"),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: false,
        })
        .collect::<Vec<_>>();
    Some(view("Files", context.query, items))
}

#[must_use]
pub fn command_query(input: &str, cursor: usize) -> Option<String> {
    let context = suggestion_context(input, cursor)?;
    if context.completed.is_empty() {
        Some(context.active_prefix)
    } else {
        None
    }
}

#[must_use]
pub fn parse_command(input: &str) -> Option<ParsedCommand> {
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        ["/new"] => Some(ParsedCommand::NewSession),
        ["/sessions"] => Some(ParsedCommand::Sessions),
        ["/settings"] => Some(ParsedCommand::Settings(SettingsCommand::Open)),
        ["/model"] => Some(ParsedCommand::Settings(SettingsCommand::OpenModelSelection)),
        ["/thinking"] => Some(ParsedCommand::Settings(SettingsCommand::OpenThinkingLevel)),
        ["/settings", "chat-style"] | ["/settings", "chat_style"] => {
            Some(ParsedCommand::Settings(SettingsCommand::OpenChatStyle))
        }
        ["/settings", "model", model] | ["/model", model] => Model::from_identifier(model)
            .map(SettingsCommand::SetModel)
            .map(ParsedCommand::Settings),
        ["/settings", "thinking", level] | ["/thinking", level] => parse_thinking_level(level)
            .map(SettingsCommand::SetThinkingLevel)
            .map(ParsedCommand::Settings),
        ["/settings", "collapse", target, mode] => {
            let target = parse_collapse_target(target)?;
            let mode = parse_collapse_mode(mode)?;
            Some(ParsedCommand::Settings(SettingsCommand::SetCollapseMode {
                target,
                mode,
            }))
        }
        ["/settings", "chat-style", style] | ["/settings", "chat_style", style] => {
            settings_parse_chat_style(style)
                .map(SettingsCommand::SetChatStyle)
                .map(ParsedCommand::Settings)
        }
        _ => None,
    }
}

#[must_use]
pub fn parse_thinking_level(value: &str) -> Option<ThinkingLevel> {
    match value {
        "off" => Some(ThinkingLevel::Off),
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" => Some(ThinkingLevel::XHigh),
        _ => None,
    }
}

#[must_use]
pub fn thinking_level_value(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
    }
}

#[must_use]
pub fn parse_collapse_target(value: &str) -> Option<CollapseTarget> {
    match value {
        "thinking" => Some(CollapseTarget::Thinking),
        "tool" => Some(CollapseTarget::Tool),
        _ => None,
    }
}

#[must_use]
pub fn collapse_target_value(target: CollapseTarget) -> &'static str {
    match target {
        CollapseTarget::Thinking => "thinking",
        CollapseTarget::Tool => "tool",
    }
}

#[must_use]
pub fn parse_collapse_mode(value: &str) -> Option<CollapseMode> {
    match value {
        "full" => Some(CollapseMode::Full),
        "truncate" => Some(CollapseMode::Truncate),
        "collapse" => Some(CollapseMode::Collapse),
        _ => None,
    }
}

#[must_use]
pub fn collapse_mode_value(mode: CollapseMode) -> &'static str {
    match mode {
        CollapseMode::Full => "full",
        CollapseMode::Truncate => "truncate",
        CollapseMode::Collapse => "collapse",
    }
}

#[must_use]
pub fn chat_style_value(style: ChatStyle) -> &'static str {
    settings_chat_style_value(style)
}

#[must_use]
pub fn parse_chat_style(value: &str) -> Option<ChatStyle> {
    settings_parse_chat_style(value)
}

fn root_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let items = COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(context.active_prefix.as_str()))
        .map(|command| CommandSuggestionItem {
            label: command.name.into(),
            summary: command.summary.into(),
            replacement: command.name.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
        })
        .collect::<Vec<_>>();
    Some(view("Commands", context.active_prefix, items))
}

fn settings_subject_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let subjects = [
        ("model", "Set selected model"),
        ("thinking", "Set thinking level"),
        ("collapse", "Set thinking/tool collapse mode"),
        ("chat-style", "Set transcript rendering style"),
    ];
    let items = subjects
        .into_iter()
        .filter(|(subject, _)| subject.starts_with(context.active_prefix.as_str()))
        .map(|(subject, summary)| incomplete_item(subject, summary, &context))
        .collect::<Vec<_>>();
    Some(view("Settings", context.active_prefix, items))
}

fn model_suggestions(
    context: SuggestionContext,
    models: &[ModelOption],
) -> Option<CommandSuggestionsView> {
    let query = context.active_prefix.to_lowercase();
    let items = models
        .iter()
        .filter(|model| {
            model.id.to_lowercase().contains(&query)
                || model.display_name.to_lowercase().contains(&query)
        })
        .map(|model| CommandSuggestionItem {
            label: model.id.clone(),
            summary: model.display_name.clone(),
            replacement: model.id.clone(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
        })
        .collect::<Vec<_>>();
    Some(view("Models", context.active_prefix, items))
}

fn thinking_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let items = [
        (ThinkingLevel::Off, "Disable provider reasoning"),
        (ThinkingLevel::Minimal, "Minimal reasoning"),
        (ThinkingLevel::Low, "Low reasoning"),
        (ThinkingLevel::Medium, "Medium reasoning"),
        (ThinkingLevel::High, "High reasoning"),
        (ThinkingLevel::XHigh, "Extra-high reasoning"),
    ]
    .into_iter()
    .filter(|(level, _)| thinking_level_value(*level).starts_with(context.active_prefix.as_str()))
    .map(|(level, summary)| {
        let value = thinking_level_value(level);
        CommandSuggestionItem {
            label: value.into(),
            summary: summary.into(),
            replacement: value.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
        }
    })
    .collect::<Vec<_>>();
    Some(view("Thinking", context.active_prefix, items))
}

fn collapse_target_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let targets = [
        ("thinking", "Thinking section"),
        ("tool", "Tool result bubbles"),
    ];
    let items = targets
        .into_iter()
        .filter(|(target, _)| target.starts_with(context.active_prefix.as_str()))
        .map(|(target, summary)| incomplete_item(target, summary, &context))
        .collect::<Vec<_>>();
    Some(view("Collapse Target", context.active_prefix, items))
}

fn collapse_mode_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let modes = [
        ("full", "Show full content"),
        ("truncate", "Show short preview"),
        ("collapse", "Hide content behind placeholder"),
    ];
    let items = modes
        .into_iter()
        .filter(|(mode, _)| mode.starts_with(context.active_prefix.as_str()))
        .map(|(mode, summary)| CommandSuggestionItem {
            label: mode.into(),
            summary: summary.into(),
            replacement: mode.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
        })
        .collect::<Vec<_>>();
    Some(view("Collapse Mode", context.active_prefix, items))
}

fn chat_style_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let styles = [
        (ChatStyle::Chat, "Current bubble-style transcript"),
        (ChatStyle::Agentic, "Codex-like agent activity transcript"),
        (ChatStyle::Minimal, "jcode-like compact transcript"),
    ];
    let items = styles
        .into_iter()
        .filter(|(style, _)| chat_style_value(*style).starts_with(context.active_prefix.as_str()))
        .map(|(style, summary)| {
            let value = chat_style_value(style);
            CommandSuggestionItem {
                label: value.into(),
                summary: summary.into(),
                replacement: value.into(),
                replace_start: context.replace_start,
                replace_end: context.replace_end,
                complete_on_enter: true,
            }
        })
        .collect::<Vec<_>>();
    Some(view("Chat Style", context.active_prefix, items))
}

fn incomplete_item(
    value: &str,
    summary: &str,
    context: &SuggestionContext,
) -> CommandSuggestionItem {
    CommandSuggestionItem {
        label: value.into(),
        summary: summary.into(),
        replacement: value.into(),
        replace_start: context.replace_start,
        replace_end: context.replace_end,
        complete_on_enter: false,
    }
}

fn view(
    title: impl Into<String>,
    query: String,
    items: Vec<CommandSuggestionItem>,
) -> CommandSuggestionsView {
    CommandSuggestionsView {
        query,
        title: title.into(),
        items,
        selected: 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SuggestionContext {
    completed: Vec<String>,
    active_prefix: String,
    replace_start: usize,
    replace_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSuggestionContext {
    query: String,
    replace_start: usize,
    replace_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    text: String,
    start: usize,
    end: usize,
}

fn suggestion_context(input: &str, cursor: usize) -> Option<SuggestionContext> {
    if input.contains('\n') || !input.starts_with('/') {
        return None;
    }
    let len = char_count(input);
    let cursor = cursor.min(len);
    let tokens = tokens_with_ranges(input);
    if tokens.is_empty() {
        return None;
    }

    let previous_is_whitespace =
        cursor > 0 && char_at(input, cursor.saturating_sub(1)).is_some_and(char::is_whitespace);
    let active = if previous_is_whitespace {
        None
    } else {
        tokens
            .iter()
            .find(|token| token.start <= cursor && cursor <= token.end)
    };

    let (completed, active_prefix, replace_start, replace_end) = if let Some(active) = active {
        let completed = tokens
            .iter()
            .filter(|token| token.end < active.start)
            .map(|token| token.text.clone())
            .collect::<Vec<_>>();
        let active_prefix = char_range(input, active.start, cursor);
        (completed, active_prefix, active.start, active.end)
    } else {
        let completed = tokens
            .iter()
            .filter(|token| token.end <= cursor)
            .map(|token| token.text.clone())
            .collect::<Vec<_>>();
        (completed, String::new(), cursor, cursor)
    };

    if tokens.iter().any(|token| token.start > replace_end) {
        return None;
    }

    Some(SuggestionContext {
        completed,
        active_prefix,
        replace_start,
        replace_end,
    })
}

fn file_suggestion_context(input: &str, cursor: usize) -> Option<FileSuggestionContext> {
    let len = char_count(input);
    let cursor = cursor.min(len);
    let token = tokens_with_ranges(input)
        .into_iter()
        .find(|token| token.start < cursor.saturating_add(1) && cursor <= token.end)?;
    if !token.text.starts_with('@') {
        return None;
    }
    let query_end = cursor
        .saturating_sub(token.start)
        .min(char_count(&token.text));
    let query = char_range(&token.text, 1, query_end);
    Some(FileSuggestionContext {
        query,
        replace_start: token.start,
        replace_end: token.end,
    })
}

fn fuzzy_file_score(file: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(1);
    }
    let lower = file.to_lowercase();
    if lower == query {
        return Some(10_000);
    }
    if lower.starts_with(query) {
        return Some(8_000usize.saturating_sub(file.len()));
    }
    if lower.contains(query) {
        return Some(6_000usize.saturating_sub(file.len()));
    }

    let mut score = 0usize;
    let mut search_start = 0usize;
    for ch in query.chars() {
        let found = lower[search_start..].find(ch)?;
        score = score.saturating_add(100).saturating_sub(found);
        search_start = search_start
            .saturating_add(found)
            .saturating_add(ch.len_utf8());
    }
    Some(score)
}

fn tokens_with_ranges(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut current = String::new();
    for (index, ch) in input.chars().enumerate() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                tokens.push(Token {
                    text: std::mem::take(&mut current),
                    start: token_start,
                    end: index,
                });
            }
        } else {
            if start.is_none() {
                start = Some(index);
            }
            current.push(ch);
        }
    }
    if let Some(token_start) = start {
        tokens.push(Token {
            text: current,
            start: token_start,
            end: char_count(input),
        });
    }
    tokens
}

fn char_at(input: &str, index: usize) -> Option<char> {
    input.chars().nth(index)
}

fn char_range(input: &str, start: usize, end: usize) -> String {
    let start_byte = byte_index_at_char(input, start);
    let end_byte = byte_index_at_char(input, end);
    input[start_byte..end_byte].to_string()
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

fn char_count(text: &str) -> usize {
    text.chars().count()
}

fn byte_index_at_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> Vec<ModelOption> {
        vec![
            ModelOption::new("openrouter:xai/glm-5.1").with_display_name("GLM 5.1"),
            ModelOption::new("openrouter:openai/gpt-4o-mini").with_display_name("GPT 4o Mini"),
        ]
    }

    #[test]
    fn suggestions_progress_through_settings_model_command() {
        let view = command_suggestions_for("/settings ", 10, &models())
            .unwrap_or_else(|| panic!("missing view"));
        assert!(view.items.iter().any(|item| item.label == "model"));

        let view = command_suggestions_for("/settings model openrouter:xai", 30, &models())
            .unwrap_or_else(|| panic!("missing model suggestions"));
        assert_eq!(view.title, "Models");
        assert_eq!(view.items[0].label, "openrouter:xai/glm-5.1");
        assert!(view.items[0].complete_on_enter);
    }

    #[test]
    fn model_and_thinking_aliases_suggest_direct_values() {
        let view = command_suggestions_for("/model openrouter:xai", 21, &models())
            .unwrap_or_else(|| panic!("missing model alias suggestions"));
        assert_eq!(view.title, "Models");
        assert_eq!(view.items[0].label, "openrouter:xai/glm-5.1");

        let view = command_suggestions_for("/thinking h", 11, &models())
            .unwrap_or_else(|| panic!("missing thinking alias suggestions"));
        assert_eq!(view.title, "Thinking");
        assert_eq!(view.items[0].label, "high");
    }

    #[test]
    fn model_suggestions_include_all_matches() {
        let many_models = (0..60)
            .map(|index| ModelOption::new(format!("openrouter:model-{index}")))
            .collect::<Vec<_>>();
        let view = command_suggestions_for("/settings model ", 16, &many_models)
            .unwrap_or_else(|| panic!("missing model suggestions"));
        assert_eq!(view.items.len(), 60);
    }

    #[test]
    fn suggestions_cover_nested_settings_values() {
        let view = command_suggestions_for("/settings thinking h", 20, &models())
            .unwrap_or_else(|| panic!("missing thinking suggestions"));
        assert_eq!(view.items[0].label, "high");

        let view = command_suggestions_for("/settings collapse ", 19, &models())
            .unwrap_or_else(|| panic!("missing target suggestions"));
        assert!(view.items.iter().any(|item| item.label == "thinking"));

        let view = command_suggestions_for("/settings collapse thinking t", 29, &models())
            .unwrap_or_else(|| panic!("missing mode suggestions"));
        assert_eq!(view.items[0].label, "truncate");

        let view = command_suggestions_for("/settings chat-style a", 22, &models())
            .unwrap_or_else(|| panic!("missing chat style suggestions"));
        assert_eq!(view.title, "Chat Style");
        assert_eq!(view.items[0].label, "agentic");
    }

    #[test]
    fn file_suggestions_rank_and_replace_at_mentions() {
        let files = vec![
            "README.md".to_string(),
            "crates/oino-tui/src/app.rs".to_string(),
            "crates/oino-app/src/main.rs".to_string(),
        ];
        let view = file_suggestions_for("please inspect @tui/app", 23, &files)
            .unwrap_or_else(|| panic!("missing file suggestions"));

        assert_eq!(view.title, "Files");
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].replacement, "@crates/oino-tui/src/app.rs");
        assert_eq!(view.items[0].replace_start, 15);
    }

    #[test]
    fn suggestions_stay_at_first_slash_command() {
        assert!(command_suggestions_for("/", 1, &models()).is_some());
        assert!(command_suggestions_for("/set", 4, &models()).is_some());
        assert!(command_suggestions_for("hello /", 7, &models()).is_none());
        assert!(command_suggestions_for("/settings model x extra", 17, &models()).is_none());
    }

    #[test]
    fn filters_commands_by_prefix() {
        let view =
            command_suggestions_for("/set", 4, &models()).unwrap_or_else(|| panic!("missing view"));
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].label, "/settings");
        let view = command_suggestions_for("/", 1, &models())
            .unwrap_or_else(|| panic!("missing root view"));
        assert!(view.items.iter().any(|item| item.label == "/new"));
        assert!(view.items.iter().any(|item| item.label == "/sessions"));
        assert!(view.items.iter().any(|item| item.label == "/model"));
        assert!(view.items.iter().any(|item| item.label == "/thinking"));
        let view = command_suggestions_for("/nope", 5, &models())
            .unwrap_or_else(|| panic!("missing view"));
        assert!(view.items.is_empty());
    }

    #[test]
    fn parses_settings_commands() {
        assert_eq!(parse_command("/new"), Some(ParsedCommand::NewSession));
        assert_eq!(parse_command("/sessions"), Some(ParsedCommand::Sessions));
        assert_eq!(
            parse_command("/settings"),
            Some(ParsedCommand::Settings(SettingsCommand::Open))
        );
        assert_eq!(
            parse_command("/model"),
            Some(ParsedCommand::Settings(SettingsCommand::OpenModelSelection))
        );
        assert_eq!(
            parse_command("/thinking"),
            Some(ParsedCommand::Settings(SettingsCommand::OpenThinkingLevel))
        );
        assert_eq!(
            parse_command("/settings model openrouter:xai/glm-5.1"),
            Some(ParsedCommand::Settings(SettingsCommand::SetModel(
                Model::new("openrouter", "xai/glm-5.1")
            )))
        );
        assert_eq!(
            parse_command("/model openrouter:xai/glm-5.1"),
            Some(ParsedCommand::Settings(SettingsCommand::SetModel(
                Model::new("openrouter", "xai/glm-5.1")
            )))
        );
        assert_eq!(
            parse_command("/settings thinking high"),
            Some(ParsedCommand::Settings(SettingsCommand::SetThinkingLevel(
                ThinkingLevel::High
            )))
        );
        assert_eq!(
            parse_command("/thinking high"),
            Some(ParsedCommand::Settings(SettingsCommand::SetThinkingLevel(
                ThinkingLevel::High
            )))
        );
        assert_eq!(
            parse_command("/settings collapse tool truncate"),
            Some(ParsedCommand::Settings(SettingsCommand::SetCollapseMode {
                target: CollapseTarget::Tool,
                mode: CollapseMode::Truncate,
            }))
        );
        assert_eq!(
            parse_command("/settings chat-style agentic"),
            Some(ParsedCommand::Settings(SettingsCommand::SetChatStyle(
                ChatStyle::Agentic
            )))
        );
        assert!(parse_command("/settings model xai/glm-5.1").is_none());
        assert!(parse_command("/set").is_none());
    }
}
