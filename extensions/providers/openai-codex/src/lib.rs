//! `oino.openai-codex`: `OpenAI` Codex Responses over a `ChatGPT` Plus/Pro subscription (pi-ai
//! 1.4.0 `openai-codex` provider). Login and refresh follow pi's browser / device-code flows
//! ([`oauth`]); wire logic: `oino_provider_kit::codex`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK `Extension::init` is async even when registration is synchronous"
)]

mod oauth;

use std::sync::OnceLock;

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{Provider, ProviderStream};
use oino_provider_kit::catalog::Catalog;
use oino_provider_kit::guest::{self, StreamOptions};

const CATALOG: &str = include_str!("../catalog.json");
const PROVIDER: &str = "openai-codex";

/// Component state.
#[derive(Default)]
struct OpenAiCodex {
    catalog: OnceLock<Catalog>,
}

impl OpenAiCodex {
    fn catalog(&self) -> &Catalog {
        self.catalog
            .get_or_init(|| Catalog::parse(CATALOG).unwrap_or_default())
    }
}

fn unsupported(provider_id: &str) -> Error {
    Error::msg(format!("OAuth is not supported for {provider_id}"))
}

impl Provider for OpenAiCodex {
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>> {
        Box::pin(async move { guest::stream(&request, StreamOptions::default()) })
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
            if provider_id != PROVIDER {
                return Err(unsupported(&provider_id));
            }
            oauth::login().await
        })
    }

    fn oauth_refresh(
        &'static self,
        provider_id: String,
        credential: Value,
    ) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async move {
            if provider_id != PROVIDER {
                return Err(unsupported(&provider_id));
            }
            oauth::refresh(&credential).await
        })
    }
}

impl Extension for OpenAiCodex {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        guest::register_all(self.catalog())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(OpenAiCodex);
