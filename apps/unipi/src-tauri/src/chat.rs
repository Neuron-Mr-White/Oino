//! Session streams and the hub event channel over `net.rs` / `local_host.rs`
//! (host.md §7.3–7.5, app spec §3.2–3.4; WP-H4).
//!
//! ```text
//! sessions_list(hostId)                      → [SessionCard…]
//! session_open|new|fork(hostId, …, onEvent)  → stream id
//!   on_event: {kind:"snapshot", snapshot}     the first frame (§7.3)
//!             {kind:"events", items}          one 16 ms flush tick's batch (§7.4)
//!             {kind:"closed"|"error", …}      exactly one, at the end
//! session_cmd(streamId, cmd)                 → the command's `data` (correlated by id)
//! session_stream_close(streamId)             local hang-up (the session keeps running)
//! events_open(hostId, onEvent)               → stream id; {kind:"hub_event", event}
//! events_close(streamId)                     local hang-up
//! session_archive|restore|delete|rename|close   one-shot session ops
//! trash_empty(hostId)                          delete every archived session
//! ```
//!
//! `session_cmd` generates the correlation id itself and waits for the stream's
//! `ok{id,data}` / `err{id,error}` (30 s; 2 s for `_oino/render`, the same
//! budgets the host enforces — app spec §3.2), so the UI never has to match
//! replies by hand. Event payloads are oino's `ClientEvent` JSON and are not
//! re-interpreted here: the session store narrows them by `type` (ui-protocol
//! D-U14).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, ProtoError, Request, SessionEventDto, SessionName,
    SessionSnapshot, SessionTarget, PROTO_VERSION,
};

use crate::net::NetState;

/// The host may take a moment to open (wake) a session: 30 s, like every other
/// op on a fresh connection.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// A session command's reply budget (the host answers within 30 s and
/// `_oino/render` within 2 s; these are the backstops, app spec §3.2).
const CMD_TIMEOUT: Duration = Duration::from_secs(35);
const RENDER_TIMEOUT: Duration = Duration::from_secs(3);
/// Largest command frame the UI may send (a prompt with a few photos).
const MAX_SEND: usize = MAX_FRAME - 1024;

/// One frame of a session stream (or `events_open`) → a UI message.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionMsg {
    /// The stream's first frame: a full snapshot (`reset:true`) or a resume
    /// patch (`reset:false`, `replay`).
    Snapshot { snapshot: SessionSnapshot },
    /// One flush tick's events, in order (delta events already coalesced).
    Events { items: Vec<SessionEventDto> },
    /// A hub event on an `events_open` stream (`session_changed`,
    /// `session_removed`, `host_notice`, `ui`), verbatim.
    HubEvent { event: serde_json::Value },
    Closed {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<AppErrorCode>,
    },
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn schema(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
}

fn hello_frame(request: Request) -> Result<Vec<u8>, AppErrorDto> {
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
        req: request,
    };
    let bytes = serde_json::to_vec(&hello)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(encode(FrameKind::Control, &bytes))
}

fn control_frame(control: &Control) -> Result<Vec<u8>, AppErrorDto> {
    let bytes = serde_json::to_vec(control)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    if bytes.len() > MAX_FRAME {
        return Err(schema("That message is too large to send"));
    }
    Ok(encode(FrameKind::Control, &bytes))
}

async fn read_frame<R: AsyncRead + Unpin>(
    recv: &mut R,
    timeout: Option<Duration>,
) -> Result<Option<(u8, Vec<u8>)>, AppErrorDto> {
    let read = async {
        let kind = match recv.read_u8().await {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(transport(error)),
        };
        let len = recv.read_u32().await.map_err(transport)? as usize;
        if len > MAX_FRAME {
            return Err(schema("oversize frame from host"));
        }
        let mut payload = vec![0u8; len];
        recv.read_exact(&mut payload).await.map_err(transport)?;
        Ok(Some((kind, payload)))
    };
    match timeout {
        Some(limit) => tokio::time::timeout(limit, read)
            .await
            .map_err(|_| ProtoError::Timeout("The PC stopped answering".into()).to_app_error())?,
        None => read.await,
    }
}

/// Sends `hello` and returns the host's first control frame (its reply).
async fn hello_reply<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    req: Request,
    limit: Duration,
) -> Result<Control, AppErrorDto> {
    send.write_all(&hello_frame(req)?)
        .await
        .map_err(transport)?;
    let (kind, payload) = read_frame(recv, Some(limit))
        .await?
        .ok_or_else(|| transport("The PC closed the stream without a reply"))?;
    if kind != 0 {
        return Err(schema("expected a control reply"));
    }
    serde_json::from_slice(&payload).map_err(|e| schema(e.to_string()))
}

/// Sends `hello` and returns the host's `ok` data (or its error).
pub(crate) async fn request<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    req: Request,
) -> Result<Option<serde_json::Value>, AppErrorDto> {
    match hello_reply(send, recv, req, REPLY_TIMEOUT).await? {
        Control::Ok { data } => Ok(data),
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected reply")),
    }
}

/// A session stream's first frame: the snapshot, or the host's error.
async fn open_reply<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    req: Request,
) -> Result<SessionSnapshot, AppErrorDto> {
    match hello_reply(send, recv, req, REPLY_TIMEOUT).await? {
        Control::Ok { data } => {
            serde_json::from_value(data.ok_or_else(|| schema("missing snapshot"))?)
                .map_err(|e| schema(e.to_string()))
        }
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected reply")),
    }
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
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

async fn open_stream(
    state: &NetState,
    dir: &std::path::Path,
    ticket: &str,
    id: &str,
    fresh: bool,
) -> Result<(crate::op_stream::OpSend, crate::op_stream::OpRecv), AppErrorDto> {
    if fresh {
        state.drop_connection(id);
    }
    let relay = state.current_relay().await;
    state.open_op(dir, relay.as_deref(), ticket).await
}

/// One request/reply op on a fresh stream.
pub(crate) async fn one_shot(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    req: Request,
    limit: Duration,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(app)?;
    let (ticket, id) = host_ticket(&dir, host_id)?;
    let (mut send, mut recv) = open_stream(state, &dir, &ticket, &id, false).await?;
    let reply = hello_reply(&mut send, &mut recv, req, limit).await;
    let _ = send.finish();
    match reply? {
        Control::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected reply")),
    }
}

/// The open session streams and hub event streams, keyed by stream id.
#[derive(Clone, Default)]
pub struct SessionStreams {
    streams: Arc<Mutex<HashMap<String, StreamHandle>>>,
    next: Arc<AtomicU64>,
}

struct StreamHandle {
    send: Arc<tokio::sync::Mutex<crate::op_stream::OpSend>>,
    reader: tokio::task::JoinHandle<()>,
    /// Command replies this stream still waits for, by correlation id.
    pending: Pending,
}

/// Replies a `session_cmd` is waiting for (`id` → the waiting caller).
type Pending =
    Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Option<serde_json::Value>, AppErrorDto>>>>>;

impl SessionStreams {
    fn new_id(&self) -> String {
        format!("s-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, StreamHandle>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn pending(
    streams: &SessionStreams,
    stream_id: &str,
) -> Option<(Pending, Arc<tokio::sync::Mutex<crate::op_stream::OpSend>>)> {
    streams
        .lock()
        .get(stream_id)
        .map(|handle| (Arc::clone(&handle.pending), Arc::clone(&handle.send)))
}

/// Ends a local stream (the session itself keeps running on the host, §7.3).
fn close_stream(streams: &SessionStreams, stream_id: &str) {
    if let Some(handle) = streams.lock().remove(stream_id) {
        for (_, waiting) in handle
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
        {
            let _ = waiting.send(Err(AppErrorDto::new(
                AppErrorCode::E_TRANSPORT,
                "The session stream closed",
                true,
            )));
        }
        handle.reader.abort();
        if let Ok(mut send) = handle.send.try_lock() {
            let _ = send.finish();
        }
    }
}

/// Opens one session stream (`session_open` / `session_new` / `session_fork`), sends the
/// snapshot as the channel's first message and starts the reader.
async fn open_session(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, SessionStreams>,
    host_id: String,
    req: Request,
    on_event: Channel<SessionMsg>,
) -> Result<String, AppErrorDto> {
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let (send, mut recv, snapshot) = crate::retry::retry_once(|fresh| {
        let state = &state;
        let dir = &dir;
        let ticket = &ticket;
        let id = &id;
        let req = req.clone();
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            let snapshot = open_reply(&mut send, &mut recv, req).await?;
            Ok((send, recv, snapshot))
        }
    })
    .await?;
    let stream_id = streams.new_id();
    let registry = streams.inner().clone();
    let reader_id = stream_id.clone();
    let waiters: Pending = Arc::new(Mutex::new(HashMap::new()));
    let reader_waiters = Arc::clone(&waiters);
    let reader = tokio::spawn(async move {
        // The snapshot goes out before any event of the stream (§7.3 ordering): the reader
        // is the only reader, so nothing can overtake it.
        let _ = on_event.send(SessionMsg::Snapshot { snapshot });
        pump_session(
            &mut recv,
            &mut |message| on_event.send(message).is_ok(),
            &reader_waiters,
        )
        .await;
        registry.lock().remove(&reader_id);
    });
    streams.lock().insert(
        stream_id.clone(),
        StreamHandle {
            send: Arc::new(tokio::sync::Mutex::new(send)),
            reader,
            pending: waiters,
        },
    );
    Ok(stream_id)
}

/// Pumps one open session stream into `emit`: `events` batches and command replies, until
/// the stream ends (exactly one final message unless `emit` fails).
async fn pump_session<R: AsyncRead + Unpin>(
    recv: &mut R,
    emit: &mut impl FnMut(SessionMsg) -> bool,
    waiters: &Pending,
) {
    loop {
        match read_frame(recv, None).await {
            Ok(Some((kind, payload))) => {
                if kind != 0 {
                    continue;
                }
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&payload) else {
                    emit(SessionMsg::Error {
                        message: "The PC sent something unreadable".into(),
                        code: Some(AppErrorCode::E_TRANSPORT),
                    });
                    return;
                };
                match value.get("type").and_then(|t| t.as_str()) {
                    Some("events") => {
                        let items: Vec<SessionEventDto> =
                            match serde_json::from_value(value["items"].clone()) {
                                Ok(items) => items,
                                Err(error) => {
                                    emit(SessionMsg::Error {
                                        message: error.to_string(),
                                        code: Some(AppErrorCode::E_TRANSPORT),
                                    });
                                    return;
                                }
                            };
                        if !emit(SessionMsg::Events { items }) {
                            return;
                        }
                    }
                    Some("ok") | Some("err") => {
                        if !resolve(emit, waiters, value) {
                            return;
                        }
                    }
                    // Host-only `crashed` / `taken_over` events ride the `events` batches
                    // like core's own (same numbered envelope, §7.4); nothing else is
                    // expected on a session stream.
                    Some(_) | None => {}
                }
            }
            Ok(None) => {
                emit(SessionMsg::Closed { message: None });
                return;
            }
            Err(error) => {
                emit(SessionMsg::Error {
                    message: error.message.as_str().to_owned(),
                    code: Some(error.code),
                });
                return;
            }
        }
    }
}

/// Hands one `ok`/`err` frame to the `session_cmd` waiting for its id. Returns whether the
/// stream should keep going.
fn resolve(
    emit: &mut impl FnMut(SessionMsg) -> bool,
    waiters: &Pending,
    value: serde_json::Value,
) -> bool {
    let id = value.get("id").and_then(|id| {
        id.as_u64()
            .or_else(|| id.as_str().and_then(|text| text.parse().ok()))
    });
    let waiter = id.and_then(|id| {
        waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
    });
    match (value.get("type").and_then(|t| t.as_str()), waiter) {
        (Some("ok"), Some(waiter)) => {
            let _ = waiter.send(Ok(value.get("data").cloned()));
            true
        }
        (Some("err"), Some(waiter)) => {
            let error = serde_json::from_value::<AppErrorDto>(value["error"].clone())
                .unwrap_or_else(|_| schema("The session failed"));
            let _ = waiter.send(Err(error));
            true
        }
        // An error nobody is waiting for (the stream was taken over, the runtime died):
        // surface it and end the stream.
        (Some("err"), None) => {
            let error = serde_json::from_value::<AppErrorDto>(value["error"].clone())
                .unwrap_or_else(|_| schema("The session failed"));
            emit(SessionMsg::Error {
                message: error.message.as_str().to_owned(),
                code: Some(error.code),
            });
            false
        }
        _ => true,
    }
}

/// Pumps an `events_open` stream into `emit`: hub events verbatim until it ends.
async fn pump_events<R: AsyncRead + Unpin>(
    recv: &mut R,
    emit: &mut impl FnMut(SessionMsg) -> bool,
) {
    loop {
        match read_frame(recv, None).await {
            Ok(Some((kind, payload))) => {
                if kind != 0 {
                    continue;
                }
                match serde_json::from_slice::<serde_json::Value>(&payload) {
                    Ok(event) if event.get("kind").is_some() => {
                        if !emit(SessionMsg::HubEvent { event }) {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        emit(SessionMsg::Error {
                            message: error.to_string(),
                            code: Some(AppErrorCode::E_SCHEMA),
                        });
                        return;
                    }
                }
            }
            Ok(None) => {
                emit(SessionMsg::Closed { message: None });
                return;
            }
            Err(error) => {
                emit(SessionMsg::Error {
                    message: error.message.as_str().to_owned(),
                    code: Some(error.code),
                });
                return;
            }
        }
    }
}

/// `sessions_list` (host.md §7.2/§4.2): every session of this host, newest first.
#[tauri::command]
pub async fn sessions_list(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    archived: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionsList {
            archived: archived.unwrap_or(false),
        },
        REPLY_TIMEOUT,
    )
    .await
}

/// `session_open` (host.md §7.3): open or resume a session as a live stream.
#[tauri::command]
pub async fn session_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, SessionStreams>,
    host_id: String,
    session: SessionTarget,
    since_seq: Option<u64>,
    on_event: Channel<SessionMsg>,
) -> Result<String, AppErrorDto> {
    open_session(
        app,
        state,
        streams,
        host_id,
        Request::SessionOpen { session, since_seq },
        on_event,
    )
    .await
}

/// `session_new` (host.md §4.2): a new session, opened as a live stream.
#[tauri::command]
pub async fn session_new(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, SessionStreams>,
    host_id: String,
    cwd: Option<unipi_proto::FsPath>,
    name: Option<SessionName>,
    on_event: Channel<SessionMsg>,
) -> Result<String, AppErrorDto> {
    open_session(
        app,
        state,
        streams,
        host_id,
        Request::SessionNew { cwd, name },
        on_event,
    )
    .await
}

/// `session_fork` (host.md §4.2): fork a session and keep the fork as the stream.
#[tauri::command]
pub async fn session_fork(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, SessionStreams>,
    host_id: String,
    session: SessionTarget,
    entry_id: Option<String>,
    name: Option<SessionName>,
    on_event: Channel<SessionMsg>,
) -> Result<String, AppErrorDto> {
    open_session(
        app,
        state,
        streams,
        host_id,
        Request::SessionFork {
            session,
            entry_id,
            name,
        },
        on_event,
    )
    .await
}

/// One `session_cmd` (host.md §7.3) with an id this side generated: resolves with the
/// command's `data` once the stream carries `ok{id,data}` (or rejects with its `err`).
#[tauri::command]
pub async fn session_cmd(
    streams: State<'_, SessionStreams>,
    stream_id: String,
    cmd: serde_json::Value,
    id: Option<u64>,
) -> Result<serde_json::Value, AppErrorDto> {
    let Some(kind) = cmd.get("type").and_then(|t| t.as_str()).map(str::to_owned) else {
        return Err(schema("cmd must be an RPC command object with a type"));
    };
    let Some((waiters, send)) = pending(&streams, &stream_id) else {
        return Err(AppErrorDto::new(
            AppErrorCode::E_TRANSPORT,
            "The session is not connected",
            true,
        ));
    };
    let command_id = id.unwrap_or_else(|| {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    });
    let frame = control_frame(&Control::SessionCmd {
        id: Some(serde_json::json!(command_id)),
        cmd,
    })?;
    if frame.len() > MAX_SEND {
        return Err(schema(
            "That message is too large to send (photos too big?)",
        ));
    }
    // app spec §3.2: 30 s for commands, 2 s for `_oino/render` (the host enforces the same;
    // these are the local backstops).
    let limit = if kind == "_oino/render" {
        RENDER_TIMEOUT
    } else {
        CMD_TIMEOUT
    };
    let (answer, reply) = oneshot::channel();
    waiters
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(command_id, answer);
    {
        let mut send = send.lock().await;
        if let Err(error) = send.write_all(&frame).await {
            waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&command_id);
            return Err(transport(error));
        }
    }
    let result = tokio::time::timeout(limit, reply).await;
    if result.is_err() {
        waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&command_id);
    }
    let reply = result
        .map_err(|_| {
            ProtoError::Timeout("The session did not answer in time".into()).to_app_error()
        })?
        .map_err(|_| {
            AppErrorDto::new(AppErrorCode::E_TRANSPORT, "The session stream closed", true)
        })?;
    reply.map(|data| data.unwrap_or(serde_json::Value::Null))
}

/// `session_stream_close` (host.md §7.3: closing the stream only unsubscribes).
#[tauri::command]
pub async fn session_stream_close(
    streams: State<'_, SessionStreams>,
    stream_id: String,
) -> Result<(), AppErrorDto> {
    close_stream(&streams, &stream_id);
    Ok(())
}

/// `events_open` (host.md §7.4): the one per-connection hub event stream.
#[tauri::command]
pub async fn events_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, SessionStreams>,
    host_id: String,
    on_event: Channel<SessionMsg>,
) -> Result<String, AppErrorDto> {
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let (send, mut recv) = crate::retry::retry_once(|fresh| {
        let state = &state;
        let dir = &dir;
        let ticket = &ticket;
        let id = &id;
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            match hello_reply(&mut send, &mut recv, Request::EventsOpen, REPLY_TIMEOUT).await? {
                Control::Ok { .. } => Ok((send, recv)),
                Control::Err { error } => Err(error),
                _ => Err(schema("unexpected reply")),
            }
        }
    })
    .await?;
    let stream_id = streams.new_id();
    let registry = streams.inner().clone();
    let reader_id = stream_id.clone();
    let reader = tokio::spawn(async move {
        pump_events(&mut recv, &mut |message| on_event.send(message).is_ok()).await;
        registry.lock().remove(&reader_id);
    });
    streams.lock().insert(
        stream_id.clone(),
        StreamHandle {
            send: Arc::new(tokio::sync::Mutex::new(send)),
            reader,
            pending: Arc::new(Mutex::new(HashMap::new())),
        },
    );
    Ok(stream_id)
}

/// `events_close`: hang up the local hub event stream.
#[tauri::command]
pub async fn events_close(
    streams: State<'_, SessionStreams>,
    stream_id: String,
) -> Result<(), AppErrorDto> {
    close_stream(&streams, &stream_id);
    Ok(())
}

/// `session_archive` (host.md §4.2): move the session's file to the host's `trash/`.
#[tauri::command]
pub async fn session_archive(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionArchive { session },
        REPLY_TIMEOUT,
    )
    .await
}

/// `session_restore` (app spec §3.14, WP-A5, Z-13): move an archived session back into
/// its project's folder. → the restored card.
#[tauri::command]
pub async fn session_restore(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionRestore { session },
        REPLY_TIMEOUT,
    )
    .await
}

/// `session_delete` (app spec §3.14, WP-A5, Z-13/Z-20): delete one **archived** session
/// file. The UI archives first, then deletes.
#[tauri::command]
pub async fn session_delete(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionDelete { session },
        REPLY_TIMEOUT,
    )
    .await
}

/// `trash_empty` (app spec §3.14, WP-A5, Z-13): delete every archived session.
/// → `{"deleted": N}`.
#[tauri::command]
pub async fn trash_empty(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::TrashEmpty, REPLY_TIMEOUT).await
}

/// `session_rename` (host.md §7.2): set the session's display name.
#[tauri::command]
pub async fn session_rename(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
    name: SessionName,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionRename { session, name },
        REPLY_TIMEOUT,
    )
    .await
}

/// `session_close` (host.md §4.2): dispose the runtime, keep the file.
#[tauri::command]
pub async fn session_close(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionClose { session },
        REPLY_TIMEOUT,
    )
    .await
}

/// `session_meta_set` (app spec §3.14, WP-A4): pin / unread / title-lock flags,
/// persisted by the host in `session-meta.json`. → the updated `SessionCard`.
#[tauri::command]
pub async fn session_meta_set(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
    pinned: Option<bool>,
    unread: Option<bool>,
    title_locked: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    let patch = unipi_proto::session::SessionMetaPatch {
        pinned,
        unread,
        title_locked,
    };
    if patch == unipi_proto::session::SessionMetaPatch::default() {
        return Err(schema("session_meta_set needs at least one flag"));
    }
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionMetaSet { session, patch },
        REPLY_TIMEOUT,
    )
    .await
}

/// `events_focus` (app spec §3.14, WP-A4): which session the connection behind
/// `stream_id` focuses (`null` = none); the host clears that session's `unread`
/// and counts the focus for its Z-86 rule. Fire-and-forget by design.
#[tauri::command]
pub async fn events_focus(
    streams: State<'_, SessionStreams>,
    stream_id: String,
    session: Option<String>,
) -> Result<(), AppErrorDto> {
    let Some((_, send)) = pending(&streams, &stream_id) else {
        // The stream is gone (host flap): nothing to report on; the next
        // `events_open` connection starts focused on nothing again.
        return Ok(());
    };
    let frame = control_frame(&Control::Focus { session })?;
    let mut send = send.lock().await;
    send.write_all(&frame)
        .await
        .map_err(|error| transport(error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(control: &Control) -> Vec<u8> {
        encode(FrameKind::Control, &serde_json::to_vec(control).unwrap())
    }

    #[tokio::test]
    async fn request_returns_ok_data_or_host_error() {
        let (mut phone, mut host) = tokio::io::duplex(1 << 16);
        let reply = frame(&Control::Ok {
            data: Some(serde_json::json!([{"session_id": "s1"}])),
        });
        tokio::spawn(async move {
            let mut hello = vec![0u8; 5];
            host.read_exact(&mut hello).await.unwrap();
            let len = u32::from_be_bytes(hello[1..5].try_into().unwrap()) as usize;
            let mut body = vec![0u8; len];
            host.read_exact(&mut body).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["req"]["op"], "sessions_list");
            host.write_all(&reply).await.unwrap();
        });
        let (mut r, mut w) = tokio::io::split(&mut phone);
        let data = request(&mut w, &mut r, Request::SessionsList { archived: false })
            .await
            .unwrap();
        assert_eq!(data.unwrap()[0]["session_id"], "s1");

        let (mut phone, mut host) = tokio::io::duplex(1 << 16);
        let err = frame(&Control::Err {
            error: AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "gone", false),
        });
        tokio::spawn(async move {
            let mut scratch = [0u8; 256];
            let _ = host.read(&mut scratch).await;
            host.write_all(&err).await.unwrap();
        });
        let (mut r, mut w) = tokio::io::split(&mut phone);
        let error = request(
            &mut w,
            &mut r,
            Request::SessionOpen {
                session: "s1".to_string().try_into().unwrap(),
                since_seq: Some(4),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_NOT_FOUND);
    }

    #[tokio::test]
    async fn open_reply_carries_the_snapshot() {
        let snapshot = serde_json::json!({
            "session_id": "s1", "reset": false, "seq": 7,
            "replay": [{"session": "s1", "seq": 7, "event": {"type": "agent_end"}}]
        });
        let mut bytes = frame(&Control::Ok {
            data: Some(snapshot),
        });
        bytes.extend(frame(&Control::Events { items: Vec::new() }));
        let (mut w, mut r) = (tokio::io::sink(), bytes.as_slice());
        let opened = open_reply(
            &mut w,
            &mut r,
            Request::SessionOpen {
                session: "s1".to_string().try_into().unwrap(),
                since_seq: Some(6),
            },
        )
        .await
        .unwrap();
        assert!(!opened.reset);
        assert_eq!(opened.seq, 7);
        assert_eq!(opened.replay.unwrap().len(), 1);
    }

    #[test]
    fn message_shapes() {
        let msg = SessionMsg::Events {
            items: vec![SessionEventDto::new(
                "s1",
                3,
                serde_json::json!({"type": "agent_start"}),
            )],
        };
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            serde_json::json!({"kind": "events", "items": [
                {"session": "s1", "seq": 3, "event": {"type": "agent_start"}}
            ]})
        );
        assert_eq!(
            serde_json::to_value(SessionMsg::Closed { message: None }).unwrap(),
            serde_json::json!({"kind": "closed"})
        );
        assert!(matches!(
            SessionMsg::Error {
                message: "lost".into(),
                code: Some(AppErrorCode::E_TRANSPORT),
            },
            SessionMsg::Error { .. }
        ));
        assert_eq!(
            serde_json::to_value(SessionMsg::HubEvent {
                event: serde_json::json!({"kind": "host_notice", "level": "info", "text": "hi"})
            })
            .unwrap(),
            serde_json::json!({"kind": "hub_event",
                "event": {"kind": "host_notice", "level": "info", "text": "hi"}})
        );
    }

    #[tokio::test]
    async fn a_reply_resolves_the_waiting_command() {
        let (waiter, waiting) = oneshot::channel();
        let waiters: Pending = Arc::new(Mutex::new(HashMap::new()));
        waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(4, waiter);
        let mut messages: Vec<SessionMsg> = Vec::new();
        let keep = resolve(
            &mut |message| {
                messages.push(message);
                true
            },
            &waiters,
            serde_json::json!({"type": "ok", "id": 4, "data": {"ok": true}}),
        );
        assert!(keep);
        assert!(messages.is_empty(), "a correlated reply is not a message");
        let data = waiting.await.unwrap().unwrap().unwrap();
        assert_eq!(data["ok"], true);

        // An error nobody waits for ends the stream with the host's message.
        let mut messages: Vec<SessionMsg> = Vec::new();
        let keep = resolve(
            &mut |message| {
                messages.push(message);
                true
            },
            &waiters,
            serde_json::json!({
                "type": "err",
                "error": {"code": "E_TRANSPORT", "message": "taken over", "retryable": true}
            }),
        );
        assert!(!keep);
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            SessionMsg::Error { message, code } => {
                assert_eq!(message, "taken over");
                assert_eq!(*code, Some(AppErrorCode::E_TRANSPORT));
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }
}
