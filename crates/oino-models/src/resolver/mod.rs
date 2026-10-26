//! pi `model-resolver.ts` and model cycling: `--model`, `--models` / `enabledModels`, initial
//! selection, session restore.
//!
//! Pure functions take model slices; functions that need credentials take a
//! [`ModelCatalog`] (implemented by [`crate::ModelRegistry`]).

mod cli;
mod cycle;
mod defaults;
mod initial;
mod reference;
mod scope;

use oino_ai::Model;

pub use self::cli::{CliModel, CliModelError, CliModelRequest, resolve_cli_model};
pub use self::cycle::{
    CycleDirection, ThinkingSwitch, next_available_model, next_scoped_model,
    thinking_level_for_model_switch,
};
pub use self::defaults::{
    DEFAULT_MODEL_PER_PROVIDER, DEFAULT_THINKING_LEVEL, default_model_for_provider,
};
pub use self::initial::{
    InitialModel, InitialModelOptions, RestoreNotice, RestoredModel, find_initial_model,
    model_thinking_level_key, restore_model_from_session,
};
pub use self::reference::{
    ModelReferenceRequest, ModelReferenceResult, ModelReferenceTarget, ThinkingSuffix,
    find_exact_model_reference_match, resolve_model_reference,
};
pub use self::scope::{
    ResolveModelScopeResult, ScopeDiagnostic, ScopeDiagnosticCode, ScopedModel, resolve_model_scope,
};

/// The catalog view the resolver needs.
pub trait ModelCatalog {
    /// Every model.
    fn all(&self) -> Vec<Model>;
    /// Models with usable credentials.
    fn available(&self) -> Vec<Model>;
    /// A model by provider and id.
    fn find(&self, provider: &str, id: &str) -> Option<Model>;
    /// Whether the provider has usable credentials.
    fn has_configured_auth(&self, provider: &str) -> bool;
}

impl ModelCatalog for crate::ModelRegistry {
    fn all(&self) -> Vec<Model> {
        Self::all(self)
    }

    fn available(&self) -> Vec<Model> {
        Self::available(self)
    }

    fn find(&self, provider: &str, id: &str) -> Option<Model> {
        Self::find(self, provider, id)
    }

    fn has_configured_auth(&self, provider: &str) -> bool {
        Self::has_configured_auth(self, provider)
    }
}

/// [`resolve_cli_model`] against a catalog (all models, credentials as tie-breaker).
///
/// # Errors
///
/// See [`resolve_cli_model`].
pub fn resolve_cli_model_in(
    request: &CliModelRequest<'_>,
    catalog: &dyn ModelCatalog,
) -> Result<Option<CliModel>, CliModelError> {
    resolve_cli_model(request, &catalog.all(), &|provider| {
        catalog.has_configured_auth(provider)
    })
}

/// pi `resolveModelScopeWithDiagnostics`: scope patterns against the available models.
#[must_use]
pub fn resolve_model_scope_in(
    patterns: &[String],
    catalog: &dyn ModelCatalog,
) -> ResolveModelScopeResult {
    resolve_model_scope(patterns, &catalog.available())
}

#[cfg(test)]
mod tests;
