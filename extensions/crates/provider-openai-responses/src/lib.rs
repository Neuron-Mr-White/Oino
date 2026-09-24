//! Bundled OpenAI Responses provider component — faithful port of pi's
//! `api/openai-responses.ts` + shared helpers.

use std::collections::BTreeMap;
use std::sync::Mutex;

use oino_extension_guest::{host, http, Extension, Registration};
use oino_extension_guest_provider::{
    export_provider, prov::oino::extension::types::AssistantEvent, Provider, ResponseStream,
};
use oino_provider_kit::{
    catalog, piutil, responses, sanitize_error_body, CompatConfig, StreamRequest,
};
use serde_json::{json, Value};

struct Responses;

static CONFIG: Mutex<Option<CompatConfig>> = Mutex::new(None);
static MODELS: Mutex<Option<BTreeMap<String, catalog::CatalogModel>>> = Mutex::new(None);

fn config() -> Result<CompatConfig, String> {
    CONFIG
        .lock()
        .map_err(|_| "config lock".to_string())?
        .clone()
        .ok_or_else(|| "not initialized".to_string())
}

fn models() -> BTreeMap<String, catalog::CatalogModel> {
    let mut guard = match MODELS.lock() {
        Ok(g) => g,
        Err(_) => return BTreeMap::new(),
    };
    guard.get_or_insert_with(catalog::openai_models).clone()
}

fn api_key() -> Option<String> {
    match host::secret("api_key") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

impl Extension for Responses {
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

impl Provider for Responses {
    fn list_models() -> Result<Value, String> {
        let models: Vec<Value> = models()
            .values()
            .map(|m| {
                json!({
                    "id": m.id,
                    "name": m.name,
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_tokens,
                    "thinking_levels": piutil::supported_thinking_levels(
                        m.reasoning,
                        m.thinking_level_map.as_ref(),
                    ),
                })
            })
            .collect();
        Ok(json!({ "models": models }))
    }

    fn stream(request: Value) -> Result<ResponseStream, String> {
        let config = config()?;
        let request: StreamRequest =
            serde_json::from_value(request).map_err(|err| format!("request: {err}"))?;
        let catalog_models = models();
        let model = catalog::model_or_default(
            &catalog_models,
            &request.model.name,
            &config.provider_id,
            "openai-responses",
        );
        let mut options = responses::ResponsesRequestOptions::default();
        let effort = responses::reasoning_effort_for_level(&model, request.thinking_level);
        let effort_owned;
        options.reasoning_effort = {
            effort_owned = effort;
            effort_owned.as_deref()
        };
        options.max_tokens = request.max_output_tokens.map(u64::from);
        let body = responses::build_params(&model, &request, &options);

        let mut headers: Vec<(String, String)> = vec![
            ("accept".into(), "application/json".into()),
            ("Content-Type".into(), "application/json".into()),
        ];
        for (name, value) in responses::session_headers(&model, request.session_id.as_deref()) {
            headers.push((name, value));
        }
        for (name, value) in &config.headers {
            headers.push((name.clone(), value.clone()));
        }
        if let Some(key) = api_key() {
            headers.push(("Authorization".into(), format!("Bearer {key}")));
        } else if !config
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"))
        {
            return Err("No API key for provider: openai".to_string());
        }

        let url = format!("{}/responses", config.base_url.trim_end_matches('/'));
        let body_bytes = serde_json::to_vec(&body).map_err(|err| format!("body: {err}"))?;
        let response = http::request("POST", &url, &headers, Some(&body_bytes))?;
        if !(200..300).contains(&response.status) {
            let status = response.status;
            let mut response = response;
            let mut buf = Vec::new();
            while let Some(chunk) = response.read_chunk() {
                buf.extend_from_slice(&chunk);
            }
            let text = String::from_utf8_lossy(&buf).into_owned();
            return Err(format!(
                "OpenAI API error {status}: {}",
                sanitize_error_body(&text)
            ));
        }
        Ok(ResponseStream(Mutex::new(Box::new(SseStream {
            response,
            parser: responses::ResponsesSseParser::new(model),
            finished: false,
        }))))
    }
}

struct SseStream {
    response: http::Response,
    parser: responses::ResponsesSseParser,
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
                            return Some(Ok(events.iter().map(event_to_wit).collect()))
                        }
                        Ok(_) => continue,
                        Err(err) => return Some(Err(err.to_string())),
                    }
                }
                None => {
                    self.finished = true;
                    match self.parser.finish() {
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

export_provider!(Responses);
