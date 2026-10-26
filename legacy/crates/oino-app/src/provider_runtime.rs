#![forbid(unsafe_code)]

use async_trait::async_trait;
use oino_agent_loop::{AbortSignal, LoopError, StreamProvider};
use oino_types::Model;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ProviderRouter {
    /// WASM component providers (declarative contributions and the
    /// `oino.oino_router` extension provider).
    wasm_providers: std::collections::BTreeMap<String, Arc<dyn StreamProvider>>,
}

impl ProviderRouter {
    #[must_use]
    pub(crate) fn new(
        wasm_providers: std::collections::BTreeMap<String, Arc<dyn StreamProvider>>,
    ) -> Self {
        Self { wasm_providers }
    }

    pub(crate) fn provider_for_model(
        &self,
        model: &Model,
    ) -> Result<Arc<dyn StreamProvider>, LoopError> {
        if let Some(provider) = self.wasm_providers.get(&model.provider) {
            return Ok(Arc::clone(provider));
        }
        Err(LoopError::Stream(extension_runtime_missing_message(
            &model.provider,
        )))
    }
}

#[async_trait]
impl StreamProvider for ProviderRouter {
    async fn stream(
        &self,
        request: oino_agent_loop::StreamRequest,
        signal: AbortSignal,
    ) -> oino_agent_loop::LoopResult<Vec<oino_types::AssistantStreamEvent>> {
        self.provider_for_model(&request.model)?
            .stream(request, signal)
            .await
    }

    async fn stream_events(
        &self,
        request: oino_agent_loop::StreamRequest,
        signal: AbortSignal,
        sink: oino_agent_loop::StreamEventSink,
    ) -> oino_agent_loop::LoopResult<()> {
        self.provider_for_model(&request.model)?
            .stream_events(request, signal, sink)
            .await
    }
}

pub(crate) fn build_runtime_provider(
    wasm_providers: std::collections::BTreeMap<String, Arc<dyn StreamProvider>>,
) -> Arc<dyn StreamProvider> {
    Arc::new(ProviderRouter::new(wasm_providers)) as Arc<dyn StreamProvider>
}

#[must_use]
pub(crate) fn extension_runtime_missing_message(provider_id: &str) -> String {
    format!(
        "Provider `{provider_id}` is not available through Oino core. Built-in provider runtime has been removed; run `/router setup` and select a `router:<model>` model, or install/enable an extension runtime provider for `{provider_id}`."
    )
}

pub(crate) fn provider_status_for_model_identifier(model_identifier: &str) -> String {
    let Some(model) = Model::from_identifier(model_identifier) else {
        return format!(
            "Invalid model identifier `{model_identifier}`; expected provider:model-id"
        );
    };
    extension_runtime_missing_message(&model.provider)
}
