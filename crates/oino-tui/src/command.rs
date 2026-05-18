#![forbid(unsafe_code)]

use crate::fuzzy::{fuzzy_indices, FuzzyMode};
use crate::resource::{PromptResource, SkillResource};
use crate::settings::{
    chat_style_value as settings_chat_style_value, parse_chat_style as settings_parse_chat_style,
    ChatStyle, CollapseMode, CollapseTarget, ModelOption,
};
use oino_types::{Model, ThinkingLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Session,
    Settings,
    Resource,
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
    CommandSpec {
        name: "/help",
        summary: "Open keyboard and command help",
        kind: CommandKind::Settings,
    },
    CommandSpec {
        name: "/prompts",
        summary: "Browse prompt templates",
        kind: CommandKind::Resource,
    },
    CommandSpec {
        name: "/skills",
        summary: "Browse skills",
        kind: CommandKind::Resource,
    },
    CommandSpec {
        name: "/reload",
        summary: "Reload Oino resources",
        kind: CommandKind::Resource,
    },
];

const RESOURCE_PREFIX_SUGGESTIONS: &[(&str, &str, CommandSuggestionCategory)] = &[
    (
        "/prompt:",
        "Include a prompt template by name",
        CommandSuggestionCategory::Prompt,
    ),
    (
        "/skill:",
        "Include a skill by name",
        CommandSuggestionCategory::Skill,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    Help,
    NewSession,
    Sessions,
    Prompts,
    Skills,
    ReloadResources,
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
    cached: Option<CachedCommandSuggestions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedCommandSuggestions {
    input: String,
    cursor: usize,
    view: CommandSuggestionsView,
}

impl CommandSuggestionsState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_selection(&mut self, delta: isize, len: usize) {
        self.selected = move_index(self.selected, len, delta);
        self.sync_cached_selection();
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len.saturating_sub(1));
        }
        self.sync_cached_selection();
    }

    pub fn dismiss_for(&mut self, input: &str) {
        self.dismissed_input = Some(input.to_string());
        self.cached = None;
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

    pub fn cache_view(&mut self, input: &str, cursor: usize, view: Option<CommandSuggestionsView>) {
        self.cached = view.map(|mut view| {
            self.clamp_selection_to(view.items.len());
            view.selected = self.selected;
            CachedCommandSuggestions {
                input: input.to_string(),
                cursor,
                view,
            }
        });
    }

    pub fn clear_cache(&mut self) {
        self.cached = None;
    }

    #[must_use]
    pub fn cached_view(&self, input: &str, cursor: usize) -> Option<CommandSuggestionsView> {
        let cached = self.cached.as_ref()?;
        if cached.input != input || cached.cursor != cursor {
            return None;
        }
        let mut view = cached.view.clone();
        view.selected = if view.items.is_empty() {
            0
        } else {
            self.selected.min(view.items.len().saturating_sub(1))
        };
        Some(view)
    }

    fn clamp_selection_to(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len.saturating_sub(1));
        }
    }

    fn sync_cached_selection(&mut self) {
        let Some(len) = self.cached.as_ref().map(|cached| cached.view.items.len()) else {
            return;
        };
        self.clamp_selection_to(len);
        if let Some(cached) = &mut self.cached {
            cached.view.selected = self.selected;
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
    pub category: CommandSuggestionCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSuggestionCategory {
    System,
    Prompt,
    Skill,
    Model,
    File,
    Value,
}

impl CommandSuggestionCategory {
    #[must_use]
    pub const fn label(self) -> Option<&'static str> {
        match self {
            Self::System => Some("[SYS]"),
            Self::Prompt => Some("[PROMPT]"),
            Self::Skill => Some("[SKILL]"),
            Self::Model | Self::File | Self::Value => None,
        }
    }
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
    prompts: &[PromptResource],
    skills: &[SkillResource],
) -> Option<CommandSuggestionsView> {
    if let Some((context, scope)) = resource_suggestion_context(input, cursor) {
        return match scope {
            ResourceSuggestionScope::PromptShort | ResourceSuggestionScope::PromptLong => {
                prompt_suggestions(context, prompts, scope)
            }
            ResourceSuggestionScope::SkillShort | ResourceSuggestionScope::SkillLong => {
                skill_suggestions(context, skills, scope)
            }
        };
    }

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
    let items = fuzzy_indices(
        files,
        &context.query,
        FuzzyMode::Path,
        Some(10),
        Clone::clone,
    )
    .into_iter()
    .map(|index| {
        let file = &files[index];
        CommandSuggestionItem {
            label: file.clone(),
            summary: "file".into(),
            replacement: format!("@{file}"),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: false,
            category: CommandSuggestionCategory::File,
        }
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
        ["/help"] => Some(ParsedCommand::Help),
        ["/new"] => Some(ParsedCommand::NewSession),
        ["/sessions"] => Some(ParsedCommand::Sessions),
        ["/prompts"] => Some(ParsedCommand::Prompts),
        ["/skills"] => Some(ParsedCommand::Skills),
        ["/reload"] => Some(ParsedCommand::ReloadResources),
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
    let query = context.active_prefix.trim_start_matches('/');
    let mut candidates = Vec::new();
    if query.is_empty() {
        candidates.extend(root_command_items(&context));
        candidates.extend(root_resource_prefix_items(&context));
    } else {
        candidates.extend(root_resource_prefix_items(&context));
        candidates.extend(root_command_items(&context));
    }
    let mut items = fuzzy_indices(
        &candidates,
        query,
        FuzzyMode::Text,
        None,
        suggestion_match_text,
    )
    .into_iter()
    .map(|index| candidates[index].clone())
    .collect::<Vec<_>>();
    items.sort_by_key(|item| root_resource_prefix_priority(query, item));
    Some(view("Commands", context.active_prefix, items))
}

fn root_resource_prefix_priority(query: &str, item: &CommandSuggestionItem) -> usize {
    if query.len() >= 2
        && (("skill:".starts_with(query) && item.label == "/skill:")
            || ("prompt:".starts_with(query) && item.label == "/prompt:"))
    {
        0
    } else {
        1
    }
}

fn root_command_items(context: &SuggestionContext) -> Vec<CommandSuggestionItem> {
    COMMANDS
        .iter()
        .map(|command| CommandSuggestionItem {
            label: command.name.into(),
            summary: command.summary.into(),
            replacement: command.name.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
            category: CommandSuggestionCategory::System,
        })
        .collect()
}

fn root_resource_prefix_items(context: &SuggestionContext) -> Vec<CommandSuggestionItem> {
    RESOURCE_PREFIX_SUGGESTIONS
        .iter()
        .map(|(label, summary, category)| CommandSuggestionItem {
            label: (*label).into(),
            summary: (*summary).into(),
            replacement: (*label).into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: false,
            category: *category,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceSuggestionScope {
    PromptShort,
    PromptLong,
    SkillShort,
    SkillLong,
}

fn prompt_suggestions(
    context: SuggestionContext,
    prompts: &[PromptResource],
    scope: ResourceSuggestionScope,
) -> Option<CommandSuggestionsView> {
    let query = resource_query(&context.active_prefix, scope);
    let items = prompt_items(
        prompts,
        query,
        &context.active_prefix,
        context.replace_start,
        context.replace_end,
    );
    Some(view("Prompts", query.to_string(), items))
}

fn skill_suggestions(
    context: SuggestionContext,
    skills: &[SkillResource],
    scope: ResourceSuggestionScope,
) -> Option<CommandSuggestionsView> {
    let query = resource_query(&context.active_prefix, scope);
    let items = skill_items(
        skills,
        query,
        &context.active_prefix,
        context.replace_start,
        context.replace_end,
    );
    Some(view("Skills", query.to_string(), items))
}

fn resource_query(active_prefix: &str, scope: ResourceSuggestionScope) -> &str {
    match scope {
        ResourceSuggestionScope::PromptShort => active_prefix.trim_start_matches("/P:"),
        ResourceSuggestionScope::PromptLong => active_prefix.trim_start_matches("/prompt:"),
        ResourceSuggestionScope::SkillShort => active_prefix.trim_start_matches("/S:"),
        ResourceSuggestionScope::SkillLong => active_prefix.trim_start_matches("/skill:"),
    }
}

fn prompt_items(
    prompts: &[PromptResource],
    query: &str,
    active_prefix: &str,
    replace_start: usize,
    replace_end: usize,
) -> Vec<CommandSuggestionItem> {
    let candidates = prompts
        .iter()
        .map(|prompt| CommandSuggestionItem {
            label: prompt.command(),
            summary: prompt.description.clone(),
            replacement: prompt.command(),
            replace_start,
            replace_end,
            complete_on_enter: active_prefix == prompt.command(),
            category: CommandSuggestionCategory::Prompt,
        })
        .collect::<Vec<_>>();
    fuzzy_indices(
        &candidates,
        query,
        FuzzyMode::Text,
        None,
        suggestion_match_text,
    )
    .into_iter()
    .map(|index| candidates[index].clone())
    .collect()
}

fn skill_items(
    skills: &[SkillResource],
    query: &str,
    active_prefix: &str,
    replace_start: usize,
    replace_end: usize,
) -> Vec<CommandSuggestionItem> {
    let candidates = skills
        .iter()
        .map(|skill| CommandSuggestionItem {
            label: skill.command(),
            summary: skill.description.clone(),
            replacement: skill.command(),
            replace_start,
            replace_end,
            complete_on_enter: active_prefix == skill.command(),
            category: CommandSuggestionCategory::Skill,
        })
        .collect::<Vec<_>>();
    fuzzy_indices(
        &candidates,
        query,
        FuzzyMode::Text,
        None,
        suggestion_match_text,
    )
    .into_iter()
    .map(|index| candidates[index].clone())
    .collect()
}

fn suggestion_match_text(item: &CommandSuggestionItem) -> String {
    format!("{} {}", item.label.trim_start_matches('/'), item.summary)
}

fn settings_subject_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let subjects = [
        ("model", "Set selected model"),
        ("thinking", "Set thinking level"),
        ("collapse", "Set thinking/tool collapse mode"),
        ("chat-style", "Set transcript rendering style"),
    ];
    let items = fuzzy_indices(
        &subjects,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |entry| format!("{} {}", entry.0, entry.1),
    )
    .into_iter()
    .map(|index| {
        let (subject, summary) = subjects[index];
        incomplete_item(subject, summary, &context)
    })
    .collect::<Vec<_>>();
    Some(view("Settings", context.active_prefix, items))
}

fn model_suggestions(
    context: SuggestionContext,
    models: &[ModelOption],
) -> Option<CommandSuggestionsView> {
    let items = fuzzy_indices(
        models,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |model| format!("{} {}", model.id, model.display_name),
    )
    .into_iter()
    .map(|index| {
        let model = &models[index];
        CommandSuggestionItem {
            label: model.id.clone(),
            summary: model.display_name.clone(),
            replacement: model.id.clone(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
            category: CommandSuggestionCategory::Model,
        }
    })
    .collect::<Vec<_>>();
    Some(view("Models", context.active_prefix, items))
}

fn thinking_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let levels = [
        (ThinkingLevel::Off, "Disable provider reasoning"),
        (ThinkingLevel::Minimal, "Minimal reasoning"),
        (ThinkingLevel::Low, "Low reasoning"),
        (ThinkingLevel::Medium, "Medium reasoning"),
        (ThinkingLevel::High, "High reasoning"),
        (ThinkingLevel::XHigh, "Extra-high reasoning"),
    ];
    let items = fuzzy_indices(
        &levels,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |entry| format!("{} {}", thinking_level_value(entry.0), entry.1),
    )
    .into_iter()
    .map(|index| {
        let (level, summary) = levels[index];
        let value = thinking_level_value(level);
        CommandSuggestionItem {
            label: value.into(),
            summary: summary.into(),
            replacement: value.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
            category: CommandSuggestionCategory::Value,
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
    let items = fuzzy_indices(
        &targets,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |entry| format!("{} {}", entry.0, entry.1),
    )
    .into_iter()
    .map(|index| {
        let (target, summary) = targets[index];
        incomplete_item(target, summary, &context)
    })
    .collect::<Vec<_>>();
    Some(view("Collapse Target", context.active_prefix, items))
}

fn collapse_mode_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let modes = [
        ("full", "Show full content"),
        ("truncate", "Show short preview"),
        ("collapse", "Hide content behind placeholder"),
    ];
    let items = fuzzy_indices(
        &modes,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |entry| format!("{} {}", entry.0, entry.1),
    )
    .into_iter()
    .map(|index| {
        let (mode, summary) = modes[index];
        CommandSuggestionItem {
            label: mode.into(),
            summary: summary.into(),
            replacement: mode.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
            category: CommandSuggestionCategory::Value,
        }
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
    let items = fuzzy_indices(
        &styles,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |entry| format!("{} {}", chat_style_value(entry.0), entry.1),
    )
    .into_iter()
    .map(|index| {
        let (style, summary) = styles[index];
        let value = chat_style_value(style);
        CommandSuggestionItem {
            label: value.into(),
            summary: summary.into(),
            replacement: value.into(),
            replace_start: context.replace_start,
            replace_end: context.replace_end,
            complete_on_enter: true,
            category: CommandSuggestionCategory::Value,
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
        category: CommandSuggestionCategory::Value,
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

fn resource_suggestion_context(
    input: &str,
    cursor: usize,
) -> Option<(SuggestionContext, ResourceSuggestionScope)> {
    if input.contains('\n') {
        return None;
    }
    let len = char_count(input);
    let cursor = cursor.min(len);
    let previous_is_whitespace =
        cursor > 0 && char_at(input, cursor.saturating_sub(1)).is_some_and(char::is_whitespace);
    if previous_is_whitespace {
        return None;
    }

    let active = tokens_with_ranges(input)
        .into_iter()
        .find(|token| token.start <= cursor && cursor <= token.end)?;
    let active_prefix = char_range(input, active.start, cursor);
    let scope = if active_prefix.starts_with("/P:") {
        ResourceSuggestionScope::PromptShort
    } else if active_prefix.starts_with("/prompt:") {
        ResourceSuggestionScope::PromptLong
    } else if active_prefix.starts_with("/S:") {
        ResourceSuggestionScope::SkillShort
    } else if active_prefix.starts_with("/skill:") {
        ResourceSuggestionScope::SkillLong
    } else {
        return None;
    };

    Some((
        SuggestionContext {
            completed: Vec::new(),
            active_prefix,
            replace_start: active.start,
            replace_end: active.end,
        },
        scope,
    ))
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

    fn prompts() -> Vec<PromptResource> {
        vec![PromptResource {
            name: "review".into(),
            description: "Review current changes".into(),
            argument_hint: Some("[focus]".into()),
            source: ".oino/prompts/review.md".into(),
            scope: "project".into(),
            content: "Review $ARGUMENTS".into(),
        }]
    }

    fn skills() -> Vec<SkillResource> {
        vec![SkillResource {
            name: "debug".into(),
            description: "Investigate a bug".into(),
            source: ".oino/skills/debug/SKILL.md".into(),
            scope: "project".into(),
            content: "# Debug".into(),
        }]
    }

    fn suggestions(input: &str, cursor: usize) -> Option<CommandSuggestionsView> {
        command_suggestions_for(input, cursor, &models(), &prompts(), &skills())
    }

    #[test]
    fn suggestions_progress_through_settings_model_command() {
        let view = suggestions("/settings ", 10).unwrap_or_else(|| panic!("missing view"));
        assert!(view.items.iter().any(|item| item.label == "model"));

        let view = suggestions("/settings model openrouter:xai", 30)
            .unwrap_or_else(|| panic!("missing model suggestions"));
        assert_eq!(view.title, "Models");
        assert_eq!(view.items[0].label, "openrouter:xai/glm-5.1");
        assert!(view.items[0].complete_on_enter);
    }

    #[test]
    fn model_and_thinking_aliases_suggest_direct_values() {
        let view = suggestions("/model openrouter:xai", 21)
            .unwrap_or_else(|| panic!("missing model alias suggestions"));
        assert_eq!(view.title, "Models");
        assert_eq!(view.items[0].label, "openrouter:xai/glm-5.1");

        let view = suggestions("/thinking h", 11)
            .unwrap_or_else(|| panic!("missing thinking alias suggestions"));
        assert_eq!(view.title, "Thinking");
        assert_eq!(view.items[0].label, "high");
    }

    #[test]
    fn model_suggestions_include_all_matches() {
        let many_models = (0..60)
            .map(|index| ModelOption::new(format!("openrouter:model-{index}")))
            .collect::<Vec<_>>();
        let view = command_suggestions_for("/settings model ", 16, &many_models, &[], &[])
            .unwrap_or_else(|| panic!("missing model suggestions"));
        assert_eq!(view.items.len(), 60);
    }

    #[test]
    fn suggestions_cover_nested_settings_values() {
        let view = suggestions("/settings thinking h", 20)
            .unwrap_or_else(|| panic!("missing thinking suggestions"));
        assert_eq!(view.items[0].label, "high");

        let view = suggestions("/settings collapse ", 19)
            .unwrap_or_else(|| panic!("missing target suggestions"));
        assert!(view.items.iter().any(|item| item.label == "thinking"));

        let view = suggestions("/settings collapse thinking t", 29)
            .unwrap_or_else(|| panic!("missing mode suggestions"));
        assert_eq!(view.items[0].label, "truncate");

        let view = suggestions("/settings chat-style a", 22)
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
        assert!(suggestions("/", 1).is_some());
        assert!(suggestions("/set", 4).is_some());
        assert!(suggestions("hello /", 7).is_none());
        assert!(suggestions("/settings model x extra", 17).is_none());
    }

    #[test]
    fn filters_commands_by_prefix() {
        let view = suggestions("/set", 4).unwrap_or_else(|| panic!("missing view"));
        assert!(view.items.iter().any(|item| item.label == "/settings"));
        let view = suggestions("/", 1).unwrap_or_else(|| panic!("missing root view"));
        assert!(view.items.iter().any(|item| item.label == "/new"));
        assert!(view.items.iter().any(|item| item.label == "/sessions"));
        assert!(view.items.iter().any(|item| item.label == "/help"));
        assert!(view.items.iter().any(|item| item.label == "/model"));
        assert!(view.items.iter().any(|item| item.label == "/thinking"));
        assert!(view.items.iter().any(|item| item.label == "/prompts"));
        assert!(view.items.iter().any(|item| item.label == "/skills"));
        assert!(view.items.iter().any(|item| item.label == "/reload"));
        assert!(view.items.iter().any(|item| item.label == "/prompt:"));
        assert!(view.items.iter().any(|item| item.label == "/skill:"));
        let view = suggestions("/zzzz", 5).unwrap_or_else(|| panic!("missing view"));
        assert!(view.items.is_empty());
    }

    #[test]
    fn resource_suggestions_support_labels_and_scoped_search() {
        let view = suggestions("/", 1).unwrap_or_else(|| panic!("missing root view"));
        let prompt_prefix = view
            .items
            .iter()
            .find(|item| item.label == "/prompt:")
            .unwrap_or_else(|| panic!("missing prompt prefix"));
        assert_eq!(prompt_prefix.category, CommandSuggestionCategory::Prompt);
        assert_eq!(prompt_prefix.replacement, "/prompt:");
        assert!(!prompt_prefix.complete_on_enter);
        let skill_prefix = view
            .items
            .iter()
            .find(|item| item.label == "/skill:")
            .unwrap_or_else(|| panic!("missing skill prefix"));
        assert_eq!(skill_prefix.category, CommandSuggestionCategory::Skill);
        assert_eq!(skill_prefix.replacement, "/skill:");
        assert!(!skill_prefix.complete_on_enter);

        let view = suggestions("/prompt:rev", 11)
            .unwrap_or_else(|| panic!("missing prompt command suggestions"));
        let prompt = view
            .items
            .iter()
            .find(|item| item.label == "/prompt:review")
            .unwrap_or_else(|| panic!("missing review prompt"));
        assert_eq!(prompt.replacement, "/prompt:review");
        assert!(!prompt.complete_on_enter);
        assert_eq!(prompt.category, CommandSuggestionCategory::Prompt);
        assert_eq!(prompt.category.label(), Some("[PROMPT]"));

        let view =
            suggestions("please /P:rev", 13).unwrap_or_else(|| panic!("missing scoped prompts"));
        assert_eq!(view.title, "Prompts");
        assert_eq!(view.items[0].replacement, "/prompt:review");

        let view = suggestions("use /S:bug", 10).unwrap_or_else(|| panic!("missing scoped skills"));
        assert_eq!(view.title, "Skills");
        assert_eq!(view.items[0].replacement, "/skill:debug");
        assert_eq!(view.items[0].category.label(), Some("[SKILL]"));

        let view = suggestions("use /skill:bug", 14)
            .unwrap_or_else(|| panic!("missing skill command suggestions"));
        assert_eq!(view.items[0].replacement, "/skill:debug");
    }

    #[test]
    fn parses_settings_commands() {
        assert_eq!(parse_command("/help"), Some(ParsedCommand::Help));
        assert_eq!(parse_command("/new"), Some(ParsedCommand::NewSession));
        assert_eq!(parse_command("/sessions"), Some(ParsedCommand::Sessions));
        assert_eq!(parse_command("/prompts"), Some(ParsedCommand::Prompts));
        assert_eq!(parse_command("/skills"), Some(ParsedCommand::Skills));
        assert_eq!(
            parse_command("/reload"),
            Some(ParsedCommand::ReloadResources)
        );
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
