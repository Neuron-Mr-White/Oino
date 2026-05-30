#![forbid(unsafe_code)]

use crate::extension_provider_runtime::extension_runtime_config_for_provider_id;
use async_trait::async_trait;
use oino_agent_loop::{AbortSignal, LoopError, StreamProvider};
use oino_auth::AuthStorage;
use oino_extension_core::ProviderContribution;
use oino_provider_openrouter::{OpenAiCompatibleProvider, OpenRouterConfig};
use oino_types::Model;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ProviderRouter {
    auth: AuthStorage,
    extension_providers: Vec<ProviderContribution>,
}

impl ProviderRouter {
    #[must_use]
    pub(crate) fn new(auth: AuthStorage, extension_providers: Vec<ProviderContribution>) -> Self {
        Self {
            auth,
            extension_providers,
        }
    }

    pub(crate) fn provider_for_model(
        &self,
        model: &Model,
    ) -> Result<Arc<dyn StreamProvider>, LoopError> {
        if let Some(config) =
            extension_runtime_config_for_provider_id(&model.provider, &self.extension_providers)?
        {
            let provider = OpenAiCompatibleProvider::new(self.auth.clone(), config)
                .map_err(|err| LoopError::Stream(err.to_string()))?;
            return Ok(Arc::new(provider) as Arc<dyn StreamProvider>);
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
    auth: AuthStorage,
    _openrouter_config: OpenRouterConfig,
    extension_providers: Vec<ProviderContribution>,
) -> Arc<dyn StreamProvider> {
    Arc::new(ProviderRouter::new(auth, extension_providers)) as Arc<dyn StreamProvider>
}

#[must_use]
pub(crate) fn extension_runtime_missing_message(provider_id: &str) -> String {
    format!(
        "Provider `{provider_id}` is not available through Oino core. Built-in provider runtime has been removed; run `/router setup` and select a `router:<model>` model, or install/enable an extension runtime provider for `{provider_id}`."
    )
}

pub(crate) fn provider_status_for_model_identifier(
    model_identifier: &str,
) -> Result<Model, String> {
    let Some(model) = Model::from_identifier(model_identifier) else {
        return Err(format!(
            "Invalid model identifier `{model_identifier}`; expected provider:model-id"
        ));
    };
    Err(extension_runtime_missing_message(&model.provider))
}
