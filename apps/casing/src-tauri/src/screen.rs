//! Desktop streaming (UNI-216): the core's side of `desktop_status` and
//! `desktop_open` — watch and drive the PC's whole screen. (Named `screen`
//! here because `desktop.rs` is the desktop-app shell: tray, notifications.)
//!
//! - `screen_status(host_id)` → the host's `DesktopStatusData`.
//! - `screen_open(host_id, viewport, quality?, view_only?, on_event, on_frame)`
//!   → `{streamId}`. `on_event` gets `{kind:'event', event}` per host
//!   `desktop_event` (minus frames) and exactly one final `closed`/`error`.
//!   `on_frame` gets raw bytes per H.264 access unit: a 13-byte header
//!   (`u32 seq`, `u8 key`, `u64 ts_ms`, big-endian) + Annex-B, so the UI can
//!   feed WebCodecs without a JSON hop.
//! - `screen_cmd(stream_id, cmd)` sends one `DesktopCmd` (input, ack, …).
//! - `screen_stop(stream_id)` hangs up (idempotent).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, DesktopCmd, DesktopEvent, DesktopQuality, DesktopViewport, ProtoError, Request,
};

use crate::browser::request;
use crate::net::NetState;

const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScreenStreamMsg {
    Event {
        event: DesktopEvent,
    },
    Closed {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<AppErrorCode>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn schema(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
}

/// Encodes one `desktop_cmd` control frame.
pub(crate) fn cmd_frame(cmd: DesktopCmd) -> Result<Vec<u8>, AppErrorDto> {
    cmd.check().map_err(schema)?;
    let bytes = serde_json::to_vec(&Control::DesktopCmd { cmd }).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    if bytes.len() > MAX_FRAME {
        return Err(schema("That command is too large to send"));
    }
    Ok(encode(FrameKind::Control, &bytes))
}

/// The 13-byte header the UI reads before the access unit.
pub(crate) fn frame_header(seq: u64, key: bool, ts_ms: u64) -> [u8; 13] {
    let mut header = [0u8; 13];
    header[..4].copy_from_slice(&(seq as u32).to_be_bytes());
    header[4] = key as u8;
    header[5..13].copy_from_slice(&ts_ms.to_be_bytes());
    header
}

async fn read_frame<R: AsyncRead + Unpin>(recv: &mut R) -> Result<Option<(u8, Vec<u8>)>, AppErrorDto> {
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
}

#[derive(Debug, PartialEq)]
pub(crate) enum Out {
    Msg(ScreenStreamMsg),
    Frame(Vec<u8>),
}

fn error_msg(error: AppErrorDto) -> ScreenStreamMsg {
    ScreenStreamMsg::Error {
        message: error.message.as_str().to_owned(),
        code: Some(error.code),
        reason: error.detail.as_ref().and_then(|d| d.get("reason")).and_then(|r| r.as_str()).map(str::to_string),
        hint: error.hint,
    }
}

/// Pumps an open desktop stream until it ends: exactly one final message
/// unless `emit` fails. A `frame` event is joined with the bytes that follow
/// (possibly several kind-1 frames).
pub(crate) async fn pump<R: AsyncRead + Unpin>(recv: &mut R, mut emit: impl FnMut(Out) -> bool) {
    // (seq, key, ts, expected bytes, collected)
    let mut pending: Option<(u64, bool, u64, usize, Vec<u8>)> = None;
    loop {
        match read_frame(recv).await {
            Ok(Some((1, chunk))) => {
                let Some(p) = pending.as_mut() else { continue };
                p.4.extend_from_slice(&chunk);
                if p.4.len() >= p.3 {
                    let (seq, key, ts, _, data) = pending.take().expect("checked");
                    let mut bytes = frame_header(seq, key, ts).to_vec();
                    bytes.extend_from_slice(&data);
                    if !emit(Out::Frame(bytes)) {
                        return;
                    }
                }
            }
            Ok(Some((0, payload))) => match serde_json::from_slice::<Control>(&payload) {
                Ok(Control::DesktopEvent { event: DesktopEvent::Frame(meta) }) => {
                    pending = Some((meta.seq, meta.key, meta.ts_ms, meta.bytes as usize, Vec::with_capacity(meta.bytes as usize)));
                }
                Ok(Control::DesktopEvent { event }) => {
                    let last = matches!(event, DesktopEvent::Closed { .. });
                    if !emit(Out::Msg(ScreenStreamMsg::Event { event })) {
                        return;
                    }
                    if last {
                        emit(Out::Msg(ScreenStreamMsg::Closed { message: None }));
                        return;
                    }
                }
                Ok(Control::Err { error }) => {
                    emit(Out::Msg(error_msg(error)));
                    return;
                }
                _ => {}
            },
            Ok(Some(_)) => {}
            Ok(None) => {
                emit(Out::Msg(ScreenStreamMsg::Closed { message: Some("The PC closed the desktop stream".into()) }));
                return;
            }
            Err(error) => {
                emit(Out::Msg(error_msg(error)));
                return;
            }
        }
    }
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    use tauri::Manager;
    app.path().app_data_dir().map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

fn host_ticket(dir: &std::path::Path, host_id: &str) -> Result<(String, String), AppErrorDto> {
    let host = crate::local_host::hosts(dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let id = crate::net::parse_ticket(&host.ticket)?.endpoint_addr().id.to_string();
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
pub async fn screen_status(app: tauri::AppHandle, state: State<'_, NetState>, host_id: String) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let data = crate::retry::retry_once(|fresh| {
        let (state, dir, ticket, id) = (&state, &dir, &ticket, &id);
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            let data = request(&mut send, &mut recv, Request::DesktopStatus, REPLY_TIMEOUT).await;
            let _ = send.finish();
            data
        }
    })
    .await?;
    Ok(data.unwrap_or(serde_json::Value::Null))
}

struct StreamHandle {
    send: Arc<tokio::sync::Mutex<crate::op_stream::OpSend>>,
    reader: tokio::task::JoinHandle<()>,
}

/// Live desktop views, keyed by stream id.
#[derive(Clone, Default)]
pub struct ScreenStreams {
    streams: Arc<Mutex<HashMap<String, StreamHandle>>>,
    next: Arc<AtomicU64>,
}

impl ScreenStreams {
    fn new_id(&self) -> String {
        format!("screen-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, StreamHandle>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenOpenResult {
    stream_id: String,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn screen_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, ScreenStreams>,
    host_id: String,
    viewport: DesktopViewport,
    quality: Option<DesktopQuality>,
    view_only: Option<bool>,
    display: Option<String>,
    on_event: Channel<ScreenStreamMsg>,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<ScreenOpenResult, AppErrorDto> {
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let display = display.filter(|d| !d.is_empty() && d.len() <= 64);
    let req = Request::DesktopOpen { viewport, quality, view_only, display };
    let (send, mut recv) = crate::retry::retry_once(|fresh| {
        let (state, dir, ticket, id, req) = (&state, &dir, &ticket, &id, req.clone());
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            request(&mut send, &mut recv, req, REPLY_TIMEOUT).await?;
            Ok((send, recv))
        }
    })
    .await?;
    let stream_id = streams.new_id();
    let registry = streams.inner().clone();
    let reader_id = stream_id.clone();
    let mut map = streams.lock();
    let reader = tokio::spawn(async move {
        pump(&mut recv, |out| match out {
            Out::Msg(message) => on_event.send(message).is_ok(),
            Out::Frame(bytes) => on_frame.send(InvokeResponseBody::Raw(bytes)).is_ok(),
        })
        .await;
        registry.lock().remove(&reader_id);
    });
    map.insert(stream_id.clone(), StreamHandle { send: Arc::new(tokio::sync::Mutex::new(send)), reader });
    drop(map);
    Ok(ScreenOpenResult { stream_id })
}

#[tauri::command]
pub async fn screen_cmd(streams: State<'_, ScreenStreams>, stream_id: String, cmd: DesktopCmd) -> Result<(), AppErrorDto> {
    let send = streams
        .lock()
        .get(&stream_id)
        .map(|h| h.send.clone())
        .ok_or_else(|| AppErrorDto::new(AppErrorCode::E_TRANSPORT, "The desktop view is not connected", true))?;
    let frame = cmd_frame(cmd)?;
    let mut send = send.lock().await;
    send.write_all(&frame).await.map_err(transport)
}

#[tauri::command]
pub async fn screen_stop(streams: State<'_, ScreenStreams>, stream_id: String) -> Result<(), AppErrorDto> {
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
    use tokio::io::AsyncWriteExt;
    use unipi_proto::DesktopFrameMeta;

    fn control(c: &Control) -> Vec<u8> {
        encode(FrameKind::Control, &serde_json::to_vec(c).unwrap())
    }

    #[tokio::test]
    async fn pump_joins_split_frames_and_ends_once() {
        let (mut host, mut phone) = tokio::io::duplex(1 << 16);
        let mut wire = Vec::new();
        wire.extend(control(&Control::DesktopEvent { event: DesktopEvent::Config { codec: "avc1.42e01f".into(), width: 640, height: 360 } }));
        wire.extend(control(&Control::DesktopEvent { event: DesktopEvent::Frame(DesktopFrameMeta { seq: 7, key: true, ts_ms: 258, bytes: 6 }) }));
        wire.extend(encode(FrameKind::Output, &[0, 0, 0, 1]));
        wire.extend(encode(FrameKind::Output, &[0x67, 0x42]));
        wire.extend(control(&Control::DesktopEvent { event: DesktopEvent::Closed { reason: "capture_ended".into() } }));
        host.write_all(&wire).await.unwrap();
        drop(host);
        let mut out = Vec::new();
        pump(&mut phone, |o| {
            out.push(o);
            true
        })
        .await;
        assert_eq!(out.len(), 4, "{out:?}");
        assert!(matches!(&out[0], Out::Msg(ScreenStreamMsg::Event { event: DesktopEvent::Config { .. } })));
        let Out::Frame(bytes) = &out[1] else { panic!() };
        assert_eq!(&bytes[..13], &[0, 0, 0, 7, 1, 0, 0, 0, 0, 0, 0, 1, 2]);
        assert_eq!(&bytes[13..], &[0, 0, 0, 1, 0x67, 0x42]);
        assert!(matches!(&out[3], Out::Msg(ScreenStreamMsg::Closed { message: None })));
    }

    #[tokio::test]
    async fn pump_reports_errors_with_reason() {
        let (mut host, mut phone) = tokio::io::duplex(4096);
        let err = AppErrorDto::new(AppErrorCode::E_DESKTOP_UNAVAILABLE, "Cancelled", false)
            .with_detail(serde_json::json!({"reason":"denied"}))
            .with_hint("Choose Share");
        host.write_all(&control(&Control::Err { error: err })).await.unwrap();
        let mut out = Vec::new();
        pump(&mut phone, |o| {
            out.push(o);
            true
        })
        .await;
        match &out[..] {
            [Out::Msg(ScreenStreamMsg::Error { code: Some(AppErrorCode::E_DESKTOP_UNAVAILABLE), reason: Some(r), hint: Some(_), .. })] => assert_eq!(r, "denied"),
            other => panic!("{other:?}"),
        }
        let (host, mut phone) = tokio::io::duplex(64);
        drop(host);
        let mut out = Vec::new();
        pump(&mut phone, |o| {
            out.push(o);
            true
        })
        .await;
        assert!(matches!(&out[..], [Out::Msg(ScreenStreamMsg::Closed { message: Some(_) })]));
    }

    #[test]
    fn cmd_frame_validates() {
        assert_eq!(cmd_frame(DesktopCmd::Keyframe).unwrap()[0], 0);
        assert_eq!(cmd_frame(DesktopCmd::Pointer { x: 2.0, y: 0.0 }).unwrap_err().code, AppErrorCode::E_SCHEMA);
    }
}
