use crate::{
    data_dir::HostPaths,
    devices::DeviceStore,
    pairing::{now_unix, PairingStore, VerifyOutcome},
};
use iroh::{
    protocol::{AcceptError, ProtocolHandler},
    EndpointId,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::{FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, Request,
};

/// UNI-229: the device id this PC's own local client (`app.sock`) acts as in
/// per-device allow lists and logs. Never a real iroh id (those are 64 hex chars).
pub const LOCAL_DEVICE_ID: &str = "local";
/// The name shown for the local client ("This computer").
pub const LOCAL_DEVICE_NAME: &str = "This computer";

/// Who opened a stream: a paired phone over iroh, or this PC's own UniPi app
/// over the local socket (same user, checked by uid — it is the owner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    Iroh(EndpointId),
    Local,
}

impl Peer {
    /// The id used in device allow lists / push / sharing.
    pub fn device_id(&self) -> String {
        match self {
            Peer::Iroh(id) => id.to_string(),
            Peer::Local => LOCAL_DEVICE_ID.to_string(),
        }
    }
}

impl std::fmt::Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Peer::Iroh(id) => write!(f, "{id}"),
            Peer::Local => f.write_str(LOCAL_DEVICE_ID),
        }
    }
}

impl From<EndpointId> for Peer {
    fn from(id: EndpointId) -> Self {
        Peer::Iroh(id)
    }
}

#[derive(Debug, Clone)]
pub struct Handler {
    pub paths: HostPaths,
    pub host_id: EndpointId,
    pub host_name: String,
    pub lock: Arc<tokio::sync::Mutex<()>>,
    /// UNI-317: the host-owned cron scheduler, shared with the CLI's
    /// `control.sock` handler so both reach the same jobs/runs.
    pub cron: crate::cron::Scheduler,
}
impl ProtocolHandler for Handler {
    async fn accept(&self, connection: iroh::endpoint::Connection) -> Result<(), AcceptError> {
        tracing::info!(remote=%connection.remote_id(), "connect");
        loop {
            let (send, recv) = connection.accept_bi().await?;
            let handler = self.clone();
            let remote = connection.remote_id();
            tokio::spawn(async move {
                if let Err(error) = handler.handle(recv, send, Peer::Iroh(remote)).await {
                    tracing::warn!(%error,"stream error");
                }
            });
        }
    }
}
pub async fn read_frame<R: AsyncRead + Unpin>(read: &mut R) -> anyhow::Result<(u8, Vec<u8>)> {
    let kind = read.read_u8().await?;
    let len = read.read_u32().await? as usize;
    anyhow::ensure!(kind <= 2, "unknown frame kind");
    anyhow::ensure!(len <= MAX_FRAME, "oversize frame");
    let mut bytes = vec![0; len];
    read.read_exact(&mut bytes).await?;
    Ok((kind, bytes))
}
pub async fn write_control<W: AsyncWrite + Unpin>(
    write: &mut W,
    control: &Control,
) -> anyhow::Result<()> {
    write
        .write_all(&unipi_proto::frame::encode(
            FrameKind::Control,
            &serde_json::to_vec(control)?,
        ))
        .await?;
    write.flush().await?;
    Ok(())
}
fn error(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}
fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    error(AppErrorCode::E_INTERNAL, e.to_string())
}

impl Handler {
    pub async fn handle<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
        &self,
        mut read: R,
        mut write: W,
        remote: impl Into<Peer>,
    ) -> anyhow::Result<()> {
        let remote: Peer = remote.into();
        let result = self.request(&mut read, &mut write, remote).await;
        if let Err(error) = result {
            tracing::warn!(code=?error.code,"operation rejected");
            write_control(&mut write, &Control::Err { error }).await?;
        }
        // A finished `tcp_forward` pipe has already closed the send side.
        let _ = write.shutdown().await;
        Ok(())
    }
    async fn request<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
        &self,
        read: &mut R,
        write: &mut W,
        peer: Peer,
    ) -> Result<(), AppErrorDto> {
        let (kind, bytes) =
            tokio::time::timeout(std::time::Duration::from_secs(10), read_frame(read))
                .await
                .map_err(|_| {
                    error(
                        AppErrorCode::E_SCHEMA,
                        "First frame timed out after 10 seconds",
                    )
                })?
                .map_err(|e| error(AppErrorCode::E_SCHEMA, e.to_string()))?;
        if kind != 0 {
            return Err(error(
                AppErrorCode::E_SCHEMA,
                "First frame must be JSON control (kind 0)",
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| error(AppErrorCode::E_SCHEMA, e.to_string()))?;
        if value["proto"].as_str().and_then(|s| s.split('.').next()) != Some("2") {
            return Err(error(
                AppErrorCode::E_PROTO_VERSION,
                "Update the host or app: incompatible protocol major version",
            ));
        }
        let hello: Control = serde_json::from_value(value)
            .map_err(|e| error(AppErrorCode::E_SCHEMA, e.to_string()))?;
        let Control::Hello { req, .. } = hello else {
            return Err(error(AppErrorCode::E_SCHEMA, "first frame must be hello"));
        };
        let _guard = self.lock.lock().await;
        let devices = DeviceStore::new(self.paths.devices());
        if let Request::Pair { code, name } = req {
            // Nothing to pair: the local client is this PC's owner already.
            let Peer::Iroh(remote) = peer else {
                return Err(error(
                    AppErrorCode::E_UNAUTHORIZED,
                    "This computer doesn't need pairing",
                ));
            };
            let pairing = PairingStore::new(self.paths.pairing());
            let previous = pairing.current().map_err(internal)?;
            let outcome = pairing
                .verify(code.as_str(), now_unix())
                .map_err(internal)?;
            if outcome != VerifyOutcome::Accepted {
                return Err(error(
                    AppErrorCode::E_PAIRING_INVALID,
                    "Pairing code is invalid, expired or already used",
                ));
            }
            if let Err(save_error) = devices.add(
                &remote,
                name.map(|n| n.as_str().to_owned())
                    .unwrap_or_else(|| "Phone".into()),
            ) {
                if let Some(previous) = previous {
                    crate::data_dir::write_json(&self.paths.pairing(), &previous)
                        .map_err(internal)?;
                }
                return Err(internal(save_error));
            }
            let data =
                serde_json::json!({"host_id":self.host_id.to_string(),"host_name":self.host_name});
            write_control(write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
            return Ok(());
        }
        if let Peer::Iroh(remote) = peer {
            if !devices.is_paired(&remote).map_err(internal)? {
                return Err(error(
                    AppErrorCode::E_UNAUTHORIZED,
                    "Pair this device first",
                ));
            }
            devices.touch_last_seen(&remote).map_err(internal)?;
        }
        let remote = peer.device_id();
        // UNI-232: managing this host is for this PC's own app only.
        if matches!(
            req,
            Request::HostInfo
                | Request::PairBegin { .. }
                | Request::PairStatus { .. }
                | Request::DevicesList
                | Request::DeviceRevoke { .. }
        ) && peer != Peer::Local
        {
            return Err(error(
                AppErrorCode::E_UNAUTHORIZED,
                "Only this computer can manage its host",
            ));
        }
        let device_name = matches!(
            req,
            Request::PortsList
                | Request::TcpForward { .. }
                | Request::FeatureRequest { .. }
                | Request::FeatureRequests
                | Request::FeatureLeave { .. }
                | Request::BrowserStatus
                | Request::BrowserOpen { .. }
                | Request::BrowserClose
                | Request::BrowserWindow { .. }
                | Request::BrowserProfiles
                | Request::BrowserTakeover { .. }
                | Request::PushList { .. }
                | Request::PushDone { .. }
                | Request::BlobGet { .. }
                | Request::CronList
                | Request::CronShow { .. }
                | Request::CronRuns { .. }
                | Request::CronLog { .. }
                | Request::CronAdd(..)
                | Request::CronEdit { .. }
                | Request::CronRm { .. }
                | Request::CronEnable { .. }
                | Request::CronDisable { .. }
                | Request::CronRun { .. }
                | Request::CronStop { .. }
                | Request::CronPauseAll
                | Request::CronResumeAll
                | Request::CronStatus
                | Request::CronParse { .. }
                | Request::CronSettingsGet
                | Request::CronSettingsSet(..)
        )
        .then(|| match peer {
            Peer::Local => Some(LOCAL_DEVICE_NAME.to_string()),
            Peer::Iroh(_) => devices
                .list()
                .ok()
                .and_then(|list| list.into_iter().find(|d| d.id == remote).map(|d| d.name)),
        })
        .flatten();
        drop(_guard);
        match req {
            Request::BlobPut {
                sha256,
                size,
                mime,
                name,
                offset,
            } => {
                tracing::info!(
                    op = "blob_put",
                    sha256 = sha256.as_str(),
                    size = size.value(),
                    "request"
                );
                crate::blob::serve_put(
                    &self.paths.blobs(),
                    read,
                    write,
                    &sha256,
                    size.value(),
                    &mime,
                    name.as_ref(),
                    offset,
                )
                .await?;
            }
            Request::BlobGet {
                sha256,
                path,
                range,
            } => {
                tracing::info!(op = "blob_get", "request");
                // UNI-220: same scope as fs_read ($HOME + extra safe roots).
                let scope = crate::fs::FsScope::load(&self.paths.config()).ok();
                // UNI-214: a pending push's file is readable by the device
                // it was pushed to (it may live outside $HOME), while the
                // owner still has push switched on for that device.
                let pushed = path.as_ref().and_then(|p| {
                    let config = crate::forward::HostConfig::load(&self.paths.config())
                        .ok()?
                        .push;
                    let id = remote.clone();
                    (config.device_allowed(&id, device_name.as_deref())
                        && crate::push::PushHub::shared(&self.paths).authorize_fetch(
                            &id,
                            device_name.as_deref().unwrap_or("phone"),
                            p.as_str(),
                        ))
                    .then(|| std::path::PathBuf::from(p.as_str()))
                });
                crate::blob::serve_get_with(
                    &self.paths.blobs(),
                    // v2: the app-bridge attachment allow-list is gone with
                    // the bridge; path access is scope-gated (plus pushed
                    // files) until the hub registers attachments again.
                    None,
                    scope.as_ref(),
                    pushed.as_deref(),
                    write,
                    sha256.as_ref(),
                    path.as_ref(),
                    range,
                )
                .await?;
            }
            Request::FsRoots => {
                tracing::info!(op = "fs_roots", "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let data = crate::fs::roots(&scope).await?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FsList {
                path,
                hidden,
                outside,
            } => {
                tracing::info!(op = "fs_list", path = path.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved =
                    crate::fs::scoped_for_request(path.as_str(), &scope, outside.unwrap_or(false))?;
                let data = crate::fs::list(&resolved, hidden.unwrap_or(false)).await?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FsRead {
                path,
                range: _range,
                outside,
                as_text,
            } => {
                tracing::info!(op = "fs_read", path = path.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved =
                    crate::fs::scoped_for_request(path.as_str(), &scope, outside.unwrap_or(false))?;
                let (bytes, data) = crate::fs::read_text(&resolved, as_text.unwrap_or(false))?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(&data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
                const CHUNK: usize = 256 * 1024;
                for chunk in bytes.chunks(CHUNK) {
                    write
                        .write_all(&unipi_proto::frame::encode(FrameKind::Output, chunk))
                        .await
                        .map_err(internal)?;
                }
                write_control(write, &Control::Exit { code: Some(0) })
                    .await
                    .map_err(internal)?;
            }
            Request::FsWrite {
                path,
                text,
                base_hash,
                create,
                outside,
            } => {
                tracing::info!(op = "fs_write", path = path.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved =
                    crate::fs::scoped_for_write(path.as_str(), &scope, outside.unwrap_or(false))?;
                let data = crate::fs::write_text(
                    &resolved,
                    &text,
                    base_hash.as_deref(),
                    create.unwrap_or(false),
                )?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FsStat { path, outside } => {
                tracing::info!(op = "fs_stat", path = path.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved =
                    crate::fs::scoped_for_request(path.as_str(), &scope, outside.unwrap_or(false))?;
                let data = crate::fs::stat(&resolved).await?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FsSearch {
                root,
                query,
                kind,
                outside,
            } => {
                tracing::info!(op = "fs_search", root = root.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved =
                    crate::fs::scoped_for_request(root.as_str(), &scope, outside.unwrap_or(false))?;
                let data = crate::fs::search(&resolved, &query, kind).await?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FsMkdir { path, outside } => {
                tracing::info!(op = "fs_mkdir", path = path.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved =
                    crate::fs::scoped_for_write(path.as_str(), &scope, outside.unwrap_or(false))?;
                crate::fs::mkdir(&resolved)?;
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::FsRename { from, to, outside } => {
                tracing::info!(
                    op = "fs_rename",
                    from = from.as_str(),
                    to = to.as_str(),
                    "request"
                );
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let resolved_from =
                    crate::fs::scoped_for_request(from.as_str(), &scope, outside.unwrap_or(false))?;
                let resolved_to =
                    crate::fs::scoped_for_write(to.as_str(), &scope, outside.unwrap_or(false))?;
                crate::fs::rename(&resolved_from, &resolved_to)?;
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::FsImport {
                sha256,
                to,
                overwrite,
                outside,
            } => {
                tracing::info!(op = "fs_import", to = to.as_str(), "request");
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let home = scope.home.clone();
                let default_uploads = crate::fs::resolve_request_path("~/.unipi/uploads", &home)?;
                let requested = crate::fs::resolve_request_path(to.as_str(), &home)?;
                let create_parent = requested.parent() == Some(default_uploads.as_path());
                if create_parent {
                    std::fs::create_dir_all(&default_uploads).map_err(internal)?;
                }
                let resolved_to =
                    crate::fs::scoped_for_write(to.as_str(), &scope, outside.unwrap_or(false))?;
                crate::fs::import_blob(
                    &self.paths.blobs(),
                    sha256.as_str(),
                    &resolved_to,
                    overwrite.unwrap_or(false),
                    create_parent,
                )?;
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::FsDelete {
                path,
                confirm,
                outside,
            } => {
                tracing::info!(op = "fs_delete", path = path.as_str(), "request");
                if !confirm {
                    return Err(error(
                        AppErrorCode::E_SCHEMA,
                        "fs_delete needs confirm:true",
                    ));
                }
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let home = scope.home.clone();
                let resolved =
                    crate::fs::scoped_for_request(path.as_str(), &scope, outside.unwrap_or(false))?;
                let roots_data = crate::fs::roots(&scope)
                    .await
                    .unwrap_or(unipi_proto::FsRootsData { roots: Vec::new() });
                let root_paths: Vec<PathBuf> = roots_data.roots.iter().map(PathBuf::from).collect();
                crate::fs::delete(&resolved, &home, &root_paths)?;
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::Git { path, outside, cmd } => {
                tracing::info!(
                    op = "git",
                    action = cmd.name(),
                    path = path.as_str(),
                    "request"
                );
                let scope = crate::fs::FsScope::load(&self.paths.config())?;
                let outside = outside.unwrap_or(false);
                let dir = crate::git::resolve_dir(path.as_str(), &scope, outside)?;
                let data = crate::git::handle(&dir, &scope, outside, cmd).await?;
                write_control(write, &Control::Ok { data: Some(data) })
                    .await
                    .map_err(internal)?;
            }
            Request::NotifyConfig => {
                tracing::info!(op = "notify_config", "request");
                let key = crate::notify::notify_key_or_create(&self.paths).map_err(internal)?;
                let connection =
                    crate::notify::read_ntfy_config(&crate::notify::notify_config_json_path());
                let data = unipi_proto::NotifyConfigData {
                    server: connection.server,
                    topic: connection.topic,
                    token: connection.token,
                    key: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key),
                    app_detail: if connection.app_detail_full {
                        unipi_proto::NotifyAppDetail::Full
                    } else {
                        unipi_proto::NotifyAppDetail::Minimal
                    },
                };
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::NotifyRegister { endpoint } => {
                tracing::info!(op = "notify_register", "request");
                crate::notify::register_app_endpoint(
                    &crate::notify::app_endpoints_path(),
                    endpoint.as_str(),
                )
                .map_err(internal)?;
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::NotifyTest => {
                tracing::info!(op = "notify_test", "request");
                let key = crate::notify::notify_key_or_create(&self.paths).map_err(internal)?;
                let connection =
                    crate::notify::read_ntfy_config(&crate::notify::notify_config_json_path());
                crate::notify::send_test(&connection, &key)
                    .await
                    .map_err(|e| error(AppErrorCode::E_INTERNAL, e.to_string()))?;
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::PortsList => {
                tracing::info!(op = "ports_list", "request");
                let config = crate::forward::HostConfig::load(&self.paths.config())
                    .map_err(internal)?
                    .forwarding;
                config.check_device(&remote.clone(), device_name.as_deref())?;
                let data = tokio::task::spawn_blocking(move || {
                    crate::forward::list_ports(std::path::Path::new("/proc"), &config)
                })
                .await
                .map_err(internal)?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::TcpForward { port } => {
                tracing::info!(op = "tcp_forward", port = port.value(), "request");
                let config = crate::forward::HostConfig::load(&self.paths.config())
                    .map_err(internal)?
                    .forwarding;
                crate::forward::serve_forward(
                    &config,
                    &remote.clone(),
                    device_name.as_deref(),
                    read,
                    write,
                    port.value(),
                )
                .await?;
            }
            Request::PushList { wait } => {
                let config = crate::forward::HostConfig::load(&self.paths.config())
                    .map_err(internal)?
                    .push;
                let wait =
                    std::time::Duration::from_secs(wait.map(|w| w.value()).unwrap_or(0) as u64);
                let data = crate::push::list_for_device(
                    &crate::push::PushHub::shared(&self.paths),
                    &config,
                    &remote.clone(),
                    device_name.as_deref(),
                    &self.host_name,
                    wait,
                )
                .await;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::PushDone {
                id,
                outcome,
                detail,
            } => {
                tracing::info!(
                    op = "push_done",
                    id = id.as_str(),
                    outcome = outcome.as_str(),
                    "request"
                );
                let config = crate::forward::HostConfig::load(&self.paths.config())
                    .map_err(internal)?
                    .push;
                let device = remote.clone();
                if !config.device_allowed(&device, device_name.as_deref()) {
                    return Err(crate::push::not_allowed());
                }
                let detail = detail.map(|d| d.chars().take(500).collect());
                crate::push::PushHub::shared(&self.paths).done(
                    &device,
                    device_name.as_deref().unwrap_or("phone"),
                    id.as_str(),
                    outcome,
                    detail,
                );
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::BrowserStatus
            | Request::BrowserOpen { .. }
            | Request::BrowserClose
            | Request::BrowserWindow { .. }
            | Request::BrowserProfiles
            | Request::BrowserTakeover { .. } => {
                crate::browser::serve_request(
                    &self.paths,
                    req,
                    &remote.clone(),
                    device_name.as_deref(),
                    read,
                    write,
                )
                .await?;
            }
            Request::FeatureRequest {
                feature,
                scope,
                note,
            } => {
                tracing::info!(
                    op = "feature_request",
                    feature = feature.as_str(),
                    "request"
                );
                let name = device_name.clone().unwrap_or_else(|| "Phone".into());
                let note = note
                    .map(|n| n.as_str().trim().to_string())
                    .filter(|n| !n.is_empty());
                let asked = crate::sharing::ask(
                    &self.paths,
                    &remote.clone(),
                    &name,
                    feature,
                    scope,
                    note,
                    crate::push::now_ms(),
                )
                .map_err(internal)?;
                if asked.notify {
                    let paths = self.paths.clone();
                    let host_name = self.host_name.clone();
                    let item = asked.item.clone();
                    tokio::spawn(async move {
                        crate::sharing::notify_owner(&paths, &host_name, &item).await;
                    });
                }
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(&asked.item).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FeatureRequests => {
                let requests =
                    crate::sharing::for_device(&self.paths, &remote.clone(), crate::push::now_ms())
                        .map_err(internal)?;
                let data = unipi_proto::FeatureRequestsData { requests };
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::FeatureLeave { feature, scope } => {
                tracing::info!(
                    op = "feature_leave",
                    feature = feature.as_str(),
                    scope = scope.as_str(),
                    "request"
                );
                let data = crate::sharing::leave(
                    &self.paths,
                    feature,
                    scope,
                    &remote.clone(),
                    device_name.as_deref(),
                    crate::push::now_ms(),
                )
                .map_err(|e| {
                    match e.downcast_ref::<crate::sharing::Refused>() {
                        Some(refused) => refused.error.clone(),
                        None => internal(e),
                    }
                })?;
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::Pair { .. } => {}
            Request::HostInfo => {
                let owner = match crate::instance::status(&self.paths) {
                    Ok(crate::instance::Status::Running { owner: Some(o) }) => Some(o),
                    _ => None,
                };
                let devices = DeviceStore::new(self.paths.devices())
                    .list()
                    .map_err(internal)?
                    .len() as u32;
                let data = unipi_proto::HostInfoData {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    proto: unipi_proto::PROTO_VERSION.to_string(),
                    mode: owner
                        .as_ref()
                        .map(|o| o.mode.as_str())
                        .unwrap_or("manual")
                        .to_string(),
                    pid: std::process::id(),
                    started_at: owner.map(|o| o.started_at).unwrap_or(0),
                    host_id: self.host_id.to_string(),
                    host_name: self.host_name.clone(),
                    data_dir: self.paths.root.display().to_string(),
                    tools: crate::admin::tools(),
                    devices,
                };
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::PairBegin { ttl_secs } => {
                let cache: Option<crate::endpoint::EndpointCache> =
                    crate::data_dir::read_json(&self.paths.endpoint()).map_err(internal)?;
                let cache = cache.ok_or_else(|| {
                    error(
                        AppErrorCode::E_INTERNAL,
                        "The host hasn't published its address yet — try again in a moment",
                    )
                })?;
                let ttl = ttl_secs
                    .map(|t| t.value())
                    .unwrap_or(unipi_proto::admin::PAIR_TTL_DEFAULT)
                    as u64;
                let started =
                    crate::pair_cli::start_in_serve(&self.paths, &cache.addr, ttl, &self.host_name)
                        .map_err(internal)?;
                tracing::info!(op = "pair_begin", ttl, "request");
                let data = unipi_proto::PairBeginData {
                    link: started.link,
                    code: started.state.code,
                    expires_at: started.state.expires_at,
                    host_name: started.host,
                };
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::PairStatus { code, cancel } => {
                use crate::pair_cli::PairEvent;
                let mut event =
                    crate::pair_cli::status_for_code(&self.paths, code.as_str(), now_unix())
                        .map_err(internal)?;
                if cancel {
                    if let PairEvent::Waiting { .. } = event {
                        if let Some(current) = PairingStore::new(self.paths.pairing())
                            .current()
                            .map_err(internal)?
                        {
                            crate::pair_cli::cancel(&self.paths, &current).map_err(internal)?;
                        }
                        event = PairEvent::Expired {
                            reason: crate::pair_cli::ExpiredReason::Cancelled,
                        };
                    }
                }
                let data = match event {
                    PairEvent::Waiting { expires_at, .. } => {
                        unipi_proto::PairStatusData::Waiting { expires_at }
                    }
                    PairEvent::Paired { device } => unipi_proto::PairStatusData::Paired { device },
                    PairEvent::Expired { reason } => unipi_proto::PairStatusData::Expired {
                        reason: match reason {
                            crate::pair_cli::ExpiredReason::Timeout => {
                                unipi_proto::PairExpiredReason::Timeout
                            }
                            crate::pair_cli::ExpiredReason::Replaced => {
                                unipi_proto::PairExpiredReason::Replaced
                            }
                            crate::pair_cli::ExpiredReason::Locked => {
                                unipi_proto::PairExpiredReason::Locked
                            }
                            crate::pair_cli::ExpiredReason::Cancelled => {
                                unipi_proto::PairExpiredReason::Cancelled
                            }
                        },
                    },
                    PairEvent::Error { message, .. } => {
                        return Err(error(AppErrorCode::E_INTERNAL, message))
                    }
                };
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(serde_json::to_value(data).map_err(internal)?),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::DevicesList => {
                let devices = DeviceStore::new(self.paths.devices())
                    .list()
                    .map_err(internal)?
                    .into_iter()
                    .map(|d| unipi_proto::DeviceItem {
                        id: d.id,
                        name: d.name,
                        paired_at: d.paired_at,
                        last_seen: d.last_seen,
                    })
                    .collect();
                write_control(
                    write,
                    &Control::Ok {
                        data: Some(
                            serde_json::to_value(unipi_proto::DevicesListData { devices })
                                .map_err(internal)?,
                        ),
                    },
                )
                .await
                .map_err(internal)?;
            }
            Request::DeviceRevoke { id } => {
                let store = DeviceStore::new(self.paths.devices());
                let matches: Vec<_> = store
                    .list()
                    .map_err(internal)?
                    .into_iter()
                    .filter(|d| d.id == id.as_str() || d.id.starts_with(id.as_str()))
                    .collect();
                let [device] = matches.as_slice() else {
                    return Err(error(
                        AppErrorCode::E_NOT_FOUND,
                        if matches.is_empty() {
                            "No such phone"
                        } else {
                            "That id matches more than one phone"
                        },
                    ));
                };
                let parsed: EndpointId = device.id.parse().map_err(internal)?;
                store.revoke(&parsed).map_err(internal)?;
                tracing::info!(op = "device_revoke", name = device.name.as_str(), "request");
                write_control(write, &Control::Ok { data: None })
                    .await
                    .map_err(internal)?;
            }
            Request::CronList
            | Request::CronShow { .. }
            | Request::CronRuns { .. }
            | Request::CronLog { .. }
            | Request::CronAdd(..)
            | Request::CronEdit { .. }
            | Request::CronRm { .. }
            | Request::CronEnable { .. }
            | Request::CronDisable { .. }
            | Request::CronRun { .. }
            | Request::CronStop { .. }
            | Request::CronPauseAll
            | Request::CronResumeAll
            | Request::CronStatus
            | Request::CronParse { .. }
            | Request::CronSettingsGet
            | Request::CronSettingsSet(..) => {
                crate::cron_serve::serve_request(
                    &self.paths,
                    &self.cron,
                    req,
                    &remote.clone(),
                    device_name.as_deref(),
                    write,
                )
                .await?;
            }
        }
        Ok(())
    }
}
struct ClientGuard {
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    waiter: Option<std::thread::JoinHandle<Option<i64>>>,
}
impl Drop for ClientGuard {
    fn drop(&mut self) {
        let _ = self.killer.kill();
        if let Some(waiter) = self.waiter.take() {
            let _ = waiter.join();
        }
    }
}
pub async fn attach_stream<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    read: &mut R,
    write: &mut W,
    command: portable_pty::CommandBuilder,
    cols: u16,
    rows: u16,
) -> anyhow::Result<()> {
    use portable_pty::{native_pty_system, PtySize};
    use std::io::{Read, Write};
    let pair = native_pty_system().openpty(PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    tracing::info!(cols, rows, "pty attach");
    let mut child = pair.slave.spawn_command(command).map_err(|error| {
        tracing::error!("pty spawn failed: {error}");
        error
    })?;
    let (exit_tx, mut exit_rx) = tokio::sync::oneshot::channel();
    let mut guard = ClientGuard {
        killer: child.clone_killer(),
        waiter: None,
    };
    guard.waiter = Some(std::thread::spawn(move || {
        let code = child.wait().map(|status| status.exit_code() as i64).ok();
        let _ = exit_tx.send(code);
        code
    }));
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let mut input = pair.master.take_writer()?;
    // Windows: portable-pty opens ConPTY with INHERIT_CURSOR, so ConPTY's
    // first output is a cursor-position query (ESC[6n) and it renders
    // NOTHING until that is answered. The phone's terminal isn't attached
    // yet when it arrives, so the host answers it (top-left) and keeps it
    // off the wire — otherwise every herdr attach on Windows stays blank.
    let mut awaiting_dsr = cfg!(windows);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    tokio::task::spawn_blocking(move || {
        let mut buf = [0; 8192];
        let mut total = 0usize;
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    total += n;
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
        // Silent before: a PTY that produced nothing looked exactly like a
        // phone that couldn't render.
        tracing::info!(bytes = total, "pty reader ended");
    });
    loop {
        tokio::select! {
         bytes=rx.recv()=>{if let Some(mut bytes)=bytes {
             if awaiting_dsr {
                 awaiting_dsr=false;
                 if let Some(at)=bytes.windows(4).position(|w|w==b"\x1b[6n") {
                     bytes.drain(at..at+4);
                     if input.write_all(b"\x1b[1;1R").is_err() || input.flush().is_err() { break; }
                 }
                 if bytes.is_empty() { continue; }
             }
             if write.write_all(&unipi_proto::frame::encode(FrameKind::Output,&bytes)).await.is_err(){break;}}else{let code=(&mut exit_rx).await.unwrap_or(None);write_control(write,&Control::Exit{code}).await?;break;}}
         frame=read_frame(read)=>{let Ok((kind,bytes))=frame else{break;};if kind==2 {if input.write_all(&bytes).is_err(){break;}}else if kind==0 {if let Ok(Control::Resize{cols,rows})=serde_json::from_slice(&bytes){pair.master.resize(PtySize{cols:cols.value() as u16,rows:rows.value() as u16,pixel_width:0,pixel_height:0})?;}}}
         code=&mut exit_rx=>{while let Some(bytes)=rx.recv().await {write.write_all(&unipi_proto::frame::encode(FrameKind::Output,&bytes)).await?;}write_control(write,&Control::Exit{code:code.unwrap_or(None)}).await?;break;}
        }
    }
    drop(guard);
    Ok(())
}
