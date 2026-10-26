//! Faithful ports of pi's shared helpers (`utils/hash`, `utils/text`,
//! `utils/transcript`, `utils/estimate`, `api/transform-messages`,
//! `api/constrained-sampling`, `api/simple-options`, `models.ts` cost/clamp).

use std::collections::{BTreeMap, BTreeSet};

use oino_types::{transcript, ContentBlock, Message, StopReason, ToolDeclaration};
use serde_json::{json, Value};

/// pi `shortHash` — deterministic two-fold hash used for shortened ids.
#[must_use]
pub fn short_hash(input: &str) -> String {
    // JS Math.imul semantics: int32 wrapping multiplication.
    let mut h1: u32 = 0xdeadbeef;
    let mut h2: u32 = 0x41c6ce57;
    for ch in input.chars() {
        let c = ch as u32;
        h1 = (h1 ^ c).wrapping_mul(2654435761);
        h2 = (h2 ^ c).wrapping_mul(1597334677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507) ^ (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507) ^ (h1 ^ (h1 >> 13)).wrapping_mul(3266489909);
    format!("{}{}", base36(h2), base36(h1))
}

fn base36(value: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut v = value as u64;
    if v == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while v > 0 {
        out.push(DIGITS[(v % 36) as usize]);
        v /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Rust `String` is always valid UTF-8, so unpaired surrogates cannot occur;
/// kept for parity with pi call sites.
#[must_use]
pub fn sanitize_surrogates(text: &str) -> String {
    text.to_string()
}

/// pi `contentText`.
#[must_use]
pub fn content_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// pi `getSystemMessageText` — content + section texts joined by blank lines.
#[must_use]
pub fn system_message_text(message: &Message) -> String {
    let Message::System {
        content, sections, ..
    } = message
    else {
        return String::new();
    };
    let mut parts = vec![content.clone()];
    for section in sections {
        if let Some(text) = &section.text {
            parts.push(text.clone());
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join("\n\n")
}

/// pi `renderSystemMessageUpdate` — later system messages rendered with
/// named-section framing.
#[must_use]
pub fn render_system_message_update(message: &Message) -> String {
    let Message::System {
        content, sections, ..
    } = message
    else {
        return String::new();
    };
    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(content.clone());
    }
    for section in sections {
        parts.push(match &section.text {
            None => format!("Removed system prompt section \"{}\".", section.name),
            Some(text) => format!(
                "Updated system prompt section \"{}\":\n\n{text}",
                section.name
            ),
        });
    }
    parts.join("\n\n")
}

/// pi `getInitialSystemMessage`.
#[must_use]
pub fn initial_system_message(messages: &[Message]) -> Option<&Message> {
    messages
        .first()
        .filter(|m| matches!(m, Message::System { .. }))
}

/// pi `getDeclaredTools` — every tool definition in first-declaration order.
#[must_use]
pub fn declared_tools(messages: &[Message]) -> Vec<ToolDeclaration> {
    transcript::declared_tools(messages)
}

/// pi `hasToolRedefinitions` — same name declared with a different schema.
#[must_use]
pub fn has_tool_redefinitions(messages: &[Message]) -> bool {
    let mut declared: BTreeMap<&str, &ToolDeclaration> = BTreeMap::new();
    for message in messages {
        let Message::System { tools_added, .. } = message else {
            continue;
        };
        for tool in tools_added {
            if let Some(previous) = declared.get(tool.name.as_str()) {
                if !tool_declarations_equal(previous, tool) {
                    return true;
                }
            }
            declared.insert(tool.name.as_str(), tool);
        }
    }
    false
}

fn tool_declarations_equal(a: &ToolDeclaration, b: &ToolDeclaration) -> bool {
    // Same canonical comparison as pi `declarationsEqual` (JSON round-trip).
    serde_json::to_value(tool_declaration_json(a)).unwrap_or_default()
        == serde_json::to_value(tool_declaration_json(b)).unwrap_or_default()
}

fn tool_declaration_json(tool: &ToolDeclaration) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    })
}

/// pi `hasNonAdditiveToolChanges`.
#[must_use]
pub fn has_non_additive_tool_changes(messages: &[Message]) -> bool {
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    for message in messages {
        let Message::System {
            tools_added,
            tools_removed,
            ..
        } = message
        else {
            continue;
        };
        if !tools_removed.is_empty() {
            return true;
        }
        for tool in tools_added {
            if !declared.insert(tool.name.as_str()) {
                return true;
            }
        }
    }
    false
}

/// pi `TranscriptTools`.
pub struct TranscriptTools {
    pub request_tools: Vec<ToolDeclaration>,
    pub anchors_additions: bool,
}

/// pi `resolveTranscriptTools`.
#[must_use]
pub fn resolve_transcript_tools(
    messages: &[Message],
    supports_tool_additions: bool,
) -> TranscriptTools {
    let anchors_additions = supports_tool_additions && !has_non_additive_tool_changes(messages);
    TranscriptTools {
        request_tools: if anchors_additions {
            initial_system_message(messages)
                .map(|m| match m {
                    Message::System { tools_added, .. } => tools_added.clone(),
                    _ => Vec::new(),
                })
                .unwrap_or_default()
        } else {
            transcript::current_tools(messages)
        },
        anchors_additions,
    }
}

const NON_VISION_USER_IMAGE_PLACEHOLDER: &str = "(image omitted: model does not support images)";
const NON_VISION_TOOL_IMAGE_PLACEHOLDER: &str =
    "(tool image omitted: model does not support images)";

fn replace_images_with_placeholder(
    content: &[ContentBlock],
    placeholder: &str,
) -> Vec<ContentBlock> {
    let mut result = Vec::new();
    let mut previous_was_placeholder = false;
    for block in content {
        if matches!(block, ContentBlock::Image { .. }) {
            if !previous_was_placeholder {
                result.push(ContentBlock::Text {
                    text: placeholder.into(),
                    signature: None,
                });
            }
            previous_was_placeholder = true;
            continue;
        }
        previous_was_placeholder = matches!(
            block,
            ContentBlock::Text { text, .. } if text == placeholder
        );
        result.push(block.clone());
    }
    result
}

/// Model attributes needed by the transform/estimate code (subset of pi Model).
pub struct ModelFacets<'a> {
    pub provider: &'a str,
    pub api: &'a str,
    pub id: &'a str,
    pub accepts_images: bool,
}

/// pi `transformMessages` — image downgrade, thinking/tool-call-id handling,
#[allow(clippy::type_complexity)]
/// orphaned tool-call synthetic results, held-back system messages.
/// `normalize_tool_call_id` mirrors pi's per-API id normalization hook.
#[must_use]
pub fn transform_messages(
    messages: &[Message],
    model: &ModelFacets<'_>,
    normalize_tool_call_id: Option<&dyn Fn(&str, &ModelFacets<'_>, &Message) -> String>,
) -> Vec<Message> {
    let mut tool_call_id_map: BTreeMap<String, String> = BTreeMap::new();

    let downgraded: Vec<Message> = if model.accepts_images {
        messages.to_vec()
    } else {
        messages
            .iter()
            .map(|msg| match msg {
                Message::User { id, content } => Message::User {
                    id: *id,
                    content: replace_images_with_placeholder(
                        content,
                        NON_VISION_USER_IMAGE_PLACEHOLDER,
                    ),
                },
                Message::ToolResult {
                    id,
                    tool_call_id,
                    tool_name,
                    content,
                    is_error,
                    terminate,
                    details,
                    usage,
                    added_tool_names,
                } => Message::ToolResult {
                    id: *id,
                    tool_call_id: *tool_call_id,
                    tool_name: tool_name.clone(),
                    content: replace_images_with_placeholder(
                        content,
                        NON_VISION_TOOL_IMAGE_PLACEHOLDER,
                    ),
                    is_error: *is_error,
                    terminate: *terminate,
                    details: details.clone(),
                    usage: usage.clone(),
                    added_tool_names: added_tool_names.clone(),
                },
                other => other.clone(),
            })
            .collect()
    };

    let transformed: Vec<Message> = downgraded
        .iter()
        .map(|msg| {
            if matches!(msg, Message::System { .. } | Message::User { .. }) {
                return msg.clone();
            }
            if let Message::ToolResult { tool_call_id, .. } = msg {
                if let Some(normalized) = tool_call_id_map.get(&tool_call_id.to_string()) {
                    let mut updated = msg.clone();
                    if let Message::ToolResult { tool_call_id, .. } = &mut updated {
                        *tool_call_id = normalized
                            .parse()
                            .unwrap_or_else(|_| oino_types::OinoId::nil());
                    }
                    return updated;
                }
                return msg.clone();
            }
            let Message::Assistant {
                id,
                content,
                stop_reason,
                usage,
                provider,
                response_model,
                error_message,
                provider_thinking_level,
            } = msg
            else {
                return msg.clone();
            };
            let assistant_provider = provider
                .as_ref()
                .and_then(|p| p.model.as_ref())
                .map(|m| m.provider.as_str())
                .unwrap_or_default();
            let assistant_model = provider
                .as_ref()
                .and_then(|p| p.model.as_ref())
                .map(|m| m.name.as_str())
                .unwrap_or_default();
            let is_same_model = assistant_provider == model.provider && assistant_model == model.id;
            let mut new_content = Vec::new();
            for block in content {
                match block {
                    ContentBlock::Thinking {
                        text,
                        redacted,
                        signature,
                        provider_data,
                    } => {
                        if *redacted {
                            if is_same_model {
                                new_content.push(block.clone());
                            }
                            continue;
                        }
                        if is_same_model && signature.is_some() {
                            new_content.push(block.clone());
                            continue;
                        }
                        if text.trim().is_empty() {
                            continue;
                        }
                        if is_same_model {
                            new_content.push(block.clone());
                        } else {
                            new_content.push(ContentBlock::Text {
                                text: text.clone(),
                                signature: None,
                            });
                        }
                        let _ = provider_data;
                    }
                    ContentBlock::Text { .. } => {
                        if is_same_model {
                            new_content.push(block.clone());
                        } else if let ContentBlock::Text { text, .. } = block {
                            new_content.push(ContentBlock::Text {
                                text: text.clone(),
                                signature: None,
                            });
                        }
                    }
                    ContentBlock::ToolCall {
                        id: call_id,
                        name,
                        arguments,
                        native_id,
                    } => {
                        let mut normalized = block.clone();
                        if !is_same_model {
                            if let Some(normalize) = normalize_tool_call_id {
                                let raw_id =
                                    native_id.clone().unwrap_or_else(|| call_id.to_string());
                                let normalized_id = normalize(&raw_id, model, msg);
                                if normalized_id != raw_id {
                                    tool_call_id_map.insert(raw_id.clone(), normalized_id.clone());
                                    normalized = ContentBlock::ToolCall {
                                        id: *call_id,
                                        name: name.clone(),
                                        arguments: arguments.clone(),
                                        native_id: Some(normalized_id),
                                    };
                                }
                            }
                        }
                        new_content.push(normalized);
                    }
                    ContentBlock::Image { .. } => new_content.push(block.clone()),
                }
            }
            Message::Assistant {
                id: *id,
                content: new_content,
                stop_reason: stop_reason.clone(),
                usage: usage.clone(),
                provider: provider.clone(),
                response_model: response_model.clone(),
                error_message: error_message.clone(),
                provider_thinking_level: provider_thinking_level.clone(),
            }
        })
        .collect();

    // Second pass: synthetic results for orphaned tool calls + held system.
    let mut result: Vec<Message> = Vec::new();
    let mut pending_tool_calls: Vec<(oino_types::OinoId, String, Option<String>)> = Vec::new();
    let mut existing_tool_result_ids: BTreeSet<String> = BTreeSet::new();
    let mut held_system_messages: Vec<Message> = Vec::new();
    macro_rules! close_pending {
        () => {
            if !pending_tool_calls.is_empty() {
                for (call_id, name, _) in pending_tool_calls.drain(..) {
                    let key = call_id.to_string();
                    if !existing_tool_result_ids.contains(&key) {
                        result.push(Message::ToolResult {
                            id: oino_types::OinoId::nil(),
                            tool_call_id: call_id,
                            tool_name: name,
                            content: vec![ContentBlock::Text {
                                text: "No result provided".into(),
                                signature: None,
                            }],
                            is_error: true,
                            terminate: false,
                            details: None,
                            usage: None,
                            added_tool_names: Vec::new(),
                        });
                    }
                }
                existing_tool_result_ids.clear();
            }
            result.append(&mut held_system_messages);
        };
    }

    for msg in transformed {
        match &msg {
            Message::Assistant {
                content,
                stop_reason,
                ..
            } => {
                close_pending!();
                if matches!(
                    stop_reason,
                    Some(StopReason::Error) | Some(StopReason::Aborted)
                ) {
                    continue;
                }
                let calls: Vec<_> = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolCall {
                            id,
                            name,
                            native_id,
                            ..
                        } => Some((*id, name.clone(), native_id.clone())),
                        _ => None,
                    })
                    .collect();
                if !calls.is_empty() {
                    pending_tool_calls = calls;
                    existing_tool_result_ids = BTreeSet::new();
                }
                result.push(msg);
            }
            Message::ToolResult { tool_call_id, .. } => {
                existing_tool_result_ids.insert(tool_call_id.to_string());
                result.push(msg);
            }
            Message::System { .. } => {
                if pending_tool_calls.is_empty() {
                    result.push(msg);
                } else {
                    held_system_messages.push(msg);
                }
            }
            Message::User { .. } => {
                close_pending!();
                result.push(msg);
            }
            _ => result.push(msg),
        }
    }
    close_pending!();
    result
}

// ---- context estimate (pi utils/estimate) ---------------------------------

const CHARS_PER_TOKEN: usize = 4;
const ESTIMATED_IMAGE_CHARS: usize = 4800;

fn estimate_content_chars(content: &[ContentBlock]) -> usize {
    content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text, .. } => text.len(),
            ContentBlock::Image { .. } => ESTIMATED_IMAGE_CHARS,
            _ => 0,
        })
        .sum()
}

fn message_char_estimate(message: &Message) -> usize {
    match message {
        Message::System { .. } => system_message_text(message).len(),
        Message::User { content, .. } | Message::Assistant { content, .. } => {
            estimate_content_chars(content)
        }
        Message::ToolResult { content, .. } => estimate_content_chars(content),
        _ => 0,
    }
}

/// pi `estimateContextTokens`: last assistant usage total plus trailing
/// estimated tokens, else sum of estimates.
#[must_use]
pub fn estimate_context_tokens(messages: &[Message]) -> usize {
    let mut trailing_chars = 0usize;
    let mut usage_tokens: Option<usize> = None;
    let mut trailing_after = 0usize;
    for message in messages {
        let estimate = message_char_estimate(message);
        if let Message::Assistant {
            usage: Some(usage), ..
        } = message
        {
            let total = (usage.input_tokens
                + usage.output_tokens
                + usage.cache_read_tokens
                + usage.cache_write_tokens) as usize;
            usage_tokens = Some(total);
            trailing_after = 0;
            continue;
        }
        if usage_tokens.is_some() {
            trailing_after += estimate;
        } else {
            trailing_chars += estimate;
        }
    }
    match usage_tokens {
        Some(tokens) => tokens + trailing_after.div_ceil(CHARS_PER_TOKEN),
        None => trailing_chars.div_ceil(CHARS_PER_TOKEN),
    }
}

// ---- models.ts helpers ----------------------------------------------------

/// pi `ModelCostRates` (+ optional `tiers`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct CostRates {
    #[serde(rename = "input")]
    pub input: f64,
    #[serde(rename = "output")]
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
    #[serde(default)]
    pub tiers: Vec<CostTier>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct CostTier {
    #[serde(rename = "inputTokensAbove")]
    pub input_tokens_above: u64,
    #[serde(rename = "input")]
    pub input: f64,
    #[serde(rename = "output")]
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
}

/// pi `calculateCost` (mutates oino `Usage.cost`).
pub fn calculate_cost(cost: &CostRates, usage: &mut oino_types::Usage) {
    let input_tokens = usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
    let mut matched: i64 = -1;
    let mut rates = (cost.input, cost.output, cost.cache_read, cost.cache_write);
    for tier in &cost.tiers {
        if input_tokens > tier.input_tokens_above && tier.input_tokens_above as i64 > matched {
            rates = (tier.input, tier.output, tier.cache_read, tier.cache_write);
            matched = tier.input_tokens_above as i64;
        }
    }
    let long_write = usage.cache_write_1h_tokens.unwrap_or(0);
    let short_write = usage.cache_write_tokens.saturating_sub(long_write);
    let amount = rates.0 / 1e6 * usage.input_tokens as f64
        + rates.1 / 1e6 * usage.output_tokens as f64
        + rates.2 / 1e6 * usage.cache_read_tokens as f64
        + (rates.3 * short_write as f64 + rates.0 * 2.0 * long_write as f64) / 1e6;
    usage.cost = Some(oino_types::UsageCost {
        amount,
        currency: "USD".into(),
    });
}

const EXTENDED_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// pi `getSupportedThinkingLevels` over `thinkingLevelMap`.
#[must_use]
pub fn supported_thinking_levels(
    reasoning: bool,
    map: Option<&BTreeMap<String, Option<String>>>,
) -> Vec<&'static str> {
    if !reasoning {
        return vec!["off"];
    }
    EXTENDED_THINKING_LEVELS
        .iter()
        .copied()
        .filter(|level| {
            let mapped = map.and_then(|m| m.get(*level));
            match mapped {
                Some(None) => false,
                None if *level == "xhigh" || *level == "max" => false,
                _ => true,
            }
        })
        .collect()
}

/// pi `clampThinkingLevel`.
#[must_use]
pub fn clamp_thinking_level(
    reasoning: bool,
    map: Option<&BTreeMap<String, Option<String>>>,
    level: &str,
) -> String {
    let available = supported_thinking_levels(reasoning, map);
    if available.contains(&level) {
        return level.to_string();
    }
    let requested = EXTENDED_THINKING_LEVELS.iter().position(|l| *l == level);
    let Some(requested) = requested else {
        return available.first().copied().unwrap_or("off").to_string();
    };
    for candidate in EXTENDED_THINKING_LEVELS.iter().skip(requested) {
        if available.contains(candidate) {
            return (*candidate).to_string();
        }
    }
    for candidate in EXTENDED_THINKING_LEVELS.iter().take(requested).rev() {
        if available.contains(candidate) {
            return (*candidate).to_string();
        }
    }
    available.first().copied().unwrap_or("off").to_string()
}

// ---- JSON repair (pi utils/json-parse) ------------------------------------

const VALID_JSON_ESCAPES: &[char] = &['"', '\\', '/', 'b', 'f', 'n', 'r', 't', 'u'];

fn is_control_character(ch: char) -> bool {
    (ch as u32) <= 0x1f
}

/// pi `repairJson`.
#[must_use]
pub fn repair_json(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut repaired = String::new();
    let mut in_string = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if !in_string {
            repaired.push(ch);
            if ch == '"' {
                in_string = true;
            }
            index += 1;
            continue;
        }
        if ch == '"' {
            repaired.push(ch);
            in_string = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            let next = chars.get(index + 1);
            match next {
                None => {
                    repaired.push_str("\\\\");
                    index += 1;
                    continue;
                }
                Some('u') => {
                    let digits: String = chars[index + 2..index + 6.min(chars.len())]
                        .iter()
                        .take(4)
                        .collect();
                    if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_hexdigit()) {
                        repaired.push_str(&format!("\\u{digits}"));
                        index += 6;
                        continue;
                    }
                }
                Some(&n) if VALID_JSON_ESCAPES.contains(&n) => {
                    repaired.push('\\');
                    repaired.push(n);
                    index += 2;
                    continue;
                }
                _ => {
                    repaired.push_str("\\\\");
                    index += 1;
                    continue;
                }
            }
        }
        if is_control_character(ch) {
            repaired.push_str(&match ch {
                '\u{8}' => "\\b".into(),
                '\u{c}' => "\\f".into(),
                '\n' => "\\n".into(),
                '\r' => "\\r".into(),
                '\t' => "\\t".into(),
                c => format!("\\u{:04x}", c as u32),
            });
        } else {
            repaired.push(ch);
        }
        index += 1;
    }
    repaired
}

/// pi `parseJsonWithRepair`.
pub fn parse_json_with_repair(json: &str) -> Result<Value, serde_json::Error> {
    match serde_json::from_str(json) {
        Ok(value) => Ok(value),
        Err(err) => {
            let repaired = repair_json(json);
            if repaired != json {
                return serde_json::from_str(&repaired);
            }
            Err(err)
        }
    }
}

/// pi `parseStreamingJson`: complete parse, then repaired parse, then a
/// partial-parse that keeps complete leading values; `{}` on failure.
#[must_use]
pub fn parse_streaming_json(partial: &str) -> Value {
    if partial.trim().is_empty() {
        return json!({});
    }
    if let Ok(value) = parse_json_with_repair(partial) {
        return value;
    }
    partial_json_parse(partial)
        .or_else(|| partial_json_parse(&repair_json(partial)))
        .unwrap_or_else(|| json!({}))
}

/// Minimal `partial-json` port: parses as much of the value as is complete
/// (truncating mid-token), returning `None` when nothing parses.
fn partial_json_parse(input: &str) -> Option<Value> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Progressively trim to the last complete boundary inside the top object
    // and close open structures; this mirrors partial-json's lenient behavior
    // closely enough for tool-argument deltas (flat objects of scalars).
    if !trimmed.starts_with('{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut last_object_value_end = 0usize;
    let mut seen_colon = false;
    for (index, ch) in trimmed.char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
                if depth == 1 && seen_colon {
                    last_object_value_end = index + ch.len_utf8();
                }
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 1 && seen_colon {
                    last_object_value_end = index + ch.len_utf8();
                }
            }
            ':' if depth == 1 => seen_colon = true,
            ',' if depth == 1 => {}
            _ => {
                if depth == 1 && seen_colon && !ch.is_whitespace() {
                    // scalars: mark potential value end at next separator
                }
            }
        }
    }
    if last_object_value_end == 0 {
        // Maybe only an opening brace plus a complete first token cut mid-way.
        return Some(json!({}));
    }
    let mut candidate = trimmed[..last_object_value_end].to_string();
    // close open string escapes/brackets conservatively
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for ch in candidate.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    if in_string {
        candidate.push('"');
    }
    for _ in 0..depth.max(0) {
        candidate.push('}');
    }
    serde_json::from_str(&candidate).ok().or(Some(json!({})))
}
