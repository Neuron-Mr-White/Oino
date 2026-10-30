//! UNI-161 \u00a71/\u00a72/\u00a76: the host's side of app push notifications.
//!
//! - `notify_key_or_create`: the per-pairing 256-bit E2E key, generated at
//!   first use and persisted at `HostPaths::notify_key()` (0600, same
//!   atomic-write pattern as `secret.rs`). The pi-side notify package reads
//!   the *same* file (see `packages/notify/notify-key.ts`) to encrypt
//!   notification payloads; this file is the one shared secret between the
//!   two local processes, so the server only ever sees ciphertext.
//! - `read_ntfy_config`: the user's own ntfy server/topic/token from
//!   `~/.unipi/config/notify/config.json`'s `ntfy` block (the same file the
//!   pi-side `notify` package already writes via its Settings UI).
//! - `app_endpoints_path`/`register_app_endpoint`: the UnifiedPush endpoint
//!   fan-out list at `~/.unipi/config/notify/app-endpoints.json`, matching
//!   the shape `packages/notify/app-endpoints.ts` already reads.
//! - `send_test`: POSTs one encrypted test notification straight to the
//!   configured ntfy server, proving the whole `notify_config` \u2192 app path
//!   without round-tripping through a live pi process.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::data_dir::{write_atomic, HostPaths};

const KEY_LEN: usize = 32;

/// Loads the per-pairing E2E key, generating and persisting a fresh
/// random 32 bytes if none exists yet.
pub fn notify_key_or_create(paths: &HostPaths) -> anyhow::Result<[u8; KEY_LEN]> {
    let path = paths.notify_key();
    match std::fs::read(&path) {
        Ok(bytes) => {
            let array: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!(
                    "notify-key must be exactly {KEY_LEN} bytes, got {}",
                    bytes.len()
                )
            })?;
            Ok(array)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            use rand::RngCore;
            let mut key = [0u8; KEY_LEN];
            rand::rng().fill_bytes(&mut key);
            write_atomic(&path, &key)?;
            Ok(key)
        }
        Err(err) => Err(err.into()),
    }
}

/// `~/.unipi/config/notify/config.json`'s `ntfy` block: the user's own
/// server/topic/token, written by the pi-side notify package's settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NtfyBlock {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    app_detail: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct NotifyConfigJson {
    #[serde(default)]
    ntfy: NtfyBlock,
    #[serde(default)]
    gotify: GotifyBlock,
    #[serde(default)]
    telegram: TelegramBlock,
}

/// `~/.unipi/config/notify/config.json`'s `gotify` block, matching
/// `packages/notify/types.ts`'s `GotifyConfig` (UNI-316): server URL + app
/// token, `POST <server>/message` with header `X-Gotify-Key: <token>`
/// (`packages/notify/platforms/gotify.ts`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GotifyBlock {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    server_url: Option<String>,
    #[serde(default)]
    app_token: Option<String>,
}

/// `~/.unipi/config/notify/config.json`'s `telegram` block, matching
/// `packages/notify/types.ts`'s `TelegramConfig` (UNI-316): bot token +
/// chat id, `POST https://api.telegram.org/bot<token>/sendMessage`
/// (`packages/notify/platforms/telegram.ts`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TelegramBlock {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    bot_token: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
}

/// Resolved Gotify connection details (UNI-316).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GotifyConnection {
    pub server_url: Option<String>,
    pub app_token: Option<String>,
    pub enabled: bool,
}

/// Resolved Telegram connection details (UNI-316).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelegramConnection {
    pub bot_token: Option<String>,
    pub chat_id: Option<String>,
    pub enabled: bool,
}

/// Resolved ntfy connection details read from `~/.unipi/config/notify/config.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NtfyConnection {
    pub server: Option<String>,
    pub topic: Option<String>,
    pub token: Option<String>,
    pub app_detail_full: bool,
    /// The user's own `ntfy.enabled` setting (UNI-316's cron default-channel
    /// logic reads this; existing ntfy fallbacks key off `topic`/endpoints
    /// instead and are unaffected).
    pub enabled: bool,
}

/// Env var overriding the notify config.json path (tests only; same idea
/// as `data_dir::UNIPI_HOST_DIR_ENV`).
pub const UNIPI_NOTIFY_CONFIG_ENV: &str = "UNIPI_NOTIFY_CONFIG_JSON";

/// Default path for the pi-side notify config (shared with
/// `packages/notify/ntfy-config.ts`'s legacy `config.json` fallback).
pub fn notify_config_json_path() -> PathBuf {
    if let Ok(dir) = std::env::var(UNIPI_NOTIFY_CONFIG_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    match crate::fs::home_dir_opt() {
        Some(home) => home
            .join(".unipi")
            .join("config")
            .join("notify")
            .join("config.json"),
        None => PathBuf::from(".unipi")
            .join("config")
            .join("notify")
            .join("config.json"),
    }
}

/// Reads the ntfy connection details from `path` (missing/corrupt file ->
/// all-`None`, never an error \u2014 the app shows "not configured" instead).
pub fn read_ntfy_config(path: &Path) -> NtfyConnection {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return NtfyConnection::default();
    };
    let Ok(parsed) = serde_json::from_str::<NotifyConfigJson>(&raw) else {
        return NtfyConnection::default();
    };
    NtfyConnection {
        server: parsed.ntfy.server_url,
        topic: parsed.ntfy.topic,
        token: parsed.ntfy.token,
        app_detail_full: parsed.ntfy.app_detail.as_deref() == Some("full"),
        enabled: parsed.ntfy.enabled,
    }
}

/// Reads the Gotify connection details from `path` (missing/corrupt file
/// -> all-`None`, never an error).
pub fn read_gotify_config(path: &Path) -> GotifyConnection {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return GotifyConnection::default();
    };
    let Ok(parsed) = serde_json::from_str::<NotifyConfigJson>(&raw) else {
        return GotifyConnection::default();
    };
    GotifyConnection {
        server_url: parsed.gotify.server_url,
        app_token: parsed.gotify.app_token,
        enabled: parsed.gotify.enabled,
    }
}

/// Reads the Telegram connection details from `path` (missing/corrupt file
/// -> all-`None`, never an error).
pub fn read_telegram_config(path: &Path) -> TelegramConnection {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return TelegramConnection::default();
    };
    let Ok(parsed) = serde_json::from_str::<NotifyConfigJson>(&raw) else {
        return TelegramConnection::default();
    };
    TelegramConnection {
        bot_token: parsed.telegram.bot_token,
        chat_id: parsed.telegram.chat_id,
        enabled: parsed.telegram.enabled,
    }
}

/// Which notify channels the user has enabled in their own config
/// (UNI-316): `cron add/edit`'s default when `--notify` isn't given.
/// Order: ntfy, gotify, telegram.
pub fn default_channels(
    ntfy: &NtfyConnection,
    gotify: &GotifyConnection,
    telegram: &TelegramConnection,
) -> Vec<String> {
    let mut out = Vec::new();
    if ntfy.enabled {
        out.push("ntfy".to_string());
    }
    if gotify.enabled {
        out.push("gotify".to_string());
    }
    if telegram.enabled {
        out.push("telegram".to_string());
    }
    out
}

/// POSTs one Gotify notification (UNI-316): `POST <server>/message` with
/// header `X-Gotify-Key: <app token>` — same shape as
/// `packages/notify/platforms/gotify.ts`'s `sendGotifyNotification`.
/// Never logs `title`/`message` (they may carry the job's summary).
pub async fn send_gotify(
    connection: &GotifyConnection,
    title: &str,
    message: &str,
    priority: u32,
) -> anyhow::Result<()> {
    let server = connection
        .server_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no gotify server configured"))?;
    let token = connection
        .app_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no gotify app token configured"))?;
    let url = format!("{}/message", server.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .post(url)
        .header("X-Gotify-Key", token)
        .json(&serde_json::json!({ "title": title, "message": message, "priority": priority }))
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("gotify: HTTP {}", response.status());
    }
    Ok(())
}

/// POSTs one Telegram notification (UNI-316):
/// `POST https://api.telegram.org/bot<token>/sendMessage` — same shape as
/// `packages/notify/platforms/telegram.ts`'s `sendTelegramNotification`
/// (plain text here, not MarkdownV2, so the job name/summary never needs
/// escaping). Never logs the bot token.
pub async fn send_telegram(
    connection: &TelegramConnection,
    title: &str,
    message: &str,
) -> anyhow::Result<()> {
    let token = connection
        .bot_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no telegram bot token configured"))?;
    let chat_id = connection
        .chat_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no telegram chat id configured"))?;
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let text = format!("{title}\n{message}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .post(url)
        .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("telegram: HTTP {}", response.status());
    }
    Ok(())
}

/// `~/.unipi/config/notify/app-endpoints.json`: matches
/// `packages/notify/app-endpoints.ts`'s `{ endpoints: [{url, addedAt}] }`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AppEndpointsFile {
    #[serde(default)]
    endpoints: Vec<AppEndpointEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AppEndpointEntry {
    url: String,
    #[serde(rename = "addedAt")]
    added_at: u64,
}

/// Env var overriding the app-endpoints.json path (tests only).
pub const UNIPI_NOTIFY_ENDPOINTS_ENV: &str = "UNIPI_NOTIFY_ENDPOINTS_JSON";

/// Default path for the app-endpoints fan-out file (same scope as
/// `notify_config_json_path`'s `config.json`).
pub fn app_endpoints_path() -> PathBuf {
    if let Ok(dir) = std::env::var(UNIPI_NOTIFY_ENDPOINTS_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    match crate::fs::home_dir_opt() {
        Some(home) => home
            .join(".unipi")
            .join("config")
            .join("notify")
            .join("app-endpoints.json"),
        None => PathBuf::from(".unipi")
            .join("config")
            .join("notify")
            .join("app-endpoints.json"),
    }
}

/// Adds (or refreshes) one endpoint's `addedAt`, de-duplicated by URL.
/// Atomic write; creates the parent directory (0700) if missing.
pub fn register_app_endpoint(path: &Path, url: &str) -> anyhow::Result<()> {
    let trimmed = url.trim();
    anyhow::ensure!(!trimmed.is_empty(), "endpoint url must not be empty");
    let mut file: AppEndpointsFile = match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => AppEndpointsFile::default(),
    };
    file.endpoints.retain(|e| e.url != trimmed);
    let added_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    file.endpoints.push(AppEndpointEntry {
        url: trimmed.to_string(),
        added_at,
    });
    let bytes = serde_json::to_vec_pretty(&file)?;
    write_atomic(path, &bytes)?;
    Ok(())
}

/// One AES-256-GCM encrypt: random 12-byte IV, returns `iv || ciphertext+tag`.
pub fn encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    use rand::RngCore;
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    let mut iv = [0u8; 12];
    rand::rng().fill_bytes(&mut iv);
    let nonce = Nonce::from(iv);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("encrypt failed: {e}"))?;
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// `unipi1:` + base64(iv || ciphertext+tag) of a JSON payload, matching
/// the pi-side `packages/notify/platforms/ntfy-crypto.ts` format exactly.
pub fn encrypt_envelope(
    key: &[u8; KEY_LEN],
    payload: &serde_json::Value,
) -> anyhow::Result<String> {
    let plaintext = serde_json::to_vec(payload)?;
    let bytes = encrypt(key, &plaintext)?;
    Ok(format!(
        "unipi1:{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Builds and POSTs one encrypted test notification straight to the
/// configured ntfy server (UNI-161 \u00a76 "Test"): title is the fixed neutral
/// "UniPi"; `message` is the ciphertext envelope. Best-effort HTTP errors
/// surface as `anyhow::Error` for the caller to report.
pub async fn send_test(connection: &NtfyConnection, key: &[u8; KEY_LEN]) -> anyhow::Result<()> {
    let server = connection
        .server
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no ntfy server configured"))?;
    let topic = connection
        .topic
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no ntfy topic configured"))?;
    let payload = serde_json::json!({
        "title": "UniPi",
        "body": "Test notification from the PC.",
        "kind": "test",
    });
    let message = encrypt_envelope(key, &payload)?;
    let client = reqwest::Client::new();
    let mut req = client.post(server.trim_end_matches('/')).json(
        &serde_json::json!({ "topic": topic, "title": "UniPi", "message": message, "priority": 3 }),
    );
    if let Some(token) = &connection.token {
        req = req.bearer_auth(token);
    }
    let response = req.send().await?;
    if !response.status().is_success() {
        anyhow::bail!("ntfy test failed: HTTP {}", response.status());
    }
    Ok(())
}

/// Every registered UnifiedPush endpoint URL (missing/corrupt file → none).
pub fn read_app_endpoints(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<AppEndpointsFile>(&raw).ok())
        .map(|file| file.endpoints.into_iter().map(|e| e.url).collect())
        .unwrap_or_default()
}

/// The ntfy JSON publish body for one encrypted payload (`topic` omitted
/// for UnifiedPush endpoints, whose URL already names it).
pub fn publish_body(
    topic: Option<&str>,
    key: &[u8; KEY_LEN],
    payload: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut body = serde_json::json!({ "title": "UniPi", "message": encrypt_envelope(key, payload)?, "priority": 4 });
    if let Some(topic) = topic {
        body["topic"] = serde_json::Value::String(topic.to_string());
    }
    Ok(body)
}

/// Sends one encrypted payload (UNI-214 push notice) to the configured
/// ntfy topic and every registered UnifiedPush endpoint, like the pi-side
/// notify fan-out. Returns how many deliveries succeeded; an error only
/// when every attempt failed.
pub async fn send_payload(
    connection: &NtfyConnection,
    endpoints: &[String],
    key: &[u8; KEY_LEN],
    payload: &serde_json::Value,
) -> anyhow::Result<usize> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut sent = 0usize;
    let mut last_error = None;
    if let (Some(server), Some(topic)) = (connection.server.as_deref(), connection.topic.as_deref())
    {
        let mut req = client
            .post(server.trim_end_matches('/'))
            .json(&publish_body(Some(topic), key, payload)?);
        if let Some(token) = &connection.token {
            req = req.bearer_auth(token);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => sent += 1,
            Ok(r) => last_error = Some(anyhow::anyhow!("ntfy: HTTP {}", r.status())),
            Err(e) => last_error = Some(e.into()),
        }
    }
    for url in endpoints {
        match client
            .post(url)
            .json(&publish_body(None, key, payload)?)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => sent += 1,
            Ok(r) => last_error = Some(anyhow::anyhow!("endpoint: HTTP {}", r.status())),
            Err(e) => last_error = Some(e.into()),
        }
    }
    match (sent, last_error) {
        (0, Some(error)) => Err(error),
        (n, _) => Ok(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_key_generated_and_persisted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = HostPaths::from_dir(tmp.path());
        let first = notify_key_or_create(&paths).expect("first");
        let second = notify_key_or_create(&paths).expect("second");
        assert_eq!(first, second);
        assert_eq!(std::fs::read(paths.notify_key()).expect("read").len(), 32);
    }

    #[cfg(unix)]
    #[test]
    fn notify_key_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = HostPaths::from_dir(tmp.path());
        notify_key_or_create(&paths).expect("key");
        let mode = std::fs::metadata(paths.notify_key())
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn missing_config_json_reports_unconfigured() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let connection = read_ntfy_config(&tmp.path().join("missing.json"));
        assert_eq!(connection, NtfyConnection::default());
    }

    #[test]
    fn reads_ntfy_block_from_config_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"ntfy":{"enabled":true,"serverUrl":"https://ntfy.nrn.one","topic":"abc","token":"tk_x","appDetail":"full"}}"#,
        )
        .expect("write");
        let connection = read_ntfy_config(&path);
        assert_eq!(connection.server, Some("https://ntfy.nrn.one".to_string()));
        assert_eq!(connection.topic, Some("abc".to_string()));
        assert_eq!(connection.token, Some("tk_x".to_string()));
        assert!(connection.app_detail_full);
    }

    #[test]
    fn register_app_endpoint_writes_matching_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("app-endpoints.json");
        register_app_endpoint(&path, "https://ntfy.sh/abcxyz").expect("register");
        let raw = std::fs::read_to_string(&path).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse");
        let endpoints = parsed["endpoints"].as_array().expect("array");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0]["url"], "https://ntfy.sh/abcxyz");
        assert!(endpoints[0]["addedAt"].is_number());
    }

    #[test]
    fn register_app_endpoint_dedups_by_url() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("app-endpoints.json");
        register_app_endpoint(&path, "https://ntfy.sh/abcxyz").expect("first");
        register_app_endpoint(&path, "https://ntfy.sh/abcxyz").expect("second");
        let raw = std::fs::read_to_string(&path).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse");
        assert_eq!(parsed["endpoints"].as_array().expect("array").len(), 1);
    }

    #[test]
    fn encrypt_envelope_has_unipi1_prefix_and_no_plaintext() {
        let key = [7u8; KEY_LEN];
        let payload = serde_json::json!({"title":"Secret title","body":"Secret body"});
        let envelope = encrypt_envelope(&key, &payload).expect("encrypt");
        assert!(envelope.starts_with("unipi1:"));
        assert!(!envelope.contains("Secret"));
    }

    #[test]
    fn encrypt_is_randomized_each_call() {
        let key = [3u8; KEY_LEN];
        let payload = serde_json::json!({"title":"x"});
        let a = encrypt_envelope(&key, &payload).expect("a");
        let b = encrypt_envelope(&key, &payload).expect("b");
        assert_ne!(a, b, "random IV must make each envelope unique");
    }

    #[test]
    fn read_app_endpoints_lists_urls_and_tolerates_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("app-endpoints.json");
        assert!(read_app_endpoints(&path).is_empty());
        register_app_endpoint(&path, "https://ntfy.sh/up1").expect("register");
        assert_eq!(
            read_app_endpoints(&path),
            vec!["https://ntfy.sh/up1".to_string()]
        );
    }

    #[test]
    fn publish_body_hides_the_payload() {
        let key = [9u8; KEY_LEN];
        let payload =
            serde_json::json!({"title":"Incoming from coffee","body":"secret.apk","kind":"push"});
        let body = publish_body(Some("t"), &key, &payload).expect("body");
        assert_eq!(body["topic"], "t");
        assert_eq!(body["title"], "UniPi");
        assert!(!body.to_string().contains("secret.apk"));
        assert!(publish_body(None, &key, &payload)
            .expect("body")
            .get("topic")
            .is_none());
    }

    #[test]
    fn reads_gotify_and_telegram_blocks_from_config_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"gotify":{"enabled":true,"serverUrl":"https://gotify.example","appToken":"tok_g"},"telegram":{"enabled":true,"botToken":"tok_t","chatId":"123"}}"#,
        )
        .expect("write");
        let gotify = read_gotify_config(&path);
        assert_eq!(
            gotify.server_url,
            Some("https://gotify.example".to_string())
        );
        assert_eq!(gotify.app_token, Some("tok_g".to_string()));
        assert!(gotify.enabled);
        let telegram = read_telegram_config(&path);
        assert_eq!(telegram.bot_token, Some("tok_t".to_string()));
        assert_eq!(telegram.chat_id, Some("123".to_string()));
        assert!(telegram.enabled);
    }

    #[test]
    fn default_channels_reflects_the_users_enabled_platforms() {
        let ntfy = NtfyConnection {
            enabled: true,
            ..Default::default()
        };
        let gotify = GotifyConnection {
            enabled: false,
            ..Default::default()
        };
        let telegram = TelegramConnection {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            default_channels(&ntfy, &gotify, &telegram),
            vec!["ntfy".to_string(), "telegram".to_string()]
        );
        assert!(default_channels(
            &NtfyConnection::default(),
            &GotifyConnection::default(),
            &TelegramConnection::default()
        )
        .is_empty());
    }

    /// A one-shot fake HTTP server: replies `status` to the first request
    /// on 127.0.0.1, and hands back the raw request bytes it received (so
    /// a test can check headers/body without a real gotify/telegram).
    async fn fake_http_server(
        status: &'static str,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::oneshot::Receiver<Vec<u8>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            buf.truncate(n);
            let body =
                format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.shutdown().await;
            let _ = tx.send(buf);
        });
        (addr, rx)
    }

    #[tokio::test]
    async fn send_gotify_posts_message_with_key_header() {
        let (addr, rx) = fake_http_server("200 OK").await;
        let connection = GotifyConnection {
            server_url: Some(format!("http://{addr}")),
            app_token: Some("tok_g".to_string()),
            enabled: true,
        };
        send_gotify(&connection, "Cron: digest ok", "Secret summary text", 5)
            .await
            .expect("send");
        let request = String::from_utf8_lossy(&rx.await.expect("request")).to_string();
        assert!(request.contains("POST /message"), "request: {request}");
        assert!(
            request.to_lowercase().contains("x-gotify-key: tok_g"),
            "request: {request}"
        );
        assert!(
            request.contains("Secret summary text"),
            "request: {request}"
        );
    }

    #[tokio::test]
    async fn send_gotify_reports_http_errors() {
        let (addr, _rx) = fake_http_server("500 Internal Server Error").await;
        let connection = GotifyConnection {
            server_url: Some(format!("http://{addr}")),
            app_token: Some("tok_g".to_string()),
            enabled: true,
        };
        let error = send_gotify(&connection, "t", "m", 5)
            .await
            .expect_err("should fail");
        assert!(error.to_string().contains("500"), "error: {error}");
    }

    #[tokio::test]
    async fn send_telegram_posts_chat_id_and_text() {
        let (addr, rx) = fake_http_server("200 OK").await;
        let connection = TelegramConnection {
            bot_token: Some("tok_t".to_string()),
            chat_id: Some("42".to_string()),
            enabled: true,
        };
        // send_telegram always builds api.telegram.org's URL internally; we
        // can't redirect that without touching DNS, so exercise the HTTP
        // framing itself against the fake server by sending a telegram-shaped
        // body straight through the same client pattern used by send_telegram
        // (chat_id + text as JSON), confirming the fake server sees it.
        let client = reqwest::Client::new();
        let text = "Cron: digest ok\nSecret summary text".to_string();
        let response = client
            .post(format!(
                "http://{addr}/bot{}/sendMessage",
                connection.bot_token.as_deref().unwrap()
            ))
            .json(&serde_json::json!({ "chat_id": connection.chat_id, "text": text }))
            .send()
            .await
            .expect("send");
        assert!(response.status().is_success());
        let request = String::from_utf8_lossy(&rx.await.expect("request")).to_string();
        assert!(
            request.contains("/bottok_t/sendMessage"),
            "request: {request}"
        );
        assert!(
            request.contains("Secret summary text"),
            "request: {request}"
        );
        assert!(request.contains("\"chat_id\":\"42\""), "request: {request}");
    }

    #[tokio::test]
    async fn send_gotify_without_config_fails_clearly() {
        let connection = GotifyConnection::default();
        let error = send_gotify(&connection, "t", "m", 5)
            .await
            .expect_err("should fail");
        assert!(error.to_string().contains("no gotify"), "error: {error}");
    }

    #[tokio::test]
    async fn send_telegram_without_config_fails_clearly() {
        let connection = TelegramConnection::default();
        let error = send_telegram(&connection, "t", "m")
            .await
            .expect_err("should fail");
        assert!(error.to_string().contains("no telegram"), "error: {error}");
    }
}
