//! Board relay commands (DESIGN.md §3.9 "Data path"): the UI's
//! `boardTransport()` (apps/mobile/src/lib/board/transport.ts) calls these;
//! they ride the existing iroh connection to the paired PC, where
//! `unipi-host` forwards to the local kanboard daemon. The phone never talks
//! to the daemon's port.
//!
//! - `board_availability { hostId }` → `{state:'ok', version?} | {state:'offline'}
//!   | {state:'not_installed', hint} | {state:'error', message}`
//! - `board_request { hostId, method, path, contentType, body }` →
//!   `{status, contentType, body: number[]}` (daemon status/body verbatim)
//! - `board_events { hostId, project, onEvent }` → stream id; channel gets
//!   `{kind:'open'}`, `{kind:'event', event?, data}`, `{kind:'closed'|'error', message?}`
//! - `board_events_close { streamId }`

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, BoardHttpData, BoardMethod, BoardPath, BoardProject, BodyLen,
    ContentType, Control, PathSegment, ProtoError, Request, PROTO_VERSION,
};

use crate::net::NetState;

/// Largest response body accepted from the host (attachments are ≤ 25 MB).
const MAX_RESPONSE: usize = 64 * 1024 * 1024;
/// How long the host may stay silent while answering one request (it may
/// have to start the daemon first, ~10 s worst case).
const FRAME_TIMEOUT: Duration = Duration::from_secs(60);
/// Fallback install hint (the host normally supplies its own).
const INSTALL_HINT: &str = "Install unipi on this PC: pi install npm:@pi-unipi/unipi";

/// `board_availability` result (tagged on `state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BoardAvailability {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    Offline,
    NotInstalled {
        hint: String,
    },
    Error {
        message: String,
    },
}

/// `board_request` result: the daemon's reply verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Messages on the `board_events` channel (tagged on `kind`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoardStreamMsg {
    Open,
    Event {
        #[serde(skip_serializing_if = "Option::is_none")]
        event: Option<String>,
        data: String,
    },
    Closed {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

impl BoardStreamMsg {
    fn is_final(&self) -> bool {
        matches!(
            self,
            BoardStreamMsg::Closed { .. } | BoardStreamMsg::Error { .. }
        )
    }
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn schema(field: &str, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
        .with_path(vec![PathSegment::Key(field.into())])
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

/// Reads one frame; `Ok(None)` on a clean end of stream.
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
            return Err(schema("frame", "oversize frame from host"));
        }
        let mut payload = vec![0u8; len];
        recv.read_exact(&mut payload).await.map_err(transport)?;
        Ok(Some((kind, payload)))
    };
    match timeout {
        Some(limit) => tokio::time::timeout(limit, read).await.map_err(|_| {
            ProtoError::Timeout("The PC stopped answering the board request".into()).to_app_error()
        })?,
        None => read.await,
    }
}

fn parse_control(payload: &[u8]) -> Result<Control, AppErrorDto> {
    serde_json::from_slice(payload).map_err(|e| schema("control", e.to_string()))
}

/// A failed exchange, and whether the full request had left the phone
/// (after which a POST must not be resent).
#[derive(Debug)]
pub(crate) struct Failure {
    pub error: AppErrorDto,
    pub sent: bool,
}

/// One `board_http` exchange on an open stream: hello + body frames, then
/// `ok{status, content_type}`, kind-1 body frames and `exit` (DESIGN.md
/// §5.4; see `unipi_proto::board`).
pub(crate) async fn http_exchange<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    request: Request,
    body: &[u8],
) -> Result<BoardResponse, Failure> {
    let not_sent = |error| Failure { error, sent: false };
    let sent = |error| Failure { error, sent: true };
    let hello = hello_frame(request).map_err(not_sent)?;
    let mut write_result = send.write_all(&hello).await;
    if write_result.is_ok() {
        for piece in body.chunks(MAX_FRAME) {
            write_result = send.write_all(&encode(FrameKind::Input, piece)).await;
            if write_result.is_err() {
                break;
            }
        }
    }
    if let Err(error) = write_result {
        // The host may have refused early (bad path, unauthorized) and
        // stopped reading; prefer its error envelope when there is one.
        if let Ok(Ok(Some((0, payload)))) =
            tokio::time::timeout(Duration::from_secs(2), read_frame(recv, None)).await
        {
            if let Ok(Control::Err { error }) = parse_control(&payload) {
                return Err(not_sent(error));
            }
        }
        return Err(not_sent(transport(error)));
    }
    let _ = send.shutdown().await;

    let first = read_frame(recv, Some(FRAME_TIMEOUT))
        .await
        .map_err(sent)?
        .ok_or_else(|| sent(transport("The PC closed the board request without a reply")))?;
    if first.0 != 0 {
        return Err(sent(schema("frame", "expected a control reply")));
    }
    let data: BoardHttpData = match parse_control(&first.1).map_err(sent)? {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
            .map_err(|e| sent(schema("data", e.to_string())))?,
        Control::Err { error } => return Err(sent(error)),
        _ => return Err(sent(schema("type", "unexpected board reply"))),
    };
    let mut out = Vec::new();
    loop {
        let Some((kind, payload)) = read_frame(recv, Some(FRAME_TIMEOUT)).await.map_err(sent)?
        else {
            return Err(sent(transport("The board response was cut off")));
        };
        match kind {
            1 => {
                if out.len() + payload.len() > MAX_RESPONSE {
                    return Err(sent(AppErrorDto::new(
                        AppErrorCode::E_BOARD_REJECTED,
                        "The board response is too large",
                        false,
                    )));
                }
                out.extend_from_slice(&payload);
            }
            0 => match parse_control(&payload).map_err(sent)? {
                Control::Exit { .. } => break,
                Control::Err { error } => return Err(sent(error)),
                _ => return Err(sent(schema("type", "unexpected board frame"))),
            },
            _ => return Err(sent(schema("frame", "unexpected frame kind"))),
        }
    }
    Ok(BoardResponse {
        status: data.status.value(),
        content_type: data.content_type,
        body: out,
    })
}

/// Maps one frame of an open `board_sse` stream to a channel message;
/// `None` for frames to ignore.
pub(crate) fn sse_message(kind: u8, payload: &[u8]) -> Option<BoardStreamMsg> {
    if kind != 0 {
        return None;
    }
    match parse_control(payload) {
        Ok(Control::BoardEvent { event, data }) => Some(BoardStreamMsg::Event { event, data }),
        Ok(Control::Exit { .. }) => Some(BoardStreamMsg::Closed {
            message: Some("The board daemon closed the event stream".into()),
        }),
        Ok(Control::Err { error }) => Some(BoardStreamMsg::Error {
            message: Some(error.message.as_str().to_owned()),
        }),
        Ok(_) => None,
        Err(error) => Some(BoardStreamMsg::Error {
            message: Some(error.message.as_str().to_owned()),
        }),
    }
}

/// Sends the `board_sse` hello and waits for the host's `ok`.
pub(crate) async fn sse_open<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    project: BoardProject,
) -> Result<(), AppErrorDto> {
    send.write_all(&hello_frame(Request::BoardSse { project })?)
        .await
        .map_err(transport)?;
    let (kind, payload) = read_frame(recv, Some(FRAME_TIMEOUT))
        .await?
        .ok_or_else(|| transport("The PC closed the board stream without a reply"))?;
    if kind != 0 {
        return Err(schema("frame", "expected a control reply"));
    }
    match parse_control(&payload)? {
        Control::Ok { .. } => Ok(()),
        Control::Err { error } => Err(error),
        _ => Err(schema("type", "unexpected board reply")),
    }
}

/// Pumps an open `board_sse` stream into `emit` until it ends. Exactly one
/// final (`closed`/`error`) message is emitted unless `emit` fails.
pub(crate) async fn sse_pump<R: AsyncRead + Unpin>(
    recv: &mut R,
    mut emit: impl FnMut(BoardStreamMsg) -> bool,
) {
    loop {
        match read_frame(recv, None).await {
            Ok(Some((kind, payload))) => {
                if let Some(message) = sse_message(kind, &payload) {
                    let last = message.is_final();
                    if !emit(message) || last {
                        return;
                    }
                }
            }
            Ok(None) => {
                emit(BoardStreamMsg::Closed {
                    message: Some("The PC closed the board stream".into()),
                });
                return;
            }
            Err(error) => {
                emit(BoardStreamMsg::Error {
                    message: Some(error.message.as_str().to_owned()),
                });
                return;
            }
        }
    }
}

/// Maps a `GET /api/health` outcome to the availability state.
pub(crate) fn availability_from(result: Result<BoardResponse, AppErrorDto>) -> BoardAvailability {
    match result {
        Ok(response) if (200..300).contains(&response.status) => {
            let health: serde_json::Value =
                serde_json::from_slice(&response.body).unwrap_or_default();
            BoardAvailability::Ok {
                version: health["version"].as_str().map(str::to_owned),
            }
        }
        Ok(response) => BoardAvailability::Error {
            message: format!("The board daemon answered HTTP {}", response.status),
        },
        Err(error) => match error.code {
            AppErrorCode::E_TRANSPORT | AppErrorCode::E_TIMEOUT => BoardAvailability::Offline,
            AppErrorCode::E_BOARD_UNAVAILABLE
                if error
                    .detail
                    .as_ref()
                    .and_then(|detail| detail["reason"].as_str())
                    == Some("not_installed") =>
            {
                BoardAvailability::NotInstalled {
                    hint: error.hint.unwrap_or_else(|| INSTALL_HINT.into()),
                }
            }
            // A pre-board host fails to parse the new op.
            AppErrorCode::E_SCHEMA if error.message.as_str().contains("board_http") => {
                BoardAvailability::Error {
                    message: "Update unipi-host on this PC to use the board".into(),
                }
            }
            _ => BoardAvailability::Error {
                message: error.message.as_str().to_owned(),
            },
        },
    }
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

/// The paired host's ticket and endpoint id.
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
    // Reuse the bound endpoint's relay so the board never rebinds it.
    let relay = state.current_relay().await;
    state.open_op(dir, relay.as_deref(), ticket).await
}

/// Runs one `board_http` request against a paired host. A transport
/// failure redials once; a POST/PUT is only resent when it never fully
/// left the phone (no duplicate writes).
async fn board_http(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    request: Request,
    body: &[u8],
) -> Result<BoardResponse, AppErrorDto> {
    let idempotent = matches!(
        request,
        Request::BoardHttp {
            method: BoardMethod::Get,
            ..
        }
    );
    let dir = directory(app)?;
    let (ticket, id) = host_ticket(&dir, host_id)?;
    let mut fresh = false;
    loop {
        let attempt = async {
            let (mut send, mut recv) = open_stream(state, &dir, &ticket, &id, fresh)
                .await
                .map_err(|error| Failure { error, sent: false })?;
            http_exchange(&mut send, &mut recv, request.clone(), body).await
        };
        match attempt.await {
            Ok(response) => return Ok(response),
            Err(Failure { error, sent })
                if !fresh && error.code == AppErrorCode::E_TRANSPORT && (idempotent || !sent) =>
            {
                log::info!(
                    "board request transport error, redialing: {}",
                    error.message
                );
                fresh = true;
            }
            Err(failure) => return Err(failure.error),
        }
    }
}

fn board_request_from(
    method: &str,
    path: String,
    content_type: Option<String>,
    body_len: usize,
) -> Result<Request, AppErrorDto> {
    let method: BoardMethod = method.parse().map_err(|e: String| schema("method", e))?;
    let path = BoardPath::try_from(path).map_err(|e| schema("path", e))?;
    let content_type = content_type
        .map(ContentType::try_from)
        .transpose()
        .map_err(|e| schema("contentType", e))?;
    let body_len = BodyLen::try_from(body_len as u64).map_err(|_| {
        AppErrorDto::new(
            AppErrorCode::E_BOARD_REJECTED,
            "That file is too large for the board (max 25 MB)",
            false,
        )
    })?;
    Ok(Request::BoardHttp {
        method,
        path,
        content_type,
        body_len,
    })
}

#[tauri::command]
pub async fn board_availability(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<BoardAvailability, AppErrorDto> {
    let request = board_request_from("GET", "/api/health".into(), None, 0)?;
    Ok(availability_from(
        board_http(&app, &state, &host_id, request, &[]).await,
    ))
}

#[tauri::command]
pub async fn board_request(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    method: String,
    path: String,
    content_type: Option<String>,
    body: Option<Vec<u8>>,
) -> Result<BoardResponse, AppErrorDto> {
    let body = body.unwrap_or_default();
    let request = board_request_from(&method, path, content_type, body.len())?;
    board_http(&app, &state, &host_id, request, &body).await
}

struct StreamHandle {
    send: crate::op_stream::OpSend,
    reader: tokio::task::JoinHandle<()>,
}

/// Live `board_events` subscriptions, keyed by stream id.
#[derive(Clone, Default)]
pub struct BoardStreams {
    streams: Arc<Mutex<HashMap<String, StreamHandle>>>,
    next: Arc<AtomicU64>,
}

impl BoardStreams {
    fn new_id(&self) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        format!(
            "board-{:x}-{}",
            nanos.wrapping_mul(0x9e37_79b9),
            self.next.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, StreamHandle>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[tauri::command]
pub async fn board_events(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, BoardStreams>,
    host_id: String,
    project: String,
    on_event: Channel<BoardStreamMsg>,
) -> Result<String, AppErrorDto> {
    let project = BoardProject::try_from(project).map_err(|e| schema("project", e))?;
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let (send, mut recv) = crate::retry::retry_once(|fresh| {
        let state = &state;
        let dir = &dir;
        let ticket = &ticket;
        let id = &id;
        let project = project.clone();
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            sse_open(&mut send, &mut recv, project).await?;
            Ok((send, recv))
        }
    })
    .await?;
    let stream_id = streams.new_id();
    let _ = on_event.send(BoardStreamMsg::Open);
    let registry = streams.inner().clone();
    let reader_id = stream_id.clone();
    // Hold the registry lock across spawn+insert so the reader's own
    // removal can never run before the insert.
    let mut map = streams.lock();
    let reader = tokio::spawn(async move {
        sse_pump(&mut recv, |message| on_event.send(message).is_ok()).await;
        registry.lock().remove(&reader_id);
    });
    map.insert(stream_id.clone(), StreamHandle { send, reader });
    drop(map);
    Ok(stream_id)
}

#[tauri::command]
pub async fn board_events_close(
    streams: State<'_, BoardStreams>,
    stream_id: String,
) -> Result<(), AppErrorDto> {
    // Idempotent; stop the reader first so nothing reaches the UI after an
    // unsubscribe, then finish the send half (the host unsubscribes).
    let handle = streams.lock().remove(&stream_id);
    if let Some(mut handle) = handle {
        handle.reader.abort();
        let _ = handle.send.finish();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_frame(data: serde_json::Value) -> Vec<u8> {
        encode(
            FrameKind::Control,
            &serde_json::to_vec(&Control::Ok { data: Some(data) }).unwrap(),
        )
    }

    fn control_frame(control: &Control) -> Vec<u8> {
        encode(FrameKind::Control, &serde_json::to_vec(control).unwrap())
    }

    /// A fake host: reads the request, checks it, writes `reply`.
    async fn exchange(
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
        reply: Vec<u8>,
    ) -> (Result<BoardResponse, Failure>, serde_json::Value, Vec<u8>) {
        let request = board_request_from(
            method,
            path.into(),
            content_type.map(str::to_owned),
            body.len(),
        )
        .unwrap();
        let (client, server) = tokio::io::duplex(1 << 16);
        let (mut host_read, mut host_write) = tokio::io::split(server);
        let host = tokio::spawn(async move {
            let (_, hello) = read_frame(&mut host_read, None).await.unwrap().unwrap();
            let hello: serde_json::Value = serde_json::from_slice(&hello).unwrap();
            let len = hello["req"]["body_len"].as_u64().unwrap_or(0) as usize;
            let mut received = Vec::new();
            while received.len() < len {
                let (kind, bytes) = read_frame(&mut host_read, None).await.unwrap().unwrap();
                assert_eq!(kind, 2);
                received.extend(bytes);
            }
            host_write.write_all(&reply).await.unwrap();
            host_write.shutdown().await.unwrap();
            (hello, received)
        });
        let (mut recv, mut send) = tokio::io::split(client);
        let result = http_exchange(&mut send, &mut recv, request, body).await;
        let (hello, received) = host.await.unwrap();
        (result, hello, received)
    }

    #[tokio::test]
    async fn http_exchange_round_trips_status_and_binary_body() {
        let upload: Vec<u8> = (0..(2 * MAX_FRAME + 7)).map(|i| (i % 253) as u8).collect();
        let mut reply =
            ok_frame(serde_json::json!({"status":409,"content_type":"application/json"}));
        reply.extend(encode(FrameKind::Output, br#"{"error":"needs a comment","#));
        reply.extend(encode(FrameKind::Output, br#""needsComment":true}"#));
        reply.extend(control_frame(&Control::Exit { code: Some(0) }));
        let (result, hello, received) = exchange(
            "POST",
            "/api/tasks/p/P-1/attachments?name=a.png",
            Some("application/octet-stream"),
            &upload,
            reply,
        )
        .await;
        let response = result.unwrap();
        assert_eq!(response.status, 409);
        assert_eq!(response.content_type, "application/json");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["needsComment"],
            true
        );
        assert_eq!(received, upload, "body crossed in kind-2 frames");
        assert_eq!(
            hello,
            serde_json::json!({
                "type": "hello",
                "proto": PROTO_VERSION,
                "req": {
                    "op": "board_http",
                    "method": "POST",
                    "path": "/api/tasks/p/P-1/attachments?name=a.png",
                    "content_type": "application/octet-stream",
                    "body_len": upload.len()
                }
            })
        );
        // The command result shape the TS contract expects.
        let json = serde_json::to_value(BoardResponse {
            status: 200,
            content_type: "image/png".into(),
            body: vec![1, 2, 255],
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"status":200,"contentType":"image/png","body":[1,2,255]})
        );
    }

    #[tokio::test]
    async fn http_exchange_surfaces_host_errors_and_truncation() {
        let err = AppErrorDto::new(AppErrorCode::E_BOARD_UNAVAILABLE, "down", true);
        let (result, _, _) = exchange(
            "GET",
            "/api/projects",
            None,
            b"",
            control_frame(&Control::Err { error: err }),
        )
        .await;
        let failure = result.unwrap_err();
        assert_eq!(failure.error.code, AppErrorCode::E_BOARD_UNAVAILABLE);
        assert!(failure.sent);

        // Body without the exit marker = cut off = transport error.
        let mut reply = ok_frame(serde_json::json!({"status":200,"content_type":"text/plain"}));
        reply.extend(encode(FrameKind::Output, b"partial"));
        let (result, _, _) = exchange("GET", "/api/projects", None, b"", reply).await;
        assert_eq!(result.unwrap_err().error.code, AppErrorCode::E_TRANSPORT);

        // A mid-body host error wins.
        let mut reply = ok_frame(serde_json::json!({"status":200,"content_type":"text/plain"}));
        reply.extend(encode(FrameKind::Output, b"partial"));
        reply.extend(control_frame(&Control::Err {
            error: AppErrorDto::new(AppErrorCode::E_BOARD_UNAVAILABLE, "interrupted", true),
        }));
        let (result, _, _) = exchange("GET", "/api/projects", None, b"", reply).await;
        assert_eq!(result.unwrap_err().error.message.as_str(), "interrupted");
    }

    #[test]
    fn request_validation() {
        assert!(board_request_from("GET", "/api/projects".into(), None, 0).is_ok());
        let error = board_request_from("DELETE", "/api/projects".into(), None, 0).unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_SCHEMA);
        let error = board_request_from("GET", "/events?project=x".into(), None, 0).unwrap_err();
        assert_eq!(error.path, Some(vec![PathSegment::Key("path".into())]));
        assert!(board_request_from("GET", "/api/../x".into(), None, 0).is_err());
        let error = board_request_from(
            "POST",
            "/api/x".into(),
            Some("application/octet-stream".into()),
            (unipi_proto::BOARD_MAX_BODY + 1) as usize,
        )
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BOARD_REJECTED);
        assert!(board_request_from(
            "POST",
            "/api/x".into(),
            Some("application/octet-stream".into()),
            25 * 1024 * 1024
        )
        .is_ok());
    }

    #[test]
    fn availability_mapping_and_shape() {
        let ok = availability_from(Ok(BoardResponse {
            status: 200,
            content_type: "application/json".into(),
            body: br#"{"ok":true,"version":"3.0.0-alpha.30","pid":1}"#.to_vec(),
        }));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({"state":"ok","version":"3.0.0-alpha.30"})
        );
        let offline = availability_from(Err(AppErrorDto::new(
            AppErrorCode::E_TIMEOUT,
            "connect timed out",
            true,
        )));
        assert_eq!(
            serde_json::to_value(&offline).unwrap(),
            serde_json::json!({"state":"offline"})
        );
        assert_eq!(
            availability_from(Err(transport("closed"))),
            BoardAvailability::Offline
        );
        let not_installed = availability_from(Err(AppErrorDto::new(
            AppErrorCode::E_BOARD_UNAVAILABLE,
            "unipi-kanboard is not installed on this PC",
            false,
        )
        .with_hint("Install unipi on this PC: pi install npm:@pi-unipi/unipi")
        .with_detail(serde_json::json!({"reason":"not_installed"}))));
        assert_eq!(
            serde_json::to_value(&not_installed).unwrap(),
            serde_json::json!({"state":"not_installed","hint":"Install unipi on this PC: pi install npm:@pi-unipi/unipi"})
        );
        let start_failed = availability_from(Err(AppErrorDto::new(
            AppErrorCode::E_BOARD_UNAVAILABLE,
            "The board daemon did not start within 10s",
            true,
        )));
        assert_eq!(
            serde_json::to_value(&start_failed).unwrap(),
            serde_json::json!({"state":"error","message":"The board daemon did not start within 10s"})
        );
        let old_host = availability_from(Err(AppErrorDto::new(
            AppErrorCode::E_SCHEMA,
            "unknown variant `board_http`, expected one of `list`, ...",
            false,
        )));
        assert!(
            matches!(old_host, BoardAvailability::Error { message } if message.contains("Update unipi-host"))
        );
        let http_error = availability_from(Ok(BoardResponse {
            status: 500,
            content_type: String::new(),
            body: Vec::new(),
        }));
        assert!(matches!(http_error, BoardAvailability::Error { .. }));
    }

    #[tokio::test]
    async fn sse_open_and_pump_map_frames_to_channel_messages() {
        let (client, server) = tokio::io::duplex(1 << 16);
        let (mut host_read, mut host_write) = tokio::io::split(server);
        let host = tokio::spawn(async move {
            let (_, hello) = read_frame(&mut host_read, None).await.unwrap().unwrap();
            let hello: serde_json::Value = serde_json::from_slice(&hello).unwrap();
            assert_eq!(
                hello["req"],
                serde_json::json!({"op":"board_sse","project":"p-1"})
            );
            let mut frames = control_frame(&Control::Ok { data: None });
            frames.extend(control_frame(&Control::BoardEvent {
                event: Some("revision".into()),
                data: "1".into(),
            }));
            frames.extend(control_frame(&Control::BoardEvent {
                event: None,
                data: "a\nb".into(),
            }));
            frames.extend(control_frame(&Control::Exit { code: None }));
            host_write.write_all(&frames).await.unwrap();
            host_write.shutdown().await.unwrap();
        });
        let (mut recv, mut send) = tokio::io::split(client);
        sse_open(
            &mut send,
            &mut recv,
            BoardProject::try_from("p-1".to_string()).unwrap(),
        )
        .await
        .unwrap();
        let mut messages = Vec::new();
        sse_pump(&mut recv, |message| {
            messages.push(message);
            true
        })
        .await;
        host.await.unwrap();
        let json: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap())
            .collect();
        assert_eq!(
            json,
            vec![
                serde_json::json!({"kind":"event","event":"revision","data":"1"}),
                serde_json::json!({"kind":"event","data":"a\nb"}),
                serde_json::json!({"kind":"closed","message":"The board daemon closed the event stream"}),
            ]
        );
        assert_eq!(
            serde_json::to_value(BoardStreamMsg::Open).unwrap(),
            serde_json::json!({"kind":"open"})
        );
    }

    #[tokio::test]
    async fn sse_pump_reports_errors_and_eof() {
        let err = control_frame(&Control::Err {
            error: AppErrorDto::new(
                AppErrorCode::E_BOARD_UNAVAILABLE,
                "Board event stream lost",
                true,
            ),
        });
        let mut reader: &[u8] = &err;
        let mut messages = Vec::new();
        sse_pump(&mut reader, |m| {
            messages.push(m);
            true
        })
        .await;
        assert_eq!(
            messages,
            vec![BoardStreamMsg::Error {
                message: Some("Board event stream lost".into())
            }]
        );
        let mut empty: &[u8] = &[];
        let mut messages = Vec::new();
        sse_pump(&mut empty, |m| {
            messages.push(m);
            true
        })
        .await;
        assert!(matches!(
            messages.as_slice(),
            [BoardStreamMsg::Closed { .. }]
        ));

        // A refused subscription surfaces the host's error.
        let mut reply: &[u8] = &err;
        let mut sink = Vec::new();
        let error = sse_open(
            &mut sink,
            &mut reply,
            BoardProject::try_from("p".to_string()).unwrap(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BOARD_UNAVAILABLE);
    }
}
