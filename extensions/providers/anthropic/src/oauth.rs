//! Claude Pro/Max OAuth (pi-ai `auth/oauth/anthropic.js`): PKCE authorization-code flow with a
//! loopback callback (or a pasted redirect URL / code), and token refresh. Credentials are pi's
//! `{type: "oauth", refresh, access, expires}`.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use oino_extension::futures::future::{self, Either};
use oino_extension::http::{self, Listener};
use oino_extension::prelude::*;
use sha2::{Digest, Sha256};

/// pi decodes this from base64 at runtime.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CALLBACK_PORT: u16 = 53692;
const CALLBACK_PATH: &str = "/callback";
const COPY_CODE_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const BROWSER: &str = "Browser login (default)";
const COPY_CODE: &str = "Copy code login (headless)";

struct Pkce {
    verifier: String,
    challenge: String,
}

fn pkce() -> Pkce {
    let bytes = oino_extension::bindings::wasi::random::random::get_random_bytes(32);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'*') {
            out.push(char::from(byte));
        } else if byte == b' ' {
            out.push('+');
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn decode_component(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                if let Some(value) = text
                    .get(i + 1..i + 3)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    out.push(value);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (decode_component(key) == name).then(|| decode_component(value))
    })
}

fn authorize_url(redirect_uri: &str, pkce: &Pkce) -> String {
    let params = [
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", SCOPES),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", &pkce.verifier),
    ];
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode_component(v)))
        .collect();
    format!("{AUTHORIZE_URL}?{}", query.join("&"))
}

/// pi `parseAuthorizationInput` → `(code, state)`.
fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }
    if let Some((_, query)) = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .and_then(|rest| rest.split_once('?'))
    {
        let query = query.split('#').next().unwrap_or(query);
        return (query_param(query, "code"), query_param(query, "state"));
    }
    if let Some((code, state)) = value.split_once('#') {
        return (Some(code.to_owned()), Some(state.to_owned()));
    }
    if value.contains("code=") {
        return (query_param(value, "code"), query_param(value, "state"));
    }
    (Some(value.to_owned()), None)
}

async fn post_token(body: &Value) -> Result<Value> {
    let response = http::post(TOKEN_URL)
        .header("accept", "application/json")
        .timeout_ms(30_000)
        .json(body)?
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await;
    if !(200..300).contains(&status) {
        return Err(Error::msg(format!(
            "HTTP request failed. status={status}; url={TOKEN_URL}; body={text}"
        )));
    }
    serde_json::from_str(&text).map_err(|e| {
        Error::msg(format!(
            "Token response was invalid JSON. url={TOKEN_URL}; body={text}; details={e}"
        ))
    })
}

fn credential(token: &Value) -> Value {
    let expires_in = token["expires_in"].as_i64().unwrap_or(0);
    json!({
        "type": "oauth",
        "refresh": token["refresh_token"],
        "access": token["access_token"],
        "expires": timers::now_ms() + expires_in * 1000 - 5 * 60 * 1000,
    })
}

async fn exchange(code: &str, state: &str, verifier: &str, redirect_uri: &str) -> Result<Value> {
    let token = post_token(&json!({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "state": state,
        "redirect_uri": redirect_uri,
        "code_verifier": verifier,
    }))
    .await
    .map_err(|e| {
        Error::msg(format!(
            "Token exchange request failed. url={TOKEN_URL}; redirect_uri={redirect_uri}; response_type=authorization_code; details={e}"
        ))
    })?;
    Ok(credential(&token))
}

/// `oauth-login`: asks for the login method, then runs the browser or copy-code flow.
pub(crate) async fn login() -> Result<Value> {
    let method = ui::select("Select Anthropic login method:", &[BROWSER, COPY_CODE])
        .await?
        .ok_or_else(|| Error::msg("Login cancelled"))?;
    if method == COPY_CODE {
        return login_copy_code().await;
    }
    login_browser().await
}

async fn login_copy_code() -> Result<Value> {
    let pkce = pkce();
    let url = authorize_url(COPY_CODE_REDIRECT_URI, &pkce);
    let _ = ui::open_url(&url);
    ui::notify(
        &format!(
            "Complete login in your browser, then copy the code Anthropic shows and paste it here.\n{url}"
        ),
        "info",
    )?;
    let input = ui::input(
        "Paste the code Anthropic shows after you sign in:",
        Some("code#state"),
    )
    .await?
    .unwrap_or_default();
    let (code, state) = parse_authorization_input(&input);
    if state.as_deref().is_some_and(|s| s != pkce.verifier) {
        return Err(Error::msg("OAuth state mismatch"));
    }
    let code = code.ok_or_else(|| Error::msg("Missing authorization code"))?;
    exchange(
        &code,
        state.as_deref().unwrap_or(&pkce.verifier),
        &pkce.verifier,
        COPY_CODE_REDIRECT_URI,
    )
    .await
}

async fn login_browser() -> Result<Value> {
    let pkce = pkce();
    let listener = Listener::bind(CALLBACK_PORT)
        .or_else(|_| Listener::bind(0))
        .ok();
    let port = listener.as_ref().map_or(CALLBACK_PORT, Listener::port);
    let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");
    let url = authorize_url(&redirect_uri, &pkce);
    let _ = ui::open_url(&url);
    ui::notify(
        &format!(
            "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here.\n{url}"
        ),
        "info",
    )?;
    let manual = ui::input(
        "Complete login in your browser, or paste the authorization code / redirect URL here:",
        Some(&redirect_uri),
    );
    let (code, state) = match &listener {
        Some(listener) => {
            let callback = wait_for_callback(listener, &pkce.verifier);
            match future::select(Box::pin(callback), Box::pin(manual)).await {
                Either::Left((result, _)) => (Some(result?), None),
                Either::Right((input, _)) => parse_authorization_input(&input?.unwrap_or_default()),
            }
        }
        None => parse_authorization_input(&manual.await?.unwrap_or_default()),
    };
    if state.as_deref().is_some_and(|s| s != pkce.verifier) {
        return Err(Error::msg("OAuth state mismatch"));
    }
    let code = code.ok_or_else(|| Error::msg("Missing authorization code"))?;
    exchange(
        &code,
        state.as_deref().unwrap_or(&pkce.verifier),
        &pkce.verifier,
        &redirect_uri,
    )
    .await
}

async fn wait_for_callback(listener: &Listener, verifier: &str) -> Result<String> {
    loop {
        let incoming = listener
            .accept()
            .await
            .ok_or_else(|| Error::msg("OAuth callback server closed"))?;
        let (path, query) = incoming
            .path
            .split_once('?')
            .unwrap_or((&incoming.path, ""));
        if path != CALLBACK_PATH {
            let _ = listener.respond(incoming.id, 404, &[], b"Not found");
            continue;
        }
        let state = query_param(query, "state");
        let page = |text: &str| {
            format!("<!doctype html><html><body><p>{text}</p></body></html>").into_bytes()
        };
        let headers = vec![(
            "content-type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        )];
        if state.as_deref() != Some(verifier) {
            let _ = listener.respond(incoming.id, 400, &headers, &page("OAuth state mismatch."));
            return Err(Error::msg("OAuth state mismatch"));
        }
        let Some(code) = query_param(query, "code") else {
            let _ = listener.respond(
                incoming.id,
                400,
                &headers,
                &page("Missing authorization code."),
            );
            return Err(Error::msg("Missing authorization code"));
        };
        let _ = listener.respond(
            incoming.id,
            200,
            &headers,
            &page("Anthropic login complete. You can close this window and return to Oino."),
        );
        return Ok(code);
    }
}

/// `oauth-refresh`.
pub(crate) async fn refresh(credential_json: &Value) -> Result<Value> {
    let refresh_token = credential_json["refresh"]
        .as_str()
        .ok_or_else(|| Error::msg("credential has no refresh token"))?;
    let token = post_token(&json!({
        "grant_type": "refresh_token",
        "client_id": CLIENT_ID,
        "refresh_token": refresh_token,
    }))
    .await
    .map_err(|e| {
        Error::msg(format!(
            "Anthropic token refresh request failed. url={TOKEN_URL}; details={e}"
        ))
    })?;
    Ok(credential(&token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authorization_input() {
        assert_eq!(
            parse_authorization_input("http://localhost:1/callback?code=a%2Bb&state=s"),
            (Some("a+b".into()), Some("s".into()))
        );
        assert_eq!(
            parse_authorization_input("abc#xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(parse_authorization_input("abc"), (Some("abc".into()), None));
    }
}
