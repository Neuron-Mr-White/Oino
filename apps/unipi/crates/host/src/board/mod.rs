//! Kanboard relay (DESIGN.md §3.9 "Data path"): `board_http` and
//! `board_sse` streams are forwarded to the local `unipi-kanboard` daemon
//! over loopback HTTP. The phone never sees the daemon's port.
//!
//! Discovery: `<kanboard home>/daemon.json` → probe `GET /api/health`. With
//! no live daemon, the host starts `unipi-kanboard serve` detached (loopback
//! bind, OS-picked port, default idle shutdown) and waits for a healthy
//! `daemon.json`. The daemon holds a single-instance lock, so racing starts
//! are safe. A request that cannot connect (the daemon idled out between
//! calls) rediscovers / restarts once and is resent.
//!
//! The daemon's HTTP API already acts as the user, so writes need nothing
//! extra. No `Origin` header is sent (the daemon refuses cross-site writes);
//! a token from `daemon.json` (remote bind / `--require-auth`) goes out as
//! `Authorization: Bearer`. Plain HTTP/1.1 over a fresh loopback connection
//! per request (hyper; no TLS, proxy or pooling involved).

pub mod discovery;
pub mod sse;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::FrameKind, AppErrorCode, AppErrorDto, BoardHttpData, BoardMethod, BoardPath,
    BoardProject, BodyLen, ContentType, Control, HttpStatus,
};

use crate::protocol::{read_frame, write_control};
use discovery::{BinaryLookup, DaemonInfo};
use sse::SseParser;

/// Shown when no `unipi-kanboard` binary can be found on the PC.
pub const NOT_INSTALLED_HINT: &str =
    "Install unipi on this PC: pi install npm:@pi-unipi/unipi@alpha";

/// Output frames are split at this size (well under `MAX_FRAME`).
const CHUNK: usize = 256 * 1024;
/// How long one request body frame may take to arrive.
const BODY_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// Ceiling for one forwarded (non-SSE) daemon request / body chunk.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the daemon lives and how to start it. `from_env` for production;
/// tests build one explicitly.
#[derive(Debug, Clone)]
pub struct BoardConfig {
    /// The kanboard home (`UNIPI_KANBOARD_HOME`, else `~/.unipi/kanboard`).
    pub home: Option<PathBuf>,
    pub lookup: BinaryLookup,
    /// How long to wait for a spawned daemon to become healthy.
    pub start_timeout: Duration,
    /// Extra `serve` arguments (tests: `--idle-secs`).
    pub serve_args: Vec<String>,
    /// Extra environment for the spawned daemon.
    pub serve_env: Vec<(String, String)>,
}

impl BoardConfig {
    pub fn from_env() -> Self {
        BoardConfig {
            home: discovery::kanboard_home(),
            lookup: BinaryLookup::from_env(),
            start_timeout: Duration::from_secs(10),
            serve_args: Vec::new(),
            serve_env: Vec::new(),
        }
    }
}

/// A healthy daemon endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Daemon {
    /// `host:port` to dial.
    pub authority: String,
    pub token: Option<String>,
    pub version: Option<String>,
}

/// A daemon response whose body is read chunk by chunk. Dropping it closes
/// the connection (which unsubscribes an SSE stream on the daemon).
#[derive(Debug)]
pub struct DaemonResponse {
    pub status: u16,
    pub content_type: String,
    body: hyper::body::Incoming,
    connection: tokio::task::JoinHandle<()>,
}

impl Drop for DaemonResponse {
    fn drop(&mut self) {
        self.connection.abort();
    }
}

impl DaemonResponse {
    /// The next body chunk; `Ok(None)` at the end of the body. Cancel-safe.
    pub async fn chunk(&mut self) -> Result<Option<bytes::Bytes>, String> {
        use http_body_util::BodyExt;
        loop {
            match self.body.frame().await {
                None => return Ok(None),
                Some(Err(error)) => return Err(error.to_string()),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        if !data.is_empty() {
                            return Ok(Some(data));
                        }
                    }
                }
            }
        }
    }

    /// The whole body, bounded by `limit` bytes.
    pub async fn bytes(&mut self, limit: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        while let Some(chunk) = self.chunk().await? {
            if out.len() + chunk.len() > limit {
                return Err("response too large".into());
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }
}

/// Why a daemon request failed.
#[derive(Debug)]
enum SendError {
    /// Nothing reached the daemon (no listener): safe to restart and resend.
    Connect(String),
    Other(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Connect(message) | SendError::Other(message) => f.write_str(message),
        }
    }
}

#[derive(Debug)]
struct Inner {
    config: BoardConfig,
    start_lock: tokio::sync::Mutex<()>,
    cached: Mutex<Option<Daemon>>,
}

/// The relay, shared by every stream of one `unipi-host` process.
#[derive(Debug, Clone)]
pub struct Board {
    inner: Arc<Inner>,
}

impl Default for Board {
    fn default() -> Self {
        Board::new(BoardConfig::from_env())
    }
}

fn unavailable(message: impl Into<String>, retryable: bool) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_BOARD_UNAVAILABLE, message, retryable)
}

fn rejected(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_BOARD_REJECTED, message, false)
}

/// `E_BOARD_UNAVAILABLE` with `detail.reason = "not_installed"` and the
/// install hint; the app maps it to `{state:'not_installed', hint}`.
pub fn not_installed() -> AppErrorDto {
    unavailable("unipi-kanboard is not installed on this PC", false)
        .with_hint(NOT_INSTALLED_HINT)
        .with_detail(serde_json::json!({"reason": "not_installed"}))
}

fn internal(error: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, error.to_string(), false)
}

/// One HTTP/1.1 request on a fresh loopback connection.
async fn http_request(
    daemon: &Daemon,
    method: BoardMethod,
    path: &str,
    content_type: Option<&str>,
    body: bytes::Bytes,
    accept_sse: bool,
) -> Result<DaemonResponse, SendError> {
    use hyper::header;
    let stream = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(daemon.authority.as_str()),
    )
    .await
    .map_err(|_| SendError::Connect("connect timed out".into()))?
    .map_err(|error| SendError::Connect(format!("connect {}: {error}", daemon.authority)))?;
    let _ = stream.set_nodelay(true);
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|error| SendError::Other(error.to_string()))?;
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let method = match method {
        BoardMethod::Get => hyper::Method::GET,
        BoardMethod::Post => hyper::Method::POST,
        BoardMethod::Put => hyper::Method::PUT,
    };
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, daemon.authority.as_str());
    if let Some(token) = &daemon.token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if accept_sse {
        builder = builder.header(header::ACCEPT, "text/event-stream");
    }
    let request = match builder.body(http_body_util::Full::new(body)) {
        Ok(request) => request,
        Err(error) => {
            connection.abort();
            return Err(SendError::Other(format!("bad request: {error}")));
        }
    };
    let response = match sender.send_request(request).await {
        Ok(response) => response,
        Err(error) => {
            connection.abort();
            return Err(SendError::Other(error.to_string()));
        }
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    Ok(DaemonResponse {
        status,
        content_type,
        body: response.into_body(),
        connection,
    })
}

impl Board {
    pub fn new(config: BoardConfig) -> Self {
        Board {
            inner: Arc::new(Inner {
                config,
                start_lock: tokio::sync::Mutex::new(()),
                cached: Mutex::new(None),
            }),
        }
    }

    pub fn config(&self) -> &BoardConfig {
        &self.inner.config
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Option<Daemon>> {
        self.inner
            .cached
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Reads `daemon.json` and checks `GET /api/health`.
    pub async fn probe(&self) -> Option<Daemon> {
        let info = DaemonInfo::read(self.inner.config.home.as_ref()?)?;
        let daemon = Daemon {
            authority: info.authority(),
            token: info.bearer().map(str::to_owned),
            version: info.version.clone(),
        };
        let health = tokio::time::timeout(Duration::from_secs(2), async {
            let mut response = http_request(
                &daemon,
                BoardMethod::Get,
                "/api/health",
                None,
                bytes::Bytes::new(),
                false,
            )
            .await
            .ok()?;
            if !(200..300).contains(&response.status) {
                return None;
            }
            let body = response.bytes(64 * 1024).await.ok()?;
            Some(serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default())
        })
        .await
        .ok()??;
        let version = health["version"]
            .as_str()
            .map(str::to_owned)
            .or(daemon.version.clone());
        Some(Daemon { version, ..daemon })
    }

    /// A healthy daemon, starting one when none is running.
    pub async fn ensure(&self) -> Result<Daemon, AppErrorDto> {
        if let Some(daemon) = self.cache().clone() {
            return Ok(daemon);
        }
        let daemon = self.discover_or_start().await?;
        *self.cache() = Some(daemon.clone());
        Ok(daemon)
    }

    fn forget(&self) {
        *self.cache() = None;
    }

    async fn discover_or_start(&self) -> Result<Daemon, AppErrorDto> {
        if let Some(daemon) = self.probe().await {
            return Ok(daemon);
        }
        let _guard = self.inner.start_lock.lock().await;
        if let Some(daemon) = self.probe().await {
            return Ok(daemon);
        }
        let config = &self.inner.config;
        let home = config
            .home
            .clone()
            .ok_or_else(|| unavailable("Cannot locate the kanboard home (HOME unset)", false))?;
        let bin = config.lookup.find().ok_or_else(not_installed)?;
        tracing::info!(bin=%bin.display(), home=%home.display(), "starting kanboard daemon");
        let mut command = tokio::process::Command::new(&bin);
        command
            .arg("serve")
            .args(&config.serve_args)
            .env("UNIPI_KANBOARD_HOME", &home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(false);
        for (key, value) in &config.serve_env {
            command.env(key, value);
        }
        // Detached: its own process group, so it outlives this host and
        // ignores the host's terminal signals; it exits on its idle timer.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| {
            unavailable(
                format!(
                    "Could not start unipi-kanboard ({}): {error}",
                    bin.display()
                ),
                true,
            )
        })?;
        let deadline = tokio::time::Instant::now() + config.start_timeout;
        loop {
            if let Some(daemon) = self.probe().await {
                tracing::info!(authority = daemon.authority, "kanboard daemon ready");
                return Ok(daemon);
            }
            // A start that lost the lock race exits 0 and leaves the winner's
            // daemon.json; a crash exits non-zero.
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    return Err(unavailable(
                        format!("unipi-kanboard serve exited ({status})"),
                        true,
                    ));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(unavailable(
                    format!(
                        "The board daemon did not start within {}s",
                        config.start_timeout.as_secs()
                    ),
                    true,
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Sends one request to the daemon. A connect failure (the daemon idled
    /// out or restarted on another port) rediscovers/restarts once and
    /// resends; nothing reached the daemon in that case, so POSTs are safe.
    pub async fn send(
        &self,
        method: BoardMethod,
        path: &str,
        content_type: Option<&str>,
        body: bytes::Bytes,
        stream: bool,
    ) -> Result<DaemonResponse, AppErrorDto> {
        let mut last_error = String::new();
        for attempt in 0..2 {
            let daemon = self.ensure().await?;
            let request = http_request(&daemon, method, path, content_type, body.clone(), stream);
            let result = if stream {
                request.await
            } else {
                tokio::time::timeout(REQUEST_TIMEOUT, request)
                    .await
                    .unwrap_or_else(|_| Err(SendError::Other("request timed out".into())))
            };
            match result {
                Ok(response) => return Ok(response),
                Err(SendError::Connect(error)) if attempt == 0 => {
                    tracing::info!(%error, "board daemon unreachable; rediscovering");
                    self.forget();
                    last_error = error;
                }
                Err(error) => {
                    self.forget();
                    return Err(unavailable(
                        format!("Board daemon request failed: {error}"),
                        true,
                    ));
                }
            }
        }
        Err(unavailable(
            format!("Board daemon unreachable: {last_error}"),
            true,
        ))
    }
}

/// Reads exactly `len` request-body bytes from kind-2 frames.
async fn read_body<R: AsyncRead + Unpin>(read: &mut R, len: u64) -> Result<Vec<u8>, AppErrorDto> {
    let len = usize::try_from(len).map_err(internal)?;
    let mut body = Vec::with_capacity(len);
    while body.len() < len {
        let (kind, bytes) = tokio::time::timeout(BODY_FRAME_TIMEOUT, read_frame(read))
            .await
            .map_err(|_| {
                AppErrorDto::new(
                    AppErrorCode::E_TIMEOUT,
                    "Request body stalled for 30 seconds",
                    true,
                )
            })?
            .map_err(|e| {
                AppErrorDto::new(
                    AppErrorCode::E_SCHEMA,
                    format!("Request body ended early: {e}"),
                    false,
                )
            })?;
        if kind != FrameKind::Input as u8 {
            return Err(AppErrorDto::new(
                AppErrorCode::E_SCHEMA,
                "Request body must be kind-2 frames",
                false,
            ));
        }
        if body.len() + bytes.len() > len {
            return Err(AppErrorDto::new(
                AppErrorCode::E_SCHEMA,
                "Request body is longer than body_len",
                false,
            ));
        }
        body.extend_from_slice(&bytes);
    }
    Ok(body)
}

async fn write_output<W: AsyncWrite + Unpin>(
    write: &mut W,
    bytes: &[u8],
) -> Result<(), AppErrorDto> {
    for piece in bytes.chunks(CHUNK) {
        write
            .write_all(&unipi_proto::frame::encode(FrameKind::Output, piece))
            .await
            .map_err(internal)?;
    }
    Ok(())
}

/// Handles a `board_http` stream after authorization.
pub async fn serve_http<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    board: &Board,
    read: &mut R,
    write: &mut W,
    method: BoardMethod,
    path: &BoardPath,
    content_type: Option<&ContentType>,
    body_len: BodyLen,
) -> Result<(), AppErrorDto> {
    // Defence in depth: the type already validated it on deserialize.
    unipi_proto::check_board_path(path.as_str()).map_err(rejected)?;
    let body = read_body(read, body_len.value()).await?;
    let mut response = board
        .send(
            method,
            path.as_str(),
            content_type.map(ContentType::as_str),
            bytes::Bytes::from(body),
            false,
        )
        .await?;
    let status = HttpStatus::try_from(response.status).map_err(internal)?;
    let data = serde_json::to_value(BoardHttpData {
        status,
        content_type: response.content_type.clone(),
    })
    .map_err(internal)?;
    write_control(write, &Control::Ok { data: Some(data) })
        .await
        .map_err(internal)?;
    loop {
        let chunk = tokio::time::timeout(REQUEST_TIMEOUT, response.chunk())
            .await
            .unwrap_or_else(|_| Err("timed out".into()));
        match chunk {
            Ok(Some(bytes)) => write_output(write, &bytes).await?,
            Ok(None) => break,
            Err(error) => {
                board.forget();
                return Err(unavailable(
                    format!("Board response interrupted: {error}"),
                    true,
                ));
            }
        }
    }
    write_control(write, &Control::Exit { code: Some(0) })
        .await
        .map_err(internal)
}

/// Handles a `board_sse` stream after authorization: forwards the
/// daemon's `/events?project=<slug>` messages until either side ends.
pub async fn serve_sse<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    board: &Board,
    read: &mut R,
    write: &mut W,
    project: &BoardProject,
) -> Result<(), AppErrorDto> {
    // The slug is `[A-Za-z0-9._-]` (validated), so it needs no escaping.
    let path = format!("/events?project={}", project.as_str());
    let mut response = board
        .send(BoardMethod::Get, &path, None, bytes::Bytes::new(), true)
        .await?;
    if !(200..300).contains(&response.status) {
        let status = response.status;
        let body = response.bytes(64 * 1024).await.unwrap_or_default();
        let text: String = String::from_utf8_lossy(&body).chars().take(200).collect();
        return Err(rejected(format!(
            "The board refused the event stream (HTTP {status}): {text}"
        )));
    }
    write_control(write, &Control::Ok { data: None })
        .await
        .map_err(internal)?;
    let mut parser = SseParser::new();
    let mut scratch = [0u8; 512];
    loop {
        tokio::select! {
            chunk = response.chunk() => match chunk {
                Ok(Some(bytes)) => {
                    for message in parser.push(&bytes) {
                        let control = Control::BoardEvent { event: message.event, data: message.data };
                        if write_control(write, &control).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                Ok(None) => {
                    tracing::info!(project = project.as_str(), "board event stream ended by daemon");
                    board.forget();
                    let _ = write_control(write, &Control::Exit { code: None }).await;
                    return Ok(());
                }
                Err(error) => {
                    board.forget();
                    return Err(unavailable(format!("Board event stream lost: {error}"), true));
                }
            },
            // The client closes (or resets) its send half to unsubscribe.
            // `read` into a buffer is cancel-safe, unlike a framed read.
            read_result = read.read(&mut scratch) => match read_result {
                Ok(0) | Err(_) => return Ok(()),
                Ok(_) => {}
            },
        }
    }
}
