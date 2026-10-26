//! Azure `OpenAI` Responses (pi-ai `api/azure-openai-responses.js` + `azure-openai-config.js`):
//! the shared Responses input/stream code with Azure endpoints, deployment names and the
//! `AzureOpenAI` SDK's `api-key` header.

use oino_ai::transcript::{collapse_system_messages, declared_tools, resolve_transcript_tools};
use oino_ai::{JsonObject, MapEntry, ThinkingLevel};
use serde_json::{Value, json};

use crate::constrained::grammar_input_properties;
use crate::emit::Emitter;
use crate::exchange::{ErrorStyle, Exchange};
use crate::options::{clamp_max_tokens_to_context, resolve_sampling_params};
use crate::providers::{azure_api_version, resolve_azure_base_url, resolve_deployment_name};
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT};
use crate::responses::{
    Flavor, MessageOptions, Parser, ToolOptions, compat, convert_messages, convert_tools,
    tool_support,
};
use crate::util::{clamp_prompt_cache_key, clamp_thinking_level, compat_bool, mapped_level};

const MIN_OUTPUT_TOKENS: u64 = 16;

/// A prepared Azure request.
pub struct Prepared {
    /// Outbound request.
    pub http: HttpRequest,
    /// Streaming state.
    pub exchange: Exchange,
}

/// Builds the request (pi `streamSimple` + `stream` up to `responses.create`).
///
/// # Errors
/// Missing key or endpoint, or a tool that cannot be expressed.
#[allow(
    clippy::too_many_lines,
    reason = "1:1 port of pi buildParams + createClient"
)]
pub fn prepare(request: &ProviderRequest, now: i64) -> Result<Prepared, String> {
    let model = &request.model;
    let key = request
        .api_key
        .clone()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| format!("No API key for provider: {}", model.provider))?;
    let base_url = resolve_azure_base_url(request)?;
    let deployment = resolve_deployment_name(request);
    let mut compat = compat(model);
    compat.supports_strict_mode = compat_bool(model, "supportsStrictMode").unwrap_or(true);
    let normalized = if compat.supports_mid_convo_system_messages {
        request.context.clone()
    } else {
        collapse_system_messages(request.context.clone())
    };
    let grammar = grammar_input_properties(
        &declared_tools(&normalized.messages),
        compat.supports_openai_grammar_tools,
    )?;
    let transcript_tools = resolve_transcript_tools(&normalized.messages, tool_support(&compat));
    let input = convert_messages(
        model,
        &normalized,
        &MessageOptions {
            grammar: &grammar,
            compat: &compat,
            configuration_update_effort: None,
            flavor: Flavor::Azure,
            include_system_prompt: true,
            default_strict: Some(false),
        },
        now,
    )?;
    let options = &request.options;
    let effort = options
        .reasoning
        .map(|level| clamp_thinking_level(model, level))
        .filter(|level| *level != ThinkingLevel::Off);
    let mut params = JsonObject::new();
    params.insert("model".into(), json!(deployment));
    params.insert("input".into(), Value::Array(input));
    params.insert("stream".into(), json!(true));
    if let Some(session) = &options.session_id {
        params.insert(
            "prompt_cache_key".into(),
            json!(clamp_prompt_cache_key(session)),
        );
    }
    params.insert("store".into(), json!(false));
    let max_tokens = clamp_max_tokens_to_context(
        model,
        &request.context.messages,
        options.max_tokens.unwrap_or(model.max_tokens),
    );
    if max_tokens > 0 {
        params.insert(
            "max_output_tokens".into(),
            json!(max_tokens.max(MIN_OUTPUT_TOKENS)),
        );
    }
    if let Some(temperature) = options.temperature {
        params.insert("temperature".into(), json!(temperature));
    }
    if !transcript_tools.request_tools.is_empty() {
        params.insert(
            "tools".into(),
            Value::Array(convert_tools(
                &transcript_tools.request_tools,
                ToolOptions {
                    supports_strict_mode: compat.supports_strict_mode,
                    supports_grammar: compat.supports_openai_grammar_tools,
                    tool_search_result: false,
                    default_strict: Some(false),
                },
            )?),
        );
    }
    if let Some(choice) = options.tool_choice.as_ref().filter(|c| !c.is_null()) {
        params.insert("tool_choice".into(), choice.clone());
    }
    let summary = request.extra.reasoning_summary.clone();
    let reasoning_effort = effort.map_or_else(
        || summary.as_ref().map(|_| json!("medium")),
        |level| Some(json!(level.as_str())),
    );
    if model.reasoning {
        if reasoning_effort.is_some() {
            let value = effort.map_or_else(
                || json!("medium"),
                |level| match mapped_level(model, level) {
                    MapEntry::Value(v) => json!(v),
                    _ => json!(level.as_str()),
                },
            );
            params.insert(
                "reasoning".into(),
                json!({"effort": value, "summary": summary.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "auto".to_owned())}),
            );
            params.insert("include".into(), json!(["reasoning.encrypted_content"]));
        } else if mapped_level(model, ThinkingLevel::Off) != MapEntry::Null {
            let off = match mapped_level(model, ThinkingLevel::Off) {
                MapEntry::Value(v) => v.to_owned(),
                _ => "none".to_owned(),
            };
            params.insert("reasoning".into(), json!({"effort": off}));
        }
    }
    let sampling_level = effort.unwrap_or(if summary.is_some() {
        ThinkingLevel::Medium
    } else {
        ThinkingLevel::Off
    });
    let base = resolve_sampling_params(
        model,
        options.reasoning.unwrap_or(ThinkingLevel::Off),
        options.sampling_params.as_ref(),
    );
    if let Some(sampling) = resolve_sampling_params(model, sampling_level, base.as_ref()) {
        for (k, v) in sampling {
            params.insert(k, v);
        }
    }
    let mut headers = Headers::new();
    headers.set("accept", "application/json");
    headers.set("content-type", "application/json");
    headers.set("user-agent", USER_AGENT);
    for (name, value) in model.headers.iter().flatten() {
        headers.set(name, value);
    }
    for (name, value) in request.header_overrides() {
        headers.apply(&name, value.as_deref());
    }
    headers.set("api-key", key);
    let url = format!(
        "{}/responses?api-version={}",
        base_url.trim_end_matches('/'),
        azure_api_version(request)
    );
    let parser = Parser::new(model, grammar, None, Flavor::Azure);
    Ok(Prepared {
        http: HttpRequest {
            url,
            headers,
            payload: Value::Object(params),
            timeout_ms: options.timeout_ms,
            signing: None,
        },
        exchange: Exchange::new(
            Emitter::new(model, now),
            Box::new(parser),
            ErrorStyle::Prefixed {
                prefix: "Azure OpenAI API error".into(),
            },
        ),
    })
}
