//! pi `resolveModelScopeFromModels`: `--models` / `enabledModels` patterns (globs, fuzzy
//! references, `:<thinking>` suffixes) to scoped models with diagnostics.

use oino_ai::{Model, ThinkingLevel};
use serde::Serialize;

use super::reference::{
    ModelReferenceRequest, ModelReferenceResult, ThinkingSuffix, resolve_model_reference,
    split_thinking_suffix,
};
use crate::collate::locale_compare;
use crate::glob::glob_match;

/// A model in the scope with an explicit level from its pattern (`model:high`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedModel {
    /// The model.
    pub model: Model,
    /// Level given in the pattern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
}

/// Diagnostic kind (pi `ModelScopeDiagnostic.code`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScopeDiagnosticCode {
    /// Pattern matched nothing.
    NoMatch,
    /// Invalid `:<thinking>` suffix.
    InvalidThinkingLevel,
    /// Pattern matched several providers.
    Ambiguous,
}

/// A warning produced while resolving a scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScopeDiagnostic {
    /// Kind.
    pub code: ScopeDiagnosticCode,
    /// pi message (printed as `Warning: <message>`).
    pub message: String,
    /// The pattern.
    pub pattern: String,
}

/// Scope and its diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveModelScopeResult {
    /// Models in pattern order, deduplicated.
    pub scoped_models: Vec<ScopedModel>,
    /// Warnings.
    pub diagnostics: Vec<ScopeDiagnostic>,
}

impl ResolveModelScopeResult {
    fn add(&mut self, model: &Model, thinking_level: Option<ThinkingLevel>) {
        let present = self
            .scoped_models
            .iter()
            .any(|s| s.model.id == model.id && s.model.provider == model.provider);
        if !present {
            self.scoped_models.push(ScopedModel {
                model: model.clone(),
                thinking_level,
            });
        }
    }

    fn warn(&mut self, code: ScopeDiagnosticCode, message: String, pattern: &str) {
        self.diagnostics.push(ScopeDiagnostic {
            code,
            message,
            pattern: pattern.to_owned(),
        });
    }

    fn no_match(&mut self, pattern: &str) {
        self.warn(
            ScopeDiagnosticCode::NoMatch,
            format!("No models match pattern \"{pattern}\""),
            pattern,
        );
    }

    fn ambiguous(&mut self, pattern: &str, candidates: &[&Model]) {
        let mut providers: Vec<&str> = candidates.iter().map(|m| m.provider.as_str()).collect();
        providers.sort_by(|a, b| locale_compare(a, b));
        providers.dedup();
        self.warn(
            ScopeDiagnosticCode::Ambiguous,
            format!(
                "Pattern \"{pattern}\" matches several providers: {}. Use provider/id to choose one.",
                providers.join(", ")
            ),
            pattern,
        );
    }

    fn add_glob(&mut self, pattern: &str, models: &[Model]) {
        let (glob, level) = split_thinking_suffix(pattern)
            .map_or((pattern, None), |(base, level)| (base, Some(level)));
        // Ids such as `bracketed-model[1m]` are not globs.
        let exact = resolve_model_reference(
            &ModelReferenceRequest {
                text: glob,
                provider: None,
                thinking_suffix: ThinkingSuffix::None,
                fuzzy: false,
            },
            models,
            &|_| true,
        );
        let matches: Vec<&Model> = match exact {
            ModelReferenceResult::Resolved { model, .. } => vec![model],
            ModelReferenceResult::Ambiguous { candidates, .. } => {
                self.ambiguous(pattern, &candidates);
                candidates
            }
            ModelReferenceResult::NotFound { .. } => models
                .iter()
                .filter(|m| {
                    glob_match(glob, &format!("{}/{}", m.provider, m.id)) || glob_match(glob, &m.id)
                })
                .collect(),
        };
        if matches.is_empty() {
            self.no_match(pattern);
        }
        for model in matches {
            self.add(model, level);
        }
    }

    fn add_reference(&mut self, pattern: &str, models: &[Model]) {
        let reference = resolve_model_reference(
            &ModelReferenceRequest {
                text: pattern,
                provider: None,
                thinking_suffix: ThinkingSuffix::Lenient,
                fuzzy: true,
            },
            models,
            &|_| true,
        );
        let (matches, thinking_level, warning, ambiguous) = match reference {
            ModelReferenceResult::NotFound { .. } => return self.no_match(pattern),
            ModelReferenceResult::Resolved {
                model,
                thinking_level,
                warning,
            } => (vec![model], thinking_level, warning, false),
            ModelReferenceResult::Ambiguous {
                candidates,
                thinking_level,
                warning,
                ..
            } => (candidates, thinking_level, warning, true),
        };
        if let Some(warning) = warning {
            self.warn(ScopeDiagnosticCode::InvalidThinkingLevel, warning, pattern);
        }
        if ambiguous {
            self.ambiguous(pattern, &matches);
        }
        for model in matches {
            self.add(model, thinking_level);
        }
    }
}

/// Resolves scope patterns against `models` (pi passes the *available* models). Every model is
/// usable, so a pattern matching several providers adds all of them with a warning.
#[must_use]
pub fn resolve_model_scope(patterns: &[String], models: &[Model]) -> ResolveModelScopeResult {
    let mut result = ResolveModelScopeResult::default();
    for pattern in patterns {
        if pattern.contains(['*', '?', '[']) {
            result.add_glob(pattern, models);
        } else {
            result.add_reference(pattern, models);
        }
    }
    result
}
