use oino_provider_catalog::{ProviderDescriptor, ProviderTarget};
use oino_tui::AuthStatusItem;

#[must_use]
pub fn quickstart() -> String {
    vec![
        "Auth Quickstart Guide".to_string(),
        "=".repeat(50),
        "".to_string(),
        "Oino now uses extension-managed auth and routing.".to_string(),
        "Recommended path: 9router extension".to_string(),
        "   /extensions              Install builtin:9router if not installed".to_string(),
        "   /9router setup           Show external/managed setup steps".to_string(),
        "   /9router use-managed     Let Oino manage the local sidecar".to_string(),
        "   /9router start           Start with pinned/last-good fallback".to_string(),
        "   /9router models          Refresh live/cached 9router models".to_string(),
        "   /model 9router:<model>   Select a 9router model or combo".to_string(),
        "".to_string(),
        "Built-in provider OAuth/API-key commands have been removed.".to_string(),
        "Configure provider credentials in the 9router dashboard or through an auth extension."
            .to_string(),
        "".to_string(),
        "Useful commands:".to_string(),
        "   /9router status       - Check router health/config".to_string(),
        "   /9router rollback     - Roll back to last-good/known-good tag".to_string(),
        "   /auth                 - Show extension auth/runtime readiness".to_string(),
        "   /model                - Select a model".to_string(),
    ]
    .join("\n")
}

#[must_use]
pub fn format_auth_status(items: &[AuthStatusItem]) -> String {
    if items.is_empty() {
        return "No extension auth/runtime readiness rows found. Recommended: `/9router setup` for extension-managed auth/routing.".into();
    }
    let mut lines = vec![
        "Auth/runtime readiness (extension-managed; use `/9router setup` for 9router):".to_string(),
    ];
    for item in items {
        let current = if item.current { " current" } else { "" };
        lines.push(format!(
            "- {} ({}){}: {} / {} [{}; {}; source: {}]",
            item.display_name,
            item.provider_id,
            current,
            item.state,
            item.readiness,
            item.auth_kind,
            item.runtime,
            item.source
        ));
        if !item.detail.trim().is_empty() {
            lines.push(format!("  {}", item.detail));
        }
        if let Some(url) = &item.setup_url {
            lines.push(format!("  setup: {url}"));
        }
    }
    lines.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemovedProviderRuntimeInfo {
    pub hint: &'static str,
    pub historical_required_env: &'static [&'static str],
    pub historical_optional_env: &'static [&'static str],
    pub historical_example_model: Option<&'static str>,
}

pub fn removed_provider_runtime_info(
    provider: ProviderDescriptor,
) -> Option<RemovedProviderRuntimeInfo> {
    match provider.target {
        ProviderTarget::Claude => Some(RemovedProviderRuntimeInfo {
            hint: "Claude/Anthropic direct runtime has been removed from core; use 9router or an extension runtime provider.",
            historical_required_env: &["ANTHROPIC_API_KEY"],
            historical_optional_env: &[],
            historical_example_model: Some("claude:claude-3-5-sonnet-latest"),
        }),
        ProviderTarget::OpenAi => Some(RemovedProviderRuntimeInfo {
            hint: "OpenAI ChatGPT/OAuth runtime has been removed from core; use 9router or an extension runtime provider.",
            historical_required_env: &[],
            historical_optional_env: &["OPENAI_ACCESS_TOKEN", "OPENAI_REFRESH_TOKEN", "OPENAI_API_KEY"],
            historical_example_model: Some("openai-api:gpt-4o-mini"),
        }),
        ProviderTarget::Azure => Some(RemovedProviderRuntimeInfo {
            hint: "Azure OpenAI built-in auth/runtime has been removed from core; configure Azure in 9router or an extension.",
            historical_required_env: &["AZURE_OPENAI_ENDPOINT", "AZURE_OPENAI_DEPLOYMENT", "AZURE_OPENAI_API_KEY"],
            historical_optional_env: &["AZURE_OPENAI_API_VERSION"],
            historical_example_model: Some("azure:<deployment-or-model>"),
        }),
        ProviderTarget::Bedrock => Some(RemovedProviderRuntimeInfo {
            hint: "AWS Bedrock built-in auth/runtime has been removed from core; use 9router or an extension runtime provider.",
            historical_required_env: &["AWS_REGION"],
            historical_optional_env: &["AWS_PROFILE", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_BEARER_TOKEN_BEDROCK"],
            historical_example_model: Some("bedrock:<model-id>"),
        }),
        ProviderTarget::Cursor => Some(RemovedProviderRuntimeInfo {
            hint: "Cursor built-in auth/import/runtime has been removed from core; use 9router or an extension runtime provider.",
            historical_required_env: &[],
            historical_optional_env: &["CURSOR_API_KEY", "CURSOR_ACCESS_TOKEN", "CURSOR_REFRESH_TOKEN", "CURSOR_CLIENT_VERSION", "CURSOR_AGENT_PATH"],
            historical_example_model: Some("cursor:auto"),
        }),
        ProviderTarget::Copilot => Some(RemovedProviderRuntimeInfo {
            hint: "GitHub Copilot device-code auth/runtime has been removed from core; use an extension if Copilot auth is needed.",
            historical_required_env: &[],
            historical_optional_env: &["GITHUB_TOKEN", "GITHUB_COPILOT_CLIENT_ID"],
            historical_example_model: Some("copilot:<model-id>"),
        }),
        ProviderTarget::Gemini => Some(RemovedProviderRuntimeInfo {
            hint: "Gemini/Google OAuth auth/runtime has been removed from core; configure Gemini in 9router or an extension.",
            historical_required_env: &[],
            historical_optional_env: &["GOOGLE_CLOUD_PROJECT", "GOOGLE_CLOUD_PROJECT_ID"],
            historical_example_model: Some("gemini:gemini-2.5-pro"),
        }),
        ProviderTarget::Google => Some(RemovedProviderRuntimeInfo {
            hint: "Google OAuth auth/runtime has been removed from core; use an extension if Google account auth is needed.",
            historical_required_env: &[],
            historical_optional_env: &["GOOGLE_OAUTH_CLIENT_ID", "GOOGLE_OAUTH_CLIENT_SECRET"],
            historical_example_model: None,
        }),
        ProviderTarget::Antigravity => Some(RemovedProviderRuntimeInfo {
            hint: "Antigravity OAuth auth/runtime has been removed from core; use an extension runtime provider.",
            historical_required_env: &[],
            historical_optional_env: &["ANTIGRAVITY_CLIENT_ID", "ANTIGRAVITY_CLIENT_SECRET"],
            historical_example_model: Some("antigravity:<model-id>"),
        }),
        ProviderTarget::AutoImport => Some(RemovedProviderRuntimeInfo {
            hint: "External credential import has been removed from core; credentials should be configured in 9router or an extension.",
            historical_required_env: &[],
            historical_optional_env: &[],
            historical_example_model: None,
        }),
        _ => None,
    }
}

#[must_use]
pub fn removed_provider_runtime_detail(provider: ProviderDescriptor) -> String {
    removed_provider_runtime_info(provider)
        .map(|info| {
            let mut detail = info.hint.to_string();
            if !info.historical_required_env.is_empty() {
                detail.push_str(" Historical required env: ");
                detail.push_str(&info.historical_required_env.join(", "));
                detail.push('.');
            }
            if !info.historical_optional_env.is_empty() {
                detail.push_str(" Historical optional env: ");
                detail.push_str(&info.historical_optional_env.join(", "));
                detail.push('.');
            }
            if let Some(example) = info.historical_example_model {
                detail.push_str(" Historical model prefix: ");
                detail.push_str(example);
                detail.push('.');
            }
            detail
        })
        .unwrap_or_else(|| "Use `/9router setup` or install an extension runtime provider.".into())
}

#[must_use]
pub fn removed_builtin_auth_message(action: &str) -> String {
    format!(
        "Built-in provider auth has been removed for `{action}`. Use `/9router setup` to configure provider auth/routing via 9router, or install an auth extension."
    )
}
