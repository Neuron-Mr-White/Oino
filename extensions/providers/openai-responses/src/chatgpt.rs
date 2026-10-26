//! Sign in with `ChatGPT` for the `openai` provider (pi-ai `auth/oauth/openai-chatgpt.js`): a
//! public-client PKCE flow against `auth.openai.com` with dynamic client registration, the
//! loopback callback on `127.0.0.1:1455` (or a pasted redirect URL), and refresh. The resulting
//! user access token is sent to `api.openai.com` directly; the Responses wire then omits the
//! fields Sign in with `ChatGPT` rejects. Credentials: `{type: "oauth", access, refresh, expires,
//! clientId, scopes}`.

use oino_extension::http::Listener;
use oino_extension::kv::{self, Scope};
use oino_extension::prelude::*;
use oino_provider_kit::guest::oauth::{
    Callback, callback_or_manual, post_form, random_bytes, show_auth_url,
};
use oino_provider_kit::oauth::{form, pkce, query_param, url_query};

const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
/// pi `AGENT_NAME_HINT` with the product substituted.
const AGENT_NAME_HINT: &str = "Oino";
const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";
const DIRECT_TOKEN_SCOPE: &str = "chatgpt.tokens.use.direct";
const EXPIRY_MARGIN_MS: i64 = 3 * 60 * 1000;
const DEVICE_ID_KEY: &str = "openai-chatgpt-device-id";

fn random_value() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(32))
}

/// A stable installation id (pi `getDeviceId`, a UUID) kept in global KV.
fn device_id() -> String {
    if let Some(id) = kv::get(Scope::Global, DEVICE_ID_KEY)
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|id| is_uuid(id))
    {
        return id;
    }
    let mut bytes = random_bytes(16);
    bytes.resize(16, 0);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = oino_provider_kit::oauth::hex(&bytes);
    let id = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    let _ = kv::set(Scope::Global, DEVICE_ID_KEY, id.as_bytes());
    id
}

fn is_uuid(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// pi `authorizationResultFromCallback`: `(code, client_id)`.
fn callback_result(query: &str, expected_state: &str) -> Result<(String, String)> {
    let code =
        query_param(query, "code").ok_or_else(|| Error::msg("Missing authorization code"))?;
    let state = query_param(query, "state").ok_or_else(|| Error::msg("Missing OAuth state"))?;
    if state != expected_state {
        return Err(Error::msg("OAuth state mismatch"));
    }
    let client_id = query_param(query, "client_id")
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            Error::msg("OpenAI OAuth registration callback did not contain an issued client ID")
        })?;
    Ok((code, client_id))
}

/// pi `authorizationResultFromManualInput`.
fn manual_result(input: &str, expected_state: &str) -> Result<(String, String)> {
    let input = input.trim();
    let query = url_query(input)
        .ok_or_else(|| Error::msg("Paste the full callback URL from the browser"))?;
    let base = input.split(['?', '#']).next().unwrap_or(input);
    if base != REDIRECT_URI {
        return Err(Error::msg(format!(
            "The pasted callback URL must start with {REDIRECT_URI}"
        )));
    }
    if let Some(error) = query_param(query, "error") {
        return Err(Error::msg(format!("ChatGPT authorization failed: {error}")));
    }
    callback_result(query, expected_state)
}

/// pi `requestToken`.
async fn request_token(pairs: &[(&str, &str)]) -> Result<Value> {
    let reply = post_form(TOKEN_URL, &[("accept", "application/json")], pairs).await?;
    if !reply.ok() {
        return Err(Error::msg(format!(
            "OpenAI OAuth token request failed ({}): {}",
            reply.status, reply.text
        )));
    }
    let data = reply.json();
    if !data.is_object() {
        return Err(Error::msg("OpenAI OAuth token response must be an object"));
    }
    Ok(data)
}

/// pi `credentialFromTokenResponse`.
fn credential(token: &Value, client_id: &str) -> Result<Value> {
    let field = |name: &str| {
        token[name]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::msg(format!("OpenAI OAuth token response has invalid {name}")))
    };
    let access = field("access_token")?;
    let refresh = field("refresh_token")?;
    let scope = field("scope")?;
    let expires_in = token["expires_in"]
        .as_f64()
        .filter(|e| e.is_finite() && *e > 0.0)
        .ok_or_else(|| Error::msg("OpenAI OAuth token response has invalid expires_in"))?;
    let scopes: Vec<&str> = scope.split_whitespace().collect();
    if !scopes.contains(&DIRECT_TOKEN_SCOPE) {
        return Err(Error::msg(format!(
            "OpenAI OAuth grant did not include {DIRECT_TOKEN_SCOPE}"
        )));
    }
    #[allow(clippy::cast_possible_truncation, reason = "seconds to ms")]
    let expires_ms = (expires_in * 1000.0) as i64;
    Ok(json!({
        "type": "oauth",
        "access": access,
        "refresh": refresh,
        "expires": timers::now_ms() + expires_ms - EXPIRY_MARGIN_MS,
        "clientId": client_id,
        "scopes": scopes,
    }))
}

/// `oauth-login` for `openai` (Sign in with `ChatGPT`).
pub(crate) async fn login() -> Result<Value> {
    let host_id = format!("urn:uuid:{}", device_id().to_lowercase());
    let pkce = pkce(&random_bytes(32));
    let state = random_value();
    let nonce = random_value();
    let listener = Listener::bind(CALLBACK_PORT).map_err(|_| {
        Error::msg(format!(
            "Port {CALLBACK_PORT} is in use, probably by an unfinished login in another session or by the Codex CLI. Cancel that login and try again."
        ))
    })?;
    let url = format!(
        "{AUTHORIZE_URL}?{}",
        form(&[
            ("client_id", DYNAMIC_CLIENT_ID),
            ("agent_name_hint", AGENT_NAME_HINT),
            ("ext_agent_host_id", &host_id),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("resource", RESOURCE),
            (
                "scope",
                &format!(
                    "openid profile email offline_access resource.invoke {DIRECT_TOKEN_SCOPE}"
                )
            ),
            ("state", &state),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("nonce", &nonce),
        ])
    );
    show_auth_url(
        &url,
        "Complete sign-in in your browser. If the callback does not complete, paste the final redirect URL here.",
    );
    let check = |query: &str| {
        callback_result(query, &state)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let outcome = callback_or_manual(
        Some(&listener),
        CALLBACK_PATH,
        &check,
        "ChatGPT",
        "Complete login in your browser, or paste the final redirect URL here:",
        REDIRECT_URI,
    )
    .await?;
    let (code, client_id) = match outcome {
        Callback::Query(query) => callback_result(&query, &state)?,
        Callback::Manual(input) => manual_result(&input, &state)?,
    };
    let _ = ui::notify("Exchanging authorization code for tokens...", "info");
    let token = request_token(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", &code),
        ("code_verifier", &pkce.verifier),
        ("redirect_uri", REDIRECT_URI),
        ("resource", RESOURCE),
    ])
    .await?;
    if token["id_token"]
        .as_str()
        .is_none_or(|t| t.trim().is_empty())
    {
        return Err(Error::msg(
            "OpenAI OAuth token response did not contain an ID token",
        ));
    }
    credential(&token, &client_id)
}

/// `oauth-refresh` for `openai`.
pub(crate) async fn refresh(stored: &Value) -> Result<Value> {
    let client_id = stored["clientId"]
        .as_str()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            Error::msg(
                "Stored OpenAI OAuth credential does not contain an issued client ID; reconnect ChatGPT",
            )
        })?;
    let refresh_token = stored["refresh"]
        .as_str()
        .ok_or_else(|| Error::msg("credential has no refresh token"))?;
    let token = request_token(&[
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("resource", RESOURCE),
    ])
    .await?;
    credential(&token, client_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn manual_input() {
        let (code, client) = manual_result(
            "http://127.0.0.1:1455/auth/callback?code=c&state=s&client_id=app_1",
            "s",
        )
        .unwrap();
        assert_eq!((code.as_str(), client.as_str()), ("c", "app_1"));
        assert!(manual_result("http://localhost:1455/auth/callback?code=c&state=s", "s").is_err());
        assert!(manual_result("not a url", "s").is_err());
    }

    #[test]
    fn uuid() {
        assert!(is_uuid("123e4567-e89b-42d3-a456-426614174000"));
        assert!(!is_uuid("nope"));
    }
}
