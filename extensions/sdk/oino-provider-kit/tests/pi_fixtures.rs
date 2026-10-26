//! Replays the pi-ai 1.4.0 oracle fixtures (`scripts/pi-oracle/providers-v1/payload.mjs`):
//! request URL, headers and body must equal pi's captured request, and the recorded SSE (or
//! error response) must produce pi's events, final message, usage and cost.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::fs;
use std::path::{Path, PathBuf};

use oino_ai::json::json_semantic_eq;
use oino_provider_kit::{PrepareOptions, ProviderRequest, Wire, finalize, prepare};
use serde_json::{Value, json};

fn fixtures(api: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(api);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
}

/// Headers compared with pi: everything except SDK telemetry and the product user agent.
fn comparable_headers(headers: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (key, value) in headers.as_object().into_iter().flatten() {
        let key = key.to_ascii_lowercase();
        if key.starts_with("x-stainless")
            || (key == "user-agent"
                && !value.as_str().unwrap_or_default().starts_with("claude-cli"))
        {
            continue;
        }
        out.insert(key, value.clone());
    }
    Value::Object(out)
}

/// Drops what typed `oino-ai` values imply: `role` on assistant messages and `type` on
/// tool calls.
fn strip_implied(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if object.get("role") == Some(&json!("assistant")) {
            object.remove("role");
        }
        if object.get("type") == Some(&json!("toolCall")) {
            object.remove("type");
        }
        for child in object.values_mut() {
            strip_implied(child);
        }
    } else if let Some(list) = value.as_array_mut() {
        list.iter_mut().for_each(strip_implied);
    }
}

fn normalize_event(mut event: Value) -> Value {
    for key in ["toolCall", "message", "error"] {
        if let Some(inner) = event.get_mut(key) {
            strip_implied(inner);
        }
    }
    if event["type"] == "toolcall_start"
        && let Some(object) = event.as_object_mut()
    {
        object.remove("id");
        object.remove("toolName");
    }
    for key in ["message", "error"] {
        if let Some(message) = event.get_mut(key).and_then(Value::as_object_mut) {
            message.insert("timestamp".into(), json!(0));
            message.remove("durationMs");
            if let Some(diagnostics) = message.get_mut("diagnostics").and_then(Value::as_array_mut)
            {
                for d in diagnostics {
                    d["timestamp"] = json!(0);
                }
            }
        }
    }
    event
}

fn run(wire: Wire, path: &Path) -> Result<(), String> {
    let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let request_json = json!({
        "model": fixture["model"],
        "context": fixture["context"],
        "options": fixture["options"],
        "apiKey": fixture["apiKey"],
    });
    let request = ProviderRequest::parse(&request_json)?;
    let mut prepared = prepare(wire, &request, PrepareOptions::default(), 0)?;
    let mut problems = Vec::new();
    let pi_request = &fixture["request"];
    if pi_request["url"].is_string() {
        if pi_request["url"] != json!(prepared.http.url) {
            problems.push(format!(
                "url: pi {} ours {}",
                pi_request["url"], prepared.http.url
            ));
        }
        let mut headers = prepared.http.headers.clone();
        let body = finalize(wire, prepared.http.payload.clone(), &mut headers);
        if !json_semantic_eq(&pi_request["body"], &body) {
            problems.push(format!(
                "body:\n  pi   {}\n  ours {}",
                pi_request["body"], body
            ));
        }
        let ours: serde_json::Map<String, Value> = headers
            .pairs()
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let (pi_h, ours_h) = (
            comparable_headers(&pi_request["headers"]),
            comparable_headers(&Value::Object(ours)),
        );
        if pi_h != ours_h {
            problems.push(format!("headers:\n  pi   {pi_h}\n  ours {ours_h}"));
        }
    }
    let exchange = &mut prepared.exchange;
    let status = u16::try_from(fixture["status"].as_u64().unwrap()).unwrap();
    let sse = fixture["sse"].as_str().unwrap_or_default().as_bytes();
    let mut events = Vec::new();
    if status == 200 {
        events.extend(exchange.start());
        for chunk in sse.chunks(7) {
            events.extend(exchange.feed(chunk));
        }
        events.extend(exchange.finish());
    } else {
        events.extend(exchange.http_error(status, sse));
    }
    let ours: Vec<Value> = events
        .into_iter()
        .map(|e| normalize_event(serde_json::to_value(e).unwrap()))
        .collect();
    let pi: Vec<Value> = fixture["events"]
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(normalize_event)
        .collect();
    if ours.len() == pi.len() {
        for (index, (a, b)) in pi.iter().zip(&ours).enumerate() {
            if !json_semantic_eq(a, b) {
                problems.push(format!("event {index}:\n  pi   {a}\n  ours {b}"));
            }
        }
    } else {
        problems.push(format!(
            "event count {} vs pi {}:\n  pi   {:?}\n  ours {:?}",
            ours.len(),
            pi.len(),
            pi.iter().map(|e| e["type"].clone()).collect::<Vec<_>>(),
            ours.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

fn check(api: &str, wire: Wire) {
    let mut failures = Vec::new();
    let files = fixtures(api);
    assert!(files.len() >= 10, "{api}: expected at least 10 fixtures");
    for path in &files {
        if let Err(problem) = run(wire, path) {
            failures.push(format!("== {}\n{problem}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixtures differ from pi:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn anthropic_messages_matches_pi() {
    check("anthropic-messages", Wire::Anthropic);
}

#[test]
fn openai_completions_matches_pi() {
    check("openai-completions", Wire::Completions);
}

#[test]
fn openai_responses_matches_pi() {
    check("openai-responses", Wire::Responses);
}
