//! Port forwarding, phone side (UNI-213): a loopback listener on the
//! device (`127.0.0.1:<port>`, the host's port number when free, else an
//! ephemeral one). Every accepted TCP connection opens ONE `tcp_forward`
//! stream to the paired PC's `unipi-host`, which pipes it to the PC's
//! `127.0.0.1:<port>` — like `ssh -L`. HTTP, WebSockets and Vite HMR work
//! unchanged because the bytes are never interpreted.
//!
//! Commands (the UI's `src/lib/forward.ts`):
//! - `ports_list { hostId }` → `PortsListData` (or `E_FORWARD_DISABLED`)
//! - `forward_start { hostId, port, localPort? }` → `ForwardInfo`. Probes
//!   the host once first so "off on this PC" / "not allowed" / "nothing
//!   listening" come back synchronously. Reuses an existing forward for the
//!   same host+port.
//! - `forward_stop { id }`, `forward_list` → `ForwardInfo[]` (with live
//!   connection counts and the last per-connection error, which the
//!   Preview page polls to show "port closed" / "connection lost").

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, PortsListData, ProtoError, Request, PROTO_VERSION,
};

use crate::net::NetState;

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn internal(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Internal(error.to_string()).to_app_error()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// How long the host may take to answer a `tcp_forward` hello (it
/// connects to its own loopback, ≤ 5 s per address family).
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

fn hello_frame(request: Request) -> Result<Vec<u8>, AppErrorDto> {
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| internal(e))?,
        req: request,
    };
    Ok(encode(
        FrameKind::Control,
        &serde_json::to_vec(&hello).map_err(internal)?,
    ))
}

/// Reads the single control reply that precedes the raw bytes.
async fn read_reply<R: AsyncRead + Unpin>(recv: &mut R) -> Result<Control, AppErrorDto> {
    let read = async {
        let kind = recv
            .read_u8()
            .await
            .map_err(|_| transport("The PC closed the forward without a reply"))?;
        let len = recv.read_u32().await.map_err(transport)? as usize;
        if kind != 0 || len > MAX_FRAME {
            return Err(internal("Invalid reply frame from the PC"));
        }
        let mut bytes = vec![0; len];
        recv.read_exact(&mut bytes).await.map_err(transport)?;
        serde_json::from_slice::<Control>(&bytes).map_err(internal)
    };
    tokio::time::timeout(REPLY_TIMEOUT, read)
        .await
        .map_err(|_| {
            ProtoError::Timeout("The PC did not answer the forward".into()).to_app_error()
        })?
}

/// Sends the `tcp_forward` hello on a fresh stream and waits for `ok`.
pub(crate) async fn handshake<R, W>(
    send: &mut W,
    recv: &mut R,
    port: u16,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let request = Request::TcpForward {
        port: port
            .try_into()
            .map_err(|e: String| AppErrorDto::new(AppErrorCode::E_SCHEMA, e, false))?,
    };
    send.write_all(&hello_frame(request)?)
        .await
        .map_err(transport)?;
    match read_reply(recv).await? {
        Control::Ok { .. } => Ok(()),
        Control::Err { error } => Err(error),
        _ => Err(internal("Unexpected reply to tcp_forward")),
    }
}

/// Pipes a local socket and a forward stream both ways; each direction
/// half-closes its destination when its source ends.
pub(crate) async fn pipe<R, W>(socket: TcpStream, recv: &mut R, send: &mut W) -> (u64, u64)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut local_read, mut local_write) = socket.into_split();
    let up = async {
        let n = tokio::io::copy(&mut local_read, send).await.unwrap_or(0);
        let _ = send.shutdown().await;
        n
    };
    let down = async {
        let n = tokio::io::copy(recv, &mut local_write).await.unwrap_or(0);
        let _ = local_write.shutdown().await;
        n
    };
    tokio::join!(up, down)
}

type BoxStream = (
    Box<dyn AsyncWrite + Send + Unpin>,
    Box<dyn AsyncRead + Send + Unpin>,
);
type BoxFut = Pin<Box<dyn Future<Output = Result<BoxStream, AppErrorDto>> + Send>>;
/// Opens one fresh bistream to the host (redialing as needed).
pub(crate) type Opener = Arc<dyn Fn(bool) -> BoxFut + Send + Sync>;

/// The last per-connection failure, for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardError {
    pub error: AppErrorDto,
    pub at: u64,
}

#[derive(Default)]
pub(crate) struct Stats {
    active: AtomicU64,
    total: AtomicU64,
    bytes_up: AtomicU64,
    bytes_down: AtomicU64,
    last_ok: AtomicU64,
    last_error: Mutex<Option<ForwardError>>,
}

/// Opens one forward stream for an accepted socket, retrying once on a
/// transport error with a fresh connection, then pipes it.
pub(crate) async fn serve_connection(
    socket: TcpStream,
    port: u16,
    opener: Opener,
    stats: Arc<Stats>,
) {
    let _ = socket.set_nodelay(true);
    stats.active.fetch_add(1, Ordering::SeqCst);
    stats.total.fetch_add(1, Ordering::SeqCst);
    let mut result = Err(transport("not attempted"));
    for fresh in [false, true] {
        result = async {
            let (mut send, mut recv) = opener(fresh).await?;
            handshake(&mut send, &mut recv, port).await?;
            Ok::<_, AppErrorDto>((send, recv))
        }
        .await;
        match &result {
            Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => continue,
            _ => break,
        }
    }
    match result {
        Ok((mut send, mut recv)) => {
            stats.last_ok.store(now_ms(), Ordering::SeqCst);
            let (up, down) = pipe(socket, &mut recv, &mut send).await;
            stats.bytes_up.fetch_add(up, Ordering::SeqCst);
            stats.bytes_down.fetch_add(down, Ordering::SeqCst);
        }
        Err(error) => {
            log::warn!(
                "UniPiForward port={port} error={:?} {}",
                error.code,
                error.message.as_str()
            );
            *stats.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(ForwardError {
                error,
                at: now_ms(),
            });
            drop(socket);
        }
    }
    stats.active.fetch_sub(1, Ordering::SeqCst);
}

/// Accept loop. Aborting the returned task drops the JoinSet, which
/// aborts every live connection too.
pub(crate) fn spawn_accept(
    listener: TcpListener,
    port: u16,
    opener: Opener,
    stats: Arc<Stats>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else { break };
                    connections.spawn(serve_connection(socket, port, opener.clone(), stats.clone()));
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    })
}

/// Binds `127.0.0.1:<preferred>` (the host's port number, so absolute
/// `localhost:<port>` URLs in the app keep working), else an ephemeral port.
pub(crate) async fn bind_local(preferred: Option<u16>) -> Result<TcpListener, AppErrorDto> {
    if let Some(port) = preferred.filter(|p| *p >= 1024) {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok(listener);
        }
    }
    TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| internal(format!("Could not open a local port: {e}")))
}

struct Entry {
    host_id: String,
    remote_port: u16,
    local_port: u16,
    started_at: u64,
    stats: Arc<Stats>,
    task: tauri::async_runtime::JoinHandle<()>,
}

/// Running forwards, keyed by id (`<hostId>:<port>`).
#[derive(Default)]
pub struct ForwardRegistry {
    entries: Mutex<HashMap<String, Entry>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardInfo {
    pub id: String,
    pub host_id: String,
    pub remote_port: u16,
    pub local_port: u16,
    pub url: String,
    pub started_at: u64,
    pub active: u64,
    pub total: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub last_ok: Option<u64>,
    pub last_error: Option<ForwardError>,
}

fn info(id: &str, entry: &Entry) -> ForwardInfo {
    let last_ok = entry.stats.last_ok.load(Ordering::SeqCst);
    ForwardInfo {
        id: id.to_string(),
        host_id: entry.host_id.clone(),
        remote_port: entry.remote_port,
        local_port: entry.local_port,
        url: format!("http://127.0.0.1:{}/", entry.local_port),
        started_at: entry.started_at,
        active: entry.stats.active.load(Ordering::SeqCst),
        total: entry.stats.total.load(Ordering::SeqCst),
        bytes_up: entry.stats.bytes_up.load(Ordering::SeqCst),
        bytes_down: entry.stats.bytes_down.load(Ordering::SeqCst),
        last_ok: (last_ok > 0).then_some(last_ok),
        last_error: entry
            .stats
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    }
}

impl ForwardRegistry {
    pub fn list(&self) -> Vec<ForwardInfo> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<_> = entries.iter().map(|(id, entry)| info(id, entry)).collect();
        list.sort_by_key(|f| f.started_at);
        list
    }
    pub fn get(&self, id: &str) -> Option<ForwardInfo> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|entry| info(id, entry))
    }
    pub fn stop(&self, id: &str) -> bool {
        match self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
        {
            Some(entry) => {
                entry.task.abort();
                true
            }
            None => false,
        }
    }
    /// Starts (or returns the running) forward for `host_id:port`.
    pub(crate) async fn start(
        &self,
        host_id: &str,
        port: u16,
        local_port: Option<u16>,
        opener: Opener,
    ) -> Result<ForwardInfo, AppErrorDto> {
        let id = format!("{host_id}:{port}");
        if let Some(existing) = self.get(&id) {
            if local_port.is_none() || local_port == Some(existing.local_port) {
                return Ok(existing);
            }
            self.stop(&id);
        }
        // Probe: one handshake surfaces disabled / not allowed / closed now.
        let mut probe = Err(transport("not attempted"));
        for fresh in [false, true] {
            probe = async {
                let (mut send, mut recv) = opener(fresh).await?;
                handshake(&mut send, &mut recv, port).await?;
                let _ = send.shutdown().await;
                Ok::<_, AppErrorDto>(())
            }
            .await;
            match &probe {
                Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => continue,
                _ => break,
            }
        }
        probe?;
        let listener = bind_local(local_port.or(Some(port))).await?;
        let bound = listener.local_addr().map_err(internal)?.port();
        let stats = Arc::new(Stats::default());
        let task = spawn_accept(listener, port, opener, stats.clone());
        let entry = Entry {
            host_id: host_id.to_string(),
            remote_port: port,
            local_port: bound,
            started_at: now_ms(),
            stats,
            task,
        };
        let result = info(&id, &entry);
        if let Some(old) = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, entry)
        {
            old.task.abort();
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------- commands

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    use tauri::Manager;
    app.path().app_data_dir().map_err(internal)
}

fn host_ticket(dir: &std::path::Path, host_id: &str) -> Result<(String, String), AppErrorDto> {
    let host = crate::local_host::hosts(dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let id = crate::net::parse_ticket(&host.ticket)?
        .endpoint_addr()
        .id
        .to_string();
    Ok((host.ticket, id))
}

/// An opener over the shared iroh endpoint/connection cache (`NetState`
/// lives for the app's lifetime in Tauri's managed state).
pub(crate) fn net_opener(app: &tauri::AppHandle, host_id: &str) -> Result<Opener, AppErrorDto> {
    let dir = directory(app)?;
    let (ticket, remote) = host_ticket(&dir, host_id)?;
    let app = app.clone();
    Ok(Arc::new(move |fresh: bool| {
        let app = app.clone();
        let dir = dir.clone();
        let ticket = ticket.clone();
        let remote = remote.clone();
        Box::pin(async move {
            use tauri::Manager;
            let state = app.state::<NetState>();
            if fresh {
                state.drop_connection(&remote);
            }
            let relay = state.current_relay().await;
            let (send, recv) = state.open_op(&dir, relay.as_deref(), &ticket).await?;
            Ok((
                Box::new(send) as Box<dyn AsyncWrite + Send + Unpin>,
                Box::new(recv) as Box<dyn AsyncRead + Send + Unpin>,
            ))
        }) as BoxFut
    }))
}

#[tauri::command]
pub async fn ports_list(
    app: tauri::AppHandle,
    host_id: String,
) -> Result<PortsListData, AppErrorDto> {
    let opener = net_opener(&app, &host_id)?;
    let mut last = transport("not attempted");
    for fresh in [false, true] {
        let attempt = async {
            let (mut send, mut recv) = opener(fresh).await?;
            send.write_all(&hello_frame(Request::PortsList)?)
                .await
                .map_err(transport)?;
            let _ = send.shutdown().await;
            match read_reply(&mut recv).await? {
                Control::Ok { data } => {
                    serde_json::from_value(data.unwrap_or_default()).map_err(internal)
                }
                Control::Err { error } => Err(error),
                _ => Err(internal("Unexpected reply to ports_list")),
            }
        }
        .await;
        match attempt {
            Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => last = error,
            other => return other,
        }
    }
    Err(last)
}

#[tauri::command]
pub async fn forward_start(
    app: tauri::AppHandle,
    registry: tauri::State<'_, ForwardRegistry>,
    host_id: String,
    port: u16,
    local_port: Option<u16>,
) -> Result<ForwardInfo, AppErrorDto> {
    let opener = net_opener(&app, &host_id)?;
    registry.start(&host_id, port, local_port, opener).await
}

#[tauri::command]
pub fn forward_stop(registry: tauri::State<'_, ForwardRegistry>, id: String) -> bool {
    registry.stop(&id)
}

#[tauri::command]
pub fn forward_list(registry: tauri::State<'_, ForwardRegistry>) -> Vec<ForwardInfo> {
    registry.list()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake host over in-memory duplex pipes: answers each stream's hello
    /// per `reply`, then (when ok) echoes raw bytes back.
    fn fake_opener(reply: Control, transport_failures: Arc<AtomicU64>) -> (Opener, Arc<AtomicU64>) {
        let opened = Arc::new(AtomicU64::new(0));
        let counter = opened.clone();
        let opener: Opener = Arc::new(move |_fresh| {
            let reply = reply.clone();
            let counter = counter.clone();
            let failures = transport_failures.clone();
            Box::pin(async move {
                if failures.load(Ordering::SeqCst) > 0 {
                    failures.fetch_sub(1, Ordering::SeqCst);
                    return Err(transport("connection lost"));
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let (phone, host) = tokio::io::duplex(64 * 1024);
                tokio::spawn(async move {
                    let (mut r, mut w) = tokio::io::split(host);
                    let kind = r.read_u8().await.unwrap();
                    let len = r.read_u32().await.unwrap() as usize;
                    let mut hello = vec![0; len];
                    r.read_exact(&mut hello).await.unwrap();
                    assert_eq!(kind, 0);
                    let hello: serde_json::Value = serde_json::from_slice(&hello).unwrap();
                    assert_eq!(hello["req"]["op"], "tcp_forward");
                    w.write_all(&encode(
                        FrameKind::Control,
                        &serde_json::to_vec(&reply).unwrap(),
                    ))
                    .await
                    .unwrap();
                    if matches!(reply, Control::Ok { .. }) {
                        let _ = tokio::io::copy(&mut r, &mut w).await;
                    }
                    let _ = w.shutdown().await;
                });
                let (r, w) = tokio::io::split(phone);
                Ok((
                    Box::new(w) as Box<dyn AsyncWrite + Send + Unpin>,
                    Box::new(r) as Box<dyn AsyncRead + Send + Unpin>,
                ))
            }) as BoxFut
        });
        (opener, opened)
    }

    #[tokio::test]
    async fn local_listener_forwards_each_connection_over_its_own_stream() {
        let registry = ForwardRegistry::default();
        let (opener, opened) = fake_opener(Control::Ok { data: None }, Arc::new(AtomicU64::new(0)));
        let info = registry.start("h1", 5173, Some(0), opener).await.unwrap();
        assert_eq!(opened.load(Ordering::SeqCst), 1, "probe");
        assert!(info.local_port > 0);
        assert_eq!(info.url, format!("http://127.0.0.1:{}/", info.local_port));
        for round in 0..3u8 {
            let mut socket = TcpStream::connect(("127.0.0.1", info.local_port))
                .await
                .unwrap();
            socket.write_all(&[round; 1000]).await.unwrap();
            socket.shutdown().await.unwrap();
            let mut back = Vec::new();
            socket.read_to_end(&mut back).await.unwrap();
            assert_eq!(back, vec![round; 1000]);
        }
        assert_eq!(
            opened.load(Ordering::SeqCst),
            4,
            "probe + one stream per connection"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        let listed = registry.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].total, 3);
        assert_eq!(listed[0].active, 0);
        assert_eq!(listed[0].bytes_up, 3000);
        assert!(listed[0].last_error.is_none());
        // Same host+port again: the running forward is reused.
        let (opener2, opened2) =
            fake_opener(Control::Ok { data: None }, Arc::new(AtomicU64::new(0)));
        let again = registry.start("h1", 5173, None, opener2).await.unwrap();
        assert_eq!(again.local_port, info.local_port);
        assert_eq!(opened2.load(Ordering::SeqCst), 0);
        // Stop closes the listener.
        assert!(registry.stop(&info.id));
        assert!(!registry.stop(&info.id));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(TcpStream::connect(("127.0.0.1", info.local_port))
            .await
            .is_err());
        assert!(registry.list().is_empty());
    }

    #[tokio::test]
    async fn start_surfaces_host_refusals_without_binding() {
        let registry = ForwardRegistry::default();
        let refusal = AppErrorDto::new(
            AppErrorCode::E_FORWARD_DISABLED,
            "Port forwarding is off on this PC",
            false,
        )
        .with_hint("unipi-host forward enable --any");
        let (opener, _) = fake_opener(Control::Err { error: refusal }, Arc::new(AtomicU64::new(0)));
        let error = registry
            .start("h1", 8080, Some(0), opener)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_FORWARD_DISABLED);
        assert!(error.hint.unwrap().contains("forward enable"));
        assert!(registry.list().is_empty());
    }

    #[tokio::test]
    async fn transport_errors_retry_once_then_record_last_error() {
        let registry = ForwardRegistry::default();
        // Probe: first attempt fails with transport, retry succeeds.
        let failures = Arc::new(AtomicU64::new(1));
        let (opener, _) = fake_opener(Control::Ok { data: None }, failures.clone());
        let info = registry.start("h1", 3000, Some(0), opener).await.unwrap();
        // Now the PC is gone: both attempts fail → socket closed, error kept.
        failures.store(2, Ordering::SeqCst);
        let mut socket = TcpStream::connect(("127.0.0.1", info.local_port))
            .await
            .unwrap();
        let mut back = Vec::new();
        socket.read_to_end(&mut back).await.unwrap();
        assert!(back.is_empty());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let listed = registry.get(&info.id).unwrap();
        let last = listed.last_error.expect("last error recorded");
        assert_eq!(last.error.code, AppErrorCode::E_TRANSPORT);
        registry.stop(&info.id);
    }

    #[tokio::test]
    async fn closed_port_on_host_is_reported_per_connection() {
        let registry = ForwardRegistry::default();
        let (ok, _) = fake_opener(Control::Ok { data: None }, Arc::new(AtomicU64::new(0)));
        let info = registry.start("h1", 4000, Some(0), ok).await.unwrap();
        // Swap in a host that now says nothing listens there.
        registry.stop(&info.id);
        let closed = AppErrorDto::new(
            AppErrorCode::E_NOT_FOUND,
            "Nothing is listening on port 4000 on this PC",
            false,
        );
        let (opener, _) = fake_opener(Control::Err { error: closed }, Arc::new(AtomicU64::new(0)));
        let listener = bind_local(Some(0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stats = Arc::new(Stats::default());
        let task = spawn_accept(listener, 4000, opener, stats.clone());
        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut back = Vec::new();
        socket.read_to_end(&mut back).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let last = stats.last_error.lock().unwrap().clone().unwrap();
        assert_eq!(last.error.code, AppErrorCode::E_NOT_FOUND);
        task.abort();
    }

    #[tokio::test]
    async fn bind_local_prefers_the_same_port_and_falls_back() {
        let taken = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let busy = taken.local_addr().unwrap().port();
        let fallback = bind_local(Some(busy)).await.unwrap();
        assert_ne!(fallback.local_addr().unwrap().port(), busy);
        drop(taken);
        let same = bind_local(Some(busy)).await.unwrap();
        assert_eq!(same.local_addr().unwrap().port(), busy);
        // Privileged ports are never tried.
        let low = bind_local(Some(80)).await.unwrap();
        assert_ne!(low.local_addr().unwrap().port(), 80);
    }
}

/// Live check against a separately started test host (never the user's):
/// pairs, starts a real `ForwardRegistry` forward over iroh and keeps it
/// open for `UNIPI_FWD_HOLD_SECS` so a browser can load it (HTTP, WS, HMR).
/// `UNIPI_FWD_E2E_DIR=/tmp/fwdhost UNIPI_FWD_PORT=8797 cargo test -p
///  unipi-mobile forward::live -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_forward_against_test_host() {
        let dir = std::env::var("UNIPI_FWD_E2E_DIR").expect("UNIPI_FWD_E2E_DIR");
        assert!(!dir.contains(".unipi/app-host"), "never the real host dir");
        let port: u16 = std::env::var("UNIPI_FWD_PORT")
            .unwrap_or("8797".into())
            .parse()
            .unwrap();
        let hold: u64 = std::env::var("UNIPI_FWD_HOLD_SECS")
            .unwrap_or("0".into())
            .parse()
            .unwrap();
        let cache: serde_json::Value =
            serde_json::from_slice(&std::fs::read(format!("{dir}/endpoint.json")).unwrap())
                .unwrap();
        let addr: iroh::EndpointAddr = serde_json::from_value(cache["addr"].clone()).unwrap();
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
            .bind()
            .await
            .unwrap();
        let conn = crate::net::dial(&endpoint, addr).await.unwrap();
        // Pair through a fresh code written into the test host's pairing.json.
        let code = "ABCDEFGH23";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            format!("{dir}/pairing.json"),
            serde_json::to_vec(&serde_json::json!({"code":code,"created_at":now,"expires_at":now+600,"attempts":0,"used":false})).unwrap(),
        )
        .unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(
            &hello_frame(Request::Pair {
                code: code.to_string().try_into().unwrap(),
                name: Some("live test".to_string().try_into().unwrap()),
            })
            .unwrap(),
        )
        .await
        .unwrap();
        send.finish().unwrap();
        assert!(matches!(
            read_reply(&mut recv).await.unwrap(),
            Control::Ok { .. }
        ));
        let opener: Opener = Arc::new(move |_| {
            let conn = conn.clone();
            Box::pin(async move {
                let (send, recv) = crate::net::open_op_stream(&conn).await?;
                Ok((
                    Box::new(send) as Box<dyn AsyncWrite + Send + Unpin>,
                    Box::new(recv) as Box<dyn AsyncRead + Send + Unpin>,
                ))
            }) as BoxFut
        });
        let registry = ForwardRegistry::default();
        let info = registry.start("live", port, None, opener).await.unwrap();
        println!("FORWARD_URL={}", info.url);
        let mut socket = TcpStream::connect(("127.0.0.1", info.local_port))
            .await
            .unwrap();
        socket
            .write_all(b"GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut page = Vec::new();
        socket.read_to_end(&mut page).await.unwrap();
        let page = String::from_utf8_lossy(&page);
        println!("{}", page.lines().next().unwrap_or(""));
        assert!(page.contains(" 200 "));
        for _ in 0..hold {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if std::path::Path::new("/tmp/fwdwww/stop").exists() {
                break;
            }
        }
        println!(
            "stats: {}",
            serde_json::to_string(&registry.list()).unwrap()
        );
    }
}
