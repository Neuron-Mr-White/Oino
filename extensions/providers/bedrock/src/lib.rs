//! `oino.bedrock`: Amazon Bedrock `ConverseStream` (pi-ai 1.4.0 `amazon-bedrock` provider).
//! Bearer tokens (`AWS_BEARER_TOKEN_BEDROCK` or a stored key) are sent as-is; IAM credentials
//! come from `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN` or a static
//! `~/.aws` profile (read through the `aws` preopen) and are signed with `SigV4` by the kit.
//! Wire logic: `oino_provider_kit::bedrock`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK `Extension::init` is async even when registration is synchronous"
)]

use std::sync::OnceLock;

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{Provider, ProviderStream};
use oino_provider_kit::bedrock::{apply_profile, profile_credentials, profile_to_load};
use oino_provider_kit::catalog::Catalog;
use oino_provider_kit::guest::{self, StreamOptions};

const CATALOG: &str = include_str!("../catalog.json");

/// Component state.
#[derive(Default)]
struct Bedrock {
    catalog: OnceLock<Catalog>,
}

impl Bedrock {
    fn catalog(&self) -> &Catalog {
        self.catalog
            .get_or_init(|| Catalog::parse(CATALOG).unwrap_or_default())
    }
}

/// `~/.aws/<name>` through the `aws` preopen (`None` when missing or not granted).
fn read_aws_file(name: &str) -> Option<String> {
    std::fs::read_to_string(format!("/aws/{name}")).ok()
}

impl Provider for Bedrock {
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>> {
        Box::pin(async move {
            let mut request = guest::parse(&request)?;
            if let Some(profile) = profile_to_load(&request) {
                let loaded = profile_credentials(
                    read_aws_file("credentials").as_deref(),
                    read_aws_file("config").as_deref(),
                    &profile,
                );
                apply_profile(&mut request, &loaded);
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

impl Extension for Bedrock {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        guest::register_all(self.catalog())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(Bedrock);
