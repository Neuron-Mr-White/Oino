//! pi `auth-guidance.ts`: user-facing texts when no model or credential is available
//! (product name and docs location substituted per D13).

use std::path::Path;

const UNKNOWN_PROVIDER: &str = "unknown";

/// pi `getProviderLoginHelp`; `docs_dir` is the installed documentation directory.
#[must_use]
pub fn provider_login_help(docs_dir: &Path) -> String {
    [
        "Use /login to log into a provider via OAuth or API key. See:".to_owned(),
        format!("  {}", docs_dir.join("providers.md").display()),
        format!("  {}", docs_dir.join("models.md").display()),
    ]
    .join("\n")
}

/// pi `formatNoModelsAvailableMessage`.
#[must_use]
pub fn no_models_available_message(docs_dir: &Path) -> String {
    format!("No models available. {}", provider_login_help(docs_dir))
}

/// pi `formatNoModelSelectedMessage`.
#[must_use]
pub fn no_model_selected_message(docs_dir: &Path) -> String {
    format!(
        "No model selected.\n\n{}\n\nThen use /model to select a model.",
        provider_login_help(docs_dir)
    )
}

/// pi `formatNoApiKeyFoundMessage`.
#[must_use]
pub fn no_api_key_found_message(provider: &str, docs_dir: &Path) -> String {
    let display = if provider == UNKNOWN_PROVIDER {
        "the selected model"
    } else {
        provider
    };
    format!(
        "No API key found for {display}.\n\n{}",
        provider_login_help(docs_dir)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        let docs = Path::new("/d");
        assert_eq!(
            no_api_key_found_message("unknown", docs),
            "No API key found for the selected model.\n\nUse /login to log into a provider via OAuth or API key. See:\n  /d/providers.md\n  /d/models.md"
        );
        assert!(no_models_available_message(docs).starts_with("No models available. Use /login"));
    }
}
