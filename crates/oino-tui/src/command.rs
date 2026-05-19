#![forbid(unsafe_code)]

use crate::fuzzy::{
    ascii_subsequence_match, ascii_subsequence_match_parts, fuzzy_indices, FuzzyMode,
};
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
        name: "/title",
        summary: "Set the current session title",
        kind: CommandKind::Session,
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
        name: "/inspect",
        summary: "Inspect debug runtime state",
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
    Inspect,
    SetSessionTitle(String),
    Settings(SettingsCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsCommand {
    Open,
    OpenModelSelection,
    OpenThinkingLevel,
    OpenChatStyle,
    OpenTools,
    OpenKeymaps,
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
        [settings, subject]
            if settings == "/settings" && (subject == "tools" || subject == "keymaps") =>
        {
            None
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
    let items = if context.query.trim().is_empty() {
        files
            .iter()
            .take(10)
            .map(|file| file_suggestion_item(file, context.replace_start, context.replace_end))
            .collect::<Vec<_>>()
    } else {
        let candidate_indices = file_suggestion_candidate_indices(files, &context.query);
        fuzzy_indices(
            &candidate_indices,
            &context.query,
            FuzzyMode::Path,
            Some(10),
            |index| files[*index].clone(),
        )
        .into_iter()
        .map(|candidate_index| {
            let index = candidate_indices[candidate_index];
            file_suggestion_item(&files[index], context.replace_start, context.replace_end)
        })
        .collect::<Vec<_>>()
    };
    Some(view("Files", context.query, items))
}

fn file_suggestion_item(
    file: &str,
    replace_start: usize,
    replace_end: usize,
) -> CommandSuggestionItem {
    CommandSuggestionItem {
        label: file.to_string(),
        summary: "file".into(),
        replacement: format!("@{file}"),
        replace_start,
        replace_end,
        complete_on_enter: false,
        category: CommandSuggestionCategory::File,
    }
}

fn file_suggestion_candidate_indices(files: &[String], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() || !query.is_ascii() {
        return (0..files.len()).collect();
    }
    files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| ascii_subsequence_match(file, query).then_some(index))
        .collect()
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
    let input = input.trim();
    if let Some(title) = input.strip_prefix("/title ") {
        let title = title.trim();
        if !title.is_empty() {
            return Some(ParsedCommand::SetSessionTitle(title.to_string()));
        }
    }
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        ["/help"] => Some(ParsedCommand::Help),
        ["/new"] => Some(ParsedCommand::NewSession),
        ["/sessions"] => Some(ParsedCommand::Sessions),
        ["/prompts"] => Some(ParsedCommand::Prompts),
        ["/skills"] => Some(ParsedCommand::Skills),
        ["/reload"] => Some(ParsedCommand::ReloadResources),
        ["/inspect"] => Some(ParsedCommand::Inspect),
        ["/settings"] => Some(ParsedCommand::Settings(SettingsCommand::Open)),
        ["/model"] => Some(ParsedCommand::Settings(SettingsCommand::OpenModelSelection)),
        ["/thinking"] => Some(ParsedCommand::Settings(SettingsCommand::OpenThinkingLevel)),
        ["/settings", "chat-style"] | ["/settings", "chat_style"] => {
            Some(ParsedCommand::Settings(SettingsCommand::OpenChatStyle))
        }
        ["/settings", "tools"] => Some(ParsedCommand::Settings(SettingsCommand::OpenTools)),
        ["/settings", "keymaps"] | ["/settings", "keymap"] => {
            Some(ParsedCommand::Settings(SettingsCommand::OpenKeymaps))
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
    if query.trim().is_empty() {
        return prompts
            .iter()
            .map(|prompt| prompt_suggestion_item(prompt, active_prefix, replace_start, replace_end))
            .collect();
    }

    let candidate_indices = prompt_suggestion_candidate_indices(prompts, query);
    fuzzy_indices(&candidate_indices, query, FuzzyMode::Text, None, |index| {
        let prompt = &prompts[*index];
        format!("prompt:{} {}", prompt.name, prompt.description)
    })
    .into_iter()
    .map(|candidate_index| {
        let index = candidate_indices[candidate_index];
        prompt_suggestion_item(&prompts[index], active_prefix, replace_start, replace_end)
    })
    .collect()
}

fn prompt_suggestion_item(
    prompt: &PromptResource,
    active_prefix: &str,
    replace_start: usize,
    replace_end: usize,
) -> CommandSuggestionItem {
    let command = prompt.command();
    CommandSuggestionItem {
        label: command.clone(),
        summary: prompt.description.clone(),
        replacement: command.clone(),
        replace_start,
        replace_end,
        complete_on_enter: active_prefix == command,
        category: CommandSuggestionCategory::Prompt,
    }
}

fn prompt_suggestion_candidate_indices(prompts: &[PromptResource], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() || !query.is_ascii() {
        return (0..prompts.len()).collect();
    }
    prompts
        .iter()
        .enumerate()
        .filter_map(|(index, prompt)| {
            ascii_subsequence_match_parts(
                [
                    "prompt:",
                    prompt.name.as_str(),
                    " ",
                    prompt.description.as_str(),
                ],
                query,
            )
            .then_some(index)
        })
        .collect()
}

fn skill_items(
    skills: &[SkillResource],
    query: &str,
    active_prefix: &str,
    replace_start: usize,
    replace_end: usize,
) -> Vec<CommandSuggestionItem> {
    if query.trim().is_empty() {
        return skills
            .iter()
            .map(|skill| skill_suggestion_item(skill, active_prefix, replace_start, replace_end))
            .collect();
    }

    let candidate_indices = skill_suggestion_candidate_indices(skills, query);
    fuzzy_indices(&candidate_indices, query, FuzzyMode::Text, None, |index| {
        let skill = &skills[*index];
        format!("skill:{} {}", skill.name, skill.description)
    })
    .into_iter()
    .map(|candidate_index| {
        let index = candidate_indices[candidate_index];
        skill_suggestion_item(&skills[index], active_prefix, replace_start, replace_end)
    })
    .collect()
}

fn skill_suggestion_item(
    skill: &SkillResource,
    active_prefix: &str,
    replace_start: usize,
    replace_end: usize,
) -> CommandSuggestionItem {
    let command = skill.command();
    CommandSuggestionItem {
        label: command.clone(),
        summary: skill.description.clone(),
        replacement: command.clone(),
        replace_start,
        replace_end,
        complete_on_enter: active_prefix == command,
        category: CommandSuggestionCategory::Skill,
    }
}

fn skill_suggestion_candidate_indices(skills: &[SkillResource], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() || !query.is_ascii() {
        return (0..skills.len()).collect();
    }
    skills
        .iter()
        .enumerate()
        .filter_map(|(index, skill)| {
            ascii_subsequence_match_parts(
                [
                    "skill:",
                    skill.name.as_str(),
                    " ",
                    skill.description.as_str(),
                ],
                query,
            )
            .then_some(index)
        })
        .collect()
}

fn suggestion_match_text(item: &CommandSuggestionItem) -> String {
    format!("{} {}", item.label.trim_start_matches('/'), item.summary)
}

fn settings_subject_suggestions(context: SuggestionContext) -> Option<CommandSuggestionsView> {
    let subjects = [
        ("model", "Set selected model", false),
        ("thinking", "Set thinking level", false),
        ("collapse", "Set thinking/tool collapse mode", false),
        ("chat-style", "Set transcript rendering style", true),
        ("tools", "Show registered agent tools by scope", true),
        ("keymaps", "Configure keyboard shortcuts", true),
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
        let (subject, summary, complete_on_exact) = subjects[index];
        let mut item = incomplete_item(subject, summary, &context);
        item.complete_on_enter = complete_on_exact && context.active_prefix == subject;
        item
    })
    .collect::<Vec<_>>();
    Some(view("Settings", context.active_prefix, items))
}

fn model_suggestions(
    context: SuggestionContext,
    models: &[ModelOption],
) -> Option<CommandSuggestionsView> {
    let candidate_indices = model_suggestion_candidate_indices(models, &context.active_prefix);
    let items = fuzzy_indices(
        &candidate_indices,
        &context.active_prefix,
        FuzzyMode::Text,
        None,
        |index| {
            let model = &models[*index];
            format!("{} {}", model.id, model.display_name)
        },
    )
    .into_iter()
    .map(|candidate_index| {
        let index = candidate_indices[candidate_index];
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

fn model_suggestion_candidate_indices(models: &[ModelOption], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() || !query.is_ascii() {
        return (0..models.len()).collect();
    }
    models
        .iter()
        .enumerate()
        .filter_map(|(index, model)| {
            ascii_subsequence_match_parts(
                [model.id.as_str(), " ", model.display_name.as_str()],
                query,
            )
            .then_some(index)
        })
        .collect()
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
        ("collapse", "Hide detailed content"),
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
    let (token_start, token_end) = token_bounds_at_cursor(input, cursor)?;
    if char_at(input, token_start) != Some('@') {
        return None;
    }
    let query_end = cursor.min(token_end);
    let query = char_range(input, token_start.saturating_add(1), query_end);
    Some(FileSuggestionContext {
        query,
        replace_start: token_start,
        replace_end: token_end,
    })
}

fn token_bounds_at_cursor(input: &str, cursor: usize) -> Option<(usize, usize)> {
    let mut start = None;
    for (index, ch) in input.chars().enumerate() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                if token_start < cursor.saturating_add(1) && cursor <= index {
                    return Some((token_start, index));
                }
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    start.and_then(|token_start| {
        let token_end = char_count(input);
        (token_start < cursor.saturating_add(1) && cursor <= token_end)
            .then_some((token_start, token_end))
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

        let view = suggestions("/settings too", 13)
            .unwrap_or_else(|| panic!("missing tools settings suggestion"));
        assert!(view.items.iter().any(|item| item.label == "tools"));
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
    fn file_suggestion_context_finds_active_token_without_tokenizing_all_input() {
        let context = file_suggestion_context("before @src/main.rs after", 16)
            .unwrap_or_else(|| panic!("missing file context"));

        assert_eq!(context.query, "src/main");
        assert_eq!(context.replace_start, 7);
        assert_eq!(context.replace_end, 19);
        assert_eq!(
            token_bounds_at_cursor("before @src/main.rs after", 16),
            Some((7, 19))
        );
    }

    #[test]
    fn empty_file_suggestions_are_limited_without_fuzzy_scan() {
        let files = (0..20)
            .map(|index| format!("file-{index}.rs"))
            .collect::<Vec<_>>();
        let view = file_suggestions_for("attach @", 8, &files)
            .unwrap_or_else(|| panic!("missing file suggestions"));

        assert_eq!(view.items.len(), 10);
        assert_eq!(view.items[0].replacement, "@file-0.rs");
        assert_eq!(view.items[9].replacement, "@file-9.rs");
    }

    #[test]
    fn file_suggestions_prefilter_keeps_case_insensitive_subsequence_matches() {
        let files = vec![
            "README.md".to_string(),
            "crates/Oino-Tui/src/App.rs".to_string(),
            "crates/oino-app/src/main.rs".to_string(),
        ];
        let view = file_suggestions_for("open @TUI/App", 13, &files)
            .unwrap_or_else(|| panic!("missing file suggestions"));

        assert_eq!(view.items[0].replacement, "@crates/Oino-Tui/src/App.rs");
        assert!(ascii_subsequence_match(
            "crates/Oino-Tui/src/App.rs",
            "TUI/App"
        ));
    }

    #[test]
    fn model_suggestions_prefilter_checks_id_and_display_name() {
        let models = vec![
            ModelOption::new("openrouter:a/alpha"),
            ModelOption::new("openrouter:b/bravo").with_display_name("Displayed Model"),
        ];
        let view = command_suggestions_for("/model displayed", 16, &models, &[], &[])
            .unwrap_or_else(|| panic!("missing model suggestions"));

        assert_eq!(view.items[0].label, "openrouter:b/bravo");
        assert!(ascii_subsequence_match_parts(
            ["openrouter:b/bravo", "Displayed Model"],
            "displayed"
        ));
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
        assert_eq!(parse_command("/inspect"), Some(ParsedCommand::Inspect));
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
            parse_command("/title Design polish pass"),
            Some(ParsedCommand::SetSessionTitle("Design polish pass".into()))
        );
        assert_eq!(
            parse_command("/settings tools"),
            Some(ParsedCommand::Settings(SettingsCommand::OpenTools))
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
