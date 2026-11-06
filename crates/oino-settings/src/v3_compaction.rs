//! v3 compaction input-token threshold resolution.
use crate::Settings;

/// Resolve session → exact model → longest family prefix → global → model → 250k,
/// clamped to the model's maximum input. Only positive integer settings apply.
#[must_use]
pub fn token_limit(
    settings: &Settings,
    model: &str,
    session: Option<u64>,
    model_default: Option<u64>,
    max_input: u64,
) -> u64 {
    let positive = |value: &serde_json::Value| value.as_u64().filter(|n| *n > 0);
    let models = settings
        .value(&["compaction", "tokenLimitPerModel"])
        .and_then(serde_json::Value::as_object);
    let per_model = models.and_then(|models| {
        models.get(model).and_then(positive).or_else(|| {
            models
                .iter()
                .filter_map(|(family, value)| {
                    model
                        .starts_with(family.trim_end_matches('*'))
                        .then(|| positive(value).map(|limit| (family.len(), limit)))
                        .flatten()
                })
                .max_by_key(|(length, _)| *length)
                .map(|(_, limit)| limit)
        })
    });
    session
        .filter(|n| *n > 0)
        .or(per_model)
        .or_else(|| {
            settings
                .value(&["compaction", "tokenLimit"])
                .and_then(positive)
        })
        .or(model_default.filter(|n| *n > 0))
        .unwrap_or(250_000)
        .min(max_input)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolution_chain_and_clamp() {
        let settings = crate::SettingsManager::in_memory(serde_json::from_value(serde_json::json!({"compaction":{"tokenLimit":100,"tokenLimitPerModel":{"gpt":80,"gpt-5":70,"gpt-5.4":60}}})).unwrap());
        let settings = settings.settings();
        assert_eq!(token_limit(settings, "gpt-5.4", None, Some(50), 200), 60);
        assert_eq!(token_limit(settings, "gpt-5.3", None, None, 200), 70);
        assert_eq!(token_limit(settings, "other", None, None, 200), 100);
        assert_eq!(token_limit(settings, "gpt-5.4", Some(90), None, 75), 75);
    }
}
