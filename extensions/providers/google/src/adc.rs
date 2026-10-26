//! Application Default Credentials for Vertex AI (what `google-auth-library` does for pi):
//! reads the ADC file (`GOOGLE_APPLICATION_CREDENTIALS` or
//! `~/.config/gcloud/application_default_credentials.json`, through the `gcloud` preopen) and
//! mints an OAuth access token for `authorized_user` (refresh-token grant) and
//! `service_account` (RS256 JWT-bearer grant) credentials. Tokens are cached until a minute
//! before they expire.

use std::cell::RefCell;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use oino_extension::timers;
use oino_provider_kit::ProviderRequest;
use oino_provider_kit::guest::oauth::post_form;
use rsa::RsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::sha2::Sha256;
use rsa::signature::{SignatureEncoding, Signer};
use serde_json::{Value, json};

const DEFAULT_ADC: &str = "/gcloud/application_default_credentials.json";
const GCLOUD_DIR: &str = ".config/gcloud/";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

thread_local! {
    /// `(credentials file contents, token, expiry ms)`.
    static CACHE: RefCell<Option<(String, String, i64)>> = const { RefCell::new(None) };
}

/// Guest path of the ADC file.
fn adc_path(request: &ProviderRequest) -> Result<String, String> {
    let Some(configured) = request.env_value("GOOGLE_APPLICATION_CREDENTIALS") else {
        return Ok(DEFAULT_ADC.to_owned());
    };
    let home = request
        .env
        .get("HOME")
        .cloned()
        .or_else(|| oino_extension::host::env("HOME").ok().flatten())
        .unwrap_or_default();
    let relative = configured
        .strip_prefix("~/")
        .or_else(|| {
            (!home.is_empty())
                .then(|| configured.strip_prefix(&format!("{}/", home.trim_end_matches('/'))))
                .flatten()
        })
        .and_then(|rest| rest.strip_prefix(GCLOUD_DIR));
    relative.map(|rest| format!("/gcloud/{rest}")).ok_or_else(|| {
        format!(
            "GOOGLE_APPLICATION_CREDENTIALS must point into ~/.config/gcloud (the only directory the Vertex component may read): {configured}"
        )
    })
}

fn jwt_assertion(key: &Value, now_s: i64) -> Result<String, String> {
    let email = key["client_email"]
        .as_str()
        .ok_or("service account credentials have no client_email")?;
    let pem = key["private_key"]
        .as_str()
        .ok_or("service account credentials have no private_key")?;
    let token_uri = key["token_uri"].as_str().unwrap_or(TOKEN_URL);
    let private = RsaPrivateKey::from_pkcs8_pem(pem)
        .map_err(|e| format!("invalid service account private key: {e}"))?;
    let header = URL_SAFE_NO_PAD.encode(json!({"alg": "RS256", "typ": "JWT"}).to_string());
    let claims = URL_SAFE_NO_PAD.encode(
        json!({"iss": email, "scope": SCOPE, "aud": token_uri, "iat": now_s, "exp": now_s + 3600})
            .to_string(),
    );
    let input = format!("{header}.{claims}");
    let signature = SigningKey::<Sha256>::new(private).sign(input.as_bytes());
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

async fn mint(key: &Value) -> Result<(String, i64), String> {
    let now = timers::now_ms();
    let reply = match key["type"].as_str() {
        Some("authorized_user") => {
            let field = |name: &str| {
                key[name]
                    .as_str()
                    .ok_or_else(|| format!("authorized_user credentials have no {name}"))
            };
            post_form(
                TOKEN_URL,
                &[("accept", "application/json")],
                &[
                    ("client_id", field("client_id")?),
                    ("client_secret", field("client_secret")?),
                    ("refresh_token", field("refresh_token")?),
                    ("grant_type", "refresh_token"),
                ],
            )
            .await
        }
        Some("service_account") => {
            let assertion = jwt_assertion(key, now / 1000)?;
            let token_uri = key["token_uri"].as_str().unwrap_or(TOKEN_URL);
            post_form(
                token_uri,
                &[("accept", "application/json")],
                &[("grant_type", JWT_BEARER), ("assertion", &assertion)],
            )
            .await
        }
        other => {
            return Err(format!(
                "Unsupported Application Default Credentials type: {}",
                other.unwrap_or("unknown")
            ));
        }
    }
    .map_err(|e| format!("Could not refresh access token: {e}"))?;
    if !reply.ok() {
        return Err(format!(
            "Could not refresh access token: {} {}",
            reply.status, reply.text
        ));
    }
    let body = reply.json();
    let token = body["access_token"]
        .as_str()
        .ok_or("Could not refresh access token: no access_token in the response")?;
    #[allow(clippy::cast_possible_truncation, reason = "seconds to ms")]
    let expires = now + (body["expires_in"].as_f64().unwrap_or(3600.0) * 1000.0) as i64;
    Ok((token.to_owned(), expires))
}

/// An access token for `request`'s Vertex call.
pub(crate) async fn access_token(request: &ProviderRequest) -> Result<String, String> {
    let path = adc_path(request)?;
    let contents = std::fs::read_to_string(&path).map_err(|_| {
        "Vertex AI Application Default Credentials not found: run `gcloud auth application-default login` or set GOOGLE_APPLICATION_CREDENTIALS".to_owned()
    })?;
    let now = timers::now_ms();
    let cached = CACHE.with(|cache| {
        cache.borrow().as_ref().and_then(|(file, token, expiry)| {
            (file == &contents && *expiry - 60_000 > now).then(|| token.clone())
        })
    });
    if let Some(token) = cached {
        return Ok(token);
    }
    let key: Value = serde_json::from_str(&contents)
        .map_err(|e| format!("invalid Application Default Credentials file {path}: {e}"))?;
    let (token, expiry) = mint(&key).await?;
    CACHE.with(|cache| *cache.borrow_mut() = Some((contents, token.clone(), expiry)));
    Ok(token)
}
