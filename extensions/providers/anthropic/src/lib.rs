//! `oino.anthropic`: Anthropic Messages for the bundled pi-ai 1.4.0 catalog (Anthropic,
//! `MiniMax`) and for any model whose `api` is `anthropic-messages` (e.g. `OpenRouter`'s Claude
//! models registered by `oino.openai-compat`). Claude Pro/Max OAuth follows pi's flow
//! ([`oauth`]). Wire logic: `oino_provider_kit::anthropic`.

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

/// Component state.
#[derive(Default)]
struct Anthropic {
    catalog: OnceLock<Catalog>,
}

impl Anthropic {
    fn catalog(&self) -> &Catalog {
        self.catalog
            .get_or_init(|| Catalog::parse(CATALOG).unwrap_or_default())
    }
}

impl Provider for Anthropic {
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
            if provider_id != "anthropic" {
                return Err(Error::msg(format!(
                    "OAuth is not supported for {provider_id}"
                )));
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
            if provider_id != "anthropic" {
                return Err(Error::msg(format!(
                    "OAuth is not supported for {provider_id}"
                )));
            }
            oauth::refresh(&credential).await
        })
    }
}

impl Extension for Anthropic {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        guest::register_all(self.catalog())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(Anthropic);
