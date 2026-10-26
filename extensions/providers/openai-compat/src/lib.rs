//! `oino.openai-compat`: every `OpenAI`-compatible chat-completions provider of the bundled
//! pi-ai 1.4.0 catalog (`OpenRouter`, Groq, `DeepSeek`, Together, Fireworks, Cerebras, …) plus local
//! servers (Ollama, LM Studio). Wire logic: `oino_provider_kit::completions`; models of these
//! providers using other APIs (e.g. `OpenRouter`'s Claude models on `anthropic-messages`) are
//! listed here and streamed by the component implementing that API.
//!
//! Also registers GitHub Copilot (device-code OAuth, [`copilot`]) and Cloudflare AI Gateway /
//! Workers AI; their models on other APIs are streamed by the components implementing those
//! APIs, with the provider adaptations in `oino_provider_kit::providers`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK `Extension::init` is async even when registration is synchronous"
)]

mod copilot;

use std::sync::OnceLock;

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{Provider, ProviderStream};
use oino_provider_kit::catalog::Catalog;
use oino_provider_kit::guest::{self, StreamOptions};

const CATALOG: &str = include_str!("../catalog.json");

/// Component state.
#[derive(Default)]
struct OpenAiCompat {
    catalog: OnceLock<Catalog>,
}

impl OpenAiCompat {
    fn catalog(&self) -> &Catalog {
        self.catalog
            .get_or_init(|| Catalog::parse(CATALOG).unwrap_or_default())
    }
}

impl Provider for OpenAiCompat {
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>> {
        Box::pin(async move {
            let local = request["model"]["provider"]
                .as_str()
                .and_then(|id| self.catalog().provider(id))
                .is_some_and(|p| p.local);
            guest::stream(
                &request,
                StreamOptions {
                    wire: None,
                    allow_missing_key: local,
                },
            )
        })
    }

    fn list_models(
        &'static self,
        provider_id: String,
        credential: Option<Value>,
    ) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async move {
            guest::list_models(self.catalog(), &provider_id, credential.as_ref()).await
        })
    }

    fn oauth_login(&'static self, provider_id: String) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async move {
            if provider_id != "github-copilot" {
                return Err(Error::msg(format!(
                    "OAuth is not supported for {provider_id}"
                )));
            }
            copilot::login(self.catalog()).await
        })
    }

    fn oauth_refresh(
        &'static self,
        provider_id: String,
        credential: Value,
    ) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async move {
            if provider_id != "github-copilot" {
                return Err(Error::msg(format!(
                    "OAuth is not supported for {provider_id}"
                )));
            }
            copilot::refresh(self.catalog(), &credential).await
        })
    }
}

impl Extension for OpenAiCompat {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        guest::register_all(self.catalog())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(OpenAiCompat);
