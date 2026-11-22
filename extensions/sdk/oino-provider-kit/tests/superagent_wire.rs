//! Superagent declarations and cross-provider tool histories at each wire serializer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use oino_provider_kit::{PrepareOptions, ProviderRequest, Wire, finalize, prepare};
use serde_json::{Value, json};
use std::collections::HashSet;

fn valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn body(wire: Wire, custom: bool) -> Value {
    // The host integration test proves this fixture is exactly the output of
    // request_json for the dotted historical transcript and real session tools.
    let normalized: Value =
        serde_json::from_str(include_str!("fixtures/superagent-normalized.json")).unwrap();
    let mut messages = normalized[wire.api()].clone();
    messages[0]["toolsAdded"].as_array_mut().unwrap().push(json!({"name":"tool_search","description":"Load deferred tools","parameters":{"type":"object","properties":{}}}));
    if custom {
        let tools = messages[0]["toolsAdded"].as_array_mut().unwrap();
        let relay = tools
            .iter_mut()
            .find(|tool| tool["name"] == "superagent_relay")
            .unwrap();
        relay["parameters"] = json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false});
        relay["constrainedSampling"] = json!({"type":"grammar","variants":{"openai_regex":".*"}});
        for message in messages.as_array_mut().unwrap() {
            if let Some(blocks) = message["content"].as_array_mut() {
                for block in blocks {
                    if block["type"] == "toolCall" && block["name"] == "superagent_relay" {
                        block["arguments"] = json!({"input":"legacy relay"});
                    }
                }
            }
        }
    }
    let request = ProviderRequest::parse(&json!({"model":{"id":"target","name":"target","api":wire.api(),"provider":"gateway","baseUrl":"https://example.com/v1","reasoning":false,"compat":{"supportsOpenAIGrammarTools":custom},"input":["text"],"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"contextWindow":200_000,"maxTokens":8192},"context":{"messages":messages},"options":{},"apiKey":"test"})).unwrap();
    let prepared = prepare(wire, &request, PrepareOptions::default(), 100).unwrap();
    let mut headers = prepared.http.headers.clone();
    finalize(wire, prepared.http.payload, &mut headers)
}

#[allow(
    clippy::too_many_lines,
    reason = "compare the three wire formats in one test"
)]
fn check(wire: Wire, custom: bool) {
    let body = body(wire, custom);
    let tools = body["tools"].as_array().unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/superagent-tools.json")).unwrap();
    assert_eq!(tools.len(), expected.as_array().unwrap().len() + 1);
    let actual_names: HashSet<&str> = tools
        .iter()
        .map(|tool| {
            if matches!(wire, Wire::Completions) {
                tool["function"]["name"].as_str().unwrap()
            } else {
                tool["name"].as_str().unwrap()
            }
        })
        .collect();
    for name in [
        "superagent_start",
        "superagent_stop",
        "superagent_relay",
        "superagent_set_mode",
        "project_resolver",
        "kanboard",
        "ask_user",
        "memory_search",
        "memory_list",
        "global_memory_search",
        "global_memory_list",
        "tool_search",
    ] {
        assert!(actual_names.contains(name), "missing {name}");
    }
    assert_eq!(actual_names.len(), tools.len());
    for tool in tools {
        let (name, parameters) = if matches!(wire, Wire::Completions) {
            (&tool["function"]["name"], &tool["function"]["parameters"])
        } else if matches!(wire, Wire::Anthropic) {
            (&tool["name"], &tool["input_schema"])
        } else {
            (&tool["name"], &tool["parameters"])
        };
        assert!(valid(name.as_str().unwrap()), "{tool}");
        if tool["type"] == "custom" {
            assert!(custom && matches!(wire, Wire::Responses));
            assert_eq!(name, "superagent_relay");
        } else {
            assert_eq!(parameters["type"], "object", "{tool}");
        }
    }
    if custom {
        assert!(tools.iter().any(|tool| tool["type"] == "custom"));
    }
    let mut calls = HashSet::new();
    let mut outputs = HashSet::new();
    let mut names = HashSet::new();
    match wire {
        Wire::Anthropic => {
            for msg in body["messages"].as_array().unwrap() {
                if let Some(blocks) = msg["content"].as_array() {
                    for block in blocks {
                        if block["type"] == "tool_use" {
                            assert!(valid(block["name"].as_str().unwrap()));
                            names.insert(block["name"].as_str().unwrap().to_owned());
                            let id = block["id"].as_str().unwrap();
                            assert!(valid(id), "invalid id {id}");
                            assert!(calls.insert(id.to_owned()));
                        } else if block["type"] == "tool_result" {
                            assert!(
                                outputs.insert(block["tool_use_id"].as_str().unwrap().to_owned())
                            );
                        }
                    }
                }
            }
        }
        Wire::Completions => {
            for msg in body["messages"].as_array().unwrap() {
                if let Some(items) = msg["tool_calls"].as_array() {
                    for call in items {
                        assert!(valid(call["function"]["name"].as_str().unwrap()));
                        names.insert(call["function"]["name"].as_str().unwrap().to_owned());
                        let id = call["id"].as_str().unwrap();
                        assert!(valid(id), "invalid id {id}");
                        assert!(calls.insert(id.to_owned()));
                    }
                }
                if msg["role"] == "tool" {
                    assert!(outputs.insert(msg["tool_call_id"].as_str().unwrap().to_owned()));
                }
            }
        }
        Wire::Responses => {
            for item in body["input"].as_array().unwrap() {
                if item["type"] == "function_call" || item["type"] == "custom_tool_call" {
                    assert!(valid(item["name"].as_str().unwrap()));
                    names.insert(item["name"].as_str().unwrap().to_owned());
                    let id = item["call_id"].as_str().unwrap();
                    assert!(valid(id), "invalid id {id}");
                    assert!(calls.insert(id.to_owned()));
                } else if item["type"] == "function_call_output"
                    || item["type"] == "custom_tool_call_output"
                {
                    assert!(outputs.insert(item["call_id"].as_str().unwrap().to_owned()));
                }
            }
        }
        _ => unreachable!(),
    }
    assert_eq!(
        names,
        [
            "superagent_start",
            "superagent_stop",
            "superagent_relay",
            "superagent_set_mode"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    assert_eq!(calls.len(), 4, "{body}");
    assert_eq!(calls, outputs, "{body}");
}
#[test]
fn anthropic_model_switch() {
    check(Wire::Anthropic, false);
}
#[test]
fn completions_model_switch() {
    check(Wire::Completions, false);
}
#[test]
fn responses_model_switch() {
    check(Wire::Responses, false);
}

#[test]
fn responses_custom_tool_history() {
    check(Wire::Responses, true);
}
