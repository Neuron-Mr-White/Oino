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
//! session_archive|restore|delete|rename|close|move   one-shot session ops
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
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, ProtoError, Request, SessionEventDto, SessionName,
    SessionSnapshot, SessionTarget, PROTO_VERSION,
};

use crate::frames::FrameRead;
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
    /// fix-frames: the stream skipped a frame it could not use (oversize, unreadable), so
    /// events may be missing: the UI reopens with a full snapshot. The stream stays open.
    Resync { reason: String },
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

/// Reads one frame (fix-frames: an oversize frame is drained and reported as
/// [`FrameRead::Skipped`] instead of ending the stream).
async fn read_frame<R: AsyncRead + Unpin>(
    recv: &mut R,
    timeout: Option<Duration>,
) -> Result<FrameRead, AppErrorDto> {
    let read = crate::frames::read_lenient(recv);
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
    let (kind, payload) = match read_frame(recv, Some(limit)).await? {
        FrameRead::Frame(kind, payload) => (kind, payload),
        FrameRead::Skipped { len, .. } => return Err(crate::frames::oversize_error(len)),
        FrameRead::End => return Err(transport("The PC closed the stream without a reply")),
    };
    if kind != 0 {
        return Err(schema("expected a control reply"));
    }
    serde_json::from_slice(&payload).map_err(|e| schema(e.to_string()))
}

/// Sends `hello` and returns the host's `ok` data (or its error).
/// Only the tests drive a bare request/response pair now (the commands
/// all stream or use `hello_reply` through `open_reply`), so it lives
/// with them.
#[cfg(test)]
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

fn is_oversize(error: &AppErrorDto) -> bool {
    error.message.as_str() == crate::frames::OVERSIZE
}

/// A `session_open` that resumed (`since_seq`), as a plain open (full snapshot).
fn without_resume(req: Request) -> Option<Request> {
    match req {
        Request::SessionOpen {
            session,
            since_seq: Some(_),
        } => Some(Request::SessionOpen {
            session,
            since_seq: None,
        }),
        _ => None,
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
            match open_reply(&mut send, &mut recv, req.clone()).await {
                Ok(snapshot) => Ok((send, recv, snapshot)),
                // fix-frames: a resume whose replay was too large for one frame (an older
                // host) — ask again for a full snapshot, which the host bounds.
                Err(error) if is_oversize(&error) => match without_resume(req) {
                    Some(req) => {
                        let _ = send.finish();
                        log::warn!("session resume reply too large; reopening with a snapshot");
                        let (mut send, mut recv) =
                            open_stream(state, dir, ticket, id, false).await?;
                        let snapshot = open_reply(&mut send, &mut recv, req).await?;
                        Ok((send, recv, snapshot))
                    }
                    None => Err(error),
                },
                Err(error) => Err(error),
            }
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
///
/// fix-frames: a frame this side cannot use — over the frame cap, not JSON, an `events`
/// batch that does not parse — is logged and skipped, and the UI is told to `resync` (a
/// fresh snapshot: whatever the frame carried is lost). The stream itself keeps going; it
/// used to end with "oversize frame from host" and leave the chat dead while the turn ran.
async fn pump_session<R: AsyncRead + Unpin>(
    recv: &mut R,
    emit: &mut impl FnMut(SessionMsg) -> bool,
    waiters: &Pending,
) {
    loop {
        match read_frame(recv, None).await {
            Ok(FrameRead::Frame(kind, payload)) => {
                if kind != 0 {
                    continue;
                }
                let value = match serde_json::from_slice::<serde_json::Value>(&payload) {
                    Ok(value) => value,
                    Err(error) => {
                        log::warn!(
                            "session stream: unreadable frame ({} bytes): {error}",
                            payload.len()
                        );
                        if !emit(SessionMsg::Resync {
                            reason: "unreadable frame".into(),
                        }) {
                            return;
                        }
                        continue;
                    }
                };
                match value.get("type").and_then(|t| t.as_str()) {
                    Some("events") => {
                        let items: Vec<SessionEventDto> =
                            match serde_json::from_value(value["items"].clone()) {
                                Ok(items) => items,
                                Err(error) => {
                                    log::warn!("session stream: bad events batch: {error}");
                                    if !emit(SessionMsg::Resync {
                                        reason: "bad events batch".into(),
                                    }) {
                                        return;
                                    }
                                    continue;
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
            Ok(FrameRead::Skipped { len, .. }) => {
                // Probably an events batch (a reply would leave its command to time out):
                // the events in it are gone, so the UI reopens with a full snapshot.
                log::warn!(
                    "session stream: skipped an oversize frame ({len} bytes); asking for a resync"
                );
                if !emit(SessionMsg::Resync {
                    reason: "oversize frame".into(),
                }) {
                    return;
                }
            }
            Ok(FrameRead::End) => {
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

/// Pumps an `events_open` stream into `emit`: hub events verbatim until it ends. A frame it
/// cannot use (oversize, not JSON) is logged and skipped; the stream keeps going.
async fn pump_events<R: AsyncRead + Unpin>(
    recv: &mut R,
    emit: &mut impl FnMut(SessionMsg) -> bool,
) {
    loop {
        match read_frame(recv, None).await {
            Ok(FrameRead::Frame(kind, payload)) => {
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
                        log::warn!("events stream: unreadable frame skipped: {error}");
                    }
                }
            }
            Ok(FrameRead::Skipped { len, .. }) => {
                log::warn!("events stream: skipped an oversize frame ({len} bytes)");
            }
            Ok(FrameRead::End) => {
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

/// `session_search` (owner fix 2026-11-03): one page of this host's sessions whose
/// name / folder / id match every word of `query`, newest first (the command center's
/// Sessions scope; never the whole list).
#[tauri::command]
pub async fn session_search(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    query: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
    archived: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionSearch {
            query: query.unwrap_or_default(),
            limit,
            cursor,
            archived: archived.unwrap_or(false),
        },
        REPLY_TIMEOUT,
    )
    .await
}

/// `memory_browse` (owner fix 2026-11-03): one page of the memory store on this host
/// (Settings › Memory › Browse memories), newest first; `project` narrows it to one
/// folder, `query` searches title / body / project.
#[tauri::command]
pub async fn memory_browse(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    project: Option<String>,
    query: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::MemoryBrowse {
            project,
            query: query.unwrap_or_default(),
            limit,
            cursor,
        },
        REPLY_TIMEOUT,
    )
    .await
}

/// `usage_stats` (owner fix 2026-11-04): token / cost statistics over the last `days`
/// local days of this host's sessions (Settings › Usage).
#[tauri::command]
pub async fn usage_stats(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    days: u32,
    tz_offset_min: Option<i32>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::UsageStats {
            days,
            tz_offset_min: tz_offset_min.unwrap_or(0),
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

/// List restricted Superagent sessions on this host.
#[tauri::command]
pub async fn superagent_sessions(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentSessions, REPLY_TIMEOUT).await
}

/// Authenticated Superagent dispatcher operation.
#[tauri::command]
pub async fn superagent_status(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentStatus, REPLY_TIMEOUT).await
}

/// Authenticated Superagent dispatcher operation.
#[tauri::command]
pub async fn superagent_configure(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    enabled: bool,
    concurrency_cap: usize,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentConfigure { enabled, concurrency_cap }, REPLY_TIMEOUT).await
}

/// Authenticated Superagent dispatcher operation.
#[tauri::command]
pub async fn superagent_start(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    project_id: String,
    task_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentStart { project_id, task_id: Some(task_id), task_ids: vec![], preset_id: None, overrides: None }, REPLY_TIMEOUT).await
}

/// Authenticated Superagent dispatcher operation.
#[tauri::command]
pub async fn superagent_stop(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    project_id: String,
    task_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentStop { project_id, task_id }, REPLY_TIMEOUT).await
}

/// Authenticated Superagent dispatcher operation.
#[tauri::command]
pub async fn superagent_relay(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    project_id: String,
    task_id: String,
    text: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentRelay { project_id, task_id: Some(task_id), text, task_ids: vec![], delivery: None, priority: false }, REPLY_TIMEOUT).await
}

/// Answer a pending approval on a live dispatcher worker.
#[tauri::command]
pub async fn superagent_approval_response(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    project_id: String,
    task_id: String,
    worker_session_id: String,
    request_id: String,
    decision: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::SuperagentApprovalResponse { project_id, task_id, worker_session_id, request_id, decision }, REPLY_TIMEOUT).await
}

/// Opens the restricted Superagent through the existing authenticated session stream.
#[tauri::command]
pub async fn superagent_session(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, SessionStreams>,
    host_id: String,
    on_event: Channel<SessionMsg>,
) -> Result<String, AppErrorDto> {
    open_session(app, state, streams, host_id, Request::SuperagentSession, on_event).await
}

/// `session_fork` (host.md §4.2): fork a session and keep the fork as the stream.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
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

/// How long one batch op may take (2,000 chats on a slow disk; the host announces
/// progress on `events_open` meanwhile).
const BATCH_TIMEOUT: Duration = Duration::from_secs(600);

/// `sessions_archive` (owner fix 2026-11-03, `[ux-sessions]`): archive many sessions
/// in one host op. → `{done, failed?}`.
#[tauri::command]
pub async fn sessions_archive(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    sessions: Vec<SessionTarget>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionsArchive { sessions },
        BATCH_TIMEOUT,
    )
    .await
}

/// `sessions_restore` (owner fix 2026-11-03, `[ux-sessions]`): restore many archived
/// sessions in one host op. → `{done, failed?}`.
#[tauri::command]
pub async fn sessions_restore(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    sessions: Vec<SessionTarget>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionsRestore { sessions },
        BATCH_TIMEOUT,
    )
    .await
}

/// `sessions_query` (owner fix 2026-11-03, `[ux-sessions]`): one page of the sidebar's
/// session search (title + first user message, live and/or archived, the list's filters).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sessions_query(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    query: Option<String>,
    scope: Option<unipi_proto::session_bulk::SessionsQueryScope>,
    filters: Option<unipi_proto::session_bulk::SessionsQueryFilters>,
    sort: Option<unipi_proto::session_bulk::SessionsQuerySort>,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionsQuery {
            query: query.unwrap_or_default(),
            scope: scope.unwrap_or_default(),
            filters: filters.unwrap_or_default(),
            sort: sort.unwrap_or_default(),
            limit,
            cursor,
        },
        REPLY_TIMEOUT,
    )
    .await
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

/// `session_move` (UniPi Oino "Move to a workspace…"): a "No workspace" chat moves to a
/// real folder from its next turn on — one way only. JS args: `{hostId, session, cwd,
/// copyFiles}`. → `{card, copied, skipped}`.
#[tauri::command]
pub async fn session_move(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
    cwd: unipi_proto::FsPath,
    copy_files: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::SessionMove {
            session,
            cwd,
            copy_files: copy_files.unwrap_or(false),
        },
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

/// `draft_get` (owner 2026-11-06): the session's composer draft kept by the host. → `{draft}`.
#[tauri::command]
pub async fn draft_get(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::DraftGet { session },
        REPLY_TIMEOUT,
    )
    .await
}

/// `draft_set` (owner 2026-11-06): store (or clear with `null`) the session's composer draft;
/// the card's `draft_at` follows. → the updated `SessionCard`.
#[tauri::command]
pub async fn draft_set(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    session: SessionTarget,
    draft: Option<serde_json::Value>,
) -> Result<serde_json::Value, AppErrorDto> {
    let draft = draft.filter(|d| !d.is_null());
    one_shot(
        &app,
        &state,
        &host_id,
        Request::DraftSet { session, draft },
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
    send.write_all(&frame).await.map_err(transport)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn frame(control: &Control) -> Vec<u8> {
        encode(FrameKind::Control, &serde_json::to_vec(control).unwrap())
    }

    #[test]
    fn superagent_requests_use_authenticated_hello_framing() {
        let requests = [
            (Request::SuperagentStatus, "superagent.status"),
            (Request::SuperagentSessions, "superagent.sessions"),
            (Request::SuperagentSession, "superagent.session"),
            (Request::SuperagentConfigure { enabled: false, concurrency_cap: 2 }, "superagent.configure"),
            (Request::SuperagentStart { project_id: "p".into(), task_id: Some("t".into()), task_ids: vec![], preset_id: None, overrides: None }, "superagent.start"),
            (Request::SuperagentStop { project_id: "p".into(), task_id: "t".into() }, "superagent.stop"),
            (Request::SuperagentRelay { project_id: "p".into(), task_id: Some("t".into()), text: "hello".into(), task_ids: vec![], delivery: None, priority: false }, "superagent.relay"),
            (Request::SuperagentApprovalResponse { project_id: "p".into(), task_id: "t".into(), worker_session_id: "w".into(), request_id: "r".into(), decision: "deny".into() }, "superagent.approval_response"),
        ];
        for (request, op) in requests {
            let bytes = hello_frame(request).unwrap();
            let hello: serde_json::Value = serde_json::from_slice(&bytes[5..]).unwrap();
            assert_eq!(hello["type"], "hello");
            assert_eq!(hello["req"]["op"], op);
        }
        let registration = include_str!("lib.rs");
        for name in ["status", "session", "configure", "start", "stop", "relay"] {
            assert!(registration.contains(&format!("chat::superagent_{name},")));
        }
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

    /// fix-frames P0: an oversize frame (a live event with a screenshot) used to end the
    /// stream with "oversize frame from host" and leave the chat dead. Now it is skipped,
    /// the UI is told to resync, and later events and replies still arrive.
    #[tokio::test]
    async fn an_oversize_or_unreadable_frame_resyncs_and_the_stream_keeps_going() {
        let mut bytes = Vec::new();
        bytes.extend(frame(&Control::Events {
            items: vec![SessionEventDto::new(
                "s1",
                1,
                serde_json::json!({"type": "agent_start"}),
            )],
        }));
        // An `events` frame over the cap: header + 2 MiB of payload.
        bytes.push(0);
        bytes.extend_from_slice(&((2 * MAX_FRAME) as u32).to_be_bytes());
        bytes.extend(std::iter::repeat_n(b' ', 2 * MAX_FRAME));
        // Not JSON.
        bytes.extend(encode(FrameKind::Control, b"{not json"));
        bytes.extend(frame(&Control::Events {
            items: vec![SessionEventDto::new(
                "s1",
                9,
                serde_json::json!({"type": "agent_end"}),
            )],
        }));
        let (waiter, waiting) = oneshot::channel();
        let waiters: Pending = Arc::new(Mutex::new(HashMap::new()));
        waiters.lock().unwrap().insert(5, waiter);
        bytes.extend(encode(
            FrameKind::Control,
            br#"{"type":"ok","id":5,"data":{"fine":true}}"#,
        ));
        let mut read = bytes.as_slice();
        let mut messages: Vec<SessionMsg> = Vec::new();
        pump_session(
            &mut read,
            &mut |message| {
                messages.push(message);
                true
            },
            &waiters,
        )
        .await;
        let kinds: Vec<&str> = messages
            .iter()
            .map(|m| match m {
                SessionMsg::Events { .. } => "events",
                SessionMsg::Resync { .. } => "resync",
                SessionMsg::Closed { .. } => "closed",
                SessionMsg::Error { .. } => "error",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["events", "resync", "resync", "events", "closed"]);
        assert_eq!(waiting.await.unwrap().unwrap().unwrap()["fine"], true);
        assert_eq!(
            serde_json::to_value(SessionMsg::Resync {
                reason: "oversize frame".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "resync", "reason": "oversize frame"})
        );
    }

    #[tokio::test]
    async fn an_oversize_open_reply_is_retryable_and_falls_back_to_a_snapshot() {
        let mut bytes = vec![0u8];
        bytes.extend_from_slice(&((MAX_FRAME + 1) as u32).to_be_bytes());
        bytes.extend(std::iter::repeat_n(b' ', MAX_FRAME + 1));
        let (mut w, mut r) = (tokio::io::sink(), bytes.as_slice());
        let req = Request::SessionOpen {
            session: "s1".to_string().try_into().unwrap(),
            since_seq: Some(6),
        };
        let error = open_reply(&mut w, &mut r, req.clone()).await.unwrap_err();
        assert!(error.retryable, "never a dead, non-retryable banner");
        assert!(is_oversize(&error));
        match without_resume(req) {
            Some(Request::SessionOpen { since_seq, .. }) => assert_eq!(since_seq, None),
            other => panic!("expected a plain open, got {other:?}"),
        }
        assert!(without_resume(Request::SessionsList { archived: false }).is_none());
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
