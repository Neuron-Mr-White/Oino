//! pi `findInitialModel` and `restoreModelFromSession`.

use indexmap::IndexMap;
use oino_ai::{Model, ThinkingLevel};

use super::ModelCatalog;
use super::defaults::{DEFAULT_MODEL_PER_PROVIDER, DEFAULT_THINKING_LEVEL};
use super::scope::ScopedModel;

/// Settings and session inputs of [`find_initial_model`].
#[derive(Clone, Copy, Debug, Default)]
pub struct InitialModelOptions<'a> {
    /// Resolved `--models` / `enabledModels` scope.
    pub scoped_models: &'a [ScopedModel],
    /// Continuing or resuming a session (the scope does not choose).
    pub is_continuing: bool,
    /// Settings `defaultProvider`.
    pub default_provider: Option<&'a str>,
    /// Settings `defaultModel`.
    pub default_model_id: Option<&'a str>,
    /// Settings `defaultThinkingLevel`.
    pub default_thinking_level: Option<ThinkingLevel>,
    /// Settings `modelThinkingLevels` (`"provider/id"` → level).
    pub model_thinking_levels: Option<&'a IndexMap<String, ThinkingLevel>>,
}

/// Initial selection.
#[derive(Clone, Debug, PartialEq)]
pub struct InitialModel {
    /// The model, if any is usable.
    pub model: Option<Model>,
    /// Thinking level to start with.
    pub thinking_level: ThinkingLevel,
    /// Always `None` here (kept for pi shape parity).
    pub fallback_message: Option<String>,
}

/// `"provider/id"` key used by `modelThinkingLevels`.
#[must_use]
pub fn model_thinking_level_key(provider: &str, id: &str) -> String {
    format!("{provider}/{id}")
}

fn preferred_available(available: &[Model]) -> Option<&Model> {
    DEFAULT_MODEL_PER_PROVIDER
        .iter()
        .find_map(|(provider, id)| {
            available
                .iter()
                .find(|m| m.provider == *provider && m.id == *id)
        })
        .or_else(|| available.first())
}

/// pi `findInitialModel`: (1) first scoped model unless continuing, (2) settings default when
/// its provider has auth, (3) a known provider default among available models, else the first
/// available. The CLI `--model` is applied by the caller before this.
#[must_use]
pub fn find_initial_model(
    options: &InitialModelOptions<'_>,
    catalog: &dyn ModelCatalog,
) -> InitialModel {
    let per_model = |provider: &str, id: &str| {
        options
            .model_thinking_levels
            .and_then(|levels| levels.get(&model_thinking_level_key(provider, id)).copied())
    };
    if let (Some(scoped), false) = (options.scoped_models.first(), options.is_continuing) {
        return InitialModel {
            thinking_level: scoped
                .thinking_level
                .or_else(|| per_model(&scoped.model.provider, &scoped.model.id))
                .or(options.default_thinking_level)
                .unwrap_or(DEFAULT_THINKING_LEVEL),
            model: Some(scoped.model.clone()),
            fallback_message: None,
        };
    }
    if let (Some(provider), Some(id)) = (options.default_provider, options.default_model_id)
        && let Some(found) = catalog.find(provider, id)
        && catalog.has_configured_auth(&found.provider)
    {
        let thinking_level = per_model(provider, id)
            .or(options.default_thinking_level)
            .unwrap_or(DEFAULT_THINKING_LEVEL);
        return InitialModel {
            model: Some(found),
            thinking_level,
            fallback_message: None,
        };
    }
    let available = catalog.available();
    InitialModel {
        model: preferred_available(&available).cloned(),
        thinking_level: DEFAULT_THINKING_LEVEL,
        fallback_message: None,
    }
}

/// A line pi prints while restoring (when messages are enabled).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreNotice {
    /// Dimmed info line.
    Info(String),
    /// Yellow warning line (stderr).
    Warning(String),
}

/// Result of [`restore_model_from_session`].
#[derive(Clone, Debug, PartialEq)]
pub struct RestoredModel {
    /// Model to use.
    pub model: Option<Model>,
    /// pi `modelFallbackMessage` when the stored model was not used.
    pub fallback_message: Option<String>,
    /// Lines pi prints in this order.
    pub notices: Vec<RestoreNotice>,
}

/// pi `restoreModelFromSession`.
#[must_use]
pub fn restore_model_from_session(
    saved_provider: &str,
    saved_model_id: &str,
    current_model: Option<&Model>,
    catalog: &dyn ModelCatalog,
) -> RestoredModel {
    let saved = format!("{saved_provider}/{saved_model_id}");
    let restored = catalog.find(saved_provider, saved_model_id);
    let has_auth = restored
        .as_ref()
        .is_some_and(|m| catalog.has_configured_auth(&m.provider));
    if let (Some(model), true) = (&restored, has_auth) {
        return RestoredModel {
            model: Some(model.clone()),
            fallback_message: None,
            notices: vec![RestoreNotice::Info(format!("Restored model: {saved}"))],
        };
    }
    let reason = if restored.is_none() {
        "model no longer exists"
    } else {
        "no auth configured"
    };
    let mut notices = vec![RestoreNotice::Warning(format!(
        "Warning: Could not restore model {saved} ({reason})."
    ))];
    let fallback = current_model.cloned().or_else(|| {
        let available = catalog.available();
        preferred_available(&available).cloned()
    });
    let Some(fallback) = fallback else {
        return RestoredModel {
            model: None,
            fallback_message: None,
            notices,
        };
    };
    let fallback_key = format!("{}/{}", fallback.provider, fallback.id);
    notices.push(RestoreNotice::Info(format!(
        "Falling back to: {fallback_key}"
    )));
    RestoredModel {
        fallback_message: Some(format!(
            "Could not restore model {saved} ({reason}). Using {fallback_key}."
        )),
        model: Some(fallback),
        notices,
    }
}
