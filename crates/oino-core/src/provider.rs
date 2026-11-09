//! Provider dispatch: the [`StreamFn`] that talks only to provider extensions.
//!
//! Core has no provider code (charter). A request for a model goes to the provider component
//! whose manifest `provides.apis` contains the model's `api`; its [`ProviderEvent`]s are
//! assembled into `AssistantMessageEvent`s with live partials. Every stream ends with
//! `done` or `error`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::StreamExt;
use futures::stream::{self, BoxStream};
use oino_agent::{StreamFn, StreamRequest, now_ms};
use oino_ai::{
    AssistantContent, AssistantMessage, AssistantMessageEvent, ErrorReason, JsonValue,
    PartialAssembler, ProviderEvent, StopReason,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::extensions::api::ExtensionCallError;
use crate::extensions::runner::{ExtensionSet, LoadedExtension};

/// Error text when no component implements a model's API.
#[must_use]
pub fn no_provider_message(api: &str) -> String {
    format!("No provider extension implements API \"{api}\"")
}

/// [`StreamFn`] over the provider components of an [`ExtensionSet`].
#[derive(Clone, Debug)]
pub struct ProviderStream {
    extensions: Arc<ExtensionSet>,
}

impl ProviderStream {
    /// Dispatches to the providers in `extensions`.
    #[must_use]
    pub const fn new(extensions: Arc<ExtensionSet>) -> Self {
        Self { extensions }
    }

    fn component(&self, api: &str) -> Option<Arc<LoadedExtension>> {
        self.extensions
            .extensions()
            .iter()
            .find(|extension| {
                extension.instance.provider().is_some()
                    && extension.state().apis.iter().any(|a| a == api)
            })
            .cloned()
    }
}

/// The provider request JSON (`{model, context, options, apiKey?}`).
#[must_use]
pub fn request_json(request: &StreamRequest) -> JsonValue {
    let mut value = json!({
        "model": request.model,
        "context": request.context,
        "options": request.options,
    });
    if let Some(key) = &request.api_key {
        value["apiKey"] = json!(key);
    }
    normalize_legacy_transcript(&mut value);
    value
}

// All providers receive this same normalized transcript, including system tool patches.
fn normalize_legacy_transcript(value: &mut JsonValue) -> HashMap<String, String> {
    let completions = value["model"]["api"] == "openai-completions";
    let Some(messages) = value["context"]["messages"].as_array_mut() else {
        return HashMap::new();
    };
    let mut names = Vec::new();
    for message in messages.iter() {
        if let Some(tools) = message["toolsAdded"].as_array() {
            for tool in tools {
                if let Some(name) = tool["name"].as_str()
                    && !names.iter().any(|n| n == name)
                {
                    names.push(name.to_owned());
                }
            }
        }
    }
    for message in messages.iter() {
        let calls = message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "toolCall")
            .filter_map(|block| block["name"].as_str());
        let result = (message["role"] == "toolResult")
            .then(|| message["toolName"].as_str())
            .flatten();
        for name in calls.chain(result) {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
    }
    // Reserve valid names before assigning aliases, regardless of declaration order.
    names.sort_by_key(|name| (sanitize_name(name) != *name, name.clone()));
    let mut forward = HashMap::new();
    let mut reverse = HashMap::new();
    let mut used = HashSet::new();
    for name in &names {
        let canonical = canonical_name(name);
        if let Some(wire) = forward.get(canonical).cloned() {
            forward.insert(name.clone(), wire);
            continue;
        }
        let base = sanitize_name(canonical);
        let mut wire = base.clone();
        let mut suffix = 2_u64;
        while used.contains(&wire) {
            let tail = format!("_{suffix}");
            wire = format!("{}{}", &base[..base.len().min(128 - tail.len())], tail);
            suffix += 1;
        }
        used.insert(wire.clone());
        reverse.insert(wire.clone(), canonical.to_owned());
        forward.insert(name.clone(), wire);
    }
    // Historical calls may refer to tools no longer declared.
    let mut active: HashSet<String> = HashSet::new();
    for message in messages {
        if message["role"] == "system" {
            if let Some(removed) = message["toolsRemoved"].as_array_mut() {
                let mut emitted = HashSet::new();
                let original: Vec<String> = removed
                    .iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_owned))
                    .collect();
                for name in &original {
                    active.remove(name);
                }
                removed.retain_mut(|tool| {
                    let Some(name) = tool["name"].as_str() else {
                        return true;
                    };
                    let wire = mapped_name(name, &forward);
                    if active.iter().any(|n| mapped_name(n, &forward) == wire)
                        || !emitted.insert(wire.clone())
                    {
                        return false;
                    }
                    tool["name"] = json!(wire);
                    true
                });
            }
            if let Some(added) = message["toolsAdded"].as_array_mut() {
                let mut emitted = HashSet::new();
                added.retain_mut(|tool| {
                    let Some(name) = tool["name"].as_str() else {
                        return true;
                    };
                    active.insert(name.to_owned());
                    let wire = mapped_name(name, &forward);
                    if !emitted.insert(wire.clone()) {
                        return false;
                    }
                    tool["name"] = json!(wire);
                    true
                });
            }
        } else if message["role"] == "assistant" {
            if let Some(blocks) = message["content"].as_array_mut() {
                for block in blocks {
                    if block["type"] == "toolCall" {
                        map_json_name(&mut block["name"], &forward);
                        if completions {
                            normalize_completions_id(&mut block["id"]);
                        }
                    }
                }
            }
        } else if message["role"] == "toolResult" {
            map_json_name(&mut message["toolName"], &forward);
            if completions {
                normalize_completions_id(&mut message["toolCallId"]);
            }
        }
    }
    reverse
}

fn canonical_name(name: &str) -> &str {
    match name {
        "superagent.start" => "superagent_start",
        "superagent.stop" => "superagent_stop",
        "superagent.relay" => "superagent_relay",
        "superagent.set_mode" => "superagent_set_mode",
        _ => name,
    }
}

fn sanitize_name(name: &str) -> String {
    let result: String = name
        .chars()
        .take(128)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if result.is_empty() {
        "_".to_owned()
    } else {
        result
    }
}

fn mapped_name(name: &str, forward: &HashMap<String, String>) -> String {
    forward
        .get(name)
        .or_else(|| forward.get(canonical_name(name)))
        .cloned()
        .unwrap_or_else(|| sanitize_name(canonical_name(name)))
}

fn map_json_name(value: &mut JsonValue, forward: &HashMap<String, String>) {
    if let Some(name) = value.as_str() {
        *value = json!(mapped_name(name, forward));
    }
}

fn restore_event(event: &mut ProviderEvent, reverse: &HashMap<String, String>) {
    fn restore(name: &mut String, reverse: &HashMap<String, String>) {
        if let Some(original) = reverse.get(name) {
            name.clone_from(original);
        }
    }
    fn message(message: &mut AssistantMessage, reverse: &HashMap<String, String>) {
        for block in &mut message.content {
            if let AssistantContent::ToolCall(call) = block {
                restore(&mut call.name, reverse);
            }
        }
    }
    match event {
        ProviderEvent::ToolcallStart {
            tool_name: Some(name),
            ..
        } => restore(name, reverse),
        ProviderEvent::ToolcallEnd { tool_call, .. } => restore(&mut tool_call.name, reverse),
        ProviderEvent::Done { message: msg, .. } => message(msg, reverse),
        ProviderEvent::Error { error, .. } => message(error, reverse),
        _ => {}
    }
}

fn normalize_completions_id(value: &mut JsonValue) {
    let Some(id) = value.as_str() else { return };
    if !id.is_empty()
        && id.len() <= 40
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return;
    }
    // Hash rather than replace characters: different source ids must not collide.
    let digest = format!("{:x}", Sha256::digest(id.as_bytes()));
    *value = json!(format!("call_{}", &digest[..35]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_catalog_names_and_collision_round_trip() {
        let names = [
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
            "superagent.start",
            "a.b",
            "a_b",
            "",
            &"x".repeat(140),
        ];
        for api in [
            "openai-completions",
            "openai-responses",
            "anthropic-messages",
            "openai-compat",
        ] {
            let tools: Vec<_> = names
                .iter()
                .map(|name| json!({"name":name,"description":"tool","parameters":{}}))
                .collect();
            let mut request = json!({"model":{"api":api},"context":{"messages":[
                {"role":"system","toolsAdded":tools},
                {"role":"assistant","content":[{"type":"toolCall","id":"1","name":"superagent.start"},{"type":"toolCall","id":"2","name":"a.b"}]},
                {"role":"toolResult","toolCallId":"1","toolName":"superagent.start"},
                {"role":"toolResult","toolCallId":"2","toolName":"a.b"}
            ]}});
            let reverse = normalize_legacy_transcript(&mut request);
            let messages = &request["context"]["messages"];
            let tools = messages[0]["toolsAdded"].as_array().unwrap();
            let wire: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert_eq!(wire.len(), names.len() - 1);
            assert_eq!(wire.iter().collect::<HashSet<_>>().len(), wire.len());
            assert!(wire.iter().all(|n| {
                !n.is_empty()
                    && n.len() <= 128
                    && n.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            }));
            assert_eq!(messages[1]["content"][0]["name"], "superagent_start");
            assert_eq!(messages[1]["content"][0]["name"], messages[2]["toolName"]);
            assert_eq!(messages[1]["content"][1]["name"], messages[3]["toolName"]);
            let collision = messages[3]["toolName"].as_str().unwrap();
            assert_ne!(collision, "a_b");
            assert_eq!(reverse.get(collision).map(String::as_str), Some("a.b"));
            let mut event = ProviderEvent::ToolcallEnd {
                content_index: 0,
                tool_call: oino_ai::ToolCall {
                    name: collision.into(),
                    ..Default::default()
                },
            };
            restore_event(&mut event, &reverse);
            assert!(
                matches!(event, ProviderEvent::ToolcallEnd { tool_call, .. } if tool_call.name == "a.b")
            );
        }
    }

    #[test]
    fn removing_legacy_alias_does_not_remove_current_tool() {
        let mut request = json!({"context":{"messages":[
            {"role":"system","toolsAdded":[{"name":"superagent.start"},{"name":"superagent_start"}]},
            {"role":"system","toolsRemoved":[{"name":"superagent.start"}]}
        ]}});
        normalize_legacy_transcript(&mut request);
        assert_eq!(request["context"]["messages"][1]["toolsRemoved"], json!([]));
    }

    #[test]
    fn completions_ids_are_valid_distinct_and_paired() {
        let mut request = json!({"model":{"api":"openai-completions"},"context":{"messages":[
            {"role":"assistant","content":[{"type":"toolCall","id":"gemini/call:0","name":"superagent.start"},{"type":"toolCall","id":"gemini_call_0","name":"superagent.stop"}]},
            {"role":"toolResult","toolCallId":"gemini/call:0","toolName":"superagent.start"},
            {"role":"toolResult","toolCallId":"gemini_call_0","toolName":"superagent.stop"}
        ]}});
        normalize_legacy_transcript(&mut request);
        let messages = request["context"]["messages"].as_array().unwrap();
        let first = messages[0]["content"][0]["id"].as_str().unwrap();
        let second = messages[0]["content"][1]["id"].as_str().unwrap();
        assert_ne!(first, second);
        assert!(
            first.len() <= 40
                && first
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        );
        assert_eq!(first, messages[1]["toolCallId"]);
        assert_eq!(second, messages[2]["toolCallId"]);
    }

    #[test]
    fn legacy_tool_names_are_normalized_without_changing_pair_ids() {
        let mut request = json!({"context":{"messages":[
            {"role":"assistant","content":[{"type":"toolCall","id":"call_1","name":"superagent.start","arguments":{}}]},
            {"role":"toolResult","toolCallId":"call_1","toolName":"superagent.start"},
            {"role":"assistant","content":[{"type":"toolCall","id":"call_2","name":"superagent.set_mode","arguments":{}}]},
            {"role":"toolResult","toolCallId":"call_2","toolName":"superagent.set_mode"}
        ]}});
        normalize_legacy_transcript(&mut request);
        let messages = request["context"]["messages"].as_array().unwrap();
        for (call, result, name) in [(0, 1, "superagent_start"), (2, 3, "superagent_set_mode")] {
            assert_eq!(messages[call]["content"][0]["name"], name);
            assert_eq!(messages[result]["toolName"], name);
            assert_eq!(
                messages[call]["content"][0]["id"],
                messages[result]["toolCallId"]
            );
        }
    }
}

fn failure(
    mut message: AssistantMessage,
    error: &str,
    cancel: &CancellationToken,
) -> AssistantMessageEvent {
    let aborted = cancel.is_cancelled();
    message.stop_reason = if aborted {
        StopReason::Aborted
    } else {
        StopReason::Error
    };
    message.error_message = Some(error.to_owned());
    AssistantMessageEvent::Error {
        reason: if aborted {
            ErrorReason::Aborted
        } else {
            ErrorReason::Error
        },
        error: message,
    }
}

enum Phase {
    Connect(Arc<LoadedExtension>, JsonValue),
    Streaming(BoxStream<'static, Result<ProviderEvent, ExtensionCallError>>),
    Done,
}

impl StreamFn for ProviderStream {
    fn stream(&self, request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        let model = &request.model;
        let template = AssistantMessage::empty(&model.api, &model.provider, &model.id, now_ms());
        let cancel = request.cancel.clone();
        let Some(component) = self.component(&model.api) else {
            let event = failure(template, &no_provider_message(&model.api), &cancel);
            return Box::pin(stream::once(async move { event }));
        };
        let mut body =
            json!({"model": request.model, "context": request.context, "options": request.options});
        if let Some(key) = &request.api_key {
            body["apiKey"] = json!(key);
        }
        let reverse = normalize_legacy_transcript(&mut body);
        let assembler = PartialAssembler::new(template);
        let state = (Phase::Connect(component, body), assembler, cancel, reverse);
        Box::pin(
            stream::unfold(
                state,
                |(phase, mut assembler, cancel, reverse)| async move {
                    match phase {
                        Phase::Done => None,
                        Phase::Connect(component, body) => {
                            let Some(provider) = component.instance.provider() else {
                                let event = failure(
                                    assembler.partial().clone(),
                                    "Provider unavailable",
                                    &cancel,
                                );
                                return Some((
                                    vec![event],
                                    (Phase::Done, assembler, cancel, reverse),
                                ));
                            };
                            match provider.stream(body, cancel.clone()).await {
                                Ok(events) => Some((
                                    Vec::new(),
                                    (Phase::Streaming(events), assembler, cancel, reverse),
                                )),
                                Err(error) => {
                                    let event = failure(
                                        assembler.partial().clone(),
                                        &error.message,
                                        &cancel,
                                    );
                                    Some((vec![event], (Phase::Done, assembler, cancel, reverse)))
                                }
                            }
                        }
                        Phase::Streaming(mut events) => {
                            let next = tokio::select! {
                                biased;
                                () = cancel.cancelled() => None,
                                next = events.next() => Some(next),
                            };
                            match next {
                                None => {
                                    let event = failure(
                                        assembler.partial().clone(),
                                        "Request was aborted",
                                        &cancel,
                                    );
                                    Some((vec![event], (Phase::Done, assembler, cancel, reverse)))
                                }
                                Some(Some(Ok(mut event))) => {
                                    restore_event(&mut event, &reverse);
                                    let terminal = matches!(
                                        event,
                                        ProviderEvent::Done { .. } | ProviderEvent::Error { .. }
                                    );
                                    let out = assembler.apply(event);
                                    let phase = if terminal {
                                        Phase::Done
                                    } else {
                                        Phase::Streaming(events)
                                    };
                                    Some((vec![out], (phase, assembler, cancel, reverse)))
                                }
                                Some(Some(Err(error))) => {
                                    let event = failure(
                                        assembler.partial().clone(),
                                        &error.message,
                                        &cancel,
                                    );
                                    Some((vec![event], (Phase::Done, assembler, cancel, reverse)))
                                }
                                Some(None) => {
                                    let event = failure(
                                        assembler.partial().clone(),
                                        "Provider stream ended without a final event",
                                        &cancel,
                                    );
                                    Some((vec![event], (Phase::Done, assembler, cancel, reverse)))
                                }
                            }
                        }
                    }
                },
            )
            .flat_map(stream::iter),
        )
    }
}
