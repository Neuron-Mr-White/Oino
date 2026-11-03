//! [graph-mode] P0: tools added mid-conversation (the long-horizon tools after `/graph`,
//! `/goal`, `/ralph`, `/swarm`) must reach the model on every Anthropic Messages endpoint.
//!
//! Native inline tool changes (`tool_addition` blocks in a mid-conversation system message,
//! the inline-tools beta) are only sent to Anthropic's own API. A gateway that forwards the
//! Messages API keeps the system text and drops the blocks, so there the request-level
//! `tools` list must carry the current tool set instead.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

use oino_provider_kit::anthropic::is_first_party_endpoint;
use oino_provider_kit::{PrepareOptions, ProviderRequest, Wire, finalize, prepare};
use serde_json::{Value, json};

fn tool(name: &str) -> Value {
    json!({"name": name, "description": format!("{name} tool"),
        "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}})
}

fn request(base_url: &str) -> ProviderRequest {
    let assistant = json!({"role": "assistant", "api": "anthropic-messages", "provider": "gw",
        "model": "claude-opus-5-5", "content": [{"type": "text", "text": "ok"}], "stopReason": "stop",
        "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
        "timestamp": 2});
    ProviderRequest::parse(&json!({
        "model": {"id": "claude-opus-5-5", "name": "Opus", "api": "anthropic-messages", "provider": "gw",
            "baseUrl": base_url, "reasoning": false, "input": ["text"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 200_000, "maxTokens": 8192,
            "compat": {"supportsMidConvoSystemMessages": true, "supportsMidConvoToolChanges": true}},
        "context": {"messages": [
            {"role": "system", "content": "Sys", "toolsAdded": [tool("bash")], "timestamp": 0},
            {"role": "user", "content": "plan it", "timestamp": 1},
            assistant,
            {"role": "system", "content": "", "sections": {"long-horizon": "Mode: Graph"},
                "toolsAdded": [tool("update_agent_graph"), tool("view_agent_graph")], "timestamp": 3},
            {"role": "user", "content": "go", "timestamp": 4},
        ]},
        "options": {"sessionId": "s1"},
        "apiKey": "test-key",
    }))
    .unwrap()
}

/// The request body as sent, plus the `anthropic-beta` header.
fn body(base_url: &str) -> (Value, String) {
    let prepared = prepare(Wire::Anthropic, &request(base_url), PrepareOptions::default(), 0).unwrap();
    let mut headers = prepared.http.headers.clone();
    let body = finalize(Wire::Anthropic, prepared.http.payload.clone(), &mut headers);
    let beta = headers
        .pairs()
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("anthropic-beta"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    (body, beta)
}

fn request_tools(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
}

fn inline_additions(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["content"].as_array())
        .flatten()
        .filter(|b| b["type"] == "tool_addition")
        .filter_map(|b| b["tool"]["definition"]["name"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn gateway_declares_mid_conversation_tools_at_request_level() {
    let (body, beta) = body("https://llm-gateway.example.com/v1");
    let tools = request_tools(&body);
    for name in ["bash", "update_agent_graph", "view_agent_graph"] {
        assert!(tools.iter().any(|t| t == name), "{name} missing from request tools {tools:?}");
    }
    assert!(inline_additions(&body).is_empty(), "no inline tool_addition through a gateway");
    assert!(!beta.contains("inline-tools"), "no inline-tools beta through a gateway: {beta}");
    // The mode text itself still arrives (mid-conversation system text is kept).
    assert!(body.to_string().contains("Mode: Graph"));
}

#[test]
fn first_party_api_keeps_native_inline_tool_changes() {
    let (body, beta) = body("https://api.anthropic.com");
    assert_eq!(inline_additions(&body), ["update_agent_graph", "view_agent_graph"]);
    assert!(!request_tools(&body).iter().any(|t| t == "update_agent_graph"));
    assert!(beta.contains("inline-tools"), "{beta}");
}

#[test]
fn first_party_endpoint_detection() {
    for url in ["", "https://api.anthropic.com", "https://API.anthropic.com/v1/", "api.anthropic.com:443"] {
        assert!(is_first_party_endpoint(url), "{url}");
    }
    for url in ["https://gateway.example.com/v1", "http://localhost:8080", "https://api.anthropic.com.evil.dev", "https://x@proxy.example/api.anthropic.com"] {
        assert!(!is_first_party_endpoint(url), "{url}");
    }
}
