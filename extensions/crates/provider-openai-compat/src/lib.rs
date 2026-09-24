//! Bundled OpenAI-compatible provider component. One instance per declarative
//! provider contribution; host injects config via `init` and the api key via
//! `host.secret("api_key")`.

use std::sync::Mutex;

use oino_extension_guest::{host, http, Extension, Registration};
use oino_extension_guest_provider::{
    export_provider, prov::oino::extension::types::AssistantEvent, Provider, ResponseStream,
};
use oino_provider_kit::{
    affinity_headers, build_openai_compatible_chat_request_with_cache, sanitize_error_body,
    CompatConfig, OpenRouterModelsResponse, SseEventParser, StreamRequest,
};
use serde_json::{json, Value};

struct Compat;

static CONFIG: Mutex<Option<CompatConfig>> = Mutex::new(None);

fn config() -> Result<CompatConfig, String> {
    CONFIG
        .lock()
        .map_err(|_| "config lock".to_string())?
        .clone()
        .ok_or_else(|| "not initialized".to_string())
}

fn api_key() -> Option<String> {
    match host::secret("api_key") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn auth_headers(config: &CompatConfig) -> Result<Vec<(String, String)>, String> {
    match config.auth.as_str() {
        "required" => {
            let key = api_key().ok_or_else(|| "no api key configured".to_string())?;
            Ok(match &config.api_key_header {
                Some(name) => vec![(name.clone(), key)],
                None => vec![("Authorization".into(), format!("Bearer {key}"))],
            })
        }
        "optional" => Ok(match (api_key(), &config.api_key_header) {
            (Some(key), Some(name)) => vec![(name.clone(), key)],
            (Some(key), None) => vec![("Authorization".into(), format!("Bearer {key}"))],
            (None, _) => Vec::new(),
        }),
        _ => Ok(Vec::new()),
    }
}

fn base_headers(config: &CompatConfig) -> Result<Vec<(String, String)>, String> {
    let mut headers: Vec<(String, String)> = config
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    headers.extend(auth_headers(config)?);
    Ok(headers)
}

impl Extension for Compat {
    fn init(config_json: Value) -> Result<Registration, String> {
        let config: CompatConfig =
            serde_json::from_value(config_json).map_err(|err| format!("config: {err}"))?;
        let provider_id = config.provider_id.clone();
        *CONFIG.lock().map_err(|_| "config lock".to_string())? = Some(config);
        Ok(Registration {
            tools: Vec::new(),
            commands: Vec::new(),
            hooks: Vec::new(),
            providers: vec![provider_id],
        })
    }
}

impl Provider for Compat {
    fn list_models() -> Result<Value, String> {
        let config = config()?;
        let headers = base_headers(&config)?;
        let mut response = http::request("GET", &config.models_endpoint(), &headers, None)?;
        if !(200..300).contains(&response.status) {
            let mut body = Vec::new();
            while let Some(chunk) = response.read_chunk() {
                body.extend_from_slice(&chunk);
            }
            return Err(format!(
                "{} models request failed with status {}",
                config.display_name, response.status
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.read_chunk() {
            body.extend_from_slice(&chunk);
        }
        let parsed: OpenRouterModelsResponse =
            serde_json::from_slice(&body).map_err(|err| format!("models json: {err}"))?;
        let models: Vec<Value> = parsed
            .data
            .into_iter()
            .map(|model| {
                json!({
                    "id": model.id,
                    "name": model.name,
                    "owned_by": model.owned_by,
                    "supported_parameters": model.supported_parameters,
                    "context_length": model.context_length,
                })
            })
            .collect();
        Ok(json!({ "models": models }))
    }

    fn stream(request: Value) -> Result<ResponseStream, String> {
        let config = config()?;
        let request: StreamRequest =
            serde_json::from_value(request).map_err(|err| format!("request: {err}"))?;
        let body = build_openai_compatible_chat_request_with_cache(&request, &config)
            .map_err(|err| err.to_string())?;
        let mut headers = base_headers(&config)?;
        headers.push(("Content-Type".into(), "application/json".into()));
        headers.extend(affinity_headers(&config, request.session_id.as_deref()));
        let body = serde_json::to_vec(&body).map_err(|err| format!("body: {err}"))?;
        let response = http::request("POST", &config.endpoint(), &headers, Some(&body))?;
        if !(200..300).contains(&response.status) {
            let status = response.status;
            let mut buf = Vec::new();
            let mut response = response;
            while let Some(chunk) = response.read_chunk() {
                buf.extend_from_slice(&chunk);
            }
            let text = String::from_utf8_lossy(&buf).into_owned();
            return Err(format!(
                "OpenRouter HTTP error: status {status} {}: {}",
                status_reason(status),
                sanitize_error_body(&text)
            ));
        }
        Ok(ResponseStream(Mutex::new(Box::new(SseStream {
            response,
            parser: SseEventParser::for_provider(config.provider_id.clone()),
            finished: false,
        }))))
    }
}

struct SseStream {
    response: http::Response,
    parser: SseEventParser,
    finished: bool,
}

impl Iterator for SseStream {
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

/// Canonical reason phrases matching reqwest's `StatusCode` display so error
/// text is byte-identical with the native provider.
fn status_reason(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "HTTP Error",
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
        E::ThinkingSignature { signature } => AssistantEvent::ThinkingSignature(signature.clone()),
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

export_provider!(Compat);
