//! Model calls outside the session loop (permission `models.complete`, `credentials`).

use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::models as wm;
pub use crate::bindings::oino::extension::models::Credential;
use crate::error::parse;

/// `CompletionRequest{model?, context:{messages,...}, options}` → `AssistantMessage`.
pub async fn complete(request: &Value) -> Result<Value> {
    parse(&wm::complete(request.to_string(), None).await?)
}

/// Streaming completion: `AssistantMessageEvent` JSON values.
pub async fn stream(request: &Value) -> Result<impl Stream<Item = Value>> {
    let reader = wm::stream(request.to_string()).await?;
    Ok(reader
        .into_stream()
        .map(|item: String| serde_json::from_str(&item).unwrap_or(Value::String(item))))
}

/// Core's compaction summarizer → `{summary, usage?}`. `previous_summary` updates an earlier
/// summary instead of starting fresh (pi `generateSummary(…, previousSummary)`).
pub async fn summarize_default(
    messages: &Value,
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
) -> Result<Value> {
    parse(
        &wm::summarize_default(
            messages.to_string(),
            custom_instructions.map(ToOwned::to_owned),
            previous_summary.map(ToOwned::to_owned),
        )
        .await?,
    )
}

/// Scoped credential headers for a provider id or base URL.
pub async fn credential_for(provider_or_base_url: &str) -> Result<Option<Credential>> {
    Ok(wm::credential_for(provider_or_base_url.to_owned()).await?)
}

/// Classifier call.
pub async fn classify(request: &Value) -> Result<Value> {
    parse(&wm::classify(request.to_string()).await?)
}

/// Image generation.
pub async fn generate_image(request: &Value) -> Result<Value> {
    parse(&wm::generate_image(request.to_string()).await?)
}
