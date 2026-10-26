//! `OpenAI` Codex (`ChatGPT` Plus/Pro) OAuth (pi-ai `auth/oauth/openai-codex.js`): browser PKCE login
//! with the loopback callback on port 1455 (shared with the Codex CLI; a pasted redirect URL
//! works when it is taken), device-code login for headless machines, and refresh.
//! Credentials: `{type: "oauth", access, refresh, expires, accountId}`.

use oino_extension::http::Listener;
use oino_extension::prelude::*;
use oino_provider_kit::codex::extract_account_id;
use oino_provider_kit::guest::oauth::{
    Callback, callback_or_manual, poll_device, post_form, post_json, random_bytes, show_auth_url,
    show_device_code, state_and_code,
};
use oino_provider_kit::oauth::{
    DevicePoll, PollStatus, form, hex, parse_authorization_input, pkce, query_param,
};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEVICE_CODE_TIMEOUT_SECONDS: f64 = 15.0 * 60.0;
const SCOPE: &str = "openid profile email offline_access";
/// pi passes its agent name as `originator`; Codex expects `pi` (not attribution).
const ORIGINATOR: &str = "pi";
const BROWSER: &str = "Browser login (default)";
const DEVICE: &str = "Device code login (headless)";

/// pi `readTokenResponse` + `credentialsFromToken`.
fn credentials(status: u16, text: &str, operation: &str) -> Result<Value> {
    if !(200..300).contains(&status) {
        return Err(Error::msg(format!(
            "OpenAI Codex token {operation} failed ({status}): {text}"
        )));
    }
    let json: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let (Some(access), Some(refresh), Some(expires_in)) = (
        json["access_token"].as_str().filter(|s| !s.is_empty()),
        json["refresh_token"].as_str().filter(|s| !s.is_empty()),
        json["expires_in"].as_f64(),
    ) else {
        return Err(Error::msg(format!(
            "OpenAI Codex token {operation} response missing fields: {json}"
        )));
    };
    let account_id = extract_account_id(access)
        .map_err(|_| Error::msg("Failed to extract accountId from token"))?;
    #[allow(clippy::cast_possible_truncation, reason = "seconds to ms")]
    let expires_ms = (expires_in * 1000.0) as i64;
    Ok(json!({
        "type": "oauth",
        "access": access,
        "refresh": refresh,
        "expires": timers::now_ms() + expires_ms,
        "accountId": account_id,
    }))
}

async fn exchange_code(code: &str, verifier: &str, redirect_uri: &str) -> Result<Value> {
    let reply = post_form(
        TOKEN_URL,
        &[],
        &[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
    )
    .await?;
    credentials(reply.status, &reply.text, "exchange")
}

/// `oauth-login`: browser (default) or device-code login.
pub(crate) async fn login() -> Result<Value> {
    let method = ui::select("Select OpenAI Codex login method:", &[BROWSER, DEVICE])
        .await?
        .ok_or_else(|| Error::msg("Login cancelled"))?;
    if method == DEVICE {
        return login_device_code().await;
    }
    login_browser().await
}

async fn login_browser() -> Result<Value> {
    let pkce = pkce(&random_bytes(32));
    let state = hex(&random_bytes(16));
    let url = format!(
        "{AUTHORIZE_URL}?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPE),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", ORIGINATOR),
        ])
    );
    let listener = Listener::bind(CALLBACK_PORT).ok();
    show_auth_url(
        &url,
        "A browser window should open. Complete login to finish.",
    );
    let check = state_and_code(&state);
    let outcome = callback_or_manual(
        listener.as_ref(),
        CALLBACK_PATH,
        &check,
        "OpenAI",
        "Complete login in your browser, or paste the authorization code / redirect URL here:",
        REDIRECT_URI,
    )
    .await?;
    let code = match outcome {
        Callback::Query(query) => query_param(&query, "code"),
        Callback::Manual(input) => {
            let (code, parsed_state) = parse_authorization_input(&input);
            if parsed_state.is_some_and(|s| s != state) {
                return Err(Error::msg("State mismatch"));
            }
            code
        }
    }
    .filter(|c| !c.is_empty())
    .ok_or_else(|| Error::msg("Missing authorization code"))?;
    exchange_code(&code, &pkce.verifier, REDIRECT_URI).await
}

async fn login_device_code() -> Result<Value> {
    let reply = post_json(DEVICE_USER_CODE_URL, &[], &json!({"client_id": CLIENT_ID})).await?;
    if !reply.ok() {
        if reply.status == 404 {
            return Err(Error::msg(
                "OpenAI Codex device code login is not enabled for this server. Use browser login or verify the server URL.",
            ));
        }
        let suffix = if reply.text.is_empty() {
            String::new()
        } else {
            format!(": {}", reply.text)
        };
        return Err(Error::msg(format!(
            "OpenAI Codex device code request failed with status {}{suffix}",
            reply.status
        )));
    }
    let json = reply.json();
    let interval = match &json["interval"] {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        other => other.as_f64(),
    };
    let (Some(device_auth_id), Some(user_code), Some(interval)) = (
        json["device_auth_id"].as_str().filter(|s| !s.is_empty()),
        json["user_code"].as_str().filter(|s| !s.is_empty()),
        interval.filter(|i| i.is_finite() && *i >= 0.0),
    ) else {
        return Err(Error::msg(format!(
            "Invalid OpenAI Codex device code response: {json}"
        )));
    };
    show_device_code(user_code, DEVICE_VERIFICATION_URI);
    let body = json!({"device_auth_id": device_auth_id, "user_code": user_code});
    let (code, verifier) = poll_device(
        DevicePoll::new(
            Some(interval),
            Some(DEVICE_CODE_TIMEOUT_SECONDS),
            timers::now_ms(),
        ),
        false,
        || {
            let body = body.clone();
            async move {
                let reply = post_json(DEVICE_TOKEN_URL, &[], &body).await?;
                if reply.ok() {
                    let json = reply.json();
                    return Ok(
                        match (
                            json["authorization_code"].as_str(),
                            json["code_verifier"].as_str(),
                        ) {
                            (Some(code), Some(verifier))
                                if !code.is_empty() && !verifier.is_empty() =>
                            {
                                PollStatus::Complete((code.to_owned(), verifier.to_owned()))
                            }
                            _ => PollStatus::Failed(format!(
                                "Invalid OpenAI Codex device auth token response: {json}"
                            )),
                        },
                    );
                }
                if reply.status == 403 || reply.status == 404 {
                    return Ok(PollStatus::Pending);
                }
                let json = reply.json();
                let code = match &json["error"] {
                    Value::Object(error) => {
                        error.get("code").and_then(Value::as_str).map(str::to_owned)
                    }
                    Value::String(code) => Some(code.clone()),
                    _ => None,
                };
                Ok(match code.as_deref() {
                    Some("deviceauth_authorization_pending") => PollStatus::Pending,
                    Some("slow_down") => PollStatus::SlowDown(None),
                    _ => {
                        let suffix = if reply.text.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", reply.text)
                        };
                        PollStatus::Failed(format!(
                            "OpenAI Codex device auth failed with status {}{suffix}",
                            reply.status
                        ))
                    }
                })
            }
        },
    )
    .await?;
    exchange_code(&code, &verifier, DEVICE_REDIRECT_URI).await
}

/// `oauth-refresh`.
pub(crate) async fn refresh(stored: &Value) -> Result<Value> {
    let refresh_token = stored["refresh"]
        .as_str()
        .ok_or_else(|| Error::msg("credential has no refresh token"))?;
    let reply = post_form(
        TOKEN_URL,
        &[],
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )
    .await
    .map_err(|e| Error::msg(format!("OpenAI Codex token refresh error: {e}")))?;
    credentials(reply.status, &reply.text, "refresh")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn token_response() {
        assert!(
            credentials(400, "nope", "exchange")
                .unwrap_err()
                .to_string()
                .contains("(400): nope")
        );
        assert!(
            credentials(200, "{}", "refresh")
                .unwrap_err()
                .to_string()
                .contains("missing fields")
        );
    }
}
