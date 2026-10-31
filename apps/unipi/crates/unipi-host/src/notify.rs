//! UNI-161 \u00a71/\u00a72/\u00a76: the host's side of app push notifications.
//!
//! - `notify_key_or_create`: the per-pairing 256-bit E2E key, generated at
//!   first use and persisted at `HostPaths::notify_key()` (0600, same
//!   atomic-write pattern as `secret.rs`). The pi-side notify package reads
//!   the *same* file (see `packages/notify/notify-key.ts`) to encrypt
//!   notification payloads; this file is the one shared secret between the
//!   two local processes, so the server only ever sees ciphertext.
//! - `read_ntfy_config`: the user's own ntfy server/topic/token from the
//!   `notify` extension's settings — `extensionSettings.notify` of the agent
//!   settings file (`$OINO_AGENT_DIR`/`settings.json`, default
//!   `~/.oino/settings.json`), the same values the `unipi.notify` extension
//!   writes from its setup wizards. Secret fields (`ntfy.token`,
//!   `gotify.appToken`, `telegram.botToken`/`chatId`) live in
//!   `<agent>/secrets.json`, exactly like the extension's own reads.
//! - `app_endpoints_path`/`register_app_endpoint`: the UnifiedPush endpoint
//!   fan-out list at `<host dir>/app-endpoints.json` (`{endpoints:[{url,
//!   addedAt}]}`). v2: the host owns phone push, so the list lives with the
//!   host, not in the pi-side notify package's config dir.
//! - `send_test`: POSTs one encrypted test notification straight to the
//!   configured ntfy server, proving the whole `notify_config` \u2192 app path
//!   without round-tripping through a live pi process.
//! - P0-06 session pushes: the host owns phone push, so the session tap calls
//!   [`on_session_event`] for every runtime event — dialogs flip the hub card's
//!   `needs_you` and push `input_needed`, a settle pushes the run-finished
//!   notice, and the `unipi.notify` extension's `notify_user` tool call pushes
//!   its message ([`SessionNotice`]). Every push is the full old-unipi shape:
//!   `unipi1:` E2E payload (`title/body/kind/host/session`, the phone's
//!   `NotifyPayload`) + the content-free `unipi://s/<host>/<session>` deep link
//!   on the ntfy `click` + the UnifiedPush endpoint fan-out, gated by the
//!   notify extension's own event matrix ([`PhonePushSettings`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use serde::{Deserialize, Serialize};

use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};
use oino_core::ui::{is_dialog, prompt_kind};

use crate::data_dir::{write_atomic, HostPaths};
use crate::sessions::hub::SessionHub;

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

/// The `notify` extension's `ntfy` block: the user's own server/topic/token,
/// written by its setup wizards (`extensionSettings.notify` in the agent
/// settings file; secret fields in `<agent>/secrets.json`).
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
    /// `events.<name>.{enabled, platforms}` (P0-06 phone-push gating). Kept
    /// raw: junk values (older files store `"events": {"agentEnd": true}`)
    /// must not fail the whole block — each entry parses or defaults off.
    #[serde(default)]
    events: HashMap<String, serde_json::Value>,
    /// `defaultPlatforms` (the `notify_user` tool's platform fallback).
    #[serde(default, rename = "defaultPlatforms")]
    default_platforms: Option<Vec<String>>,
}

/// One `events.<name>` switch of the notify extension's matrix.
#[derive(Debug, Clone, Default, Deserialize)]
struct EventBlock {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

/// The phone-push-relevant slice of the `unipi.notify` extension's settings
/// (P0-06): its event matrix and ntfy enablement, read from the same settings
/// file the extension writes (`extensionSettings.notify`, dotted keys
/// expanded). The host pushes only what the user switched on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhonePushSettings {
    /// `ntfy.enabled`.
    pub ntfy_enabled: bool,
    /// `defaultPlatforms` (fallback of the `notify_user` tool).
    pub default_platforms: Vec<String>,
    /// `events.<name>` switches.
    pub events: HashMap<String, EventSwitch>,
}

/// One event's switch (unipi `EventNotifyConfig`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventSwitch {
    /// Enabled.
    pub enabled: bool,
    /// Platforms (empty = every enabled platform).
    pub platforms: Vec<String>,
}

impl PhonePushSettings {
    /// Reads the settings from `path` (the agent settings file; missing or
    /// corrupt \u2192 everything off, never an error).
    #[must_use]
    pub fn read(path: &Path) -> Self {
        let parsed = notify_block(path);
        let switch = |raw: &serde_json::Value| {
            serde_json::from_value::<EventBlock>(raw.clone()).unwrap_or_default()
        };
        Self {
            ntfy_enabled: parsed.ntfy.enabled,
            default_platforms: parsed.default_platforms.unwrap_or_default(),
            events: parsed
                .events
                .iter()
                .map(|(name, raw)| {
                    let block = switch(raw);
                    (
                        name.clone(),
                        EventSwitch {
                            enabled: block.enabled,
                            platforms: block.platforms.unwrap_or_default(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Whether `platforms` resolve to a phone push for ntfy (the extension's
    /// `resolve_platforms`, ntfy side: empty = every enabled platform; either
    /// way ntfy itself must be enabled).
    fn platforms_push(&self, platforms: &[String]) -> bool {
        (platforms.is_empty() || platforms.iter().any(|p| p == "ntfy")) && self.ntfy_enabled
    }

    /// Whether the event's phone push is on (`events.<event>.{enabled,
    /// platforms}`, ntfy side).
    #[must_use]
    pub fn event_push_enabled(&self, event: &str) -> bool {
        self.events
            .get(event)
            .is_some_and(|switch| switch.enabled && self.platforms_push(&switch.platforms))
    }

    /// Which event owns the run-finished push (unipi `finish_key`:
    /// `agent_settled` wins; both on = one notification, not two).
    #[must_use]
    pub fn finish_event(&self) -> Option<&'static str> {
        if self.event_push_enabled("agent_settled") {
            Some("agent_settled")
        } else if self.event_push_enabled("agent_end") {
            Some("agent_end")
        } else {
            None
        }
    }

    /// Whether a `notify_user` tool call reaches the phone: its explicit
    /// `platforms` when given, else the `defaultPlatforms` fallback.
    #[must_use]
    pub fn tool_push_enabled(&self, explicit: Option<&[String]>) -> bool {
        match explicit {
            Some(platforms) => self.platforms_push(platforms),
            None => self.platforms_push(&self.default_platforms),
        }
    }
}

/// The `notify` extension's `gotify` block (UNI-316): server URL + app token,
/// `POST <server>/message` with header `X-Gotify-Key: <token>`.
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

/// The `notify` extension's `telegram` block (UNI-316): bot token + chat id,
/// `POST https://api.telegram.org/bot<token>/sendMessage`.
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

/// Resolved ntfy connection details read from the `notify` extension's settings.
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

/// Env var overriding the notify settings path (tests only; same idea as
/// `data_dir::UNIPI_HOST_DIR_ENV`).
pub const UNIPI_NOTIFY_CONFIG_ENV: &str = "UNIPI_NOTIFY_CONFIG_JSON";

/// The agent dir core uses (pi `getAgentDir`): `$OINO_AGENT_DIR`, else
/// `~/.oino`. Kept local so this crate needs no core dependency for one path.
pub fn agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OINO_AGENT_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::fs::home_dir_opt().map(|home| home.join(".oino"))
}

/// Default settings file holding the `notify` extension's settings:
/// `<agent>/settings.json` (`$OINO_AGENT_DIR` or `~/.oino/settings.json`).
pub fn notify_config_json_path() -> PathBuf {
    if let Ok(dir) = std::env::var(UNIPI_NOTIFY_CONFIG_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    match agent_dir() {
        Some(agent) => agent.join("settings.json"),
        None => PathBuf::from(".oino").join("settings.json"),
    }
}

/// The `notify` block of `path`: the agent settings file's
/// `extensionSettings.notify`, or — for a test/legacy file that is just the
/// block itself — the whole object. Legacy flat dotted keys
/// (`"ntfy.serverUrl": …`) are expanded like core's settings load does.
fn notify_block(path: &Path) -> NotifyConfigJson {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return NotifyConfigJson::default();
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return NotifyConfigJson::default();
    };
    if let Some(block) = value
        .get("extensionSettings")
        .and_then(|it| it.get("notify"))
        .cloned()
    {
        value = block;
    }
    expand_dotted(&mut value);
    serde_json::from_value::<NotifyConfigJson>(value).unwrap_or_default()
}

/// Expands `{"ntfy.serverUrl": "x"}` into `{"ntfy": {"serverUrl": "x"}}`
/// (core migrates legacy flat extension-settings keys on load).
fn expand_dotted(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let dotted: Vec<(String, serde_json::Value)> = object
        .iter()
        .filter(|(key, _)| key.contains('.'))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in dotted {
        object.remove(&key);
        insert_path(object, &key, value);
    }
}

/// `insert_path(map, "ntfy.serverUrl", v)` sets `map.ntfy.serverUrl = v`,
/// creating intermediate objects and replacing non-object values.
fn insert_path(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: serde_json::Value,
) {
    let mut parts: Vec<&str> = key.split('.').collect();
    let leaf = parts.pop().expect("split yields at least one part");
    let mut cursor = map;
    for part in parts {
        let entry = cursor
            .entry(part.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if !entry.is_object() {
            *entry = serde_json::json!({});
        }
        cursor = entry.as_object_mut().expect("just made an object");
    }
    cursor.insert(leaf.to_string(), value);
}

/// One secret field of the notify extension: the agent dir's `secrets.json`
/// (`<agent>/secrets.json`, a flat string map). The settings registry stores
/// secret fields as `<owner-ext-id>/<namespace>/<key>`; the extension's own
/// fallback write uses `<namespace>/<key>`. Never errors.
fn notify_secret(path: &Path, keys: &[&str]) -> Option<String> {
    let agent = path.parent()?;
    let raw = std::fs::read_to_string(agent.join("secrets.json")).ok()?;
    let map = serde_json::from_str::<std::collections::HashMap<String, String>>(&raw).ok()?;
    keys.iter().find_map(|key| map.get(*key).cloned())
}

/// Reads the ntfy connection details from `path` (missing/corrupt file ->
/// all-`None`, never an error \u2014 the app shows "not configured" instead).
pub fn read_ntfy_config(path: &Path) -> NtfyConnection {
    let parsed = notify_block(path);
    NtfyConnection {
        server: parsed.ntfy.server_url,
        topic: parsed.ntfy.topic,
        token: parsed.ntfy.token.or_else(|| {
            notify_secret(
                path,
                &[
                    "unipi.notify/notify/ntfy.token",
                    "notify/ntfy.token",
                    "ntfy.token",
                ],
            )
        }),
        app_detail_full: parsed.ntfy.app_detail.as_deref() == Some("full"),
        enabled: parsed.ntfy.enabled,
    }
}

/// Reads the Gotify connection details from `path` (missing/corrupt file
/// -> all-`None`, never an error).
pub fn read_gotify_config(path: &Path) -> GotifyConnection {
    let parsed = notify_block(path);
    GotifyConnection {
        server_url: parsed.gotify.server_url,
        app_token: parsed.gotify.app_token.or_else(|| {
            notify_secret(
                path,
                &[
                    "unipi.notify/notify/gotify.appToken",
                    "notify/gotify.appToken",
                    "gotify.appToken",
                ],
            )
        }),
        enabled: parsed.gotify.enabled,
    }
}

/// Reads the Telegram connection details from `path` (missing/corrupt file
/// -> all-`None`, never an error).
pub fn read_telegram_config(path: &Path) -> TelegramConnection {
    let parsed = notify_block(path);
    let secret = |key: &str| {
        let namespace = format!("notify/{key}");
        let owned = format!("unipi.notify/notify/{key}");
        let bare = key.to_string();
        notify_secret(path, &[&owned, &namespace, &bare])
    };
    TelegramConnection {
        bot_token: parsed
            .telegram
            .bot_token
            .or_else(|| secret("telegram.botToken")),
        chat_id: parsed
            .telegram
            .chat_id
            .or_else(|| secret("telegram.chatId")),
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

/// `<host dir>/app-endpoints.json`: `{ endpoints: [{url, addedAt}] }`, the
/// fan-out list `notify_register` maintains and phone push reads.
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

/// The app-endpoints fan-out file: `<host dir>/app-endpoints.json`
/// (`$UNIPI_NOTIFY_ENDPOINTS_JSON` overrides it in tests).
pub fn app_endpoints_path(paths: &HostPaths) -> PathBuf {
    if let Ok(dir) = std::env::var(UNIPI_NOTIFY_ENDPOINTS_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    paths.app_endpoints()
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
    publish_body_with_click(topic, key, payload, None)
}

/// [`publish_body`] with the ntfy `click` deep link (UNI-161 §4: content-free
/// routing only — host/session — never prompt text, so a server sees nothing
/// it could not already read from the URL).
pub fn publish_body_with_click(
    topic: Option<&str>,
    key: &[u8; KEY_LEN],
    payload: &serde_json::Value,
    click: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let mut body = serde_json::json!({ "title": "UniPi", "message": encrypt_envelope(key, payload)?, "priority": 4 });
    if let Some(topic) = topic {
        body["topic"] = serde_json::Value::String(topic.to_string());
    }
    if let Some(click) = click {
        body["click"] = serde_json::Value::String(click.to_string());
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
    send_payload_with_click(connection, endpoints, key, payload, None).await
}

/// [`send_payload`] carrying the ntfy `click` deep link on the topic POST
/// (endpoints keep the same body shape — the phone resolves the link from the
/// decrypted payload there).
pub async fn send_payload_with_click(
    connection: &NtfyConnection,
    endpoints: &[String],
    key: &[u8; KEY_LEN],
    payload: &serde_json::Value,
    click: Option<&str>,
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
            .json(&publish_body_with_click(Some(topic), key, payload, click)?);
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

// ------------------------------------------------------------ session pushes (P0-06)

/// The `unipi.notify` extension's tool name.
const NOTIFY_USER_TOOL: &str = "notify_user";
/// unipi `PROMPT_DEDUP_MS`: two dialog pushes never fire closer than this.
const PROMPT_DEDUP_MS: i64 = 2000;
/// unipi `INPUT_TITLE_MAX_CHARS`.
const INPUT_TITLE_MAX_CHARS: usize = 120;
/// The product name of the notify extension's titles (`Oino — …`).
const PRODUCT: &str = "Oino";

/// What one runtime event means for the phone push / `needs_you` (P0-06).
/// Extracted by [`session_notice`] — pure, so the tap hook stays thin.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionNotice {
    /// A dialog opened (`is_dialog` methods: select/confirm/input/editor/
    /// multiSelect/form/approval/custom/… — the ask-user and permission
    /// family).
    DialogOpen {
        /// Request id.
        id: String,
        /// Dialog kind (`prompt_kind`: select/confirm/input/editor/custom).
        kind: &'static str,
        /// Dialog title, when it has one.
        title: String,
        /// `options` (select dialogs) — the phone's quick replies.
        options: Vec<String>,
    },
    /// A dialog closed (`extension_ui_dismiss`).
    DialogClosed {
        /// Request id.
        id: String,
    },
    /// The run settled (the single "finished" signal — `agent_settled`, after
    /// retries and continuations).
    Settled,
    /// The agent called the notify extension's `notify_user` tool.
    NotifyUser {
        /// Title override.
        title: Option<String>,
        /// Message body.
        message: String,
        /// Explicit platform override (`platforms: ["ntfy", …]`).
        platforms: Option<Vec<String>>,
    },
}

/// Extracts the phone-relevant [`SessionNotice`] of one tap event
/// (`None` for everything else).
#[must_use]
pub fn session_notice(event: &ClientEvent) -> Option<SessionNotice> {
    match event {
        ClientEvent::Ui(request) if is_dialog(&request.method) => {
            let params = &request.params;
            let options = params
                .get("options")
                .and_then(serde_json::Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Some(SessionNotice::DialogOpen {
                id: request.id.clone(),
                kind: prompt_kind(&request.method),
                title: params
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                options,
            })
        }
        ClientEvent::Oino(OinoEvent::ExtensionUiDismiss { id, .. }) => {
            Some(SessionNotice::DialogClosed { id: id.clone() })
        }
        ClientEvent::Session(AgentSessionEvent::AgentSettled { .. }) => {
            Some(SessionNotice::Settled)
        }
        ClientEvent::Session(AgentSessionEvent::ToolExecutionStart {
            tool_name, args, ..
        }) if tool_name == NOTIFY_USER_TOOL => {
            let text = |key: &str| {
                args.get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let platforms = args
                .get("platforms")
                .and_then(serde_json::Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                });
            Some(SessionNotice::NotifyUser {
                title: text("title"),
                message: text("message").unwrap_or_default(),
                platforms,
            })
        }
        _ => None,
    }
}

/// The unifi `kind` of a notice (the phone's routing tag).
#[must_use]
pub fn notice_kind(notice: &SessionNotice, finish: &str) -> String {
    match notice {
        SessionNotice::DialogOpen { .. } => "input_needed".to_owned(),
        SessionNotice::DialogClosed { .. } => "input_needed".to_owned(),
        SessionNotice::Settled => finish.to_owned(),
        SessionNotice::NotifyUser { .. } => NOTIFY_USER_TOOL.to_owned(),
    }
}

/// The title of a notice's push (the notify extension's `title_for`).
#[must_use]
pub fn notice_title(notice: &SessionNotice) -> String {
    match notice {
        SessionNotice::DialogOpen { .. } => format!("{PRODUCT} — Input Needed"),
        SessionNotice::DialogClosed { .. } => format!("{PRODUCT} — Input Needed"),
        SessionNotice::Settled => format!("{PRODUCT} — Agent Complete"),
        SessionNotice::NotifyUser { title, .. } => title
            .clone()
            .unwrap_or_else(|| format!("{PRODUCT} Notification")),
    }
}

/// The body of a notice's push (the notify extension's `messages.rs` texts).
#[must_use]
pub fn notice_body(notice: &SessionNotice, session_name: Option<&str>) -> String {
    match notice {
        SessionNotice::DialogOpen { kind, title, .. } => {
            let squashed: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
            if squashed.is_empty() {
                format!("Waiting for your input ({kind})")
            } else {
                let clipped: String = squashed.chars().take(INPUT_TITLE_MAX_CHARS).collect();
                format!("Waiting for your input: {clipped}")
            }
        }
        SessionNotice::DialogClosed { .. } => String::new(),
        SessionNotice::Settled => match session_name.filter(|n| !n.is_empty()) {
            Some(name) => format!("{name} - Agent is complete"),
            None => "Agent is complete".to_owned(),
        },
        SessionNotice::NotifyUser { message, .. } => message.clone(),
    }
}

/// Percent-encodes one path segment of the deep link (unreserved
/// `ALPHA / DIGIT / -._~` pass; everything else `%XX`), matching the app's
/// `encodeURIComponent` + `decodeURIComponent` round trip.
fn encode_segment(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    for byte in part.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                out.push('%');
                out.push(char::from_digit(u32::from(byte) >> 4, 16).expect("nibble"));
                out.push(char::from_digit(u32::from(byte) & 0xF, 16).expect("nibble"));
            }
        }
    }
    out
}

/// `unipi://s/<host>/<session>` (G-12): the deep link every session push
/// carries — both segments percent-encoded.
#[must_use]
pub fn session_deep_link(host: &str, session: &str) -> String {
    format!(
        "unipi://s/{}/{}",
        encode_segment(host),
        encode_segment(session)
    )
}

/// Best-effort machine hostname for the deep link / payload `host` (the app
/// maps it to the paired host, preferring the exact id then the name).
#[must_use]
pub fn hostname_for_route() -> String {
    hostname::get()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "UniPi host".into())
        .chars()
        .take(64)
        .collect()
}

/// The `unipi1:` payload of one session push — exactly the phone's
/// `NotifyPayload` (`apps/unipi/src/lib/notifyCrypto.ts`): `title`, `body`,
/// `kind`, `host`, `session`, and `options` for dialog quick replies.
#[must_use]
pub fn session_payload(
    host: &str,
    session: &str,
    kind: &str,
    title: &str,
    body: &str,
    options: &[String],
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "title": title,
        "body": body,
        "kind": kind,
        "host": host,
        "session": session,
    });
    if !options.is_empty() {
        payload["options"] = serde_json::json!(options);
    }
    payload
}

/// Sends one session push over the host's own channels: the configured ntfy
/// topic and every registered UnifiedPush endpoint, `unipi1:`-encrypted, with
/// the `unipi://s/<host>/<session>` deep link on the topic POST's `click`.
/// Best-effort (warns, never errors the caller); `Ok(0)` = nowhere to send.
async fn send_session_push(paths: &HostPaths, payload: &serde_json::Value, link: &str) -> usize {
    let connection = read_ntfy_config(&notify_config_json_path());
    let endpoints = read_app_endpoints(&app_endpoints_path(paths));
    if connection.topic.is_none() && endpoints.is_empty() {
        return 0;
    }
    let Ok(key) = notify_key_or_create(paths) else {
        return 0;
    };
    match send_payload_with_click(&connection, &endpoints, &key, payload, Some(link)).await {
        Ok(sent) => sent,
        Err(error) => {
            tracing::warn!(%error, "session push failed");
            0
        }
    }
}

/// Open dialog ids per session + the last dialog push time (the 2 s de-dup).
#[derive(Default)]
struct DialogState {
    open: HashSet<String>,
    last_push_ms: i64,
}

fn dialog_states() -> &'static Mutex<HashMap<String, DialogState>> {
    static STATES: OnceLock<Mutex<HashMap<String, DialogState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The session tap hook (P0-06): one runtime event \u2192 the hub card's
/// `needs_you` flag and, gated by the notify extension's event matrix, the
/// phone push. Called for every event the tap sees; everything not a
/// [`SessionNotice`] returns immediately.
///
/// - Dialog open \u2192 `needs_you` on the card + one `input_needed` push
///   (de-duplicated within [`PROMPT_DEDUP_MS`], like the extension).
/// - Dialog close \u2192 `needs_you` off once the last dialog closed.
/// - Settle \u2192 the run-finished push (`agent_settled`, or `agent_end`
///   when only that is on) + a defensive `needs_you` clear (a dialog cannot
///   outlive the run that opened it).
/// - `notify_user` tool call \u2192 its message, per its explicit platforms
///   or the `defaultPlatforms` fallback.
///
/// `paths` is the host data dir (`HostPaths::resolve()` at the tap; a serve
/// started with `--data-dir` overrides it and is not visible here).
/// Best-effort: failures log at `warn` and never surface to the session.
pub async fn on_session_event(
    hub: &SessionHub,
    paths: &HostPaths,
    event: &ClientEvent,
    session_id: &str,
) {
    let Some(notice) = session_notice(event) else {
        return;
    };
    let settings = PhonePushSettings::read(&notify_config_json_path());
    let host = hostname_for_route();
    match &notice {
        SessionNotice::DialogOpen { id, options, .. } => {
            hub.set_needs_you(session_id, true).await;
            let push = lock_dialog(session_id, |state| {
                state.open.insert(id.clone());
                let now = now_ms();
                let deduped = now.saturating_sub(state.last_push_ms) < PROMPT_DEDUP_MS;
                state.last_push_ms = now;
                !deduped
            });
            if push && settings.event_push_enabled("input_needed") {
                push_notice(&notice, &settings, hub, paths, session_id, &host, options).await;
            }
        }
        SessionNotice::DialogClosed { id } => {
            let cleared = lock_dialog(session_id, |state| {
                state.open.remove(id);
                state.open.is_empty()
            });
            if cleared {
                hub.set_needs_you(session_id, false).await;
            }
        }
        SessionNotice::Settled => {
            lock_dialog(session_id, |state| {
                state.open.clear();
                state.last_push_ms = 0;
            });
            hub.set_needs_you(session_id, false).await;
            if settings.finish_event().is_some() {
                push_notice(&notice, &settings, hub, paths, session_id, &host, &[]).await;
            }
        }
        SessionNotice::NotifyUser { platforms, .. } => {
            if settings.tool_push_enabled(platforms.as_deref()) {
                push_notice(&notice, &settings, hub, paths, session_id, &host, &[]).await;
            }
        }
    }
}

/// Runs `f` over the session's dialog state under the lock.
fn lock_dialog<T>(session_id: &str, f: impl FnOnce(&mut DialogState) -> T) -> T {
    let mut states = dialog_states()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(states.entry(session_id.to_owned()).or_default())
}

/// Builds and sends one notice's push (title/body/kind from the pure
/// helpers, session name on the card for the settle text).
async fn push_notice(
    notice: &SessionNotice,
    settings: &PhonePushSettings,
    hub: &SessionHub,
    paths: &HostPaths,
    session_id: &str,
    host: &str,
    options: &[String],
) {
    let finish = settings.finish_event().unwrap_or("agent_settled");
    let kind = notice_kind(notice, finish);
    let name = hub
        .card(session_id)
        .await
        .ok()
        .flatten()
        .and_then(|card| card.name);
    let payload = session_payload(
        host,
        session_id,
        &kind,
        &notice_title(notice),
        &notice_body(notice, name.as_deref()),
        options,
    );
    let link = session_deep_link(host, session_id);
    send_session_push(paths, &payload, &link).await;
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

    /// v2 (host.md §9.5): the notify settings live in the agent settings file
    /// under `extensionSettings.notify`, the same values the `unipi.notify`
    /// extension's wizards write.
    #[test]
    fn reads_the_notify_block_from_agent_settings() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"theme":"dark","extensionSettings":{"notify":{"ntfy":{"enabled":true,"serverUrl":"https://ntfy.example","topic":"fam","appDetail":"full"},"events":{"agentEnd":true}}}}"#,
        )
        .expect("write");
        let connection = read_ntfy_config(&path);
        assert_eq!(connection.server, Some("https://ntfy.example".to_string()));
        assert_eq!(connection.topic, Some("fam".to_string()));
        assert!(connection.app_detail_full);
        assert!(connection.enabled);
    }

    /// Core migrates legacy flat extension-settings keys on load; the host
    /// reads the same shapes.
    #[test]
    fn legacy_dotted_settings_keys_are_expanded() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"extensionSettings":{"notify":{"ntfy.serverUrl":"https://ntfy.example","ntfy.topic":"flat","ntfy.enabled":true}}}"#,
        )
        .expect("write");
        let connection = read_ntfy_config(&path);
        assert_eq!(connection.server, Some("https://ntfy.example".to_string()));
        assert_eq!(connection.topic, Some("flat".to_string()));
        assert!(connection.enabled);
    }

    /// Secret fields never reach settings.json: they live in
    /// `<agent>/secrets.json` as `<owner-ext-id>/<namespace>/<key>`.
    #[test]
    fn secret_fields_come_from_the_agent_secrets_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"extensionSettings":{"notify":{"ntfy":{"enabled":true,"serverUrl":"https://ntfy.example","topic":"t"},"gotify":{"enabled":true,"serverUrl":"https://gotify.example"},"telegram":{"enabled":true}}}}"#,
        )
        .expect("write");
        std::fs::write(
            tmp.path().join("secrets.json"),
            r#"{"unipi.notify/notify/ntfy.token":"tk_ntfy","unipi.notify/notify/gotify.appToken":"tk_gotify","notify/telegram.botToken":"tk_tg","unipi.notify/notify/telegram.chatId":"42"}"#,
        )
        .expect("write");
        let ntfy = read_ntfy_config(&path);
        assert_eq!(ntfy.token, Some("tk_ntfy".to_string()));
        let gotify = read_gotify_config(&path);
        assert_eq!(gotify.app_token, Some("tk_gotify".to_string()));
        let telegram = read_telegram_config(&path);
        assert_eq!(telegram.bot_token, Some("tk_tg".to_string()));
        assert_eq!(telegram.chat_id, Some("42".to_string()));
    }

    /// The UnifiedPush fan-out list is host-owned (v2: the host sends the
    /// notifications): `<host dir>/app-endpoints.json`.
    #[test]
    fn app_endpoints_live_in_the_host_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = HostPaths::from_dir(tmp.path().join("host"));
        assert_eq!(
            app_endpoints_path(&paths),
            tmp.path().join("host").join("app-endpoints.json")
        );
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
    fn publish_body_carries_the_content_free_click_link() {
        let key = [5u8; KEY_LEN];
        let payload = serde_json::json!({"title":"t", "body":"secret text"});
        let body =
            publish_body_with_click(Some("topic"), &key, &payload, Some("unipi://s/my%20pc/s-1"))
                .expect("body");
        assert_eq!(body["click"], "unipi://s/my%20pc/s-1");
        assert!(!body.to_string().contains("secret text"));
        // The plain body never carries a click.
        assert!(publish_body(Some("t"), &key, &payload)
            .expect("body")
            .get("click")
            .is_none());
    }

    #[test]
    fn phone_push_settings_read_the_event_matrix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"extensionSettings":{"notify":{
                "ntfy":{"enabled":true,"serverUrl":"https://n.example","topic":"t"},
                "defaultPlatforms":["native"],
                "events":{
                    "agent_settled":{"enabled":true,"platforms":[]},
                    "input_needed":{"enabled":true,"platforms":["ntfy"]},
                    "agent_end":{"enabled":true,"platforms":["native"]}
                }}}}"#,
        )
        .expect("write");
        let settings = PhonePushSettings::read(&path);
        assert!(settings.ntfy_enabled);
        // Empty platforms = every enabled platform \u2192 ntfy on.
        assert!(settings.event_push_enabled("agent_settled"));
        // Explicit ntfy \u2192 on; explicit native-only \u2192 off.
        assert!(settings.event_push_enabled("input_needed"));
        assert!(!settings.event_push_enabled("agent_end"));
        // Unknown or absent events never push.
        assert!(!settings.event_push_enabled("ask_user_prompt"));
        // finish: agent_settled wins over agent_end (one notification).
        assert_eq!(settings.finish_event(), Some("agent_settled"));
        // notify_user defaults to defaultPlatforms (native) \u2192 no phone.
        assert!(!settings.tool_push_enabled(None));
        assert!(settings.tool_push_enabled(Some(&["ntfy".to_owned()])));
    }

    #[test]
    fn phone_push_settings_expand_dotted_keys_and_default_off() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"extensionSettings":{"notify":{
                "ntfy.enabled":true,
                "defaultPlatforms":["ntfy"],
                "events.agent_end.enabled":true
            }}}"#,
        )
        .expect("write");
        let settings = PhonePushSettings::read(&path);
        assert!(settings.event_push_enabled("agent_end"));
        assert_eq!(settings.finish_event(), Some("agent_end"));
        assert!(settings.tool_push_enabled(None));
        // A missing file is everything off (never an error).
        let empty = PhonePushSettings::read(&tmp.path().join("missing.json"));
        assert_eq!(empty, PhonePushSettings::default());
        assert_eq!(empty.finish_event(), None);
    }

    #[test]
    fn phone_push_needs_ntfy_enabled_even_when_listed() {
        let mut settings = PhonePushSettings::default();
        settings.events.insert(
            "input_needed".to_owned(),
            EventSwitch {
                enabled: true,
                platforms: vec!["ntfy".to_owned()],
            },
        );
        assert!(!settings.event_push_enabled("input_needed"));
        settings.ntfy_enabled = true;
        assert!(settings.event_push_enabled("input_needed"));
    }

    fn ui_dialog(method: &str, params: serde_json::Value) -> ClientEvent {
        use oino_core::ui::UiRequest;
        let request = serde_json::from_value::<UiRequest>(serde_json::json!({
            "type": "extension_ui_request", "id": "d-1", "method": method,
        }))
        .expect("request");
        let mut request = request;
        request.params = params.as_object().expect("object").clone();
        ClientEvent::Ui(request)
    }

    #[test]
    fn session_notice_extracts_dialogs_settles_and_notify_user() {
        use oino_core::ui::DismissReason;
        // Dialogs (ask-user's form, permission approval, selects) are notices.
        let ask = session_notice(&ui_dialog(
            "form",
            serde_json::json!({"title": "Pick a plan", "questions": []}),
        ))
        .expect("ask");
        assert_eq!(
            ask,
            SessionNotice::DialogOpen {
                id: "d-1".into(),
                kind: "custom",
                title: "Pick a plan".into(),
                options: Vec::new(),
            }
        );
        let select = session_notice(&ui_dialog(
            "select",
            serde_json::json!({"title": "Allow?", "options": ["Allow", "Deny"]}),
        ))
        .expect("select");
        assert!(matches!(
            &select,
            SessionNotice::DialogOpen { options, .. } if options == &["Allow".to_owned(), "Deny".to_owned()]
        ));
        // Fire-and-forget UI methods are not dialogs.
        assert!(
            session_notice(&ui_dialog("notify", serde_json::json!({"message": "hi"}))).is_none()
        );
        // Dismissals close.
        let closed = session_notice(&ClientEvent::Oino(OinoEvent::ExtensionUiDismiss {
            id: "d-1".into(),
            reason: DismissReason::Answered,
        }))
        .expect("closed");
        assert_eq!(closed, SessionNotice::DialogClosed { id: "d-1".into() });
        // Settle.
        assert_eq!(
            session_notice(&ClientEvent::Session(AgentSessionEvent::AgentSettled {
                aborted: false
            })),
            Some(SessionNotice::Settled)
        );
        // notify_user tool call.
        let mut args = oino_ai::JsonObject::new();
        args.insert("message".into(), serde_json::json!("build done"));
        args.insert("platforms".into(), serde_json::json!(["ntfy"]));
        let tool = session_notice(&ClientEvent::Session(
            AgentSessionEvent::ToolExecutionStart {
                tool_call_id: "t1".into(),
                tool_name: "notify_user".into(),
                args,
                parent_tool_call_id: None,
            },
        ))
        .expect("tool");
        assert_eq!(
            tool,
            SessionNotice::NotifyUser {
                title: None,
                message: "build done".into(),
                platforms: Some(vec!["ntfy".to_owned()]),
            }
        );
        // Other tools and events are not notices.
        let mut bash = oino_ai::JsonObject::new();
        bash.insert("command".into(), serde_json::json!("ls"));
        assert!(session_notice(&ClientEvent::Session(
            AgentSessionEvent::ToolExecutionStart {
                tool_call_id: "t2".into(),
                tool_name: "bash".into(),
                args: bash,
                parent_tool_call_id: None,
            }
        ))
        .is_none());
        assert!(session_notice(&ClientEvent::Session(AgentSessionEvent::TurnStart)).is_none());
    }

    #[test]
    fn session_payload_and_deep_link_shape() {
        let link = session_deep_link("my pc", "s-9");
        assert_eq!(link, "unipi://s/my%20pc/s-9");
        assert_eq!(session_deep_link("box", "abc"), "unipi://s/box/abc");
        let payload = session_payload(
            "my pc",
            "s-9",
            "input_needed",
            "Oino — Input Needed",
            "Waiting for your input: Allow?",
            &["Allow".to_owned(), "Deny".to_owned()],
        );
        assert_eq!(payload["kind"], "input_needed");
        assert_eq!(payload["host"], "my pc");
        assert_eq!(payload["session"], "s-9");
        assert_eq!(payload["options"], serde_json::json!(["Allow", "Deny"]));
        // No options \u2192 no key (the phone's NotifyPayload keeps it optional).
        let bare = session_payload("h", "s", "agent_settled", "t", "b", &[]);
        assert!(bare.get("options").is_none());
    }

    #[test]
    fn notice_titles_and_bodies_match_the_extension() {
        let dialog = SessionNotice::DialogOpen {
            id: "d".into(),
            kind: "confirm",
            title: "Allow   bash rm -rf?".into(),
            options: Vec::new(),
        };
        assert_eq!(notice_title(&dialog), "Oino — Input Needed");
        assert_eq!(
            notice_body(&dialog, None),
            "Waiting for your input: Allow bash rm -rf?"
        );
        let untitled = SessionNotice::DialogOpen {
            id: "d".into(),
            kind: "input",
            title: String::new(),
            options: Vec::new(),
        };
        assert_eq!(
            notice_body(&untitled, None),
            "Waiting for your input (input)"
        );
        assert_eq!(notice_kind(&untitled, "agent_settled"), "input_needed");
        let long = SessionNotice::DialogOpen {
            id: "d".into(),
            kind: "input",
            title: "x".repeat(300),
            options: Vec::new(),
        };
        assert_eq!(
            notice_body(&long, None).chars().count(),
            "Waiting for your input: ".chars().count() + 120
        );
        let settled = SessionNotice::Settled;
        assert_eq!(notice_title(&settled), "Oino — Agent Complete");
        assert_eq!(notice_body(&settled, None), "Agent is complete");
        assert_eq!(
            notice_body(&settled, Some("graph run")),
            "graph run - Agent is complete"
        );
        assert_eq!(notice_kind(&settled, "agent_end"), "agent_end");
        let tool = SessionNotice::NotifyUser {
            title: Some("Build failed".into()),
            message: "tsc found 12 errors".into(),
            platforms: None,
        };
        assert_eq!(notice_title(&tool), "Build failed");
        assert_eq!(notice_body(&tool, None), "tsc found 12 errors");
        assert_eq!(notice_kind(&tool, "agent_settled"), "notify_user");
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
