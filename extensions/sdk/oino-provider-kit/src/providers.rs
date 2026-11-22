//! Per-provider adaptations of the shared wires, ported from pi-ai 1.4.0's provider definitions:
//! GitHub Copilot (`providers/github-copilot.js`, `auth/oauth/github-copilot.js` `toAuth`,
//! `api/github-copilot-headers.js`), Cloudflare (`providers/cloudflare-*.js`) and Azure chat
//! completions (`providers/azure.js`, `api/azure-openai-config.js`).
//!
//! pi resolves these in its auth/provider layer before the wire runs; Oino has no such layer in
//! core, so [`adapt`] rewrites the [`ProviderRequest`] the same way (base URL, auth headers) and
//! [`finish`] applies what pi does through `onPayload` (Azure deployment names).

use oino_ai::{Message, UserContent, UserContentField};
use serde_json::Value;

use crate::request::ProviderRequest;

/// GitHub Copilot provider id.
pub const GITHUB_COPILOT: &str = "github-copilot";
const COPILOT_DEFAULT_BASE_URL: &str = "https://api.individual.githubcopilot.com";
const CLOUDFLARE_ACCOUNT_ID: &str = "CLOUDFLARE_ACCOUNT_ID";
const CLOUDFLARE_GATEWAY_ID: &str = "CLOUDFLARE_GATEWAY_ID";
const DEFAULT_AZURE_API_VERSION: &str = "v1";

/// Rewrites `request` for provider-specific endpoints and auth (pi's auth resolution and
/// provider stream wrappers).
///
/// # Errors
/// An unusable provider configuration (pi setup errors, e.g. a missing Azure endpoint).
pub fn adapt(request: &mut ProviderRequest) -> Result<(), String> {
    match request.model.provider.as_str() {
        GITHUB_COPILOT => {
            if let Some(base) = request.api_key.as_deref().and_then(copilot_base_url) {
                request.model.base_url = base;
            } else if request.model.base_url.is_empty() {
                COPILOT_DEFAULT_BASE_URL.clone_into(&mut request.model.base_url);
            }
        }
        "cloudflare-workers-ai" => {
            request.model.base_url = resolve_cloudflare_base_url(request)?;
        }
        "cloudflare-ai-gateway" => {
            request.model.base_url = resolve_cloudflare_base_url(request)?;
            if let Some(key) = request.api_key.take().filter(|k| !k.is_empty()) {
                request.auth_headers = vec![
                    ("cf-aig-authorization".into(), Some(format!("Bearer {key}"))),
                    ("Authorization".into(), None),
                    ("x-api-key".into(), None),
                ];
            }
        }
        "azure" if request.model.api == "openai-completions" => {
            request.model.base_url = resolve_azure_base_url(request)?;
        }
        _ => {}
    }
    Ok(())
}

/// Payload changes pi makes through provider `onPayload` wrappers, applied before the
/// `before-request` hook: Azure sends the deployment name as `model`.
pub fn finish(request: &ProviderRequest, payload: &mut Value) {
    if request.model.provider == "azure" && request.model.api == "openai-completions" {
        let deployment = resolve_deployment_name(request);
        if deployment != request.model.id
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("model".into(), Value::String(deployment));
        }
    }
}

/// pi `getBaseUrlFromToken`: `proxy-ep=proxy.<host>` in a Copilot token → `https://api.<host>`.
#[must_use]
pub fn copilot_base_url(token: &str) -> Option<String> {
    let start = token.find("proxy-ep=")? + "proxy-ep=".len();
    let host = token[start..].split(';').next()?;
    if host.is_empty() {
        return None;
    }
    let host = host
        .strip_prefix("proxy.")
        .map_or_else(|| host.to_owned(), |rest| format!("api.{rest}"));
    Some(format!("https://{host}"))
}

/// pi `buildCopilotDynamicHeaders` for `messages` (empty for other providers).
#[must_use]
pub fn copilot_dynamic_headers(
    request: &ProviderRequest,
    messages: &[Message],
) -> Vec<(String, String)> {
    if request.model.provider != GITHUB_COPILOT {
        return Vec::new();
    }
    let initiator = match messages.last() {
        Some(Message::User(_)) | None => "user",
        Some(_) => "agent",
    };
    let mut headers = vec![
        ("X-Initiator".to_owned(), initiator.to_owned()),
        ("Openai-Intent".to_owned(), "conversation-edits".to_owned()),
    ];
    if has_vision_input(messages) {
        headers.push(("Copilot-Vision-Request".to_owned(), "true".to_owned()));
    }
    headers
}

/// pi `hasCopilotVisionInput`.
fn has_vision_input(messages: &[Message]) -> bool {
    let has_image =
        |content: &[UserContent]| content.iter().any(|c| matches!(c, UserContent::Image(_)));
    messages.iter().any(|message| match message {
        Message::User(user) => match &user.content {
            UserContentField::Blocks(blocks) => has_image(blocks),
            UserContentField::Text(_) => false,
        },
        Message::ToolResult(result) => has_image(&result.content),
        _ => false,
    })
}

/// pi `resolveCloudflareModel`: fills `{CLOUDFLARE_ACCOUNT_ID}` / `{CLOUDFLARE_GATEWAY_ID}`.
/// pi's auth layer reports the provider as unconfigured when a needed id is missing; the
/// component says which variable to set instead of requesting a placeholder host.
fn resolve_cloudflare_base_url(request: &ProviderRequest) -> Result<String, String> {
    let mut url = request.model.base_url.clone();
    for name in [CLOUDFLARE_ACCOUNT_ID, CLOUDFLARE_GATEWAY_ID] {
        let placeholder = format!("{{{name}}}");
        if !url.contains(&placeholder) {
            continue;
        }
        let value = request.env_value(name).ok_or_else(|| {
            format!(
                "{} requires {name}: set it in the environment",
                request.model.provider
            )
        })?;
        url = url.replace(&placeholder, value);
    }
    Ok(url)
}

/// pi `resolveDeploymentName`.
#[must_use]
pub fn resolve_deployment_name(request: &ProviderRequest) -> String {
    if let Some(name) = &request.extra.azure_deployment_name {
        return name.clone();
    }
    request
        .env_value("AZURE_OPENAI_DEPLOYMENT_NAME_MAP")
        .and_then(|map| {
            map.split(',').find_map(|entry| {
                let (model, deployment) = entry.trim().split_once('=')?;
                let (model, deployment) = (model.trim(), deployment.trim());
                (!model.is_empty() && !deployment.is_empty() && model == request.model.id)
                    .then(|| deployment.to_owned())
            })
        })
        .unwrap_or_else(|| request.model.id.clone())
}

/// pi `resolveAzureBaseUrl`.
///
/// # Errors
/// No endpoint configured, or an invalid URL.
pub fn resolve_azure_base_url(request: &ProviderRequest) -> Result<String, String> {
    let explicit = request
        .extra
        .azure_base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            request
                .env_value("AZURE_OPENAI_BASE_URL")
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .map(str::to_owned);
    let resource = request
        .extra
        .azure_resource_name
        .clone()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            request
                .env_value("AZURE_OPENAI_RESOURCE_NAME")
                .map(str::to_owned)
        });
    let resolved = explicit
        .or_else(|| resource.map(|name| format!("https://{name}.openai.azure.com/openai/v1")))
        .or_else(|| (!request.model.base_url.is_empty()).then(|| request.model.base_url.clone()))
        .ok_or_else(|| {
            "Azure OpenAI base URL is required. Set AZURE_OPENAI_BASE_URL or AZURE_OPENAI_RESOURCE_NAME, or pass azureBaseUrl, azureResourceName, or model.baseUrl.".to_owned()
        })?;
    normalize_azure_base_url(&resolved)
}

/// pi `resolveAzureConfig().apiVersion`.
#[must_use]
pub fn azure_api_version(request: &ProviderRequest) -> String {
    request
        .extra
        .azure_api_version
        .clone()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            request
                .env_value("AZURE_OPENAI_API_VERSION")
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_AZURE_API_VERSION.to_owned())
}

/// pi `normalizeAzureBaseUrl`: Azure hosts get the `/openai/v1` base path.
fn normalize_azure_base_url(base: &str) -> Result<String, String> {
    let trimmed = base.trim().trim_end_matches('/');
    let invalid = || format!("Invalid Azure OpenAI base URL: {base}");
    let (scheme, rest) = trimmed.split_once("://").ok_or_else(invalid)?;
    if scheme.is_empty() || rest.is_empty() {
        return Err(invalid());
    }
    let (authority, path_query) = rest
        .find(['/', '?', '#'])
        .map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err(invalid());
    }
    let (path, query) = path_query
        .split_once('?')
        .map_or((path_query, ""), |(p, q)| (p, q));
    let path = path.split('#').next().unwrap_or_default();
    let normalized_path = path.trim_end_matches('/');
    let is_azure = host.ends_with(".openai.azure.com")
        || host.ends_with(".cognitiveservices.azure.com")
        || host.ends_with(".ai.azure.com");
    let authority = authority.to_ascii_lowercase();
    if is_azure
        && matches!(
            normalized_path,
            "" | "/" | "/openai" | "/openai/v1/responses"
        )
    {
        return Ok(format!("{scheme}://{authority}/openai/v1"));
    }
    let path = if path.is_empty() { "/" } else { path };
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    Ok(format!("{scheme}://{authority}{path}{query}")
        .trim_end_matches('/')
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_base_url_from_token() {
        assert_eq!(
            copilot_base_url("tid=1;proxy-ep=proxy.enterprise.githubcopilot.com;x=1").as_deref(),
            Some("https://api.enterprise.githubcopilot.com")
        );
        assert_eq!(copilot_base_url("ghu_abc"), None);
    }

    #[test]
    fn azure_urls() {
        assert_eq!(
            normalize_azure_base_url("https://r.openai.azure.com/").unwrap(),
            "https://r.openai.azure.com/openai/v1"
        );
        assert_eq!(
            normalize_azure_base_url(
                "https://r.cognitiveservices.azure.com/openai/v1/responses?x=1"
            )
            .unwrap(),
            "https://r.cognitiveservices.azure.com/openai/v1"
        );
        assert_eq!(
            normalize_azure_base_url("https://gw.example.com/azure/").unwrap(),
            "https://gw.example.com/azure"
        );
        assert!(normalize_azure_base_url("nope").is_err());
    }
}
