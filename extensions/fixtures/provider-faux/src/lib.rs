//! Faux provider: streams `metadata.fauxScript` (an array of `ProviderEvent` JSON values, or
//! `{"text": "..."}` / `{"toolCall": {...}}` / `{"error": "..."}` shorthand steps) or a default
//! one-sentence reply. Calls `provider-host` hooks like a real provider.

#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{self, Provider, ProviderStream};

/// Fixture state.
#[derive(Default)]
struct Faux;

fn message(request: &Value, content: &[Value], stop: &str, error: Option<&str>) -> Value {
    let model = &request["model"];
    json!({
        "content": content,
        "api": model["api"].as_str().unwrap_or("faux"),
        "provider": model["provider"].as_str().unwrap_or("faux"),
        "model": model["id"].as_str().unwrap_or("faux-1"),
        "usage": {"input": 3, "output": 5, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 8,
                  "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
        "stopReason": stop,
        "errorMessage": error,
        "timestamp": 0,
    })
}

fn script(request: &Value) -> Vec<Value> {
    let steps = request
        .pointer("/options/metadata/fauxScript")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![json!({"text": "Hello from faux."})]);
    let mut events = vec![json!({"type": "start"})];
    let mut content = Vec::new();
    for step in steps {
        if step.get("type").is_some() {
            events.push(step);
            continue;
        }
        let index = content.len();
        if let Some(text) = step.get("text").and_then(Value::as_str) {
            events.push(json!({"type": "text_start", "contentIndex": index}));
            events.push(json!({"type": "text_delta", "contentIndex": index, "delta": text}));
            events.push(json!({"type": "text_end", "contentIndex": index, "content": text}));
            content.push(json!({"type": "text", "text": text}));
        } else if let Some(call) = step.get("toolCall") {
            events.push(json!({"type": "toolcall_start", "contentIndex": index, "id": call["id"], "toolName": call["name"]}));
            events.push(json!({"type": "toolcall_end", "contentIndex": index, "toolCall": {"type": "toolCall", "id": call["id"], "name": call["name"], "arguments": call["arguments"]}}));
            content.push(json!({"type": "toolCall", "id": call["id"], "name": call["name"], "arguments": call["arguments"]}));
        } else if let Some(error) = step.get("error").and_then(Value::as_str) {
            events.push(json!({"type": "error", "reason": "error", "error": message(request, &content, "error", Some(error))}));
            return events;
        }
    }
    let stop = if content.iter().any(|c| c["type"] == "toolCall") {
        "toolUse"
    } else {
        "stop"
    };
    events.push(
        json!({"type": "done", "reason": stop, "message": message(request, &content, stop, None)}),
    );
    events
}

impl Provider for Faux {
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>> {
        Box::pin(async move {
            let request_id = request["requestId"].as_str().unwrap_or_default().to_owned();
            let (_payload, _headers) = provider::before_request(
                &request_id,
                &json!({"model": request["model"]["id"]}),
                Vec::new(),
            )
            .await;
            provider::after_response(&request_id, 200, &[]);
            let events = script(&request);
            Ok(ProviderStream::spawn(move |mut sink| async move {
                for event in events {
                    if !sink.send(&event).await {
                        break;
                    }
                }
            }))
        })
    }

    fn list_models(
        &'static self,
        provider_id: String,
        _credential: Option<Value>,
    ) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async move {
            Ok(
                json!([{"id": "faux-1", "name": "Faux 1", "api": "faux", "provider": provider_id, "baseUrl": "",
                       "input": ["text"], "reasoning": false, "contextWindow": 128_000, "maxTokens": 4096,
                       "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}}]),
            )
        })
    }
}

impl Extension for Faux {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        oino_extension::wit::registry::register_provider(
            &json!({"id": "faux", "name": "Faux", "api": "faux", "baseUrl": "http://127.0.0.1:1", "authEnv": [], "models": "dynamic"}).to_string(),
        )?;
        Ok(())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(Faux);
