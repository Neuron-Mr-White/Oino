//! Provider dispatch: the [`StreamFn`] that talks only to provider extensions.
//!
//! Core has no provider code (charter). A request for a model goes to the provider component
//! whose manifest `provides.apis` contains the model's `api`; its [`ProviderEvent`]s are
//! assembled into pi `AssistantMessageEvent`s with live partials. Every stream ends with
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
    value
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
