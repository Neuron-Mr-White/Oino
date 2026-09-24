//! Official "Oino Router" provider component (`oino:*` model ids).
//!
//! One bundled component serving both wire formats behind
//! `https://router.oino.dev/v1`: Anthropic Messages for claude-family models,
//! OpenAI chat completions for everything else. Format selection:
//! `format` config (`auto|openai|anthropic`), per-id `format_overrides`, then
//! auto-detect (`owned_by` claude/anthropic or `root` starting `claude-`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use oino_extension_guest::{host, http, Extension, Registration};
use oino_extension_guest_provider::{
    export_provider, prov::oino::extension::types::AssistantEvent, Provider, ResponseStream,
};
use oino_provider_kit::{
    anthropic, catalog, sanitize_error_body, CompatConfig, CompatFlags, SseEventParser,
    StreamRequest,
};
use serde_json::{json, Value};

const DEFAULT_BASE_URL: &str = "https://router.oino.dev/v1";
const PROVIDER_ID: &str = "oino";

/// Resolved component config (secrets come from `host.secret`).
#[derive(Debug, Clone, serde::Deserialize)]
struct OinoConfig {
    #[serde(default = "default_base_url")]
    base_url: String,
    /// `auto` | `openai` | `anthropic`.
    #[serde(default = "default_format")]
    format: String,
    #[serde(default)]
    format_overrides: BTreeMap<String, String>,
    #[serde(default)]
    #[allow(dead_code)]
    provider_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.into()
}
fn default_format() -> String {
    "auto".into()
}

/// Endpoint `/v1/models` row (subset).
#[derive(Debug, Clone, serde::Deserialize)]
struct RouterModel {
    id: String,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    family: Option<String>,
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    capabilities: Value,
}

impl RouterModel {
    fn effort_tiers(&self) -> Vec<String> {
        self.capabilities["effort_tiers"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|tier| tier.as_str().map(str::to_string))
            .collect()
    }
    /// claude-family → anthropic wire format (spec rule).
    fn wants_anthropic(&self) -> bool {
        let owner = self.owned_by.as_deref().unwrap_or_default();
        owner == "claude"
            || owner == "anthropic"
            || self
                .root
                .as_deref()
                .unwrap_or_default()
                .starts_with("claude-")
    }
}

static CONFIG: Mutex<Option<OinoConfig>> = Mutex::new(None);
/// Cached `/models` rows (also used for format detection + effort clamping).
static ROUTER_MODELS: Mutex<Option<BTreeMap<String, RouterModel>>> = Mutex::new(None);
static PI_CATALOG: Mutex<Option<BTreeMap<String, catalog::CatalogModel>>> = Mutex::new(None);

fn config() -> Result<OinoConfig, String> {
    CONFIG
        .lock()
        .map_err(|_| "config lock".to_string())?
        .clone()
        .ok_or_else(|| "not initialized".to_string())
}

fn pi_catalog() -> BTreeMap<String, catalog::CatalogModel> {
    let mut guard = match PI_CATALOG.lock() {
        Ok(g) => g,
        Err(_) => return BTreeMap::new(),
    };
    guard.get_or_insert_with(catalog::anthropic_models).clone()
}

fn api_key() -> Option<String> {
    for name in ["OINO_API_KEY", "OMNI_API_KEY"] {
        if let Ok(key) = host::secret(name) {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
    }
    None
}

fn require_key() -> Result<String, String> {
    api_key().ok_or_else(|| "Oino Router: set OINO_API_KEY (or OMNI_API_KEY)".to_string())
}

/// Fetch + cache the router model list.
fn router_models(config: &OinoConfig) -> Result<BTreeMap<String, RouterModel>, String> {
    if let Some(models) = ROUTER_MODELS
        .lock()
        .map_err(|_| "models lock".to_string())?
        .clone()
    {
        return Ok(models);
    }
    let key = require_key()?;
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let headers = vec![
        ("Authorization".to_string(), format!("Bearer {key}")),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let mut response = http::request("GET", &url, &headers, None)?;
    if !(200..300).contains(&response.status) {
        let status = response.status;
        let mut buf = Vec::new();
        while let Some(chunk) = response.read_chunk() {
            buf.extend_from_slice(&chunk);
        }
        return Err(format!(
            "Oino Router models request failed with status {status}: {}",
            sanitize_error_body(&String::from_utf8_lossy(&buf))
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.read_chunk() {
        body.extend_from_slice(&chunk);
    }
    let parsed: Value = serde_json::from_slice(&body).map_err(|err| format!("models: {err}"))?;
    let mut models = BTreeMap::new();
    for entry in parsed["data"].as_array().cloned().unwrap_or_default() {
        if let Ok(model) = serde_json::from_value::<RouterModel>(entry) {
            models.insert(model.id.clone(), model);
        }
    }
    *ROUTER_MODELS
        .lock()
        .map_err(|_| "models lock".to_string())? = Some(models.clone());
    Ok(models)
}

/// Wire format for an endpoint id: explicit override → `format` → auto.
fn format_for(config: &OinoConfig, id: &str) -> String {
    if let Some(format) = config.format_overrides.get(id) {
        return format.clone();
    }
    match config.format.as_str() {
        "openai" | "anthropic" => return config.format.clone(),
        _ => {}
    }
    let models = router_models(config).unwrap_or_default();
    match models.get(id) {
        Some(model) if model.wants_anthropic() => "anthropic".into(),
        _ => "openai".into(),
    }
}

/// Clamp `level` to the nearest effort tier the model supports (tier order:
/// none < low < medium < high < xhigh < max; `minimal` maps to `low`).
fn clamp_level(level: oino_types::ThinkingLevel, tiers: &[String]) -> oino_types::ThinkingLevel {
    use oino_types::ThinkingLevel as T;
    const ORDER: &[(&str, T)] = &[
        ("none", T::Off),
        ("minimal", T::Minimal),
        ("low", T::Low),
        ("medium", T::Medium),
        ("high", T::High),
        ("xhigh", T::XHigh),
        ("max", T::Max),
    ];
    if tiers.is_empty() {
        return level;
    }
    let wanted = ORDER
        .iter()
        .position(|(_, candidate)| *candidate == level)
        .unwrap_or(0);
    // nearest supported tier at or below the requested level
    let mut best: Option<usize> = None;
    for tier in tiers {
        if let Some(index) = ORDER.iter().position(|(name, _)| name == tier) {
            if index <= wanted {
                best = Some(best.map_or(index, |b| b.max(index)));
            }
        }
    }
    best.map(|index| ORDER[index].1).unwrap_or_else(|| {
        // nothing below → lowest supported tier
        let lowest = tiers
            .iter()
            .filter_map(|tier| ORDER.iter().position(|(name, _)| name == tier))
            .min();
        lowest.map(|index| ORDER[index].1).unwrap_or(level)
    })
}

struct Oino;

impl Extension for Oino {
    fn init(config_json: Value) -> Result<Registration, String> {
        let mut config: OinoConfig =
            serde_json::from_value(config_json).map_err(|err| format!("config: {err}"))?;
        if config.base_url.trim().is_empty() {
            config.base_url = DEFAULT_BASE_URL.into();
        }
        *CONFIG.lock().map_err(|_| "config lock".to_string())? = Some(config);
        Ok(Registration {
            providers: vec![PROVIDER_ID.into()],
            ..Registration::default()
        })
    }
}

impl Provider for Oino {
    fn list_models() -> Result<Value, String> {
        let config = config()?;
        let models = router_models(&config)?;
        let pi = pi_catalog();
        let list: Vec<Value> = models
            .values()
            .map(|model| {
                let mut entry = json!({
                    "id": model.id,
                    "name": model.name.clone().unwrap_or_else(|| model.id.clone()),
                    "context_window": model.context_length,
                    "max_output_tokens": model.max_output_tokens,
                    "input_modalities": model.input_modalities,
                    "owned_by": model.owned_by,
                    "thinking_levels": model
                        .effort_tiers()
                        .iter()
                        .filter_map(|tier| match tier.as_str() {
                            "none" => Some("off"),
                            other => Some(match other {
                                "low" => "low",
                                "medium" => "medium",
                                "high" => "high",
                                "xhigh" => "xhigh",
                                "max" => "max",
                                "minimal" => "minimal",
                                _ => return None,
                            }),
                        })
                        .collect::<Vec<_>>(),
                });
                // pi anthropic catalog enrichment (cache lifetimes + costs).
                if let Some(root) = model.root.as_deref() {
                    if model.wants_anthropic() {
                        if let Some(pi_model) = pi.get(root) {
                            if let Some(cache) = &pi_model.prompt_cache {
                                entry["prompt_cache"] =
                                    json!({"short_secs": cache.short, "long_secs": cache.long});
                            }
                            entry["cost"] = json!({
                                "input": pi_model.cost.input,
                                "output": pi_model.cost.output,
                                "cache_read": pi_model.cost.cache_read,
                                "cache_write": pi_model.cost.cache_write,
                            });
                        }
                    }
                }
                entry
            })
            .collect();
        Ok(json!({ "models": list }))
    }

    fn stream(request: Value) -> Result<ResponseStream, String> {
        let config = config()?;
        let mut request: StreamRequest =
            serde_json::from_value(request).map_err(|err| format!("request: {err}"))?;
        let endpoint_id = request.model.name.clone();
        let format = format_for(&config, &endpoint_id);
        let key = require_key()?;
        // Clamp thinking level to the model's declared effort tiers.
        if let Ok(models) = router_models(&config) {
            if let Some(model) = models.get(&endpoint_id) {
                request.thinking_level = clamp_level(request.thinking_level, &model.effort_tiers());
            }
        }
        match format.as_str() {
            "anthropic" => stream_anthropic(&config, &request, &endpoint_id, &key),
            _ => stream_openai(&config, &request, &endpoint_id, &key),
        }
    }
}

fn stream_openai(
    config: &OinoConfig,
    request: &StreamRequest,
    _endpoint_id: &str,
    key: &str,
) -> Result<ResponseStream, String> {
    let compat = CompatConfig {
        provider_id: PROVIDER_ID.into(),
        display_name: config
            .display_name
            .clone()
            .unwrap_or_else(|| "Oino Router".into()),
        base_url: config.base_url.clone(),
        models_url: None,
        headers: BTreeMap::new(),
        auth: "required".into(),
        api_key_header: None,
        model_id_policy: Some("strip_provider_prefix".into()),
        compat: CompatFlags {
            send_prompt_cache_key: Some(true),
            session_affinity: None,
            cache_control_format: None,
        },
    };
    let body = oino_provider_kit::build_openai_compatible_chat_request_with_cache(request, &compat)
        .map_err(|err| err.to_string())?;
    let headers = vec![
        ("Authorization".to_string(), format!("Bearer {key}")),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body_bytes = serde_json::to_vec(&body).map_err(|err| format!("body: {err}"))?;
    let response = http::request("POST", &url, &headers, Some(&body_bytes))?;
    if !(200..300).contains(&response.status) {
        return Err(error_text("Oino Router", response));
    }
    Ok(ResponseStream(Mutex::new(Box::new(OpenAiSse {
        response,
        parser: SseEventParser::for_provider(PROVIDER_ID),
        finished: false,
    }))))
}

fn stream_anthropic(
    config: &OinoConfig,
    request: &StreamRequest,
    endpoint_id: &str,
    key: &str,
) -> Result<ResponseStream, String> {
    let pi = pi_catalog();
    // pi catalog keys are bare model roots (e.g. `claude-haiku-4-5`); the
    // endpoint id may carry a routing prefix (`cc/claude-...`).
    let catalog_id = router_models(config)
        .ok()
        .and_then(|models| models.get(endpoint_id).and_then(|m| m.root.clone()))
        .unwrap_or_else(|| endpoint_id.to_string());
    let mut model = catalog::model_or_default(&pi, &catalog_id, PROVIDER_ID, "anthropic-messages");
    // Router expects the endpoint id (e.g. `cc/claude-...`), not the pi
    // catalog root id.
    model.id = endpoint_id.to_string();
    let mut options = anthropic::thinking_options_for_level(
        &model,
        request.thinking_level,
        &request.transcript.messages,
    );
    options.api_key = Some(key);
    if let Some(max) = request.max_output_tokens {
        options.max_tokens = Some(u64::from(max));
    }
    let (body, betas) = anthropic::build_params(&model, request, &BTreeMap::new(), &options)
        .map_err(|err| err.to_string())?;
    let mut headers: Vec<(String, String)> = vec![
        ("accept".into(), "application/json".into()),
        ("anthropic-version".into(), "2023-06-01".into()),
        ("Content-Type".into(), "application/json".into()),
        ("Authorization".into(), format!("Bearer {key}")),
        ("x-api-key".into(), key.to_string()),
    ];
    if !betas.is_empty() {
        headers.push(("anthropic-beta".into(), betas.join(",")));
    }
    let url = format!(
        "{}/messages?beta=true",
        config.base_url.trim_end_matches('/')
    );
    let body_bytes = serde_json::to_vec(&body).map_err(|err| format!("body: {err}"))?;
    let response = http::request("POST", &url, &headers, Some(&body_bytes))?;
    if !(200..300).contains(&response.status) {
        return Err(error_text("Oino Router", response));
    }
    Ok(ResponseStream(Mutex::new(Box::new(AnthropicSse {
        response,
        parser: anthropic::AnthropicSseParser::new(model),
        finished: false,
    }))))
}

fn error_text(label: &str, mut response: http::Response) -> String {
    let status = response.status;
    let mut buf = Vec::new();
    while let Some(chunk) = response.read_chunk() {
        buf.extend_from_slice(&chunk);
    }
    format!(
        "{label} HTTP error: status {status}: {}",
        sanitize_error_body(&String::from_utf8_lossy(&buf))
    )
}

struct OpenAiSse {
    response: http::Response,
    parser: SseEventParser,
    finished: bool,
}

impl Iterator for OpenAiSse {
    type Item = Result<Vec<AssistantEvent>, String>;
    fn next(&mut self) -> Option<Self::Item> {
        sse_next(&mut self.response, &mut self.parser, &mut self.finished)
    }
}

struct AnthropicSse {
    response: http::Response,
    parser: anthropic::AnthropicSseParser,
    finished: bool,
}

impl Iterator for AnthropicSse {
    type Item = Result<Vec<AssistantEvent>, String>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.finished {
                return None;
            }
            match self.response.read_chunk() {
                Some(chunk) => {
                    let text = match std::str::from_utf8(&chunk) {
                        Ok(text) => text,
                        Err(err) => return Some(Err(format!("sse utf8: {err}"))),
                    };
                    match self.parser.push_str(text) {
                        Ok(events) if !events.is_empty() => {
                            // Terminal events end the SSE stream — do not keep
                            // reading on keep-alive connections.
                            if events.iter().any(|e| {
                                matches!(
                                    e,
                                    oino_types::AssistantStreamEvent::Done { .. }
                                        | oino_types::AssistantStreamEvent::Aborted
                                        | oino_types::AssistantStreamEvent::Error { .. }
                                )
                            }) {
                                self.finished = true;
                            }
                            return Some(Ok(events.iter().map(event_to_wit).collect()));
                        }
                        Ok(_) => continue,
                        Err(err) => return Some(Err(err.to_string())),
                    }
                }
                None => {
                    self.finished = true;
                    match self.parser.finish() {
                        Ok(events) if !events.is_empty() => {
                            // Terminal events end the SSE stream — do not keep
                            // reading on keep-alive connections.
                            if events.iter().any(|e| {
                                matches!(
                                    e,
                                    oino_types::AssistantStreamEvent::Done { .. }
                                        | oino_types::AssistantStreamEvent::Aborted
                                        | oino_types::AssistantStreamEvent::Error { .. }
                                )
                            }) {
                                self.finished = true;
                            }
                            return Some(Ok(events.iter().map(event_to_wit).collect()));
                        }
                        Ok(_) => return None,
                        Err(err) => return Some(Err(err.to_string())),
                    }
                }
            }
        }
    }
}

fn sse_next(
    response: &mut http::Response,
    parser: &mut SseEventParser,
    finished: &mut bool,
) -> Option<Result<Vec<AssistantEvent>, String>> {
    loop {
        if *finished {
            return None;
        }
        match response.read_chunk() {
            Some(chunk) => {
                let text = match std::str::from_utf8(&chunk) {
                    Ok(text) => text,
                    Err(err) => return Some(Err(format!("sse utf8: {err}"))),
                };
                match parser.push_str(text) {
                    Ok(events) if !events.is_empty() => {
                        return Some(Ok(events.iter().map(event_to_wit).collect()))
                    }
                    Ok(_) => continue,
                    Err(err) => return Some(Err(err.to_string())),
                }
            }
            None => {
                *finished = true;
                match parser.finish() {
                    Ok(events) if !events.is_empty() => {
                        return Some(Ok(events.iter().map(event_to_wit).collect()))
                    }
                    Ok(_) => return None,
                    Err(err) => return Some(Err(err.to_string())),
                }
            }
        }
    }
}

fn event_to_wit(event: &oino_types::AssistantStreamEvent) -> AssistantEvent {
    use oino_types::AssistantStreamEvent as E;
    match event {
        E::Start {
            response_id,
            response_model,
        } => AssistantEvent::Start(
            json!({ "response_id": response_id, "response_model": response_model }).to_string(),
        ),
        E::TextDelta { delta } => AssistantEvent::TextDelta(delta.clone()),
        E::ThinkingDelta { delta } => AssistantEvent::ThinkingDelta(delta.clone()),
        E::ThinkingSignature { signature } => {
            AssistantEvent::ThinkingSignature(signature.clone())
        }
        E::ToolCallDelta {
            id,
            name,
            native_id,
            arguments_delta,
        } => AssistantEvent::ToolCallDelta(
            json!({ "id": id, "name": name, "native_id": native_id, "arguments_delta": arguments_delta })
                .to_string(),
        ),
        E::ToolCallDone {
            id,
            name,
            native_id,
            arguments,
        } => AssistantEvent::ToolCallDone(
            json!({ "id": id, "name": name, "native_id": native_id, "arguments": arguments })
                .to_string(),
        ),
        E::Usage { usage } => {
            AssistantEvent::Usage(serde_json::to_string(usage).unwrap_or_default())
        }
        E::Done {
            stop_reason,
            provider,
        } => AssistantEvent::Done(
            json!({
                "stop_reason": serde_json::to_value(stop_reason).unwrap_or_default(),
                "provider": provider,
            })
            .to_string(),
        ),
        E::Error { message } => AssistantEvent::Error(message.clone()),
        E::Aborted => AssistantEvent::Aborted,
    }
}

export_provider!(Oino);
