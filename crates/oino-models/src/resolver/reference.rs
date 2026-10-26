//! pi `resolveModelReference`: one model from `id`, `provider/id`, fuzzy text and an optional
//! `:<thinking>` suffix. Candidates are collected before a provider is chosen, so catalog order
//! never decides the provider and ambiguity is reported.

use std::cmp::Ordering;

use oino_ai::{Model, ThinkingLevel};

use crate::collate::locale_compare;

/// Handling of a trailing `:<suffix>` (only considered when the full text matches nothing,
/// since ids such as `qwen/qwen3-coder:exacto` contain colons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingSuffix {
    /// The suffix stays part of the id.
    None,
    /// A valid level is parsed; any other suffix stays part of the id.
    Strict,
    /// Like `Strict`, but an invalid suffix is dropped with a warning.
    Lenient,
}

/// Input of [`resolve_model_reference`].
#[derive(Clone, Copy, Debug)]
pub struct ModelReferenceRequest<'a> {
    /// Bare id or `provider/id`, optionally followed by `:<thinking>`.
    pub text: &'a str,
    /// Only consider this provider (case-insensitive).
    pub provider: Option<&'a str>,
    /// Suffix handling.
    pub thinking_suffix: ThinkingSuffix,
    /// Match id/name substrings when nothing matches exactly.
    pub fuzzy: bool,
}

/// Where a custom model id would go when nothing matched.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelReferenceTarget<'m> {
    /// Provider id.
    pub provider: String,
    /// The provider's models, anchor first (never empty).
    pub provider_models: Vec<&'m Model>,
    /// Text after the provider prefix, including any `:<suffix>`.
    pub model_text: String,
    /// `model_text` without a valid thinking suffix.
    pub model_id: String,
    /// Parsed suffix level.
    pub thinking_level: Option<ThinkingLevel>,
}

/// Result of [`resolve_model_reference`].
#[derive(Clone, Debug, PartialEq)]
pub enum ModelReferenceResult<'m> {
    /// One model.
    Resolved {
        /// The model.
        model: &'m Model,
        /// Level from a `:<thinking>` suffix.
        thinking_level: Option<ThinkingLevel>,
        /// Invalid-suffix warning (lenient mode).
        warning: Option<String>,
    },
    /// Several providers match equally.
    Ambiguous {
        /// Matching models.
        candidates: Vec<&'m Model>,
        /// How many of them are usable.
        usable_count: usize,
        /// Level from a suffix.
        thinking_level: Option<ThinkingLevel>,
        /// Invalid-suffix warning.
        warning: Option<String>,
    },
    /// Nothing matched.
    NotFound {
        /// Set when a provider is explicit or named by a `provider/` prefix.
        target: Option<ModelReferenceTarget<'m>>,
    },
}

impl ModelReferenceResult<'_> {
    fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }

    fn with_suffix(self, level: Option<ThinkingLevel>, new_warning: Option<String>) -> Self {
        match self {
            Self::Resolved { model, warning, .. } => Self::Resolved {
                model,
                thinking_level: level,
                warning: new_warning.or(warning),
            },
            Self::Ambiguous {
                candidates,
                usable_count,
                warning,
                ..
            } => Self::Ambiguous {
                candidates,
                usable_count,
                thinking_level: level,
                warning: new_warning.or(warning),
            },
            not_found @ Self::NotFound { .. } => not_found,
        }
    }

    fn warning(&self) -> Option<&str> {
        match self {
            Self::Resolved { warning, .. } | Self::Ambiguous { warning, .. } => warning.as_deref(),
            Self::NotFound { .. } => None,
        }
    }
}

/// pi `splitThinkingSuffix`.
pub(crate) fn split_thinking_suffix(text: &str) -> Option<(&str, ThinkingLevel)> {
    let colon = text.rfind(':')?;
    let level = text[colon + 1..].parse::<ThinkingLevel>().ok()?;
    Some((&text[..colon], level))
}

/// pi `splitProviderPrefix`: `(provider, model_text)` when the prefix names a known provider.
fn split_provider_prefix<'t>(text: &'t str, models: &[&Model]) -> Option<(String, &'t str)> {
    let slash = text.find('/')?;
    let prefix = text[..slash].trim().to_lowercase();
    let model_text = text[slash + 1..].trim();
    if prefix.is_empty() || model_text.is_empty() {
        return None;
    }
    let provider = models
        .iter()
        .find(|m| m.provider.to_lowercase() == prefix)?
        .provider
        .clone();
    Some((provider, model_text))
}

fn provider_count(models: &[&Model]) -> usize {
    let mut providers: Vec<&str> = models.iter().map(|m| m.provider.as_str()).collect();
    providers.sort_unstable();
    providers.dedup();
    providers.len()
}

/// pi `resolveModelReference`. `usable` only breaks ties between providers, so a unique match
/// resolves without credentials (`--api-key` first-time setup).
pub fn resolve_model_reference<'m>(
    request: &ModelReferenceRequest<'_>,
    models: &'m [Model],
    usable: &dyn Fn(&str) -> bool,
) -> ModelReferenceResult<'m> {
    let text = request.text.trim();
    if text.is_empty() {
        return ModelReferenceResult::NotFound { target: None };
    }
    let constraint = request.provider.map(str::to_lowercase);
    let pool: Vec<&'m Model> = models
        .iter()
        .filter(|m| {
            constraint
                .as_ref()
                .is_none_or(|c| m.provider.to_lowercase() == *c)
        })
        .collect();
    let result = resolve_text(text, request, &pool, usable);
    if !result.is_not_found() {
        return result;
    }
    let split = split_provider_prefix(text, &pool);
    let anchor = if constraint.is_some() {
        pool.first().copied()
    } else {
        split
            .as_ref()
            .and_then(|(provider, _)| pool.iter().find(|m| &m.provider == provider).copied())
    };
    let Some(anchor) = anchor else {
        return result;
    };
    let model_text = split.as_ref().map_or(text, |(_, rest)| rest);
    let suffix = match request.thinking_suffix {
        ThinkingSuffix::None => None,
        _ => split_thinking_suffix(model_text),
    };
    let mut provider_models = vec![anchor];
    provider_models.extend(
        pool.iter()
            .filter(|m| m.provider == anchor.provider && !std::ptr::eq(**m, anchor))
            .copied(),
    );
    ModelReferenceResult::NotFound {
        target: Some(ModelReferenceTarget {
            provider: anchor.provider.clone(),
            provider_models,
            model_text: model_text.to_owned(),
            model_id: suffix.map_or(model_text, |(base, _)| base).to_owned(),
            thinking_level: suffix.map(|(_, level)| level),
        }),
    }
}

/// pi `findExactModelReferenceMatch`: exact `provider/id` or bare id, no suffix or fuzzy.
#[must_use]
pub fn find_exact_model_reference_match<'m>(
    reference: &str,
    models: &'m [Model],
) -> Option<&'m Model> {
    match resolve_model_reference(
        &ModelReferenceRequest {
            text: reference,
            provider: None,
            thinking_suffix: ThinkingSuffix::None,
            fuzzy: false,
        },
        models,
        &|_| true,
    ) {
        ModelReferenceResult::Resolved { model, .. } => Some(model),
        _ => None,
    }
}

fn resolve_text<'m>(
    text: &str,
    request: &ModelReferenceRequest<'_>,
    pool: &[&'m Model],
    usable: &dyn Fn(&str) -> bool,
) -> ModelReferenceResult<'m> {
    if let Some(reference) = select_reference(text, pool, request.fuzzy, usable) {
        return reference;
    }
    if request.fuzzy
        && let Some(fuzzy) = select_fuzzy(&fuzzy_matches(text, pool), usable)
    {
        return fuzzy;
    }
    if request.thinking_suffix == ThinkingSuffix::None {
        return ModelReferenceResult::NotFound { target: None };
    }
    if let Some((base, level)) = split_thinking_suffix(text) {
        let inner = resolve_text(base, request, pool, usable);
        if inner.is_not_found() {
            return inner;
        }
        // Only use this thinking level if the inner recursion produced no warning.
        let level = if inner.warning().is_some() {
            None
        } else {
            Some(level)
        };
        return inner.with_suffix(level, None);
    }
    if request.thinking_suffix == ThinkingSuffix::Strict {
        return ModelReferenceResult::NotFound { target: None };
    }
    let Some(colon) = text.rfind(':') else {
        return ModelReferenceResult::NotFound { target: None };
    };
    let suffix = &text[colon + 1..];
    let inner = resolve_text(&text[..colon], request, pool, usable);
    if inner.is_not_found() {
        return inner;
    }
    inner.with_suffix(
        None,
        Some(format!(
            "Invalid thinking level \"{suffix}\" in pattern \"{text}\". Using default instead."
        )),
    )
}

fn resolved(model: &Model) -> ModelReferenceResult<'_> {
    ModelReferenceResult::Resolved {
        model,
        thinking_level: None,
        warning: None,
    }
}

/// pi `selectReference`: exact ids, with `provider/rest` as a hint.
fn select_reference<'m>(
    text: &str,
    pool: &[&'m Model],
    fuzzy: bool,
    usable: &dyn Fn(&str) -> bool,
) -> Option<ModelReferenceResult<'m>> {
    let lower = text.to_lowercase();
    let raw: Vec<&'m Model> = pool
        .iter()
        .filter(|m| m.id.to_lowercase() == lower)
        .copied()
        .collect();
    let named: Option<&'m Model> =
        split_provider_prefix(text, pool).and_then(|(provider, rest)| {
            let provider_models: Vec<&'m Model> = pool
                .iter()
                .filter(|m| m.provider == provider)
                .copied()
                .collect();
            let lower_rest = rest.to_lowercase();
            provider_models
                .iter()
                .find(|m| m.id.to_lowercase() == lower_rest)
                .copied()
                .or_else(|| raw.iter().find(|m| m.provider == provider).copied())
                .or_else(|| {
                    fuzzy
                        .then(|| best_fuzzy_match(&fuzzy_matches(rest, &provider_models)))
                        .flatten()
                })
        });
    let others: Vec<&'m Model> = raw
        .iter()
        .filter(|m| named.is_none_or(|n| !std::ptr::eq(**m, n)))
        .copied()
        .collect();
    let mut candidates: Vec<&'m Model> = named.into_iter().collect();
    candidates.extend(others.iter().copied());
    let first = *candidates.first()?;
    if provider_count(&candidates) == 1 {
        return Some(resolved(first));
    }
    if let Some(named) = named
        && usable(&named.provider)
    {
        return Some(resolved(named));
    }
    let usable_others: Vec<&'m Model> = others
        .iter()
        .filter(|m| usable(&m.provider))
        .copied()
        .collect();
    if let [only] = usable_others.as_slice() {
        return Some(resolved(only));
    }
    if usable_others.is_empty()
        && let Some(named) = named
    {
        return Some(resolved(named));
    }
    Some(ModelReferenceResult::Ambiguous {
        candidates,
        usable_count: usable_others.len(),
        thinking_level: None,
        warning: None,
    })
}

fn fuzzy_matches<'m>(text: &str, models: &[&'m Model]) -> Vec<&'m Model> {
    let lower = text.to_lowercase();
    models
        .iter()
        .filter(|m| m.id.to_lowercase().contains(&lower) || m.name.to_lowercase().contains(&lower))
        .copied()
        .collect()
}

fn select_fuzzy<'m>(
    matches: &[&'m Model],
    usable: &dyn Fn(&str) -> bool,
) -> Option<ModelReferenceResult<'m>> {
    if matches.is_empty() {
        return None;
    }
    let usable_matches: Vec<&'m Model> = if provider_count(matches) > 1 {
        matches
            .iter()
            .filter(|m| usable(&m.provider))
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    let pool: &[&'m Model] = if usable_matches.is_empty() {
        matches
    } else {
        &usable_matches
    };
    let best = best_fuzzy_match(pool)?;
    // The same id on several providers would otherwise be decided by catalog order.
    let tied: Vec<&'m Model> = pool.iter().filter(|m| m.id == best.id).copied().collect();
    if provider_count(&tied) == 1 {
        return Some(resolved(best));
    }
    let usable_count = if usable_matches.is_empty() {
        0
    } else {
        tied.len()
    };
    Some(ModelReferenceResult::Ambiguous {
        candidates: tied,
        usable_count,
        thinking_level: None,
        warning: None,
    })
}

/// pi `isAlias`: `-latest` or no `-YYYYMMDD` date suffix.
fn is_alias(id: &str) -> bool {
    if id.ends_with("-latest") {
        return true;
    }
    let bytes = id.as_bytes();
    let dated = bytes.len() >= 9
        && bytes[bytes.len() - 9] == b'-'
        && bytes[bytes.len() - 8..].iter().all(u8::is_ascii_digit);
    !dated
}

/// pi `bestFuzzyMatch`: prefer aliases over dated versions, then the greatest id
/// (`localeCompare`, first on ties like a stable sort).
fn best_fuzzy_match<'m>(matches: &[&'m Model]) -> Option<&'m Model> {
    let aliases: Vec<&'m Model> = matches
        .iter()
        .filter(|m| is_alias(&m.id))
        .copied()
        .collect();
    let pool = if aliases.is_empty() {
        matches
    } else {
        &aliases
    };
    let mut best: Option<&'m Model> = None;
    for candidate in pool {
        if best.is_none_or(|b| locale_compare(&candidate.id, &b.id) == Ordering::Greater) {
            best = Some(candidate);
        }
    }
    best
}
