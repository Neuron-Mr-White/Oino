//! Chat (DESIGN §3.4, docs/m5/PROTOCOL.md): `chat_list` (one request) and
//! live `chat_open` streams. Bridge lines pass through untouched; the UI
//! parses them.
//!
//! - `chat_list(host_id)` → the host's `chat_list` reply (herdr tree with
//!   `chat` on pi panes).
//! - `chat_open(host_id, pid, on_event)` → a stream id; `on_event` gets
//!   `open`, then `line{line}` per bridge message, then exactly one
//!   `closed` / `error`.
//! - `chat_send(stream_id, line)` writes one line to the pi.
//! - `chat_close(stream_id)` hangs up (idempotent).

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
    AppErrorCode, AppErrorDto, Control, ProtoError, Request, PROTO_VERSION,
};

use crate::net::NetState;

/// The host may need a moment to read the herdr tree / reach the socket.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest line the UI may send (a prompt with a few photos).
const MAX_SEND: usize = MAX_FRAME - 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatStreamMsg {
    Open,
    Line {
        line: String,
    },
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

impl ChatStreamMsg {
    fn is_final(&self) -> bool {
        matches!(self, ChatStreamMsg::Closed { .. } | ChatStreamMsg::Error { .. })
    }
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
        Some(limit) => tokio::time::timeout(limit, read).await.map_err(|_| {
            ProtoError::Timeout("The PC stopped answering".into()).to_app_error()
        })?,
        None => read.await,
    }
}

fn parse_control(payload: &[u8]) -> Result<Control, AppErrorDto> {
    serde_json::from_slice(payload).map_err(|e| schema(e.to_string()))
}

/// Sends `hello` and returns the host's `ok` data (or its error).
pub(crate) async fn request<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    req: Request,
) -> Result<Option<serde_json::Value>, AppErrorDto> {
    request_within(send, recv, req, REPLY_TIMEOUT).await
}

/// [`request`] with its own reply budget (chat_new waits for pi to start).
pub(crate) async fn request_within<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    req: Request,
    limit: Duration,
) -> Result<Option<serde_json::Value>, AppErrorDto> {
    send.write_all(&hello_frame(req)?).await.map_err(transport)?;
    let (kind, payload) = read_frame(recv, Some(limit))
        .await?
        .ok_or_else(|| transport("The PC closed the chat stream without a reply"))?;
    if kind != 0 {
        return Err(schema("expected a control reply"));
    }
    match parse_control(&payload)? {
        Control::Ok { data } => Ok(data),
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected reply")),
    }
}

/// One frame of an open chat stream → a UI message (`None` = ignore).
pub(crate) fn stream_message(kind: u8, payload: &[u8]) -> Option<ChatStreamMsg> {
    if kind != 0 {
        return None;
    }
    match parse_control(payload) {
        Ok(Control::Chat { line }) => Some(ChatStreamMsg::Line { line }),
        Ok(Control::Exit { .. }) => Some(ChatStreamMsg::Closed {
            message: Some("The pi session ended".into()),
        }),
        Ok(Control::Err { error }) => Some(ChatStreamMsg::Error {
            message: error.message.as_str().to_owned(),
            code: Some(error.code),
        }),
        Ok(_) => None,
        Err(error) => Some(ChatStreamMsg::Error {
            message: error.message.as_str().to_owned(),
            code: Some(error.code),
        }),
    }
}

/// Pumps an open chat stream into `emit` until it ends; exactly one final
/// message unless `emit` fails.
pub(crate) async fn pump<R: AsyncRead + Unpin>(
    recv: &mut R,
    mut emit: impl FnMut(ChatStreamMsg) -> bool,
) {
    loop {
        match read_frame(recv, None).await {
            Ok(Some((kind, payload))) => {
                if let Some(message) = stream_message(kind, &payload) {
                    let last = message.is_final();
                    if !emit(message) || last {
                        return;
                    }
                }
            }
            Ok(None) => {
                emit(ChatStreamMsg::Closed {
                    message: Some("The PC closed the chat".into()),
                });
                return;
            }
            Err(error) => {
                emit(ChatStreamMsg::Error {
                    message: error.message.as_str().to_owned(),
                    code: Some(error.code),
                });
                return;
            }
        }
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

#[tauri::command]
pub async fn chat_list(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let data = crate::retry::retry_once(|fresh| {
        let state = &state;
        let dir = &dir;
        let ticket = &ticket;
        let id = &id;
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            let data = request(&mut send, &mut recv, Request::ChatList).await;
            let _ = send.finish();
            data
        }
    })
    .await?;
    Ok(data.unwrap_or(serde_json::Value::Null))
}

/// One request/reply op on a fresh stream (UNI-260 chat_new / chat_stop / config).
async fn one_shot(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    req: Request,
    limit: Duration,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(app)?;
    let (ticket, id) = host_ticket(&dir, host_id)?;
    let (mut send, mut recv) = open_stream(state, &dir, &ticket, &id, false).await?;
    let data = request_within(&mut send, &mut recv, req, limit).await;
    let _ = send.finish();
    Ok(data?.unwrap_or(serde_json::Value::Null))
}

/// UNI-260: start pi in a new herdr tab / tmux window (`target` is the wire
/// `ChatNewTarget`). Answers `ChatNewData` once its bridge is up (the host
/// waits up to 45 s).
#[tauri::command]
pub async fn chat_new_session(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    target: serde_json::Value,
    cwd: Option<String>,
    outside: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    let target: unipi_proto::ChatNewTarget = serde_json::from_value(target).map_err(|e| schema(e.to_string()))?;
    let cwd = match cwd {
        Some(c) => Some(c.try_into().map_err(|e: String| schema(e))?),
        None => None,
    };
    one_shot(&app, &state, &host_id, Request::ChatNew { target, cwd, outside }, Duration::from_secs(75)).await
}

/// UNI-260: stop a pi (SIGTERM, graceful) and, with `close_tab`, close its
/// herdr tab / tmux window. Answers `ChatCloseData`.
#[tauri::command]
pub async fn chat_stop(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    pid: u32,
    close_tab: bool,
) -> Result<serde_json::Value, AppErrorDto> {
    let pid = std::num::NonZeroU32::new(pid).ok_or_else(|| schema("pid must be at least 1"))?;
    one_shot(&app, &state, &host_id, Request::ChatClose { pid, close_tab }, Duration::from_secs(40)).await
}

/// UNI-260: the PC's chat launcher (+ which multiplexers it has). `launcher`
/// given → store it first (`""` = back to `pi`).
#[tauri::command]
pub async fn chat_config(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    launcher: Option<String>,
) -> Result<serde_json::Value, AppErrorDto> {
    let req = match launcher {
        Some(l) => Request::ChatConfigSet { launcher: l.try_into().map_err(|e: String| schema(e))? },
        None => Request::ChatConfig,
    };
    one_shot(&app, &state, &host_id, req, REPLY_TIMEOUT).await
}

struct StreamHandle {
    send: Arc<tokio::sync::Mutex<crate::op_stream::OpSend>>,
    reader: tokio::task::JoinHandle<()>,
}

/// Live chats, keyed by stream id.
#[derive(Clone, Default)]
pub struct ChatStreams {
    streams: Arc<Mutex<HashMap<String, StreamHandle>>>,
    next: Arc<AtomicU64>,
}

impl ChatStreams {
    fn new_id(&self) -> String {
        format!("chat-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, StreamHandle>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[tauri::command]
pub async fn chat_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, ChatStreams>,
    host_id: String,
    pid: u32,
    on_event: Channel<ChatStreamMsg>,
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
            request(&mut send, &mut recv, Request::ChatOpen { pid }).await?;
            Ok((send, recv))
        }
    })
    .await?;
    let stream_id = streams.new_id();
    let _ = on_event.send(ChatStreamMsg::Open);
    let registry = streams.inner().clone();
    let reader_id = stream_id.clone();
    let mut map = streams.lock();
    let reader = tokio::spawn(async move {
        pump(&mut recv, |message| on_event.send(message).is_ok()).await;
        registry.lock().remove(&reader_id);
    });
    map.insert(
        stream_id.clone(),
        StreamHandle {
            send: Arc::new(tokio::sync::Mutex::new(send)),
            reader,
        },
    );
    drop(map);
    Ok(stream_id)
}

#[tauri::command]
pub async fn chat_send(
    streams: State<'_, ChatStreams>,
    stream_id: String,
    line: String,
) -> Result<(), AppErrorDto> {
    if line.len() > MAX_SEND {
        return Err(schema("That message is too large to send (photos too big?)"));
    }
    let send = streams
        .lock()
        .get(&stream_id)
        .map(|h| h.send.clone())
        .ok_or_else(|| {
            AppErrorDto::new(AppErrorCode::E_TRANSPORT, "The chat is not connected", true)
        })?;
    let frame = control_frame(&Control::Chat { line })?;
    let mut send = send.lock().await;
    send.write_all(&frame).await.map_err(transport)
}

#[tauri::command]
pub async fn chat_close(
    streams: State<'_, ChatStreams>,
    stream_id: String,
) -> Result<(), AppErrorDto> {
    let handle = streams.lock().remove(&stream_id);
    if let Some(handle) = handle {
        handle.reader.abort();
        let _ = handle.send.lock().await.finish();
    }
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
            data: Some(serde_json::json!({"herdr":{"available":true}})),
        });
        tokio::spawn(async move {
            let mut hello = vec![0u8; 5];
            host.read_exact(&mut hello).await.unwrap();
            let len = u32::from_be_bytes(hello[1..5].try_into().unwrap()) as usize;
            let mut body = vec![0u8; len];
            host.read_exact(&mut body).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["req"]["op"], "chat_list");
            host.write_all(&reply).await.unwrap();
        });
        let (mut r, mut w) = tokio::io::split(&mut phone);
        let data = request(&mut w, &mut r, Request::ChatList).await.unwrap();
        assert_eq!(data.unwrap()["herdr"]["available"], true);

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
        let error = request(&mut w, &mut r, Request::ChatOpen { pid: 9 }).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_NOT_FOUND);
    }

    #[tokio::test]
    async fn pump_maps_lines_exit_and_eof() {
        let mut bytes = frame(&Control::Chat { line: "{\"t\":\"hello\"}".into() });
        bytes.extend(encode(FrameKind::Output, b"ignored"));
        bytes.extend(frame(&Control::Exit { code: None }));
        bytes.extend(frame(&Control::Chat { line: "never".into() }));
        let mut out = Vec::new();
        pump(&mut bytes.as_slice(), |m| {
            out.push(m);
            true
        })
        .await;
        assert_eq!(
            out,
            vec![
                ChatStreamMsg::Line { line: "{\"t\":\"hello\"}".into() },
                ChatStreamMsg::Closed { message: Some("The pi session ended".into()) },
            ]
        );

        let mut out = Vec::new();
        pump(&mut frame(&Control::Chat { line: "x".into() }).as_slice(), |m| {
            out.push(m);
            true
        })
        .await;
        assert!(matches!(out.last(), Some(ChatStreamMsg::Closed { .. })));
    }

    #[test]
    fn stream_message_shapes() {
        let err = frame(&Control::Err {
            error: AppErrorDto::new(AppErrorCode::E_CHAT_UNAVAILABLE, "lost", true),
        });
        let msg = stream_message(0, &err[5..]).unwrap();
        assert_eq!(
            serde_json::to_value(&msg).unwrap(),
            serde_json::json!({"kind":"error","message":"lost","code":"E_CHAT_UNAVAILABLE"})
        );
        assert_eq!(
            serde_json::to_value(ChatStreamMsg::Line { line: "a".into() }).unwrap(),
            serde_json::json!({"kind":"line","line":"a"})
        );
        assert!(stream_message(1, b"x").is_none());
    }
}
