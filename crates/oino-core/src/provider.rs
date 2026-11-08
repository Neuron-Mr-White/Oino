//! Provider dispatch: the [`StreamFn`] that talks only to provider extensions.
//!
//! Core has no provider code (charter). A request for a model goes to the provider component
//! whose manifest `provides.apis` contains the model's `api`; its [`ProviderEvent`]s are
//! assembled into `AssistantMessageEvent`s with live partials. Every stream ends with
//! `done` or `error`.

use std::sync::Arc;

use futures::StreamExt;
use futures::stream::{self, BoxStream};
use oino_agent::{StreamFn, StreamRequest, now_ms};
use oino_ai::{
    AssistantMessage, AssistantMessageEvent, ErrorReason, JsonValue, PartialAssembler,
    ProviderEvent, StopReason,
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

// Provider components may be prebuilt: normalize names before crossing the
// native/wasm boundary, without modifying the persisted transcript.
fn normalize_legacy_transcript(value: &mut JsonValue) {
    let completions = value["model"]["api"] == "openai-completions";
    if let Some(messages) = value["context"]["messages"].as_array_mut() {
        for message in messages {
            if message["role"] == "assistant" {
                if let Some(blocks) = message["content"].as_array_mut() {
                    for block in blocks {
                        if block["type"] == "toolCall" {
                            normalize_superagent_name(&mut block["name"]);
                            if completions {
                                normalize_completions_id(&mut block["id"]);
                            }
                        }
                    }
                }
            } else if message["role"] == "toolResult" {
                normalize_superagent_name(&mut message["toolName"]);
                if completions {
                    normalize_completions_id(&mut message["toolCallId"]);
                }
            }
        }
    }
}

fn normalize_completions_id(value: &mut JsonValue) {
    let Some(id) = value.as_str() else { return };
    if !id.is_empty()
        && id.len() <= 40
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return;
    }
    // Hash rather than replace characters: different source ids must not collide.
    let digest = format!("{:x}", Sha256::digest(id.as_bytes()));
    *value = json!(format!("call_{}", &digest[..35]));
}

fn normalize_superagent_name(value: &mut JsonValue) {
    let replacement = match value.as_str() {
        Some("superagent.start") => "superagent_start",
        Some("superagent.stop") => "superagent_stop",
        Some("superagent.relay") => "superagent_relay",
        Some("superagent.set_mode") => "superagent_set_mode",
        _ => return,
    };
    *value = json!(replacement);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(first.len() <= 40 && first.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
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
            assert_eq!(messages[call]["content"][0]["id"], messages[result]["toolCallId"]);
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
        let body = request_json(&request);
        let assembler = PartialAssembler::new(template);
        let state = (Phase::Connect(component, body), assembler, cancel);
        Box::pin(
            stream::unfold(state, |(phase, mut assembler, cancel)| async move {
                match phase {
                    Phase::Done => None,
                    Phase::Connect(component, body) => {
                        let Some(provider) = component.instance.provider() else {
                            let event = failure(
                                assembler.partial().clone(),
                                "Provider unavailable",
                                &cancel,
                            );
                            return Some((vec![event], (Phase::Done, assembler, cancel)));
                        };
                        match provider.stream(body, cancel.clone()).await {
                            Ok(events) => {
                                Some((Vec::new(), (Phase::Streaming(events), assembler, cancel)))
                            }
                            Err(error) => {
                                let event =
                                    failure(assembler.partial().clone(), &error.message, &cancel);
                                Some((vec![event], (Phase::Done, assembler, cancel)))
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
                                Some((vec![event], (Phase::Done, assembler, cancel)))
                            }
                            Some(Some(Ok(event))) => {
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
                                Some((vec![out], (phase, assembler, cancel)))
                            }
                            Some(Some(Err(error))) => {
                                let event =
                                    failure(assembler.partial().clone(), &error.message, &cancel);
                                Some((vec![event], (Phase::Done, assembler, cancel)))
                            }
                            Some(None) => {
                                let event = failure(
                                    assembler.partial().clone(),
                                    "Provider stream ended without a final event",
                                    &cancel,
                                );
                                Some((vec![event], (Phase::Done, assembler, cancel)))
                            }
                        }
                    }
                }
            })
            .flat_map(stream::iter),
        )
    }
}
