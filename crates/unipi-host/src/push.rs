//! Push a file from the PC to the phone (UNI-214, forwarding phase 2).
//!
//! ```text
//! unipi-host push app.apk ──control.sock──▶ unipi-host serve (PushHub)
//!                                              │ pending push per device
//! phone: push_list{wait} (long-poll) ◀─────────┤ (+ ntfy/UnifiedPush when
//!        blob_get{path}  (the bytes)  ◀────────┤  the phone isn't live)
//!        push_done{id, outcome}       ─────────▶ CLI prints the outcome
//! ```
//!
//! Security: OFF by default. The PC owner switches it on in
//! `<host dir>/config.json` (read on every request, no restart):
//!
//! ```json
//! { "push": { "enabled": true, "devices": ["Pixel"] } }
//! ```
//!
//! `unipi-host push enable [--device X]` / `disable` / `status` edit it.
//! Only the device(s) a push was addressed to may list it or `blob_get`
//! its path, and only while it is pending. A push is a plain file the PC
//! owner picked; the phone verifies size + sha256 before using it.
//!
//! Local IPC: a Unix socket `<host dir>/control.sock` (0600, inside the
//! 0700 host dir — same trust as the files next to it). One JSON line in,
//! JSON lines out (see [`ControlRequest`] / [`ControlEvent`]).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, watch};
use unipi_proto::{AppErrorCode, AppErrorDto, PushId, PushItem, PushListData, PushOutcome, Sha256Hex};

use crate::data_dir::HostPaths;
use crate::devices::Device;

/// How the owner switches push on (shown by the CLI and in errors).
pub const ENABLE_HINT: &str = "On the PC run `unipi-host push enable` (or `--device <name>` for one phone), no restart needed.";

/// Pending pushes older than this are dropped.
pub const PUSH_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// A device counts as "live" (the app is open and long-polling) when it
/// polled this recently; otherwise the host also sends a notification.
const LIVE_WINDOW_MS: u64 = 5_000;

/// The `push` block of `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PushConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Paired device ids / id prefixes (≥ 8 chars) / names that may
    /// receive pushes; empty = every paired device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
}

impl PushConfig {
    pub fn device_allowed(&self, device_id: &str, device_name: Option<&str>) -> bool {
        self.enabled
            && (self.devices.is_empty()
                || device_id == crate::protocol::LOCAL_DEVICE_ID
                || self.devices.iter().any(|entry| matches_device(entry, device_id, device_name)))
    }
}

/// Same matching rule as forwarding: exact id, id prefix ≥ 8, or name.
/// UNI-229: this PC's own local client (`app.sock`) is the owner — always matches.
pub fn matches_device(entry: &str, device_id: &str, device_name: Option<&str>) -> bool {
    device_id == crate::protocol::LOCAL_DEVICE_ID
        || entry == device_id
        || (entry.len() >= 8 && device_id.starts_with(entry))
        || Some(entry) == device_name
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Streaming sha256 + size of a file.
pub fn sha256_file(path: &Path) -> std::io::Result<(String, u64)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let mut hex = String::with_capacity(64);
    for b in hasher.finalize() {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok((hex, size))
}

/// MIME type from the file extension (what the phone uses to pick
/// Install vs Save/Open).
pub fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("apk") => "application/vnd.android.package-archive",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("zip") => "application/zip",
        Some("txt" | "log") => "text/plain",
        Some("md") => "text/markdown",
        Some("json") => "application/json",
        Some("html" | "htm") => "text/html",
        _ => "application/octet-stream",
    }
}

/// Human size: `12.0 MB`, `640 KB`, `12 B`.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / KB / KB / KB)
    } else if b >= KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// One pending push and who still has to answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPush {
    pub item: PushItem,
    /// Device ids that haven't answered yet.
    pub remaining: BTreeSet<String>,
}

/// Events for the CLI (and tests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ControlEvent {
    /// The push is recorded; `devices` are the target names.
    Queued { id: String, devices: Vec<String>, notified: bool },
    /// A target device started downloading it.
    Fetching { id: String, device: String },
    /// A target device answered.
    Done { id: String, device: String, outcome: PushOutcome, #[serde(skip_serializing_if = "Option::is_none", default)] detail: Option<String> },
    /// Every target answered (or the push was cancelled/expired).
    Finished { id: String },
    /// Reply to `list`.
    Pending { pushes: Vec<PendingPush> },
    /// Reply to `cancel`.
    Cancelled { ids: Vec<String> },
    /// UNI-292: reply to `browser` (the action's JSON result).
    Browser { data: serde_json::Value },
    /// UNI-314: reply to `cron` (the action's JSON result).
    Cron { data: serde_json::Value },
    Error { message: String, #[serde(skip_serializing_if = "Option::is_none", default)] hint: Option<String> },
}

impl ControlEvent {
    fn id(&self) -> Option<&str> {
        match self {
            ControlEvent::Queued { id, .. }
            | ControlEvent::Fetching { id, .. }
            | ControlEvent::Done { id, .. }
            | ControlEvent::Finished { id } => Some(id),
            _ => None,
        }
    }
}

/// One JSON line from the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Record a push. The host re-checks the file and config; `sha256`
    /// and `size` are what the CLI hashed (re-verified here).
    Push {
        path: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        note: Option<String>,
        /// `--device` filters (id / prefix / name); empty = every allowed device.
        #[serde(default)]
        devices: Vec<String>,
        sha256: String,
        size: u64,
        /// Keep the connection open and stream fetching/done events.
        #[serde(default)]
        follow: bool,
    },
    List,
    Cancel {
        #[serde(default)]
        id: Option<String>,
    },
    /// UNI-292: `unipi-host browser start|stop|profiles` — the serve owns the
    /// browsers so the app's viewer and the agent share them.
    Browser { request: crate::browser::BrowserControl },
    /// UNI-314: `unipi-host cron ...` — the serve owns the scheduler.
    Cron { request: crate::cron::CronControl },
}

#[derive(Default)]
struct HubState {
    pushes: Vec<PendingPush>,
    /// device id → unix ms of its last `push_list` (start or end).
    last_poll: HashMap<String, u64>,
    /// device id → active long-polls.
    waiting: HashMap<String, usize>,
    loaded: bool,
}

/// The serve process's pending pushes. Cheap to clone.
#[derive(Clone)]
pub struct PushHub {
    state: Arc<Mutex<HubState>>,
    changed: Arc<watch::Sender<u64>>,
    events: broadcast::Sender<ControlEvent>,
    /// Where pending pushes persist (survive a host restart); `None` in tests.
    store: Option<PathBuf>,
}

impl std::fmt::Debug for PushHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushHub").finish_non_exhaustive()
    }
}

impl Default for PushHub {
    fn default() -> Self {
        PushHub::new(None)
    }
}

impl PushHub {
    pub fn new(store: Option<PathBuf>) -> Self {
        let (tx, _) = watch::channel(0u64);
        let (events, _) = broadcast::channel(256);
        PushHub { state: Arc::new(Mutex::new(HubState::default())), changed: Arc::new(tx), events, store }
    }

    /// The one hub for a host dir (persisted under `<host dir>/pushes.json`),
    /// shared by the iroh `Handler` and the control socket of the same
    /// `serve` process — a registry so `Handler` needs no extra field.
    pub fn shared(paths: &HostPaths) -> Self {
        static HUBS: std::sync::OnceLock<Mutex<HashMap<PathBuf, PushHub>>> = std::sync::OnceLock::new();
        let hubs = HUBS.get_or_init(Default::default);
        let mut hubs = hubs.lock().unwrap_or_else(|e| e.into_inner());
        hubs.entry(paths.root.clone())
            .or_insert_with(|| PushHub::new(Some(paths.root.join("pushes.json"))))
            .clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.loaded {
            state.loaded = true;
            if let Some(store) = &self.store {
                if let Ok(Some(pushes)) = crate::data_dir::read_json::<Vec<PendingPush>>(store) {
                    state.pushes = pushes;
                }
            }
        }
        state
    }

    fn save(&self, state: &HubState) {
        if let Some(store) = &self.store {
            if let Err(error) = crate::data_dir::write_json(store, &state.pushes) {
                tracing::warn!(%error, "could not persist pending pushes");
            }
        }
    }

    fn bump(&self) {
        self.changed.send_modify(|v| *v += 1);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ControlEvent> {
        self.events.subscribe()
    }

    fn emit(&self, event: ControlEvent) {
        let _ = self.events.send(event);
    }

    /// Drops expired pushes (and their files' allow-list entries).
    fn expire(&self, state: &mut HubState, now: u64) {
        let before = state.pushes.len();
        let mut gone = Vec::new();
        state.pushes.retain(|p| {
            let keep = now.saturating_sub(p.item.created_at) < PUSH_TTL_MS && !p.remaining.is_empty();
            if !keep {
                gone.push(p.item.id.as_str().to_string());
            }
            keep
        });
        if state.pushes.len() != before {
            self.save(state);
            for id in gone {
                self.emit(ControlEvent::Finished { id });
            }
        }
    }

    /// Is any of `devices` live (polled within the last few seconds or
    /// long-polling right now)?
    pub fn any_live(&self, devices: &[String], now: u64) -> bool {
        let state = self.lock();
        devices.iter().any(|d| {
            state.waiting.get(d).copied().unwrap_or(0) > 0
                || state.last_poll.get(d).is_some_and(|t| now.saturating_sub(*t) <= LIVE_WINDOW_MS)
        })
    }

    /// Records a new push for `device_ids`. Returns the id.
    pub fn add(&self, mut item: PushItem, device_ids: Vec<String>) -> String {
        let mut state = self.lock();
        let now = now_ms();
        self.expire(&mut state, now);
        let id = format!("p{:x}{:04x}", now, rand::random::<u16>());
        item.id = PushId::try_from(id.clone()).expect("generated id is valid");
        item.created_at = now;
        state.pushes.push(PendingPush { item, remaining: device_ids.into_iter().collect() });
        self.save(&state);
        drop(state);
        self.bump();
        id
    }

    /// The pending pushes addressed to `device_id`, oldest first.
    pub fn pending_for(&self, device_id: &str) -> Vec<PushItem> {
        let mut state = self.lock();
        self.expire(&mut state, now_ms());
        state
            .pushes
            .iter()
            .filter(|p| p.remaining.contains(device_id))
            .map(|p| p.item.clone())
            .collect()
    }

    pub fn all(&self) -> Vec<PendingPush> {
        let mut state = self.lock();
        self.expire(&mut state, now_ms());
        state.pushes.clone()
    }

    /// `push_list` body: waits up to `wait` for something addressed to
    /// `device_id` when nothing is pending yet.
    pub async fn list(&self, device_id: &str, wait: Duration) -> Vec<PushItem> {
        let mut rx = self.changed.subscribe();
        {
            let mut state = self.lock();
            state.last_poll.insert(device_id.to_string(), now_ms());
        }
        let first = self.pending_for(device_id);
        if !first.is_empty() || wait.is_zero() {
            return first;
        }
        *self.lock().waiting.entry(device_id.to_string()).or_default() += 1;
        let deadline = tokio::time::Instant::now() + wait;
        let result = loop {
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {
                    let found = self.pending_for(device_id);
                    if !found.is_empty() {
                        break found;
                    }
                }
                _ => break Vec::new(),
            }
        };
        let mut state = self.lock();
        if let Some(n) = state.waiting.get_mut(device_id) {
            *n = n.saturating_sub(1);
        }
        state.last_poll.insert(device_id.to_string(), now_ms());
        result
    }

    /// Is `path` a pending push for `device_id`? (blob_get allow-list.)
    /// Emits `fetching` when it is.
    pub fn authorize_fetch(&self, device_id: &str, device_name: &str, path: &str) -> bool {
        let state = self.lock();
        let hit = state
            .pushes
            .iter()
            .find(|p| p.item.path == path && p.remaining.contains(device_id))
            .map(|p| p.item.id.as_str().to_string());
        drop(state);
        if let Some(id) = &hit {
            self.emit(ControlEvent::Fetching { id: id.clone(), device: device_name.to_string() });
        }
        hit.is_some()
    }

    /// The phone's answer. Unknown ids are fine (already answered/expired).
    pub fn done(&self, device_id: &str, device_name: &str, id: &str, outcome: PushOutcome, detail: Option<String>) {
        let mut state = self.lock();
        let Some(index) = state.pushes.iter().position(|p| p.item.id.as_str() == id) else {
            return;
        };
        if !state.pushes[index].remaining.remove(device_id) {
            return;
        }
        let finished = state.pushes[index].remaining.is_empty();
        if finished {
            state.pushes.remove(index);
        }
        self.save(&state);
        drop(state);
        self.emit(ControlEvent::Done { id: id.to_string(), device: device_name.to_string(), outcome, detail });
        if finished {
            self.emit(ControlEvent::Finished { id: id.to_string() });
        }
        self.bump();
    }

    /// Cancels one push (or all with `None`). Returns the cancelled ids.
    pub fn cancel(&self, id: Option<&str>) -> Vec<String> {
        let mut state = self.lock();
        let mut ids = Vec::new();
        state.pushes.retain(|p| {
            let hit = id.is_none_or(|id| p.item.id.as_str() == id);
            if hit {
                ids.push(p.item.id.as_str().to_string());
            }
            !hit
        });
        self.save(&state);
        drop(state);
        for id in &ids {
            self.emit(ControlEvent::Finished { id: id.clone() });
        }
        self.bump();
        ids
    }
}

/// `push_list` reply for one device (gating + long-poll).
pub async fn list_for_device(
    hub: &PushHub,
    config: &PushConfig,
    device_id: &str,
    device_name: Option<&str>,
    host_name: &str,
    wait: Duration,
) -> PushListData {
    if !config.device_allowed(device_id, device_name) {
        // Off (the default) is not an error: the phone just shows nothing.
        // Still remember the poll so the host knows the app is live.
        hub.lock().last_poll.insert(device_id.to_string(), now_ms());
        return PushListData { enabled: false, host_name: host_name.to_string(), pushes: Vec::new() };
    }
    let pushes = hub.list(device_id, wait).await;
    PushListData { enabled: true, host_name: host_name.to_string(), pushes }
}

fn push_error(message: impl Into<String>) -> ControlEvent {
    ControlEvent::Error { message: message.into(), hint: None }
}

/// Validates a `push` control request against the file on disk, the
/// config and the paired devices; returns the item and target devices.
pub fn prepare_push(
    paths: &HostPaths,
    config: &PushConfig,
    devices: &[Device],
    path: &str,
    name: Option<String>,
    note: Option<String>,
    filters: &[String],
    sha256: &str,
    size: u64,
) -> Result<(PushItem, Vec<Device>), ControlEvent> {
    if !config.enabled {
        return Err(ControlEvent::Error { message: "Push is off on this PC.".into(), hint: Some(ENABLE_HINT.into()) });
    }
    let file = PathBuf::from(path);
    if !file.is_absolute() {
        return Err(push_error("The path must be absolute."));
    }
    let meta = std::fs::metadata(&file).map_err(|e| push_error(format!("Can't read {path}: {e}")))?;
    if !meta.is_file() {
        return Err(push_error(format!("{path} is not a regular file.")));
    }
    if meta.len() == 0 {
        return Err(push_error(format!("{path} is empty.")));
    }
    let (digest, actual) = sha256_file(&file).map_err(|e| push_error(format!("Can't read {path}: {e}")))?;
    if actual != size || digest != sha256 {
        return Err(push_error("The file changed while it was being pushed; run the command again."));
    }
    let allowed: Vec<Device> = devices.iter().filter(|d| config.device_allowed(&d.id, Some(&d.name))).cloned().collect();
    if devices.is_empty() {
        return Err(ControlEvent::Error { message: "No phone is paired with this PC.".into(), hint: Some("Run `unipi-host pair` and scan the QR code with UniPi.".into()) });
    }
    if allowed.is_empty() {
        return Err(ControlEvent::Error {
            message: "None of the paired phones may receive pushes.".into(),
            hint: Some(format!("Allowed: {}. Change it with `unipi-host push enable --device <name>`.", config.devices.join(", "))),
        });
    }
    let targets: Vec<Device> = if filters.is_empty() {
        allowed
    } else {
        let picked: Vec<Device> = allowed
            .iter()
            .filter(|d| filters.iter().any(|f| matches_device(f, &d.id, Some(&d.name))))
            .cloned()
            .collect();
        if picked.is_empty() {
            let names: Vec<String> = allowed.iter().map(|d| d.name.clone()).collect();
            return Err(ControlEvent::Error {
                message: format!("No allowed phone matches {}.", filters.join(", ")),
                hint: Some(format!("Allowed phones: {}", names.join(", "))),
            });
        }
        picked
    };
    let name = name
        .filter(|n| !n.is_empty())
        .or_else(|| file.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "file".into());
    let name: String = name.rsplit(['/', '\\']).next().unwrap_or("file").chars().take(200).collect();
    let item = PushItem {
        id: PushId::try_from("pending".to_string()).expect("valid"),
        name,
        size,
        sha256: Sha256Hex::try_from(sha256.to_string()).map_err(push_error)?,
        mime: mime_for(&file).to_string(),
        path: path.to_string(),
        note: note.map(|n| n.chars().take(500).collect()).filter(|n: &String| !n.trim().is_empty()),
        created_at: 0,
    };
    let _ = paths;
    Ok((item, targets))
}

/// Notification fallback (UNI-161 path): an encrypted `kind:"push"`
/// payload to the user's ntfy topic and every registered UnifiedPush
/// endpoint. Best-effort; returns whether anything was sent.
pub async fn notify_push(paths: &HostPaths, host_name: &str, item: &PushItem) -> bool {
    let connection = crate::notify::read_ntfy_config(&crate::notify::notify_config_json_path());
    let endpoints = crate::notify::read_app_endpoints(&crate::notify::app_endpoints_path());
    if connection.topic.is_none() && endpoints.is_empty() {
        return false;
    }
    let Ok(key) = crate::notify::notify_key_or_create(paths) else {
        return false;
    };
    let payload = serde_json::json!({
        "title": format!("Incoming from {host_name}"),
        "body": format!("{} ({}) — tap to open in UniPi", item.name, human_size(item.size)),
        "kind": "push",
        "host": host_name,
    });
    match crate::notify::send_payload(&connection, &endpoints, &key, &payload).await {
        Ok(sent) => sent > 0,
        Err(error) => {
            tracing::warn!(%error, "push notification failed");
            false
        }
    }
}

// ------------------------------------------------------------ control socket

/// `<host dir>/control.sock`.
pub fn socket_path(paths: &HostPaths) -> PathBuf {
    paths.root.join("control.sock")
}

/// Context the control socket needs from `serve`.
#[derive(Clone)]
pub struct ControlContext {
    pub paths: HostPaths,
    pub hub: PushHub,
    pub host_name: String,
    /// Send the ntfy/UnifiedPush fallback (off in tests).
    pub notify: bool,
    /// UNI-314: the one cron scheduler for this `serve` process.
    pub cron: crate::cron::Scheduler,
}

/// Binds `<host dir>/control.sock` and serves CLI requests until the
/// task is dropped. Refuses (returns an error) when another serve already
/// answers on it.
#[cfg(unix)]
pub async fn serve_control(ctx: ControlContext) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    use tokio::net::{UnixListener, UnixStream};
    let path = socket_path(&ctx.paths);
    if path.exists() {
        if UnixStream::connect(&path).await.is_ok() {
            anyhow::bail!("another `unipi-host serve` is already using {}", path.display());
        }
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let ctx = ctx.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_control(ctx, stream).await {
                    tracing::debug!(%error, "control connection ended");
                }
            });
        }
    }))
}

/// Windows: the control channel is a named pipe for this host dir
/// (`\\.\pipe\unipi-host-<hash of the dir>`), current user only (the pipe's
/// default DACL).
#[cfg(windows)]
pub fn pipe_name(paths: &HostPaths) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(paths.root.to_string_lossy().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!(r"\\.\pipe\unipi-host-{hex}")
}

#[cfg(windows)]
pub async fn serve_control(ctx: ControlContext) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
    let name = pipe_name(&ctx.paths);
    if ClientOptions::new().open(&name).is_ok() {
        anyhow::bail!("another `unipi-host serve` is already using {name}");
    }
    let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
    Ok(tokio::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                continue;
            }
            let connected = server;
            server = match ServerOptions::new().reject_remote_clients(true).create(&name) {
                Ok(next) => next,
                Err(error) => {
                    tracing::warn!(%error, "control pipe: can't create the next instance");
                    return;
                }
            };
            let ctx = ctx.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_control(ctx, connected).await {
                    tracing::debug!(%error, "control connection ended");
                }
            });
        }
    }))
}

async fn handle_control<S>(ctx: ControlContext, stream: S) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let Some(line) = tokio::time::timeout(Duration::from_secs(10), lines.next_line()).await?? else {
        return Ok(());
    };
    async fn send<W: tokio::io::AsyncWrite + Unpin>(write: &mut W, event: &ControlEvent) -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        write.write_all(&bytes).await?;
        write.flush().await?;
        Ok(())
    }
    let request: ControlRequest = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(error) => {
            send(&mut write, &push_error(format!("bad request: {error}"))).await?;
            return Ok(());
        }
    };
    match request {
        ControlRequest::List => send(&mut write, &ControlEvent::Pending { pushes: ctx.hub.all() }).await?,
        ControlRequest::Cancel { id } => {
            let ids = ctx.hub.cancel(id.as_deref());
            send(&mut write, &ControlEvent::Cancelled { ids }).await?
        }
        ControlRequest::Browser { request } => {
            let event = match crate::browser::control(&ctx.paths, request).await {
                Ok(data) => ControlEvent::Browser { data },
                Err(error) => ControlEvent::Error { message: error.message.as_str().to_string(), hint: error.hint.clone() },
            };
            send(&mut write, &event).await?
        }
        ControlRequest::Cron { request } => {
            let event = match ctx.cron.control(request).await {
                Ok(data) => ControlEvent::Cron { data },
                Err(message) => ControlEvent::Error { message, hint: None },
            };
            send(&mut write, &event).await?
        }
        ControlRequest::Push { path, name, note, devices: filters, sha256, size, follow } => {
            let config = crate::forward::HostConfig::load(&ctx.paths.config()).unwrap_or_default().push;
            let devices = crate::devices::DeviceStore::new(ctx.paths.devices()).list().unwrap_or_default();
            let prepared = tokio::task::spawn_blocking({
                let paths = ctx.paths.clone();
                move || prepare_push(&paths, &config, &devices, &path, name, note, &filters, &sha256, size)
            })
            .await?;
            let (item, targets) = match prepared {
                Ok(ok) => ok,
                Err(event) => {
                    send(&mut write, &event).await?;
                    return Ok(());
                }
            };
            let mut events = ctx.hub.subscribe();
            let target_ids: Vec<String> = targets.iter().map(|d| d.id.clone()).collect();
            let live = ctx.hub.any_live(&target_ids, now_ms());
            let id = ctx.hub.add(item.clone(), target_ids);
            tracing::info!(op = "push", id, name = item.name, size = item.size, live, "queued");
            let notified = if !live && ctx.notify { notify_push(&ctx.paths, &ctx.host_name, &item).await } else { false };
            send(
                &mut write,
                &ControlEvent::Queued { id: id.clone(), devices: targets.iter().map(|d| d.name.clone()).collect(), notified },
            )
            .await?;
            if !follow {
                return Ok(());
            }
            loop {
                tokio::select! {
                    event = events.recv() => match event {
                        Ok(event) if event.id() == Some(id.as_str()) => {
                            let last = matches!(event, ControlEvent::Finished { .. });
                            send(&mut write, &event).await?;
                            if last { break; }
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    },
                    // The CLI went away (Ctrl+C): the push stays pending.
                    line = lines.next_line() => if !matches!(line, Ok(Some(_))) { break },
                }
            }
        }
    }
    Ok(())
}

/// The CLI's end of the control channel.
pub type ControlLines = tokio::io::Lines<tokio::io::BufReader<Box<dyn tokio::io::AsyncRead + Send + Unpin>>>;
pub type ControlWrite = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// CLI side: sends one request and returns the event stream reader.
pub async fn connect_control(paths: &HostPaths, request: &ControlRequest) -> anyhow::Result<(ControlLines, ControlWrite)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    #[cfg(unix)]
    let (where_, stream) = {
        let path = socket_path(paths);
        (path.display().to_string(), tokio::net::UnixStream::connect(&path).await)
    };
    #[cfg(windows)]
    let (where_, stream) = {
        let name = pipe_name(paths);
        let opened = tokio::net::windows::named_pipe::ClientOptions::new().open(&name);
        (name, opened)
    };
    let stream = stream.map_err(|_| {
        anyhow::anyhow!(
            "`unipi-host serve` isn't running for {} (no {where_}).\nStart it (`unipi-host serve`) and try again.",
            paths.root.display(),
        )
    })?;
    let (read, write) = tokio::io::split(stream);
    let read: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(read);
    let mut write: ControlWrite = Box::new(write);
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    write.write_all(&bytes).await?;
    Ok((BufReader::new(read).lines(), write))
}

/// Gating error for `push_done` from a device that may not use push.
pub fn not_allowed() -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_UNAUTHORIZED, "Push is off for this phone on this PC", false).with_hint(ENABLE_HINT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str) -> PushItem {
        PushItem {
            id: PushId::try_from("pending".to_string()).unwrap(),
            name: "app.apk".into(),
            size: 3,
            sha256: Sha256Hex::try_from("a".repeat(64)).unwrap(),
            mime: "application/vnd.android.package-archive".into(),
            path: path.into(),
            note: None,
            created_at: 0,
        }
    }

    fn device(id: &str, name: &str) -> Device {
        Device { id: id.into(), name: name.into(), paired_at: 0, last_seen: None }
    }

    #[test]
    fn config_gating_is_off_by_default_and_matches_devices() {
        let off = PushConfig::default();
        assert!(!off.device_allowed("abc", Some("Pixel")));
        let all = PushConfig { enabled: true, devices: vec![] };
        assert!(all.device_allowed("abc", None));
        let pixel = PushConfig { enabled: true, devices: vec!["Pixel".into()] };
        assert!(pixel.device_allowed("abc", Some("Pixel")));
        assert!(!pixel.device_allowed("abc", Some("Redmi")));
        let prefix = PushConfig { enabled: true, devices: vec!["abcdef12".into()] };
        assert!(prefix.device_allowed("abcdef1234", None));
        assert!(!PushConfig { enabled: true, devices: vec!["abc".into()] }.device_allowed("abcdef", None));
    }

    #[test]
    fn config_round_trips_next_to_forwarding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"forwarding":{"enabled":true,"ports":"any"},"push":{"enabled":true,"devices":["Pixel"]},"other":1}"#).unwrap();
        let config = crate::forward::HostConfig::load(&path).unwrap();
        assert!(config.forwarding.enabled);
        assert_eq!(config.push, PushConfig { enabled: true, devices: vec!["Pixel".into()] });
        config.save(&path).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["other"], 1);
        assert_eq!(raw["push"]["enabled"], true);
    }

    #[tokio::test]
    async fn lifecycle_add_list_fetch_done() {
        let hub = PushHub::default();
        let mut events = hub.subscribe();
        let id = hub.add(item("/tmp/x.apk"), vec!["phone-a".into(), "phone-b".into()]);
        assert_eq!(hub.pending_for("phone-a").len(), 1);
        assert_eq!(hub.pending_for("phone-c").len(), 0);
        // Only targets may fetch, only the exact path.
        assert!(hub.authorize_fetch("phone-a", "A", "/tmp/x.apk"));
        assert!(!hub.authorize_fetch("phone-c", "C", "/tmp/x.apk"));
        assert!(!hub.authorize_fetch("phone-a", "A", "/tmp/other"));
        assert!(matches!(events.recv().await.unwrap(), ControlEvent::Fetching { .. }));
        hub.done("phone-a", "A", &id, PushOutcome::Installed, None);
        assert!(matches!(events.recv().await.unwrap(), ControlEvent::Done { outcome: PushOutcome::Installed, .. }));
        assert!(hub.pending_for("phone-a").is_empty(), "answered: gone for A");
        assert!(!hub.authorize_fetch("phone-a", "A", "/tmp/x.apk"), "answered: no more fetching");
        assert_eq!(hub.pending_for("phone-b").len(), 1, "B still has it");
        hub.done("phone-b", "B", &id, PushOutcome::Dismissed, None);
        assert!(matches!(events.recv().await.unwrap(), ControlEvent::Done { outcome: PushOutcome::Dismissed, .. }));
        assert!(matches!(events.recv().await.unwrap(), ControlEvent::Finished { .. }));
        assert!(hub.all().is_empty());
        // Idempotent: a second answer is ignored.
        hub.done("phone-b", "B", &id, PushOutcome::Saved, None);
    }

    #[tokio::test]
    async fn long_poll_wakes_on_add_and_times_out_empty() {
        let hub = PushHub::default();
        let start = std::time::Instant::now();
        assert!(hub.list("p", Duration::from_millis(150)).await.is_empty());
        assert!(start.elapsed() >= Duration::from_millis(140));
        let waiter = tokio::spawn({
            let hub = hub.clone();
            async move { hub.list("p", Duration::from_secs(10)).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(hub.any_live(&["p".into()], now_ms()), "a long-poll counts as live");
        // A push for someone else doesn't wake it with results…
        hub.add(item("/tmp/a"), vec!["other".into()]);
        hub.add(item("/tmp/b"), vec!["p".into()]);
        let got = tokio::time::timeout(Duration::from_secs(2), waiter).await.unwrap().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/tmp/b");
    }

    #[tokio::test]
    async fn disabled_config_lists_nothing_without_error() {
        let hub = PushHub::default();
        hub.add(item("/tmp/a"), vec!["p".into()]);
        let off = list_for_device(&hub, &PushConfig::default(), "p", None, "pc", Duration::ZERO).await;
        assert!(!off.enabled && off.pushes.is_empty());
        let on = list_for_device(&hub, &PushConfig { enabled: true, devices: vec![] }, "p", None, "pc", Duration::ZERO).await;
        assert!(on.enabled && on.pushes.len() == 1);
    }

    #[test]
    fn persists_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("pushes.json");
        let hub = PushHub::new(Some(store.clone()));
        let id = hub.add(item("/tmp/a"), vec!["p".into()]);
        let again = PushHub::new(Some(store.clone()));
        assert_eq!(again.all()[0].item.id.as_str(), id, "survives a restart");
        // Age it past the TTL.
        let mut pushes: Vec<PendingPush> = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
        pushes[0].item.created_at = now_ms() - PUSH_TTL_MS - 1;
        std::fs::write(&store, serde_json::to_vec(&pushes).unwrap()).unwrap();
        assert!(PushHub::new(Some(store)).all().is_empty());
    }

    #[test]
    fn cancel_one_or_all() {
        let hub = PushHub::default();
        let a = hub.add(item("/tmp/a"), vec!["p".into()]);
        hub.add(item("/tmp/b"), vec!["p".into()]);
        assert_eq!(hub.cancel(Some(&a)), vec![a]);
        assert_eq!(hub.all().len(), 1);
        assert_eq!(hub.cancel(None).len(), 1);
        assert!(hub.all().is_empty());
    }

    #[test]
    fn prepare_checks_gating_file_and_devices() {
        let dir = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(dir.path());
        let file = dir.path().join("app.apk");
        std::fs::write(&file, b"apk").unwrap();
        let (sha, size) = sha256_file(&file).unwrap();
        let path = file.to_string_lossy().into_owned();
        let devices = vec![device("aaaaaaaaaaaa", "Pixel"), device("bbbbbbbbbbbb", "Redmi")];
        let on = PushConfig { enabled: true, devices: vec![] };
        // Off by default.
        let err = prepare_push(&paths, &PushConfig::default(), &devices, &path, None, None, &[], &sha, size).unwrap_err();
        assert!(matches!(err, ControlEvent::Error { hint: Some(ref h), .. } if h.contains("push enable")));
        // Happy path, all devices, apk mime.
        let (item, targets) = prepare_push(&paths, &on, &devices, &path, None, Some("hi".into()), &[], &sha, size).unwrap();
        assert_eq!((item.name.as_str(), item.mime.as_str(), targets.len()), ("app.apk", "application/vnd.android.package-archive", 2));
        assert_eq!(item.note.as_deref(), Some("hi"));
        // --device filter.
        let (_, targets) = prepare_push(&paths, &on, &devices, &path, None, None, &["Redmi".into()], &sha, size).unwrap();
        assert_eq!(targets[0].name, "Redmi");
        assert!(prepare_push(&paths, &on, &devices, &path, None, None, &["Nope".into()], &sha, size).is_err());
        // Config device grant narrows the targets.
        let pixel_only = PushConfig { enabled: true, devices: vec!["Pixel".into()] };
        let (_, targets) = prepare_push(&paths, &pixel_only, &devices, &path, None, None, &[], &sha, size).unwrap();
        assert_eq!(targets.len(), 1);
        assert!(prepare_push(&paths, &pixel_only, &devices, &path, None, None, &["Redmi".into()], &sha, size).is_err());
        // No paired phones.
        assert!(prepare_push(&paths, &on, &[], &path, None, None, &[], &sha, size).is_err());
        // The file changed after the CLI hashed it.
        assert!(prepare_push(&paths, &on, &devices, &path, None, None, &[], &"0".repeat(64), size).is_err());
        // Not a file / relative / empty.
        assert!(prepare_push(&paths, &on, &devices, &dir.path().to_string_lossy(), None, None, &[], &sha, size).is_err());
        assert!(prepare_push(&paths, &on, &devices, "app.apk", None, None, &[], &sha, size).is_err());
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert!(prepare_push(&paths, &on, &devices, &empty.to_string_lossy(), None, None, &[], &sha, 0).is_err());
    }

    #[test]
    fn sizes_and_mimes() {
        assert_eq!(human_size(12 * 1024 * 1024), "12.0 MB");
        assert_eq!(human_size(640 * 1024), "640 KB");
        assert_eq!(human_size(12), "12 B");
        assert_eq!(mime_for(Path::new("/x/A.APK")), "application/vnd.android.package-archive");
        assert_eq!(mime_for(Path::new("/x/y")), "application/octet-stream");
    }
}
