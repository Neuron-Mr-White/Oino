//! `oino.google`: Google Generative AI (Gemini API key) and Google Vertex AI (pi-ai 1.4.0
//! `google` / `google-vertex` providers). Vertex uses an API key (`GOOGLE_CLOUD_API_KEY` or a
//! stored key) or Application Default Credentials with `GOOGLE_CLOUD_PROJECT` +
//! `GOOGLE_CLOUD_LOCATION`; ADC access tokens are minted by [`adc`]. Wire logic:
//! `oino_provider_kit::google`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK `Extension::init` is async even when registration is synchronous"
)]

mod adc;

use std::sync::OnceLock;

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{Provider, ProviderStream};
use oino_provider_kit::catalog::Catalog;
use oino_provider_kit::google::vertex_needs_adc;
use oino_provider_kit::guest::{self, StreamOptions};

const CATALOG: &str = include_str!("../catalog.json");

/// Component state.
#[derive(Default)]
struct Google {
    catalog: OnceLock<Catalog>,
}

impl Google {
    fn catalog(&self) -> &Catalog {
        self.catalog
            .get_or_init(|| Catalog::parse(CATALOG).unwrap_or_default())
    }
}

impl Provider for Google {
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>> {
        Box::pin(async move {
            let mut request = guest::parse(&request)?;
            if request.model.api == "google-vertex"
                && request.extra.vertex_access_token.is_none()
                && vertex_needs_adc(&request)
            {
                match adc::access_token(&request).await {
                    Ok(token) => request.extra.vertex_access_token = Some(token),
                    Err(message) => return Ok(guest::fail_stream(&request, message)),
                }
            }
            guest::stream_request(request, StreamOptions::default())
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
}

impl Extension for Google {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        guest::register_all(self.catalog())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(Google);
