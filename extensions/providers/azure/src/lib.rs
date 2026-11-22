//! `oino.azure`: Azure `OpenAI` Responses (pi-ai 1.4.0 `azure` provider). The endpoint comes from
//! `AZURE_OPENAI_BASE_URL` / `AZURE_OPENAI_RESOURCE_NAME` (or the `azureBaseUrl` /
//! `azureResourceName` options), deployments from `AZURE_OPENAI_DEPLOYMENT_NAME_MAP`. Azure's
//! chat-completions models are streamed by `oino.openai-compat` with the same configuration.
//! Wire logic: `oino_provider_kit::azure`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK `Extension::init` is async even when registration is synchronous"
)]

use std::sync::OnceLock;

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{Provider, ProviderStream};
use oino_provider_kit::catalog::Catalog;
use oino_provider_kit::guest::{self, StreamOptions};

const CATALOG: &str = include_str!("../catalog.json");

/// Component state.
#[derive(Default)]
struct Azure {
    catalog: OnceLock<Catalog>,
}

impl Azure {
    fn catalog(&self) -> &Catalog {
        self.catalog
            .get_or_init(|| Catalog::parse(CATALOG).unwrap_or_default())
    }
}

impl Provider for Azure {
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
}

impl Extension for Azure {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        guest::register_all(self.catalog())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(Azure);
