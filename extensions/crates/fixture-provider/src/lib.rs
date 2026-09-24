//! Faux provider component for `oino-wasm-host` streaming tests.

use oino_extension_guest::{http, Extension, Registration};
use oino_extension_guest_provider::{
    export_provider, prov::oino::extension::types::AssistantEvent, Provider, ResponseStream,
};
use serde_json::{json, Value};
use std::sync::Mutex;

struct FixtureProvider;

impl Extension for FixtureProvider {
    fn init(_config: Value) -> Result<Registration, String> {
        Ok(Registration {
            providers: vec!["faux".into()],
            ..Default::default()
        })
    }
}

impl Provider for FixtureProvider {
    fn list_models() -> Result<Value, String> {
        Ok(json!([{ "provider": "faux", "name": "faux-1" }]))
    }

    fn stream(request: Value) -> Result<ResponseStream, String> {
        // Test knobs ride in `session_id` as a JSON object so the wire
        // request keeps its real StreamRequest shape.
        let knobs: Value = request["session_id"]
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        if let Some(url) = knobs["sse_url"].as_str() {
            return sse_stream(url);
        }
        let total = knobs["faux_events"].as_u64().unwrap_or(10) as usize;
        let mut produced = 0usize;
        let iter = std::iter::from_fn(move || {
            if produced >= total {
                return None;
            }
            let batch: Vec<AssistantEvent> = (0..(total - produced).min(64))
                .map(|_| {
                    produced += 1;
                    AssistantEvent::TextDelta(format!("event-{}", produced - 1))
                })
                .collect();
            Some(Ok(batch))
        });
        Ok(ResponseStream(Mutex::new(Box::new(iter))))
    }
}

fn sse_stream(url: &str) -> Result<ResponseStream, String> {
    let mut response = http::get(url)?;
    let mut pending = String::new();
    let done = Mutex::new(false);
    let iter = std::iter::from_fn(move || {
        if *done.lock().unwrap_or_else(|e| e.into_inner()) {
            return None;
        }
        let mut events = Vec::new();
        while events.len() < 64 {
            if pending.is_empty() {
                match response.read_chunk() {
                    Some(chunk) => {
                        pending.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    None => {
                        *done.lock().unwrap_or_else(|e| e.into_inner()) = true;
                        break;
                    }
                }
            }
            while let Some(index) = pending.find('\n') {
                let line = pending[..index].trim_end_matches('\r').to_string();
                pending.drain(..=index);
                if let Some(data) = line.strip_prefix("data:") {
                    events.push(AssistantEvent::TextDelta(data.trim().to_string()));
                }
            }
            if events.is_empty() && *done.lock().unwrap_or_else(|e| e.into_inner()) {
                return None;
            }
            if events.len() >= 64 {
                break;
            }
        }
        if events.is_empty() {
            None
        } else {
            Some(Ok(events))
        }
    });
    Ok(ResponseStream(Mutex::new(Box::new(iter))))
}

export_provider!(FixtureProvider);
