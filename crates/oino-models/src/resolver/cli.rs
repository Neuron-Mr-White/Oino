//! pi `resolveCliModel`: `--provider` / `--model` (with `--thinking`) to one model.

use oino_ai::{Model, ThinkingLevel};

use super::defaults::default_model_for_provider;
use super::reference::{
    ModelReferenceRequest, ModelReferenceResult, ModelReferenceTarget, ThinkingSuffix,
    resolve_model_reference,
};
use crate::collate::locale_compare;

/// CLI inputs.
#[derive(Clone, Copy, Debug, Default)]
pub struct CliModelRequest<'a> {
    /// `--provider`.
    pub provider: Option<&'a str>,
    /// `--model`.
    pub model: Option<&'a str>,
    /// `--thinking`.
    pub thinking: Option<ThinkingLevel>,
    /// `--api-key` was given for the selected provider.
    pub inline_api_key: bool,
}

/// A resolved CLI model.
#[derive(Clone, Debug, PartialEq)]
pub struct CliModel {
    /// The model (possibly a custom id built from a provider template).
    pub model: Model,
    /// Level parsed from a `:<thinking>` suffix (caller applies it).
    pub thinking_level: Option<ThinkingLevel>,
    /// Warning to print (custom model id, invalid suffix).
    pub warning: Option<String>,
}

/// CLI resolution failure; the message is pi's text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CliModelError(pub String);

/// pi `buildFallbackModel`: a custom id on the provider's default (or first) model.
fn build_fallback_model(provider_models: &[&Model], model_id: &str) -> Option<Model> {
    let first = *provider_models.first()?;
    let base = default_model_for_provider(&first.provider)
        .and_then(|id| provider_models.iter().find(|m| m.id == id).copied())
        .unwrap_or(first);
    let mut model = base.clone();
    model_id.clone_into(&mut model.id);
    model_id.clone_into(&mut model.name);
    Some(model)
}

/// Resolves `--model` against **all** models (so `--api-key` works for first-time setup);
/// `has_auth` breaks ties between providers. `Ok(None)` when no `--model` was given.
///
/// # Errors
///
/// [`CliModelError`] with pi's messages (empty catalog, unknown provider, ambiguous, not found).
pub fn resolve_cli_model(
    request: &CliModelRequest<'_>,
    models: &[Model],
    has_auth: &dyn Fn(&str) -> bool,
) -> Result<Option<CliModel>, CliModelError> {
    let Some(cli_model) = request.model else {
        return Ok(None);
    };
    if models.is_empty() {
        return Err(CliModelError(
            "No models available. Check your installation or add models to models.json.".to_owned(),
        ));
    }
    let provider = match request.provider {
        Some(cli_provider) => Some(
            models
                .iter()
                .find(|m| m.provider.to_lowercase() == cli_provider.to_lowercase())
                .map(|m| m.provider.clone())
                .ok_or_else(|| {
                    CliModelError(format!(
                        "Unknown provider \"{cli_provider}\". Use --list-models to see available providers/models."
                    ))
                })?,
        ),
        None => None,
    };
    let never = |_: &str| false;
    // An inline key goes to whichever provider is selected: stored credentials must not choose.
    let usable: &dyn Fn(&str) -> bool = if request.inline_api_key {
        &never
    } else {
        has_auth
    };
    let result = resolve_model_reference(
        &ModelReferenceRequest {
            text: cli_model,
            provider: provider.as_deref(),
            thinking_suffix: ThinkingSuffix::Strict,
            fuzzy: true,
        },
        models,
        usable,
    );
    match result {
        ModelReferenceResult::Resolved {
            model,
            thinking_level,
            warning,
        } => Ok(Some(CliModel {
            model: model.clone(),
            thinking_level,
            warning,
        })),
        ModelReferenceResult::Ambiguous {
            candidates,
            usable_count,
            ..
        } => Err(ambiguous_error(
            cli_model,
            &candidates,
            usable_count,
            request.inline_api_key,
        )),
        ModelReferenceResult::NotFound { target: None } => Err(CliModelError(format!(
            "Model \"{cli_model}\" not found. Use --list-models to see available models."
        ))),
        ModelReferenceResult::NotFound {
            target: Some(target),
        } => custom_model(cli_model, request.thinking, &target).map(Some),
    }
}

fn ambiguous_error(
    cli_model: &str,
    candidates: &[&Model],
    usable_count: usize,
    inline_api_key: bool,
) -> CliModelError {
    let mut matches: Vec<String> = candidates
        .iter()
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect();
    matches.sort_by(|a, b| locale_compare(a, b));
    let hint = if inline_api_key {
        "--api-key does not choose a provider."
    } else if usable_count == 0 {
        "No matching provider is authenticated."
    } else {
        "More than one matching provider is authenticated."
    };
    CliModelError(format!(
        "Model \"{cli_model}\" is ambiguous across providers: {}. {hint} Use --provider or provider/model.",
        matches.join(", ")
    ))
}

/// A custom model id on a known provider (pi: "Using custom model id").
fn custom_model(
    cli_model: &str,
    thinking: Option<ThinkingLevel>,
    target: &ModelReferenceTarget<'_>,
) -> Result<CliModel, CliModelError> {
    // A suffix level applies only without --thinking; otherwise it stays in the id.
    let model_id = if thinking.is_some() {
        &target.model_text
    } else {
        &target.model_id
    };
    let fallback_thinking = if thinking.is_some() {
        None
    } else {
        target.thinking_level
    };
    let mut model = build_fallback_model(&target.provider_models, model_id).ok_or_else(|| {
        CliModelError(format!(
            "Model \"{cli_model}\" not found. Use --list-models to see available models."
        ))
    })?;
    if thinking
        .or(fallback_thinking)
        .is_some_and(|level| level != ThinkingLevel::Off)
    {
        model.reasoning = true;
    }
    Ok(CliModel {
        model,
        thinking_level: fallback_thinking,
        warning: Some(format!(
            "Model \"{model_id}\" not found for provider \"{}\". Using custom model id.",
            target.provider
        )),
    })
}
