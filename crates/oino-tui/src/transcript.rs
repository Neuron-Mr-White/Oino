#![forbid(unsafe_code)]

use crate::{
    markdown::{prefixed_markdown_lines, render_markdown_lines},
    message::{MessageView, ToolCallView},
    settings::{ChatStyle, CollapseMode},
    text::{truncate_to_width, wrap_text},
    theme::Theme,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
pub(crate) fn transcript_lines(
    messages: &[MessageView],
    error: Option<&str>,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    chat_style: ChatStyle,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let blocks = transcript_line_blocks(
        messages,
        error,
        width,
        thinking_mode,
        tool_mode,
        chat_style,
        theme,
    );
    let total_lines = blocks.iter().map(|block| block.len()).sum();
    let mut lines = Vec::with_capacity(total_lines);
    for block in blocks {
        lines.extend(block.iter().cloned());
    }
    lines
}

pub(crate) fn transcript_line_blocks(
    messages: &[MessageView],
    error: Option<&str>,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    chat_style: ChatStyle,
    theme: &Theme,
) -> Vec<Arc<Vec<Line<'static>>>> {
    match chat_style {
        ChatStyle::Chat => {
            chat_transcript_blocks(messages, error, width, thinking_mode, tool_mode, theme)
        }
        ChatStyle::Agentic => {
            agentic_transcript_blocks(messages, error, width, thinking_mode, tool_mode, theme)
        }
        ChatStyle::Minimal => {
            minimal_transcript_blocks(messages, error, width, thinking_mode, tool_mode, theme)
        }
    }
}

fn chat_transcript_blocks(
    messages: &[MessageView],
    error: Option<&str>,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Arc<Vec<Line<'static>>>> {
    let mut blocks = Vec::new();
    let theme_hash = theme_cache_hash(theme);
    for message in messages {
        append_spaced_block(
            &mut blocks,
            cached_chat_message_lines(message, width, thinking_mode, tool_mode, theme, theme_hash),
        );
    }
    if let Some(error) = error {
        let error_message = synthetic_error_message(error);
        append_spaced_block(
            &mut blocks,
            cached_chat_message_lines(
                &error_message,
                width,
                thinking_mode,
                tool_mode,
                theme,
                theme_hash,
            ),
        );
    }
    blocks
}

fn agentic_transcript_blocks(
    messages: &[MessageView],
    error: Option<&str>,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Arc<Vec<Line<'static>>>> {
    let mut blocks = Vec::new();
    let theme_hash = theme_cache_hash(theme);
    let relation_hash = message_relation_hash(messages);
    for (index, message) in messages.iter().enumerate() {
        append_spaced_block(
            &mut blocks,
            cached_agentic_message_lines(
                message,
                messages,
                index,
                width,
                thinking_mode,
                tool_mode,
                theme,
                theme_hash,
                relation_hash,
            ),
        );
    }
    if let Some(error) = error {
        let error_message = synthetic_error_message(error);
        append_spaced_block(
            &mut blocks,
            cached_agentic_message_lines(
                &error_message,
                messages,
                messages.len(),
                width,
                thinking_mode,
                tool_mode,
                theme,
                theme_hash,
                relation_hash,
            ),
        );
    }
    blocks
}

fn minimal_transcript_blocks(
    messages: &[MessageView],
    error: Option<&str>,
    width: usize,
    thinking_mode: CollapseMode,
    _tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Arc<Vec<Line<'static>>>> {
    let mut blocks = Vec::new();
    let theme_hash = theme_cache_hash(theme);
    let relation_hash = message_relation_hash(messages);
    let mut user_index = 0usize;
    for (index, message) in messages.iter().enumerate() {
        if message.is_user() {
            user_index = user_index.saturating_add(1);
        }
        append_minimal_block(
            &mut blocks,
            cached_minimal_message_lines(
                message,
                messages,
                index,
                user_index,
                width,
                thinking_mode,
                theme,
                theme_hash,
                relation_hash,
            ),
            &message.role,
        );
    }
    if let Some(error) = error {
        let error_message = synthetic_error_message(error);
        append_minimal_block(
            &mut blocks,
            cached_minimal_message_lines(
                &error_message,
                messages,
                messages.len(),
                user_index,
                width,
                thinking_mode,
                theme,
                theme_hash,
                relation_hash,
            ),
            &error_message.role,
        );
    }
    blocks
}

fn append_spaced_block(blocks: &mut Vec<Arc<Vec<Line<'static>>>>, block: Arc<Vec<Line<'static>>>) {
    if block.is_empty() {
        return;
    }
    if !blocks.is_empty() {
        blocks.push(blank_line_block());
    }
    blocks.push(block);
}

fn append_minimal_block(
    blocks: &mut Vec<Arc<Vec<Line<'static>>>>,
    block: Arc<Vec<Line<'static>>>,
    role: &str,
) {
    if block.is_empty() {
        return;
    }
    let compact_continuation = role.starts_with("tool:") || matches!(role, "compaction" | "branch");
    if !blocks.is_empty() && !compact_continuation {
        blocks.push(blank_line_block());
    }
    blocks.push(block);
}

static BLANK_LINE_BLOCK: OnceLock<Arc<Vec<Line<'static>>>> = OnceLock::new();

fn blank_line_block() -> Arc<Vec<Line<'static>>> {
    BLANK_LINE_BLOCK
        .get_or_init(|| Arc::new(vec![Line::from("")]))
        .clone()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MessageLineCacheKey {
    style: u8,
    width: usize,
    thinking_mode: u8,
    tool_mode: u8,
    message_hash: u64,
    context_hash: u64,
    theme_hash: u64,
}

#[derive(Default)]
struct MessageLineCacheState {
    entries: HashMap<MessageLineCacheKey, Arc<Vec<Line<'static>>>>,
    order: VecDeque<MessageLineCacheKey>,
}

impl MessageLineCacheState {
    fn get(&mut self, key: &MessageLineCacheKey) -> Option<Arc<Vec<Line<'static>>>> {
        let lines = self.entries.get(key)?.clone();
        if let Some(position) = self.order.iter().position(|entry| entry == key) {
            if let Some(entry) = self.order.remove(position) {
                self.order.push_back(entry);
            }
        }
        Some(lines)
    }

    fn insert(&mut self, key: MessageLineCacheKey, lines: Arc<Vec<Line<'static>>>) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key.clone(), lines);
            if let Some(position) = self.order.iter().position(|entry| entry == &key) {
                let _ = self.order.remove(position);
            }
            self.order.push_back(key);
            return;
        }

        self.entries.insert(key.clone(), lines);
        self.order.push_back(key);
        while self.order.len() > MESSAGE_LINE_CACHE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

const MESSAGE_LINE_CACHE_LIMIT: usize = 4096;
static MESSAGE_LINE_CACHE: OnceLock<Mutex<MessageLineCacheState>> = OnceLock::new();

fn message_line_cache() -> &'static Mutex<MessageLineCacheState> {
    MESSAGE_LINE_CACHE.get_or_init(|| Mutex::new(MessageLineCacheState::default()))
}

fn cached_message_lines(
    key: MessageLineCacheKey,
    render: impl FnOnce() -> Vec<Line<'static>>,
) -> Arc<Vec<Line<'static>>> {
    if cfg!(test) {
        return Arc::new(render());
    }

    let mut cache = match message_line_cache().lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(lines) = cache.get(&key) {
        return lines;
    }
    drop(cache);

    let lines = Arc::new(render());
    if let Ok(mut cache) = message_line_cache().lock() {
        cache.insert(key, lines.clone());
    }
    lines
}

fn cached_chat_message_lines(
    message: &MessageView,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
    theme_hash: u64,
) -> Arc<Vec<Line<'static>>> {
    let key = MessageLineCacheKey {
        style: chat_style_key(ChatStyle::Chat),
        width,
        thinking_mode: collapse_mode_key(thinking_mode),
        tool_mode: collapse_mode_key(tool_mode),
        message_hash: message_cache_hash(message),
        context_hash: 0,
        theme_hash,
    };
    cached_message_lines(key, || {
        bubble_lines(message, width, thinking_mode, tool_mode, theme)
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "message rendering depends on transcript context and display settings"
)]
fn cached_agentic_message_lines(
    message: &MessageView,
    messages: &[MessageView],
    index: usize,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
    theme_hash: u64,
    relation_hash: u64,
) -> Arc<Vec<Line<'static>>> {
    let key = MessageLineCacheKey {
        style: chat_style_key(ChatStyle::Agentic),
        width,
        thinking_mode: collapse_mode_key(thinking_mode),
        tool_mode: collapse_mode_key(tool_mode),
        message_hash: message_cache_hash(message),
        context_hash: relation_hash,
        theme_hash,
    };
    cached_message_lines(key, || {
        agentic_message_lines(
            message,
            messages,
            index,
            width,
            thinking_mode,
            tool_mode,
            theme,
        )
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "message rendering depends on transcript context and display settings"
)]
fn cached_minimal_message_lines(
    message: &MessageView,
    messages: &[MessageView],
    index: usize,
    user_index: usize,
    width: usize,
    thinking_mode: CollapseMode,
    theme: &Theme,
    theme_hash: u64,
    relation_hash: u64,
) -> Arc<Vec<Line<'static>>> {
    let context_hash = if message.is_user() {
        user_index as u64
    } else {
        relation_hash
    };
    let key = MessageLineCacheKey {
        style: chat_style_key(ChatStyle::Minimal),
        width,
        thinking_mode: collapse_mode_key(thinking_mode),
        tool_mode: 0,
        message_hash: message_cache_hash(message),
        context_hash,
        theme_hash,
    };
    cached_message_lines(key, || {
        minimal_message_lines(
            message,
            messages,
            index,
            user_index,
            width,
            thinking_mode,
            theme,
        )
    })
}

fn theme_cache_hash(theme: &Theme) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{theme:?}").hash(&mut hasher);
    hasher.finish()
}

const fn collapse_mode_key(mode: CollapseMode) -> u8 {
    match mode {
        CollapseMode::Full => 0,
        CollapseMode::Truncate => 1,
        CollapseMode::Collapse => 2,
    }
}

const fn chat_style_key(style: ChatStyle) -> u8 {
    match style {
        ChatStyle::Chat => 0,
        ChatStyle::Agentic => 1,
        ChatStyle::Minimal => 2,
    }
}

fn message_relation_hash(messages: &[MessageView]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for message in messages {
        for call in &message.tool_calls {
            hash_tool_call(call, &mut hasher);
        }
        if message.role.starts_with("tool:") {
            message.tool_call_id.hash(&mut hasher);
            message.role.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn message_cache_hash(message: &MessageView) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.id.hash(&mut hasher);
    message.role.hash(&mut hasher);
    message.title.hash(&mut hasher);
    message.content.hash(&mut hasher);
    message.thinking.hash(&mut hasher);
    message.thinking_redacted.hash(&mut hasher);
    message.tool_call_id.hash(&mut hasher);
    message.is_error.hash(&mut hasher);
    for call in &message.tool_calls {
        hash_tool_call(call, &mut hasher);
    }
    hasher.finish()
}

fn hash_tool_call(call: &ToolCallView, hasher: &mut impl Hasher) {
    call.id.hash(hasher);
    call.name.hash(hasher);
    hash_json_value(&call.arguments, hasher);
}

fn hash_json_value(value: &Value, hasher: &mut impl Hasher) {
    match value {
        Value::Null => 0u8.hash(hasher),
        Value::Bool(value) => {
            1u8.hash(hasher);
            value.hash(hasher);
        }
        Value::Number(value) => {
            2u8.hash(hasher);
            value.to_string().hash(hasher);
        }
        Value::String(value) => {
            3u8.hash(hasher);
            value.hash(hasher);
        }
        Value::Array(values) => {
            4u8.hash(hasher);
            values.len().hash(hasher);
            for value in values {
                hash_json_value(value, hasher);
            }
        }
        Value::Object(values) => {
            5u8.hash(hasher);
            values.len().hash(hasher);
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                key.hash(hasher);
                hash_json_value(value, hasher);
            }
        }
    }
}

fn synthetic_error_message(error: &str) -> MessageView {
    MessageView {
        id: oino_types::OinoId::nil(),
        role: "error".into(),
        title: None,
        content: error.into(),
        thinking: None,
        thinking_redacted: false,
        tool_call_id: None,
        tool_calls: Vec::new(),
        is_error: true,
    }
}

fn agentic_message_lines(
    message: &MessageView,
    messages: &[MessageView],
    index: usize,
    width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if message.is_assistant() {
        return agentic_assistant_lines(message, messages, index, width, thinking_mode, theme);
    }
    if message.is_user() {
        let initial_prefix = Line::from(Span::styled(
            "› ",
            Style::default().fg(theme.focused_border),
        ));
        let subsequent_prefix = Line::from("  ");
        if let Some(resources) = parse_resource_user_message(&message.content) {
            return prefixed_resource_user_lines(
                &resources,
                width,
                initial_prefix,
                subsequent_prefix,
                theme,
            );
        }
        return prefixed_text_lines(
            &message.content,
            width,
            initial_prefix,
            subsequent_prefix,
            Style::default().fg(theme.fg),
        );
    }
    if message.role.starts_with("tool:") {
        return agentic_tool_result_lines(message, messages, width, tool_mode, theme);
    }
    if message.is_error {
        return prefixed_text_lines(
            &message.content,
            width,
            Line::from(vec![
                Span::styled("• ", theme.error),
                Span::styled("Error ", theme.error),
            ]),
            Line::from("  "),
            theme.error,
        );
    }
    let label = match message.role.as_str() {
        "compaction" => "Context compacted",
        "branch" => "Branch",
        role => role,
    };
    prefixed_text_lines(
        &message.content,
        width,
        Line::from(vec![
            Span::styled("• ", Style::default().fg(theme.muted)),
            Span::styled(format!("{label} "), Style::default().fg(theme.muted)),
        ]),
        Line::from("  "),
        Style::default().fg(theme.muted),
    )
}

fn agentic_assistant_lines(
    message: &MessageView,
    messages: &[MessageView],
    index: usize,
    width: usize,
    thinking_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(thinking) = thinking_display_text(message, thinking_mode) {
        lines.extend(prefixed_text_lines(
            &thinking,
            width,
            Line::from(Span::styled("• ", Style::default().fg(theme.muted))),
            Line::from("  "),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    if message.content != "<empty>" {
        lines.extend(prefixed_markdown_lines(
            &message.content,
            width,
            Line::from(Span::styled("• ", Style::default().fg(theme.muted))),
            Line::from("  "),
            Style::default().fg(theme.fg),
            theme,
        ));
    }
    for call in &message.tool_calls {
        if has_later_tool_result(messages, index, call.id) {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(agentic_running_tool_lines(call, width, theme));
    }
    lines
}

fn agentic_running_tool_lines(
    call: &ToolCallView,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if is_exploration_tool(&call.name, Some(call)) {
        let mut lines = vec![Line::from(vec![
            Span::styled("• ", Style::default().fg(theme.muted)),
            Span::styled(
                "Exploring",
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ])];
        lines.extend(prefixed_text_lines(
            &tool_action_summary(&call.name, Some(call)),
            width,
            Line::from(Span::styled("  └ ", Style::default().fg(theme.muted))),
            Line::from("    "),
            Style::default().fg(theme.fg),
        ));
        return lines;
    }

    prefixed_text_lines(
        &tool_action_summary(&call.name, Some(call)),
        width,
        Line::from(vec![
            Span::styled("• ", Style::default().fg(theme.muted)),
            Span::styled(
                "Running ",
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from("  "),
        Style::default().fg(theme.fg),
    )
}

fn agentic_tool_result_lines(
    message: &MessageView,
    messages: &[MessageView],
    width: usize,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let tool_name = tool_name_from_role(&message.role);
    let call = message
        .tool_call_id
        .and_then(|id| find_tool_call(messages, id));
    let bullet_style = if message.is_error {
        theme.error
    } else {
        Style::default()
            .fg(theme.assistant_border)
            .add_modifier(Modifier::BOLD)
    };
    let mut lines = if is_exploration_tool(tool_name, call) {
        let mut lines = vec![Line::from(vec![
            Span::styled("• ", bullet_style),
            Span::styled(
                "Explored",
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ])];
        lines.extend(prefixed_text_lines(
            &tool_action_summary(tool_name, call),
            width,
            Line::from(Span::styled("  └ ", Style::default().fg(theme.muted))),
            Line::from("    "),
            Style::default().fg(theme.fg),
        ));
        lines
    } else {
        let title = if message.is_error { "Failed" } else { "Ran" };
        prefixed_text_lines(
            &tool_action_summary(tool_name, call),
            width,
            Line::from(vec![
                Span::styled("• ", bullet_style),
                Span::styled(
                    format!("{title} "),
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from("  "),
            Style::default().fg(theme.fg),
        )
    };

    if let Some(output) = tool_output_for_display(message, tool_mode) {
        lines.extend(prefixed_text_lines(
            &output,
            width,
            Line::from(Span::styled("  └ ", Style::default().fg(theme.muted))),
            Line::from("    "),
            if message.is_error {
                theme.error
            } else {
                Style::default().fg(theme.muted)
            },
        ));
    }
    lines
}

fn minimal_message_lines(
    message: &MessageView,
    messages: &[MessageView],
    index: usize,
    user_index: usize,
    width: usize,
    thinking_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if message.is_user() {
        let prefix = format!("{user_index}› ");
        let initial_prefix = Line::from(vec![
            Span::styled(
                user_index.to_string(),
                Style::default().fg(theme.focused_border),
            ),
            Span::styled("› ", Style::default().fg(theme.focused_border)),
        ]);
        let subsequent_prefix = Line::from(" ".repeat(prefix.width()));
        if let Some(resources) = parse_resource_user_message(&message.content) {
            return prefixed_resource_user_lines(
                &resources,
                width,
                initial_prefix,
                subsequent_prefix,
                theme,
            );
        }
        return prefixed_text_lines(
            &message.content,
            width,
            initial_prefix,
            subsequent_prefix,
            Style::default().fg(theme.fg),
        );
    }
    if message.is_assistant() {
        return minimal_assistant_lines(message, messages, index, width, thinking_mode, theme);
    }
    if message.role.starts_with("tool:") {
        return minimal_tool_result_lines(message, messages, theme);
    }
    if message.is_error {
        return prefixed_text_lines(
            &format!("error: {}", message.content),
            width,
            Line::from(""),
            Line::from("  "),
            theme.error,
        );
    }
    prefixed_text_lines(
        &format!("{}: {}", message.role, message.content),
        width,
        Line::from("  "),
        Line::from("  "),
        Style::default().fg(theme.muted),
    )
}

fn minimal_assistant_lines(
    message: &MessageView,
    messages: &[MessageView],
    index: usize,
    width: usize,
    thinking_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(thinking) = thinking_display_text(message, thinking_mode) {
        lines.extend(prefixed_text_lines(
            &thinking,
            width,
            Line::from(Span::styled("  ◌ ", Style::default().fg(theme.muted))),
            Line::from("    "),
            Style::default().fg(theme.muted),
        ));
    }
    if message.content != "<empty>" {
        lines.extend(render_markdown_lines(
            &message.content,
            width,
            Style::default().fg(theme.fg),
            theme,
        ));
    }
    for call in &message.tool_calls {
        if has_later_tool_result(messages, index, call.id) {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::default().fg(theme.tool_border)),
            Span::styled(
                tool_compact_summary(&call.name, Some(call)),
                Style::default().fg(theme.fg),
            ),
            Span::styled(" · running", Style::default().fg(theme.muted)),
        ]));
    }
    lines
}

fn minimal_tool_result_lines(
    message: &MessageView,
    messages: &[MessageView],
    theme: &Theme,
) -> Vec<Line<'static>> {
    let tool_name = tool_name_from_role(&message.role);
    let call = message
        .tool_call_id
        .and_then(|id| find_tool_call(messages, id));
    let icon = if message.is_error { "✗" } else { "✓" };
    let icon_style = if message.is_error {
        theme.error
    } else {
        Style::default().fg(theme.assistant_border)
    };
    let metric = content_metric(&message.content);
    let mut line = Line::from(vec![
        Span::styled(format!("  {icon} "), icon_style),
        Span::styled(
            display_tool_name(tool_name),
            Style::default().fg(theme.tool_border),
        ),
    ]);
    let summary = tool_argument_summary(tool_name, call);
    if !summary.is_empty() {
        line.push_span(Span::styled(
            format!(" {summary}"),
            Style::default().fg(theme.muted),
        ));
    }
    if message.is_error {
        if let Some(summary) = concise_error_summary(&message.content) {
            line.push_span(Span::styled(" · ", Style::default().fg(theme.muted)));
            line.push_span(Span::styled(summary, theme.error));
        } else {
            line.push_span(Span::styled(
                format!(" · {metric}"),
                Style::default().fg(theme.muted),
            ));
        }
    } else {
        line.push_span(Span::styled(
            format!(" · {metric}"),
            Style::default().fg(theme.muted),
        ));
    }
    vec![line]
}

fn prefixed_text_lines(
    text: &str,
    width: usize,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
    style: Style,
) -> Vec<Line<'static>> {
    let initial_width = line_width(&initial_prefix);
    let subsequent_width = line_width(&subsequent_prefix);
    let first_width = width.saturating_sub(initial_width).max(1);
    let rest_width = width.saturating_sub(subsequent_width).max(1);
    let mut out = Vec::new();
    let mut first = true;
    for raw in text.split('\n') {
        let wrapped = wrap_text(raw, if first { first_width } else { rest_width });
        for segment in wrapped {
            let prefix = if first {
                initial_prefix.clone()
            } else {
                subsequent_prefix.clone()
            };
            let mut line = prefix;
            if !segment.is_empty() {
                line.push_span(Span::styled(segment, style));
            }
            out.push(line);
            first = false;
        }
        if raw.is_empty() && first {
            let mut line = initial_prefix.clone();
            line.push_span(Span::styled(String::new(), style));
            out.push(line);
            first = false;
        }
    }
    if out.is_empty() {
        out.push(initial_prefix);
    }
    out
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn has_later_tool_result(
    messages: &[MessageView],
    index: usize,
    call_id: oino_types::OinoId,
) -> bool {
    messages
        .iter()
        .skip(index.saturating_add(1))
        .any(|message| message.role.starts_with("tool:") && message.tool_call_id == Some(call_id))
}

fn find_tool_call(messages: &[MessageView], call_id: oino_types::OinoId) -> Option<&ToolCallView> {
    messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .find(|call| call.id == call_id)
}

fn tool_name_from_role(role: &str) -> &str {
    role.strip_prefix("tool:").unwrap_or(role)
}

fn is_exploration_tool(name: &str, call: Option<&ToolCallView>) -> bool {
    matches!(name, "read" | "glob")
        || matches!(name, "bash" if call.and_then(|call| string_arg(&call.arguments, &["command", "cmd"])).is_some_and(|command| is_exploration_command(&command)))
}

fn is_exploration_command(command: &str) -> bool {
    let command = command.trim_start();
    ["rg ", "grep ", "find ", "ls ", "cat ", "sed ", "fd "]
        .iter()
        .any(|prefix| command.starts_with(prefix))
}

fn tool_action_summary(name: &str, call: Option<&ToolCallView>) -> String {
    let display = display_tool_name(name);
    let summary = tool_argument_summary(name, call);
    if summary.is_empty() {
        display
    } else {
        format!("{display} {summary}")
    }
}

fn tool_compact_summary(name: &str, call: Option<&ToolCallView>) -> String {
    tool_action_summary(name, call)
}

fn tool_argument_summary(name: &str, call: Option<&ToolCallView>) -> String {
    let Some(call) = call else {
        return String::new();
    };
    match name {
        "bash" => string_arg(&call.arguments, &["command", "cmd"]).unwrap_or_default(),
        "read" => path_with_range(&call.arguments),
        "write" | "edit" => string_arg(&call.arguments, &["path", "file_path"]).unwrap_or_default(),
        "glob" => string_arg(&call.arguments, &["pattern"]).unwrap_or_default(),
        "web_search" | "websearch" => string_arg(&call.arguments, &["query"]).unwrap_or_default(),
        "ask_user" => string_arg(&call.arguments, &["question"]).unwrap_or_default(),
        _ => first_argument_summary(&call.arguments),
    }
}

fn path_with_range(arguments: &Value) -> String {
    let mut summary = string_arg(arguments, &["path", "file_path"]).unwrap_or_default();
    let offset = number_arg(arguments, &["offset", "start_line"]);
    let limit = number_arg(arguments, &["limit", "end_line"]);
    match (offset, limit) {
        (Some(offset), Some(limit)) if !summary.is_empty() => {
            summary.push_str(&format!(" ({offset}:{limit})"));
        }
        (Some(offset), None) if !summary.is_empty() => {
            summary.push_str(&format!(" ({offset})"));
        }
        _ => {}
    }
    summary
}

fn string_arg(arguments: &Value, keys: &[&str]) -> Option<String> {
    let object = arguments.as_object()?;
    keys.iter().find_map(|key| match object.get(*key)? {
        Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        value if !value.is_null() => Some(value.to_string()),
        _ => None,
    })
}

fn number_arg(arguments: &Value, keys: &[&str]) -> Option<u64> {
    let object = arguments.as_object()?;
    keys.iter().find_map(|key| object.get(*key)?.as_u64())
}

fn first_argument_summary(arguments: &Value) -> String {
    let Some(object) = arguments.as_object() else {
        return String::new();
    };
    object
        .iter()
        .find_map(|(key, value)| match value {
            Value::String(value) if !value.trim().is_empty() => Some(format!("{key}={value}")),
            value if !value.is_null() => Some(format!("{key}={value}")),
            _ => None,
        })
        .unwrap_or_default()
}

fn display_tool_name(name: &str) -> String {
    match name {
        "bash" => "Bash".into(),
        "read" => "Read".into(),
        "write" => "Write".into(),
        "edit" => "Edit".into(),
        "glob" => "Glob".into(),
        "web_search" => "Web search".into(),
        "ask_user" => "Ask user".into(),
        _ => name.replace('_', " "),
    }
}

fn tool_output_for_display(message: &MessageView, tool_mode: CollapseMode) -> Option<String> {
    if message.content == "<empty>" {
        return None;
    }
    Some(match tool_mode {
        CollapseMode::Full => message.content.clone(),
        CollapseMode::Truncate => truncate_display(&message.content),
        CollapseMode::Collapse if message.is_error => truncate_display(&message.content),
        CollapseMode::Collapse => "[collapsed]".into(),
    })
}

fn concise_error_summary(content: &str) -> Option<String> {
    let first = content.lines().find_map(|line| {
        let trimmed = line.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })?;
    let mut summary = first;
    for prefix in ["tool error:", "Error:", "error:", "Failed:", "failed:"] {
        if let Some(rest) = summary.strip_prefix(prefix) {
            summary = rest.trim_start();
            break;
        }
    }
    (!summary.is_empty()).then(|| truncate_to_width(summary, 80))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourceUserMessage {
    prompts: Vec<ResourceAttachment>,
    skills: Vec<ResourceAttachment>,
    user_request: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourceAttachment {
    name: String,
    source: String,
}

fn parse_resource_user_message(content: &str) -> Option<ResourceUserMessage> {
    const INTRO: &str = "Use the following Oino resources for this request.";
    let lines = content.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some(INTRO) {
        return None;
    }

    let mut parsed = ResourceUserMessage {
        prompts: Vec::new(),
        skills: Vec::new(),
        user_request: String::new(),
    };
    let mut index = 1;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == "# User Request" {
            parsed.user_request = lines
                .iter()
                .skip(index.saturating_add(1))
                .copied()
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
            break;
        }
        if let Some(name) = included_heading_name(trimmed, "## Included Prompt: `") {
            if let Some((attachment, next_index)) = parse_resource_attachment(name, &lines, index) {
                parsed.prompts.push(attachment);
                index = next_index;
                continue;
            }
        }
        if let Some(name) = included_heading_name(trimmed, "## Included Skill: `") {
            if let Some((attachment, next_index)) = parse_resource_attachment(name, &lines, index) {
                parsed.skills.push(attachment);
                index = next_index;
                continue;
            }
        }
        index = index.saturating_add(1);
    }

    (!parsed.prompts.is_empty() || !parsed.skills.is_empty()).then_some(parsed)
}

fn included_heading_name(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let name = rest.strip_suffix('`')?;
    (!name.is_empty()).then(|| name.to_string())
}

fn parse_resource_attachment(
    name: String,
    lines: &[&str],
    heading_index: usize,
) -> Option<(ResourceAttachment, usize)> {
    let source_line = lines.get(heading_index.saturating_add(1))?.trim();
    let source = source_line
        .strip_prefix("Source: `")
        .and_then(|source| source.strip_suffix('`'))
        .unwrap_or("")
        .to_string();
    let mut index = heading_index.saturating_add(2);
    while index < lines.len() && lines[index].trim().is_empty() {
        index = index.saturating_add(1);
    }
    let fence = lines.get(index).and_then(|line| fence_closer(line.trim()));
    if let Some(fence) = fence {
        let fence = fence.as_str();
        index = index.saturating_add(1);
        while index < lines.len() && lines[index].trim() != fence {
            index = index.saturating_add(1);
        }
        if index < lines.len() {
            index = index.saturating_add(1);
        }
    }
    Some((ResourceAttachment { name, source }, index))
}

fn fence_closer(line: &str) -> Option<String> {
    let mut chars = line.chars();
    let first = chars.next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let run_len = 1 + chars.take_while(|ch| *ch == first).count();
    (run_len >= 3).then(|| first.to_string().repeat(run_len))
}

fn prefixed_resource_user_lines(
    resources: &ResourceUserMessage,
    width: usize,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let prefix_width = line_width(&initial_prefix).max(line_width(&subsequent_prefix));
    let inner_width = width.saturating_sub(prefix_width).max(1);
    let lines = resource_user_lines(resources, inner_width, theme);
    prefix_structured_lines(lines, initial_prefix, subsequent_prefix)
}

fn prefix_structured_lines(
    lines: Vec<Line<'static>>,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
) -> Vec<Line<'static>> {
    let mut first = true;
    lines
        .into_iter()
        .map(|line| {
            let mut prefixed = if first {
                first = false;
                initial_prefix.clone()
            } else {
                subsequent_prefix.clone()
            };
            prefixed.spans.extend(line.spans);
            prefixed
        })
        .collect()
}

fn resource_user_lines(
    resources: &ResourceUserMessage,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let prompt_count = resources.prompts.len();
    let skill_count = resources.skills.len();
    lines.push(Line::from(vec![
        Span::styled(
            "Attached resources",
            Style::default()
                .fg(theme.focused_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {} prompt(s), {} skill(s)", prompt_count, skill_count),
            Style::default().fg(theme.muted),
        ),
    ]));
    for prompt in &resources.prompts {
        lines.extend(resource_attachment_lines(
            "Prompt",
            prompt,
            width,
            Style::default()
                .fg(theme.focused_border)
                .add_modifier(Modifier::BOLD),
            theme,
        ));
    }
    for skill in &resources.skills {
        lines.extend(resource_attachment_lines(
            "Skill",
            skill,
            width,
            Style::default()
                .fg(theme.tool_border)
                .add_modifier(Modifier::BOLD),
            theme,
        ));
    }
    if !resources.user_request.trim().is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "Request",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        )));
        lines.extend(plain_wrapped_lines(
            &resources.user_request,
            width,
            Style::default().fg(theme.fg),
        ));
    }
    lines
}

fn resource_attachment_lines(
    kind: &str,
    attachment: &ResourceAttachment,
    width: usize,
    kind_style: Style,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let label = format!("◆ {kind} ");
    let summary_width = label.width().saturating_add(attachment.name.width());
    let source_prefix = " · ";
    let source_width = source_prefix
        .width()
        .saturating_add(attachment.source.width());
    if summary_width.saturating_add(source_width) <= width {
        lines.push(Line::from(vec![
            Span::styled(label, kind_style),
            Span::styled(
                attachment.name.clone(),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(source_prefix, Style::default().fg(theme.muted)),
            Span::styled(attachment.source.clone(), Style::default().fg(theme.muted)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(label, kind_style),
            Span::styled(
                attachment.name.clone(),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ]));
        let source_indent = "  source ";
        for segment in wrap_text(
            &attachment.source,
            width.saturating_sub(source_indent.width()).max(1),
        ) {
            lines.push(Line::from(vec![
                Span::styled(source_indent, Style::default().fg(theme.muted)),
                Span::styled(segment, Style::default().fg(theme.muted)),
            ]));
        }
    }
    lines
}

fn message_content_lines(
    message: &MessageView,
    content: &str,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if message.is_user() {
        if let Some(resources) = parse_resource_user_message(content) {
            return resource_user_lines(&resources, width, theme);
        }
    }
    if message.is_assistant() && content != "<empty>" {
        render_markdown_lines(content, width, Style::default().fg(theme.fg), theme)
    } else {
        plain_wrapped_lines(content, width, Style::default().fg(theme.fg))
    }
}

fn plain_wrapped_lines(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_text(text, width)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

fn content_metric(content: &str) -> String {
    if content == "<empty>" || content.trim().is_empty() {
        return "no output".into();
    }
    let lines = content.lines().count();
    if lines > 1 {
        format!("{lines} lines")
    } else {
        let chars = content.chars().count();
        format!("{chars} chars")
    }
}

fn bubble_lines(
    message: &MessageView,
    available_width: usize,
    thinking_mode: CollapseMode,
    tool_mode: CollapseMode,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if is_empty_assistant_message(message) {
        return Vec::new();
    }
    if available_width < 16 {
        return vec![Line::styled(
            format!("{}: {}", message.role, message.content),
            theme.bubble_border_for_role(&message.role, message.is_error),
        )];
    }

    let max_bubble_width = available_width
        .saturating_mul(95)
        .saturating_add(99)
        .saturating_div(100)
        .clamp(16, available_width);
    let content_width = max_bubble_width.saturating_sub(4).max(1);
    let message_content = display_message_content(message, tool_mode);
    let content_lines = message_content_lines(message, &message_content, content_width, theme);
    let thinking_text = thinking_display_text(message, thinking_mode);
    let thinking_wrapped = thinking_text
        .as_deref()
        .map(|thinking| wrap_text(thinking, content_width.saturating_sub(2).max(1)))
        .unwrap_or_default();
    let role_label = message
        .title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| {
            if message.role.is_empty() {
                "message".to_string()
            } else {
                message.role.clone()
            }
        });
    let content_max = content_lines
        .iter()
        .map(line_width)
        .chain(
            thinking_wrapped
                .iter()
                .map(|line| UnicodeWidthStr::width(line.as_str()).saturating_add(2)),
        )
        .max()
        .unwrap_or(0);
    let thinking_label_width = if thinking_wrapped.is_empty() {
        0
    } else {
        UnicodeWidthStr::width("◌")
    };
    let label_width = UnicodeWidthStr::width(role_label.as_str()).saturating_add(2);
    let inner_width = content_width.min(
        content_max
            .max(thinking_label_width)
            .max(label_width)
            .max(1),
    );
    let bubble_width = inner_width.saturating_add(4);
    let left_pad = if message.is_user() {
        available_width.saturating_sub(bubble_width)
    } else {
        0
    };
    let border_style = theme.bubble_border_for_role(&message.role, message.is_error);

    let mut lines = Vec::new();
    lines.push(Line::styled(
        format!(
            "{}{}",
            " ".repeat(left_pad),
            top_border(&role_label, inner_width)
        ),
        border_style,
    ));
    if !thinking_wrapped.is_empty() {
        lines.push(bubble_content_line(
            left_pad,
            inner_width,
            border_style,
            vec![Span::styled("◌", Style::default().fg(theme.muted))],
        ));
        for line in wrap_text(
            thinking_text.as_deref().unwrap_or_default(),
            inner_width.saturating_sub(2).max(1),
        ) {
            let text = format!("  {line}");
            lines.push(bubble_content_line(
                left_pad,
                inner_width,
                border_style,
                vec![Span::styled(text, Style::default().fg(theme.muted))],
            ));
        }
        if message_content != "<empty>" {
            lines.push(bubble_content_line(
                left_pad,
                inner_width,
                border_style,
                vec![],
            ));
        }
    }
    if message_content != "<empty>" || thinking_wrapped.is_empty() {
        for line in content_lines {
            lines.push(bubble_content_line(
                left_pad,
                inner_width,
                border_style,
                line.spans,
            ));
        }
    }
    lines.push(Line::styled(
        format!("{}╰{}╯", " ".repeat(left_pad), "─".repeat(inner_width + 2)),
        border_style,
    ));
    lines
}

fn bubble_content_line(
    left_pad: usize,
    inner_width: usize,
    border_style: Style,
    mut content: Vec<Span<'static>>,
) -> Line<'static> {
    let content_width = content
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum::<usize>();
    if content_width < inner_width {
        content.push(Span::raw(" ".repeat(inner_width - content_width)));
    }
    let mut spans = vec![
        Span::raw(" ".repeat(left_pad)),
        Span::styled("│ ", border_style),
    ];
    spans.extend(content);
    spans.push(Span::styled(" │", border_style));
    Line::from(spans)
}

fn is_empty_assistant_message(message: &MessageView) -> bool {
    message.is_assistant()
        && message.content == "<empty>"
        && message
            .thinking
            .as_ref()
            .is_none_or(|thinking| thinking.trim().is_empty())
        && !message.thinking_redacted
}

fn display_message_content(message: &MessageView, tool_mode: CollapseMode) -> String {
    if message.role.starts_with("tool:") {
        match tool_mode {
            CollapseMode::Full => message.content.clone(),
            CollapseMode::Truncate => truncate_display(&message.content),
            CollapseMode::Collapse => "[collapsed]".into(),
        }
    } else {
        message.content.clone()
    }
}

fn thinking_display_text(message: &MessageView, thinking_mode: CollapseMode) -> Option<String> {
    let text = if message.thinking_redacted {
        "[redacted]".to_string()
    } else {
        message
            .thinking
            .as_ref()
            .filter(|thinking| !thinking.trim().is_empty())?
            .clone()
    };
    Some(match thinking_mode {
        CollapseMode::Full => text,
        CollapseMode::Truncate => truncate_display(&text),
        CollapseMode::Collapse => "[collapsed]".into(),
    })
}

fn truncate_display(text: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut truncated = text.chars().take(MAX_CHARS).collect::<String>();
    if text.chars().count() > MAX_CHARS {
        truncated.push('…');
    }
    truncated
}

fn top_border(label: &str, inner_width: usize) -> String {
    let label = truncate_to_width(label, inner_width.saturating_sub(1));
    let used = UnicodeWidthStr::width(label.as_str()).saturating_add(2);
    let rest = inner_width.saturating_add(2).saturating_sub(used);
    format!("╭ {label} {}╮", "─".repeat(rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plain(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn assistant_tool_calls(calls: Vec<ToolCallView>) -> MessageView {
        MessageView {
            id: oino_types::OinoId::from_u128(10),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: "<empty>".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: calls,
            is_error: false,
        }
    }

    fn assistant_text(content: &str) -> MessageView {
        MessageView {
            id: oino_types::OinoId::from_u128(11),
            role: "assistant".into(),
            title: Some("test/model".into()),
            content: content.into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        }
    }

    fn user_text(content: &str) -> MessageView {
        MessageView {
            id: oino_types::OinoId::from_u128(12),
            role: "user".into(),
            title: None,
            content: content.into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        }
    }

    fn tool_result(
        id: u128,
        call_id: u128,
        name: &str,
        content: &str,
        is_error: bool,
    ) -> MessageView {
        MessageView {
            id: oino_types::OinoId::from_u128(id),
            role: format!("tool:{name}"),
            title: None,
            content: content.into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: Some(oino_types::OinoId::from_u128(call_id)),
            tool_calls: Vec::new(),
            is_error,
        }
    }

    #[test]
    fn assistant_markdown_is_rendered_in_all_chat_styles() {
        let messages = vec![assistant_text(
            "## Title\n\n- **Bold** item with `code`\n- [Link](https://example.invalid)",
        )];

        for style in [ChatStyle::Chat, ChatStyle::Agentic, ChatStyle::Minimal] {
            let lines = transcript_lines(
                &messages,
                None,
                120,
                CollapseMode::Full,
                CollapseMode::Full,
                style,
                &Theme::default(),
            );
            let rendered = lines.iter().map(plain).collect::<Vec<_>>().join("\n");

            assert!(rendered.contains("Title"), "style {style:?}: {rendered}");
            assert!(
                rendered.contains("• Bold item with code"),
                "style {style:?}: {rendered}"
            );
            assert!(
                rendered.contains("Link ↗ https://example.invalid"),
                "style {style:?}: {rendered}"
            );
            assert!(
                !rendered.contains("## Title"),
                "style {style:?}: {rendered}"
            );
            assert!(
                !rendered.contains("**Bold**"),
                "style {style:?}: {rendered}"
            );
            assert!(!rendered.contains("`code`"), "style {style:?}: {rendered}");
        }
    }

    #[test]
    fn chat_bubbles_expand_to_ninety_five_percent_for_wide_content() {
        let messages = vec![assistant_text(&"x".repeat(240))];
        let lines = transcript_lines(
            &messages,
            None,
            120,
            CollapseMode::Full,
            CollapseMode::Full,
            ChatStyle::Chat,
            &Theme::default(),
        );
        let top_border = plain(&lines[0]);

        assert_eq!(top_border.width(), 114);
    }

    #[test]
    fn user_resource_attachments_render_as_cards_in_all_chat_styles() {
        let content = "Use the following Oino resources for this request.\n\n# Included Skills\n\n## Included Skill: `first-skill`\nSource: `.oino/skills/first-skill/SKILL.md`\n\n````markdown\n# First Skill\n````\n\n## Included Skill: `second-skill`\nSource: `.oino/skills/second-skill/SKILL.md`\n\n````markdown\n# Second Skill\n````\n\n# User Request\n\nfix crash";
        let messages = vec![user_text(content)];

        for style in [ChatStyle::Chat, ChatStyle::Agentic, ChatStyle::Minimal] {
            let lines = transcript_lines(
                &messages,
                None,
                120,
                CollapseMode::Full,
                CollapseMode::Full,
                style,
                &Theme::default(),
            );
            let rendered = lines.iter().map(plain).collect::<Vec<_>>().join("\n");

            assert!(
                rendered.contains("Attached resources · 0 prompt(s), 2 skill(s)"),
                "style {style:?}: {rendered}"
            );
            assert!(
                rendered.contains("Skill first-skill"),
                "style {style:?}: {rendered}"
            );
            assert!(
                rendered.contains("Skill second-skill"),
                "style {style:?}: {rendered}"
            );
            assert!(rendered.contains("Request"), "style {style:?}: {rendered}");
            assert!(
                rendered.contains("fix crash"),
                "style {style:?}: {rendered}"
            );
            assert!(!rendered.contains("Use the following Oino resources"));
            assert!(!rendered.contains("# First Skill"));
            assert!(!rendered.contains("````markdown"));
        }
    }

    #[test]
    fn minimal_read_results_are_single_compact_adjacent_rows_when_collapsed() {
        let messages = vec![
            assistant_tool_calls(vec![
                ToolCallView {
                    id: oino_types::OinoId::from_u128(1),
                    name: "read".into(),
                    arguments: json!({ "path": "README.md" }),
                },
                ToolCallView {
                    id: oino_types::OinoId::from_u128(2),
                    name: "read".into(),
                    arguments: json!({ "path": "Cargo.toml" }),
                },
            ]),
            tool_result(20, 1, "read", "one\ntwo", false),
            tool_result(21, 2, "read", "workspace", false),
        ];

        let lines = transcript_lines(
            &messages,
            None,
            120,
            CollapseMode::Full,
            CollapseMode::Collapse,
            ChatStyle::Minimal,
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert_eq!(
            plain_lines.len(),
            2,
            "minimal read rows should not add output or spacing"
        );
        assert_eq!(plain_lines[0], "  ✓ Read README.md · 2 lines");
        assert_eq!(plain_lines[1], "  ✓ Read Cargo.toml · 9 chars");
        assert!(plain_lines.iter().all(|line| !line.contains("[collapsed]")));
    }

    #[test]
    fn minimal_read_error_shows_concise_error_inline_for_tool_call_only_message() {
        let messages = vec![
            assistant_tool_calls(vec![ToolCallView {
                id: oino_types::OinoId::from_u128(4),
                name: "read".into(),
                arguments: json!({ "path": "/home/pi/project/oino" }),
            }]),
            tool_result(
                22,
                4,
                "read",
                "tool error: io error: Is a directory (os error 21)",
                true,
            ),
        ];

        let lines = transcript_lines(
            &messages,
            None,
            120,
            CollapseMode::Full,
            CollapseMode::Collapse,
            ChatStyle::Minimal,
            &Theme::default(),
        );
        let plain_lines = lines.iter().map(plain).collect::<Vec<_>>();

        assert_eq!(plain_lines.len(), 1);
        assert_eq!(
            plain_lines[0],
            "  ✗ Read /home/pi/project/oino · io error: Is a directory (os error 21)"
        );
        assert!(!plain_lines[0].contains("[collapsed]"));
    }
}
