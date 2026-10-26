//! Golden test: session event streams of scripted runs match pi 1.4.0 `AgentSession`
//! (fixture from `scripts/pi-oracle/core-runtime/session/session-events.mjs`, pi-ai faux
//! provider). Timestamps, ids, usage, partials and system prompt texts are normalised; prompt
//! section names are compared.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::time::Duration;

use oino_ai::json::json_semantic_eq;
use oino_ai::{AssistantContent, JsonValue, StopReason, Usage};
use oino_core::session::PromptOptions;
use oino_core::testing::{FauxProvider, FauxResponse, TestSession};

fn fixture(name: &str) -> JsonValue {
    let path = format!(
        "{}/tests/fixtures/session/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

const VOLATILE: [&str; 8] = [
    "timestamp",
    "durationMs",
    "partial",
    "usage",
    "id",
    "parentId",
    "targetId",
    "responseId",
];

/// `message_update.message` is a shallow pi copy whose content keeps mutating after the event
/// (pi listeners observe later text); the wire form drops it too (modes §3.1), so only the
/// `assistantMessageEvent` is compared.
fn normalize_event(event: &JsonValue) -> JsonValue {
    let mut event = normalize(event);
    if event["type"] == "message_update" {
        if let Some(map) = event.as_object_mut() {
            map.shift_remove("message");
        }
        // `oino_ai::ToolCall` outside a content array lacks `"type":"toolCall"` (reported to
        // core-types-session).
        if let Some(call) = event["assistantMessageEvent"]["toolCall"].as_object_mut() {
            call.shift_remove("type");
        }
    }
    // The assistant `message_start` carries the provider's first partial; pi's faux provider
    // seeds it from the final message (`errorMessage`, content), so only its shape is compared.
    if event["type"] == "message_start"
        && event["message"]["role"] == "assistant"
        && let Some(message) = event["message"].as_object_mut()
    {
        message.shift_remove("errorMessage");
        message.insert("content".into(), JsonValue::Array(Vec::new()));
    }
    event
}

fn normalize(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(items) => JsonValue::Array(items.iter().map(normalize).collect()),
        JsonValue::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, item) in map {
                if VOLATILE.contains(&key.as_str()) {
                    continue;
                }
                if key == "sections" {
                    let names: Vec<JsonValue> = item
                        .as_object()
                        .map(|s| s.keys().cloned().map(JsonValue::String).collect())
                        .unwrap_or_default();
                    out.insert(key.clone(), JsonValue::Array(names));
                    continue;
                }
                out.insert(key.clone(), normalize(item));
            }
            JsonValue::Object(out)
        }
        other => other.clone(),
    }
}

fn response(spec: &JsonValue) -> FauxResponse {
    let content: Vec<AssistantContent> = serde_json::from_value(spec["content"].clone()).unwrap();
    FauxResponse::Message {
        content,
        stop_reason: serde_json::from_value::<StopReason>(spec["stopReason"].clone()).unwrap(),
        usage: Usage::default(),
        error_message: spec["errorMessage"].as_str().map(str::to_owned),
    }
}

#[tokio::test]
async fn session_events_match_pi() {
    let scenarios = fixture("scenarios.json");
    let golden = fixture("events.json");
    for (scenario, expected) in scenarios
        .as_array()
        .unwrap()
        .iter()
        .zip(golden.as_array().unwrap())
    {
        let name = scenario["name"].as_str().unwrap();
        let responses = scenario["responses"]
            .as_array()
            .unwrap()
            .iter()
            .map(response)
            .collect();
        let session = TestSession {
            settings: scenario["settings"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ..TestSession::default()
        }
        .build(FauxProvider::new(responses))
        .await;
        let events = session.subscribe();
        for prompt in scenario["prompts"].as_array().unwrap() {
            session
                .prompt(
                    prompt.as_str().unwrap().to_owned(),
                    PromptOptions::default(),
                )
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
                .await
                .unwrap();
        }
        let actual: Vec<JsonValue> = events
            .drain_now()
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .filter(|e| e["type"] != "tools_changed" && e["type"] != "model_changed")
            .map(|e| normalize_event(&e))
            .collect();
        let wanted: Vec<JsonValue> = expected["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(normalize_event)
            .collect();
        let actual_types: Vec<&JsonValue> = actual.iter().map(|e| &e["type"]).collect();
        let wanted_types: Vec<&JsonValue> = wanted.iter().map(|e| &e["type"]).collect();
        assert_eq!(actual_types, wanted_types, "{name}: event order");
        for (index, (a, w)) in actual.iter().zip(&wanted).enumerate() {
            assert!(
                json_semantic_eq(a, w),
                "{name}: event {index}\n rust: {a}\n   pi: {w}"
            );
        }
        let entries: Vec<JsonValue> = session
            .entries()
            .iter()
            .map(|e| normalize(&serde_json::to_value(e).unwrap()))
            .collect();
        let wanted_entries: Vec<JsonValue> = expected["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(normalize)
            .collect();
        assert!(
            json_semantic_eq(
                &JsonValue::Array(entries.clone()),
                &JsonValue::Array(wanted_entries.clone())
            ),
            "{name}: entries\n rust: {entries:?}\n   pi: {wanted_entries:?}"
        );
    }
}
