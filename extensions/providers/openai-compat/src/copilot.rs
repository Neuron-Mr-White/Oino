//! GitHub Copilot OAuth (pi-ai `auth/oauth/github-copilot.js`): GitHub device-code login
//! (github.com or GitHub Enterprise), Copilot token exchange, the account's model catalog
//! (`availableModelIds`) and best-effort model policy enabling. Credentials are pi's
//! `{type: "oauth", refresh, access, expires, enterpriseUrl?, availableModelIds}`; the access token
//! carries the per-account API endpoint (`proxy-ep`), which the kit turns into the base URL.

use std::time::Duration;

use oino_extension::http::Method;
use oino_extension::prelude::*;
use oino_provider_kit::guest::oauth::{Reply, poll_device, post_form, send, show_device_code};
use oino_provider_kit::oauth::{DevicePoll, PollStatus};
use oino_provider_kit::providers::copilot_base_url;

/// pi decodes this from base64 at runtime.
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const EDITOR_VERSION: &str = "vscode/1.107.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const INTEGRATION_ID: &str = "vscode-chat";
const API_VERSION: &str = "2026-06-01";
const INDIVIDUAL_BASE_URL: &str = "https://api.individual.githubcopilot.com";

/// Catalog model ids (pi `GITHUB_COPILOT_MODELS`) for policy enabling.
fn catalog_ids(catalog: &oino_provider_kit::catalog::Catalog) -> Vec<String> {
    catalog
        .provider("github-copilot")
        .map(|p| {
            p.static_models()
                .iter()
                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// pi `normalizeDomain`.
fn normalize_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let rest = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    (!host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'))
    .then_some(host)
}

fn base_url(token: &str, enterprise: Option<&str>) -> String {
    copilot_base_url(token).unwrap_or_else(|| {
        enterprise.map_or_else(
            || INDIVIDUAL_BASE_URL.to_owned(),
            |domain| format!("https://copilot-api.{domain}"),
        )
    })
}

fn copilot_headers(token: &str) -> Vec<(String, String)> {
    vec![
        ("accept".into(), "application/json".into()),
        ("authorization".into(), format!("Bearer {token}")),
        ("user-agent".into(), USER_AGENT.into()),
        ("editor-version".into(), EDITOR_VERSION.into()),
        ("editor-plugin-version".into(), EDITOR_PLUGIN_VERSION.into()),
        ("copilot-integration-id".into(), INTEGRATION_ID.into()),
    ]
}

async fn request(
    method: Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
) -> Result<Reply> {
    let pairs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    send(method, url, &pairs, body).await
}

fn http_failure(reply: &Reply) -> Error {
    Error::msg(format!("{} {}", reply.status, reply.text))
}

/// pi `fetchWithRateLimitRetry` (429 with backoff / `retry-after`).
async fn with_rate_limit_retry(
    method: Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
    max_retries: u32,
    max_elapsed_ms: i64,
) -> Result<Reply> {
    let deadline =
        (max_retries > 0 && max_elapsed_ms > 0).then(|| timers::now_ms() + max_elapsed_ms);
    let mut retry = 0;
    loop {
        let reply = request(method, url, headers, body.clone()).await?;
        if reply.status != 429 || retry == max_retries {
            return Ok(reply);
        }
        let mut delay = 500_i64 * (1_i64 << retry.min(20));
        if let Some(after) = reply.header("retry-after") {
            match after.trim().parse::<f64>() {
                #[allow(clippy::cast_possible_truncation, reason = "seconds to ms")]
                Ok(seconds) => delay = (seconds * 1000.0) as i64,
                Err(_) => return Ok(reply),
            }
        }
        let delay = delay.max(0);
        if deadline.is_some_and(|d| delay >= d - timers::now_ms()) {
            return Ok(reply);
        }
        timers::sleep(Duration::from_millis(u64::try_from(delay).unwrap_or(0))).await;
        retry += 1;
    }
}

struct Models {
    available: Vec<String>,
    policy: Vec<String>,
}

struct Row {
    id: String,
    picker: bool,
    policy: Option<String>,
}

/// pi `parseGitHubCopilotModelCatalog`.
fn parse_models(raw: &Value, allow_policy_fallback: bool, known: &[String]) -> Result<Models> {
    let data = raw["data"]
        .as_array()
        .ok_or_else(|| Error::msg("Invalid Copilot models response"))?;
    let rows: Vec<Row> = data
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?;
            if item["capabilities"]["supports"]["tool_calls"] == false {
                return None;
            }
            Some(Row {
                id: id.to_owned(),
                picker: item["model_picker_enabled"] == true,
                policy: item["policy"]["state"].as_str().map(str::to_owned),
            })
        })
        .collect();
    let picker: Vec<String> = rows
        .iter()
        .filter(|r| r.picker && r.policy.as_deref() != Some("disabled"))
        .map(|r| r.id.clone())
        .collect();
    let fallback = allow_policy_fallback && picker.is_empty();
    let available = if !picker.is_empty() || !allow_policy_fallback {
        picker
    } else {
        rows.iter()
            .filter(|r| r.policy.as_deref() == Some("enabled"))
            .map(|r| r.id.clone())
            .collect()
    };
    let policy = rows
        .iter()
        .filter(|r| {
            r.policy.as_deref() == Some("unconfigured")
                && known.contains(&r.id)
                && (r.picker || fallback)
        })
        .map(|r| r.id.clone())
        .collect();
    Ok(Models { available, policy })
}

async fn fetch_models(
    token: &str,
    enterprise: Option<&str>,
    retries: (u32, i64),
    known: &[String],
) -> Result<Models> {
    let base = base_url(token, enterprise);
    let allow_fallback = base == INDIVIDUAL_BASE_URL;
    let mut headers = copilot_headers(token);
    headers.push(("x-github-api-version".into(), API_VERSION.into()));
    let reply = with_rate_limit_retry(
        Method::Get,
        &format!("{base}/models"),
        &headers,
        None,
        retries.0,
        retries.1,
    )
    .await?;
    if !reply.ok() {
        return Err(http_failure(&reply));
    }
    parse_models(&reply.json(), allow_fallback, known)
}

/// pi `refreshGitHubCopilotAccessToken`.
async fn copilot_token(github_token: &str, enterprise: Option<&str>) -> Result<Value> {
    let domain = enterprise.unwrap_or("github.com");
    let reply = request(
        Method::Get,
        &format!("https://api.{domain}/copilot_internal/v2/token"),
        &copilot_headers(github_token),
        None,
    )
    .await?;
    if !reply.ok() {
        return Err(http_failure(&reply));
    }
    let raw = reply.json();
    if !raw.is_object() {
        return Err(Error::msg("Invalid Copilot token response"));
    }
    let (Some(token), Some(expires_at)) = (raw["token"].as_str(), raw["expires_at"].as_i64())
    else {
        return Err(Error::msg("Invalid Copilot token response fields"));
    };
    let mut credential = json!({
        "type": "oauth",
        "refresh": github_token,
        "access": token,
        "expires": expires_at * 1000 - 5 * 60 * 1000,
    });
    if let Some(domain) = enterprise {
        credential["enterpriseUrl"] = json!(domain);
    }
    Ok(credential)
}

/// pi `enableGitHubCopilotModels` (best effort; stops on exhausted rate limiting).
async fn enable_models(token: &str, ids: &[String], enterprise: Option<&str>) -> Vec<String> {
    let base = base_url(token, enterprise);
    let mut headers = copilot_headers(token);
    headers.retain(|(k, _)| k != "accept");
    headers.push(("content-type".into(), "application/json".into()));
    headers.push(("openai-intent".into(), "chat-policy".into()));
    headers.push(("x-interaction-type".into(), "chat-policy".into()));
    let mut enabled = Vec::new();
    for id in ids {
        let url = format!("{base}/models/{id}/policy");
        let body = json!({"state": "enabled"}).to_string().into_bytes();
        match with_rate_limit_retry(Method::Post, &url, &headers, Some(body), 2, 5000).await {
            Ok(reply) if reply.status == 429 => break,
            Ok(reply) if reply.ok() => enabled.push(id.clone()),
            _ => {}
        }
    }
    enabled
}

/// `oauth-login` for `github-copilot`.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi loginGitHubCopilot")]
pub(crate) async fn login(known: &oino_provider_kit::catalog::Catalog) -> Result<Value> {
    let known = catalog_ids(known);
    let input = ui::input(
        "GitHub Enterprise URL/domain (blank for github.com)",
        Some("company.ghe.com"),
    )
    .await?
    .ok_or_else(|| Error::msg("Login cancelled"))?;
    let enterprise = normalize_domain(&input);
    if !input.trim().is_empty() && enterprise.is_none() {
        return Err(Error::msg("Invalid GitHub Enterprise URL/domain"));
    }
    let domain = enterprise
        .clone()
        .unwrap_or_else(|| "github.com".to_owned());
    let form_headers = [("accept", "application/json"), ("user-agent", USER_AGENT)];
    let reply = post_form(
        &format!("https://{domain}/login/device/code"),
        &form_headers,
        &[("client_id", CLIENT_ID), ("scope", "read:user")],
    )
    .await?;
    if !reply.ok() {
        return Err(http_failure(&reply));
    }
    let data = reply.json();
    if !data.is_object() {
        return Err(Error::msg("Invalid device code response"));
    }
    let (Some(device_code), Some(user_code), Some(uri), Some(expires_in)) = (
        data["device_code"].as_str(),
        data["user_code"].as_str(),
        data["verification_uri"].as_str(),
        data["expires_in"].as_f64(),
    ) else {
        return Err(Error::msg("Invalid device code response fields"));
    };
    if !data["interval"].is_null() && !data["interval"].is_number() {
        return Err(Error::msg("Invalid device code response fields"));
    }
    if !(uri.starts_with("https://") || uri.starts_with("http://")) {
        return Err(Error::msg(
            "Untrusted verification_uri in device code response",
        ));
    }
    show_device_code(user_code, uri);
    let token_url = format!("https://{domain}/login/oauth/access_token");
    let device_code = device_code.to_owned();
    let github_token = poll_device(
        DevicePoll::new(
            data["interval"].as_f64(),
            Some(expires_in),
            timers::now_ms(),
        ),
        true,
        || {
            let token_url = token_url.clone();
            let device_code = device_code.clone();
            async move {
                let reply = post_form(
                    &token_url,
                    &form_headers,
                    &[
                        ("client_id", CLIENT_ID),
                        ("device_code", &device_code),
                        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ],
                )
                .await?;
                if !reply.ok() {
                    return Err(http_failure(&reply));
                }
                let raw = reply.json();
                if let Some(token) = raw["access_token"].as_str() {
                    return Ok(PollStatus::Complete(token.to_owned()));
                }
                Ok(match raw["error"].as_str() {
                    Some("authorization_pending") => PollStatus::Pending,
                    Some("slow_down") => PollStatus::SlowDown(raw["interval"].as_f64()),
                    Some(error) => PollStatus::Failed(match raw["error_description"].as_str() {
                        Some(d) if !d.is_empty() => format!("Device flow failed: {error}: {d}"),
                        _ => format!("Device flow failed: {error}"),
                    }),
                    None => PollStatus::Failed("Invalid device token response".to_owned()),
                })
            }
        },
    )
    .await?;
    let mut credential = copilot_token(&github_token, enterprise.as_deref()).await?;
    let access = credential["access"].as_str().unwrap_or_default().to_owned();
    let models = fetch_models(&access, enterprise.as_deref(), (2, 5000), &known).await?;
    let mut available = models.available;
    if !models.policy.is_empty() {
        let _ = ui::notify("Enabling models...", "info");
        for id in enable_models(&access, &models.policy, enterprise.as_deref()).await {
            if !available.contains(&id) {
                available.push(id);
            }
        }
    }
    credential["availableModelIds"] = json!(available);
    Ok(credential)
}

/// `oauth-refresh` for `github-copilot`.
pub(crate) async fn refresh(
    known: &oino_provider_kit::catalog::Catalog,
    credential: &Value,
) -> Result<Value> {
    let refresh_token = credential["refresh"]
        .as_str()
        .ok_or_else(|| Error::msg("credential has no refresh token"))?;
    let enterprise = credential["enterpriseUrl"]
        .as_str()
        .filter(|u| !u.is_empty())
        .and_then(normalize_domain);
    let mut fresh = copilot_token(refresh_token, enterprise.as_deref()).await?;
    let access = fresh["access"].as_str().unwrap_or_default().to_owned();
    let models = fetch_models(&access, enterprise.as_deref(), (0, 0), &catalog_ids(known)).await?;
    fresh["availableModelIds"] = json!(models.available);
    Ok(fresh)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn domains() {
        assert_eq!(
            normalize_domain("https://company.ghe.com/x").as_deref(),
            Some("company.ghe.com")
        );
        assert_eq!(
            normalize_domain("company.ghe.com").as_deref(),
            Some("company.ghe.com")
        );
        assert_eq!(normalize_domain("  "), None);
    }

    #[test]
    fn model_catalog() {
        let raw = json!({"data": [
            {"id": "a", "model_picker_enabled": true, "policy": {"state": "enabled"}},
            {"id": "b", "model_picker_enabled": true, "policy": {"state": "unconfigured"}},
            {"id": "c", "model_picker_enabled": false, "policy": {"state": "enabled"}, "capabilities": {"supports": {"tool_calls": false}}},
        ]});
        let parsed = parse_models(&raw, true, &["b".to_owned()]).unwrap();
        assert_eq!(parsed.available, vec!["a", "b"]);
        assert_eq!(parsed.policy, vec!["b"]);
    }
}
