//! oino-casing-bridge: a mini "unipi-host" for the casing spike (round 6).
//!
//! Serves the unipi app's `unipi/app/1` frame protocol on a private unix socket
//! (default `~/.oino-casing/app.sock`) and translates ONLY the chat ops to
//! `oino --mode rpc` child processes:
//!
//! - `host_info`, `chat_list`, `chat_new`, `chat_open`, `chat_close`,
//!   `chat_config` — session lifecycle;
//! - `fs_roots` / `fs_list` / `fs_stat` — the New-chat folder picker;
//! - everything else answers `E_UNSUPPORTED` (board, browser, cron, push, …).
//!
//! Chats are `oino --mode rpc` children with an ISOLATED agent dir
//! (`<dir>/agent`, seeded with auth.json only) — never `~/.oino` and never
//! `~/.unipi`. Data dir default: `~/.oino-casing` (NOT `~/.unipi`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use unipi_proto::frame::{FrameKind, MAX_FRAME, encode};
use unipi_proto::{AppErrorCode, AppErrorDto, Control, Request, PROTO_VERSION};

mod chat;
mod rpc;

use chat::Chats;

/// Bridge state shared by all connections.
pub struct Bridge {
    /// Socket/data dir (e.g. `~/.oino-casing`).
    pub dir: PathBuf,
    /// `oino` binary to spawn.
    pub oino_bin: PathBuf,
    /// Isolated agent dir for chat children.
    pub agent_dir: PathBuf,
    /// Default model (`provider/model`), optional.
    pub model: Option<String>,
    /// `-e builtin:faux` demo mode (scripted provider, no network).
    pub faux: bool,
    pub chats: Chats,
}

fn error(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    error(AppErrorCode::E_INTERNAL, e.to_string())
}

#[tokio::main]
async fn main() {
    let home = std::env::var("HOME").ok().map(PathBuf::from).unwrap_or_default();
    let dir = PathBuf::from(std::env::var("OINO_CASING_DIR").unwrap_or_else(|_| {
        home.join(".oino-casing").display().to_string()
    }));
    let oino_bin = std::env::var("OINO_CASING_OINO_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| find_oino());
    let agent_dir = dir.join("agent");
    let faux = std::env::var("OINO_CASING_FAUX").is_ok_and(|v| v == "1");
    let model = std::env::var("OINO_CASING_MODEL")
        .ok()
        .filter(|m| !m.is_empty())
        .or_else(|| faux.then(|| "faux/faux-1".to_owned()));

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("casing-bridge: cannot create {}: {e}", dir.display());
        std::process::exit(1);
    }
    // Isolated agent dir: fresh, seeded with ONLY auth.json from ~/.oino
    // (same pattern as scripts/desktop.sh). Never ~/.unipi, never live ~/.oino.
    if !agent_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&agent_dir) {
            eprintln!("casing-bridge: cannot create {}: {e}", agent_dir.display());
            std::process::exit(1);
        }
        let auth = home.join(".oino").join("auth.json");
        if auth.is_file() {
            let _ = std::fs::copy(&auth, agent_dir.join("auth.json"));
        }
    }
    if !oino_bin.is_file() {
        eprintln!(
            "casing-bridge: oino binary not found at {} (set OINO_CASING_OINO_BIN)",
            oino_bin.display()
        );
        std::process::exit(1);
    }

    // endpoint.json: a real (never-dialed) iroh ticket so the app's
    // "This computer" detection + ticket parsing succeed offline. The key is
    // persisted so the host id survives bridge restarts.
    let identity = dir.join("identity.key");
    let secret = match std::fs::read(&identity) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut array = [0u8; 32];
            array.copy_from_slice(&bytes);
            iroh::SecretKey::from_bytes(&array)
        }
        _ => {
            let secret = iroh::SecretKey::generate();
            let _ = std::fs::write(&identity, secret.to_bytes());
            secret
        }
    };
    let addr = iroh::EndpointAddr::new(secret.public());
    let ticket = iroh_tickets::endpoint::EndpointTicket::new(addr.clone()).to_string();
    let cache = json!({"addr": {"id": addr.id.to_string(), "addrs": []}, "ticket": ticket});
    if let Err(e) = std::fs::write(dir.join("endpoint.json"), cache.to_string()) {
        eprintln!("casing-bridge: cannot write endpoint.json: {e}");
        std::process::exit(1);
    }

    let bridge = Arc::new(Bridge {
        dir: dir.clone(),
        oino_bin,
        agent_dir,
        model,
        faux,
        chats: Chats::default(),
    });

    let socket = dir.join("app.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("casing-bridge: cannot bind {}: {e}", socket.display());
            std::process::exit(1);
        }
    };
    eprintln!(
        "casing-bridge: serving {} (proto {}, oino {}, agent dir {})",
        socket.display(),
        PROTO_VERSION,
        bridge.oino_bin.display(),
        bridge.agent_dir.display()
    );

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let bridge = Arc::clone(&bridge);
                tokio::spawn(async move {
                    if let Err(e) = serve_connection(bridge, stream).await {
                        eprintln!("casing-bridge: connection ended: {e}");
                    }
                });
            }
            Err(e) => {
                eprintln!("casing-bridge: accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

/// Where the oino binary lives by default: repo layout (three target dirs).
fn find_oino() -> PathBuf {
    let mut here = std::env::current_dir().ok();
    for _ in 0..6 {
        let Some(dir) = here else { break };
        let candidate = dir.join("target/release/oino");
        if candidate.is_file() {
            return candidate;
        }
        here = dir.parent().map(Path::to_path_buf);
    }
    PathBuf::from("oino")
}

pub async fn read_frame<R: AsyncReadExt + Unpin>(read: &mut R) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0u8; 5];
    match read.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let kind = header[0];
    let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if kind > 2 || len > MAX_FRAME {
        return Err(std::io::Error::other("bad frame"));
    }
    let mut payload = vec![0u8; len];
    read.read_exact(&mut payload).await?;
    Ok(Some((kind, payload)))
}

pub async fn write_control<W: AsyncWriteExt + Unpin>(write: &mut W, control: &Control) -> std::io::Result<()> {
    write
        .write_all(&encode(FrameKind::Control, &serde_json::to_vec(control).unwrap_or_default()))
        .await?;
    write.flush().await
}

/// `write_control` over a shared (locked) writer.
async fn write_control_locked<W: AsyncWriteExt + Unpin>(
    write: &Arc<tokio::sync::Mutex<W>>,
    control: &Control,
) -> std::io::Result<()> {
    let mut guard = write.lock().await;
    write_control(&mut *guard, control).await
}

/// One connection = one logical op (mirrors crates/host protocol.rs).
async fn serve_connection(bridge: Arc<Bridge>, stream: UnixStream) -> std::io::Result<()> {
    let (read, write) = stream.into_split();
    let write = Arc::new(tokio::sync::Mutex::new(write));
    let result = handle_op(&bridge, read, Arc::clone(&write)).await;
    if let Err(app_error) = result {
        let mut guard = write.lock().await;
        let _ = write_control(&mut *guard, &Control::Err { error: app_error }).await;
    }
    let mut guard = write.lock().await;
    let _ = guard.shutdown().await;
    Ok(())
}

async fn handle_op<R: AsyncReadExt + Unpin, W: AsyncWriteExt + Unpin + Send + 'static>(
    bridge: &Arc<Bridge>,
    mut read: R,
    write: Arc<tokio::sync::Mutex<W>>,
) -> Result<(), AppErrorDto> {
    let (kind, bytes) = tokio::time::timeout(std::time::Duration::from_secs(10), read_frame(&mut read))
        .await
        .map_err(|_| error(AppErrorCode::E_SCHEMA, "First frame timed out after 10 seconds"))?
        .map_err(|e| error(AppErrorCode::E_SCHEMA, e.to_string()))?
        .ok_or_else(|| error(AppErrorCode::E_SCHEMA, "no hello"))?;
    if kind != 0 {
        return Err(error(AppErrorCode::E_SCHEMA, "First frame must be JSON control (kind 0)"));
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| error(AppErrorCode::E_SCHEMA, e.to_string()))?;
    if value["proto"].as_str().and_then(|s| s.split('.').next()) != Some("1") {
        return Err(error(AppErrorCode::E_PROTO_VERSION, "incompatible protocol major version"));
    }
    let hello: Control =
        serde_json::from_value(value).map_err(|e| error(AppErrorCode::E_SCHEMA, e.to_string()))?;
    let Control::Hello { req, .. } = hello else {
        return Err(error(AppErrorCode::E_SCHEMA, "first frame must be hello"));
    };

    match req {
        Request::HostInfo => {
            let data = json!({
                "version": "1.0.0-casing",
                "proto": PROTO_VERSION,
                "mode": "manual",
                "pid": std::process::id(),
                "startedAt": 0,
                "hostId": "oino-casing",
                "hostName": "This computer",
                "dataDir": bridge.dir.display().to_string(),
                "tools": [],
                "devices": 0,
            });
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::ChatList => {
            let data = bridge.chats.list();
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::ChatNew { target, cwd, .. } => {
            // Any target shape: the bridge has exactly one "workspace" per chat.
            let cwd = cwd
                .as_ref()
                .map(|c| c.as_str().to_owned())
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_else(|| ".".into());
            let data = bridge
                .chats
                .new_chat(&bridge.oino_bin, &bridge.agent_dir, bridge.model.as_deref(), bridge.faux, target.session().to_owned(), &cwd)
                .await
                .map_err(internal)?;
            write_control_locked(&write, &Control::Ok { data: Some(serde_json::to_value(data).unwrap_or(Value::Null)) })
                .await
                .map_err(internal)?;
        }
        Request::ChatOpen { pid } => {
            bridge.chats.open(pid, read, write).await?;
        }
        Request::ChatClose { pid, .. } => {
            let data = bridge.chats.close(pid.get()).await.map_err(internal)?;
            write_control_locked(&write, &Control::Ok { data: Some(serde_json::to_value(data).unwrap_or(Value::Null)) })
                .await
                .map_err(internal)?;
        }
        Request::ChatConfig | Request::ChatConfigSet { .. } => {
            // herdr:true — the bridge fakes one herdr session ("oino") so the
            // New chat sheet offers its "new workspace" targets.
            let data = json!({"launcher": "oino", "default": true, "herdr": true, "tmux": false});
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::FsRoots => {
            let home = std::env::var("HOME").unwrap_or_default();
            let data = json!({"roots": [home]});
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::FsList { path, .. } => {
            let data = fs_list(path.as_str()).map_err(internal)?;
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::FsStat { path, .. } => {
            let data = fs_stat(path.as_str()).map_err(internal)?;
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::Tree => {
            // The same fake herdr tree as chat_list ("oino"), as raw TreeData.
            let data = bridge.chats.list();
            write_control_locked(&write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
        }
        Request::List { .. } | Request::Shortcuts => {
            // No mux: the app's Terminal screen shows nothing.
            write_control_locked(&write, &Control::Ok { data: Some(json!({"mux": "none"})) })
                .await
                .map_err(internal)?;
        }
        other => {
            let op = op_name(&other);
            return Err(error(
                AppErrorCode::E_UNSUPPORTED,
                format!("{op} is not part of the oino casing spike yet"),
            ));
        }
    }
    Ok(())
}

fn op_name(req: &Request) -> &'static str {
    match req {
        Request::Pair { .. } => "pair",
        Request::MuxAction { .. } => "mux_action",
        Request::Preview { .. } => "preview",
        Request::Attach { .. } => "attach",
        Request::Shell { .. } => "shell",
        Request::BoardHttp { .. } => "board_http",
        Request::BoardSse { .. } => "board_sse",
        Request::BlobPut { .. } => "blob_put",
        Request::BlobGet { .. } => "blob_get",
        Request::FsRead { .. } => "fs_read",
        Request::FsWrite { .. } => "fs_write",
        Request::FsSearch { .. } => "fs_search",
        Request::FsMkdir { .. } => "fs_mkdir",
        Request::FsRename { .. } => "fs_rename",
        Request::FsImport { .. } => "fs_import",
        Request::FsDelete { .. } => "fs_delete",
        Request::Git { .. } => "git",
        Request::NotifyConfig => "notify_config",
        Request::NotifyRegister { .. } => "notify_register",
        Request::NotifyTest => "notify_test",
        Request::PortsList => "ports_list",
        Request::TcpForward { .. } => "tcp_forward",
        Request::PushList { .. } => "push_list",
        Request::PushDone { .. } => "push_done",
        Request::BrowserStatus => "browser_status",
        Request::BrowserOpen { .. } => "browser_open",
        Request::BrowserClose => "browser_close",
        Request::BrowserWindow { .. } => "browser_window",
        Request::BrowserProfiles => "browser_profiles",
        Request::BrowserTakeover { .. } => "browser_takeover",
        Request::DesktopStatus => "desktop_status",
        Request::DesktopOpen { .. } => "desktop_open",
        Request::FeatureRequest { .. } => "feature_request",
        Request::FeatureRequests => "feature_requests",
        Request::FeatureLeave { .. } => "feature_leave",
        Request::PairBegin { .. } => "pair_begin",
        Request::PairStatus { .. } => "pair_status",
        Request::DevicesList => "devices_list",
        Request::DeviceRevoke { .. } => "device_revoke",
        Request::CronList => "cron_list",
        Request::CronShow { .. } => "cron_show",
        Request::CronRuns { .. } => "cron_runs",
        Request::CronLog { .. } => "cron_log",
        Request::CronAdd(..) => "cron_add",
        Request::CronEdit { .. } => "cron_edit",
        Request::CronRm { .. } => "cron_rm",
        Request::CronEnable { .. } => "cron_enable",
        Request::CronDisable { .. } => "cron_disable",
        Request::CronRun { .. } => "cron_run",
        Request::CronStop { .. } => "cron_stop",
        Request::CronPauseAll => "cron_pause_all",
        Request::CronResumeAll => "cron_resume_all",
        Request::CronStatus => "cron_status",
        Request::CronParse { .. } => "cron_parse",
        Request::CronOpenSession { .. } => "cron_open_session",
        Request::CronSettingsGet => "cron_settings_get",
        Request::CronSettingsSet(..) => "cron_settings_set",
        _ => "unknown",
    }
}

/// One directory listing, the shape `fs_list` answers (dirs first, name-sorted).
fn fs_list(path: &str) -> std::io::Result<Value> {
    let dir = resolve(path);
    let mut out: Vec<Value> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let abs = dir.join(&name).display().to_string();
        out.push(json!({
            "name": name,
            "path": abs,
            "dir": meta.is_dir(),
            "size": meta.len(),
            "modified": mtime(&meta),
        }));
    }
    out.sort_by(|a, b| {
        let dir_a = a["dir"].as_bool().unwrap_or(false);
        let dir_b = b["dir"].as_bool().unwrap_or(false);
        dir_b.cmp(&dir_a).then(a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });
    Ok(json!({
        "path": dir.display().to_string(),
        "dirs": out.iter().any(|x| x["dir"].as_bool().unwrap_or(false)),
        "items": out,
    }))
}

fn fs_stat(path: &str) -> std::io::Result<Value> {
    let p = resolve(path);
    let meta = std::fs::metadata(&p)?;
    Ok(json!({
        "path": p.display().to_string(),
        "dir": meta.is_dir(),
        "size": meta.len(),
        "modified": mtime(&meta),
    }))
}

fn mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn resolve(path: &str) -> PathBuf {
    if path == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest);
    }
    PathBuf::from(path)
}

/// Socket half types shared with chat.rs.
pub type WriteHalf = tokio::net::unix::OwnedWriteHalf;
pub type ReadHalf = tokio::net::unix::OwnedReadHalf;
