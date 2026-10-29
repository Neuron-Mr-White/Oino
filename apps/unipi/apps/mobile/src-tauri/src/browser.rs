//! Remote desktop browser (UNI-215): the core's side of `browser_status`,
//! `browser_open` (a live stream) and `browser_close`.
//!
//! - `browser_status(host_id)` → the host's `BrowserStatusData`.
//! - `browser_open(host_id, viewport, url?, target?, on_event, on_frame)` →
//!   `{streamId, open}`. `on_event` gets `{kind:'event', event}` per host
//!   `browser_event` (minus frames) and exactly one final `closed`/`error`.
//!   `on_frame` gets raw bytes per screencast frame: an 8-byte header
//!   (`u32 seq`, `u16 css_width`, `u16 css_height`, big-endian) + the JPEG,
//!   so the UI can draw and ack without a JSON hop.
//! - `browser_cmd(stream_id, cmd)` sends one `BrowserCmd` (input, nav, ack…).
//! - `browser_stop(stream_id)` hangs up (idempotent; the browser keeps running
//!   on the PC until its idle timeout).
//! - `browser_close(host_id)` shuts the PC's browser down now.
//! - `browser_window(host_id, show)` shows/hides its window on the PC (UNI-268).
//! - `browser_profiles(host_id)` lists the PC's browser profiles (UNI-296).
//! - `browser_takeover(host_id, on, profile?)` takes the browser over from
//!   agents (their input is refused) or gives it back (UNI-296).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, BrowserCmd, BrowserEvent, BrowserViewport, Control, ProtoError, Request,
    PROTO_VERSION,
};

use crate::net::NetState;

/// Launching a browser on the PC can take a while the first time.
const OPEN_TIMEOUT: Duration = Duration::from_secs(60);
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserStreamMsg {
    Event {
        event: BrowserEvent,
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
    let bytes = serde_json::to_vec(&hello).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(encode(FrameKind::Control, &bytes))
}

/// Encodes one `browser_cmd` control frame.
pub(crate) fn cmd_frame(cmd: BrowserCmd) -> Result<Vec<u8>, AppErrorDto> {
    cmd.check().map_err(schema)?;
    let bytes = serde_json::to_vec(&Control::BrowserCmd { cmd })
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    if bytes.len() > MAX_FRAME {
        return Err(schema("That command is too large to send"));
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

/// Sends `hello` and returns the host's `ok` data (or its error).
pub(crate) async fn request<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    send: &mut W,
    recv: &mut R,
    req: Request,
    timeout: Duration,
) -> Result<Option<serde_json::Value>, AppErrorDto> {
    send.write_all(&hello_frame(req)?).await.map_err(transport)?;
    let (kind, payload) = read_frame(recv, Some(timeout))
        .await?
        .ok_or_else(|| transport("The PC closed the stream without a reply"))?;
    if kind != 0 {
        return Err(schema("expected a control reply"));
    }
    match serde_json::from_slice(&payload).map_err(|e| schema(e.to_string()))? {
        Control::Ok { data } => Ok(data),
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected reply")),
    }
}

/// The 8-byte frame header the UI reads before the JPEG.
pub(crate) fn frame_header(seq: u64, css_width: f64, css_height: f64) -> [u8; 8] {
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&(seq as u32).to_be_bytes());
    header[4..6].copy_from_slice(&(css_width.round().clamp(0.0, 65535.0) as u16).to_be_bytes());
    header[6..8].copy_from_slice(&(css_height.round().clamp(0.0, 65535.0) as u16).to_be_bytes());
    header
}

/// What the pump hands the UI.
#[derive(Debug, PartialEq)]
pub(crate) enum Out {
    Msg(BrowserStreamMsg),
    Frame(Vec<u8>),
}

/// Pumps an open browser stream until it ends: exactly one final message
/// unless `emit` fails. A `frame` event is joined with the JPEG that follows.
pub(crate) async fn pump<R: AsyncRead + Unpin>(recv: &mut R, mut emit: impl FnMut(Out) -> bool) {
    let mut pending: Option<(u64, f64, f64)> = None;
    loop {
        match read_frame(recv, None).await {
            Ok(Some((1, jpeg))) => {
                if let Some((seq, w, h)) = pending.take() {
                    let mut bytes = frame_header(seq, w, h).to_vec();
                    bytes.extend_from_slice(&jpeg);
                    if !emit(Out::Frame(bytes)) {
                        return;
                    }
                }
            }
            Ok(Some((0, payload))) => match serde_json::from_slice::<Control>(&payload) {
                Ok(Control::BrowserEvent { event: BrowserEvent::Frame(meta) }) => {
                    pending = Some((meta.seq, meta.css_width, meta.css_height));
                }
                Ok(Control::BrowserEvent { event }) => {
                    let last = matches!(event, BrowserEvent::Closed { .. });
                    if !emit(Out::Msg(BrowserStreamMsg::Event { event })) {
                        return;
                    }
                    if last {
                        emit(Out::Msg(BrowserStreamMsg::Closed { message: None }));
                        return;
                    }
                }
                Ok(Control::Err { error }) => {
                    emit(Out::Msg(BrowserStreamMsg::Error {
                        message: error.message.as_str().to_owned(),
                        code: Some(error.code),
                    }));
                    return;
                }
                _ => {}
            },
            Ok(Some(_)) => {}
            Ok(None) => {
                emit(Out::Msg(BrowserStreamMsg::Closed { message: Some("The PC closed the browser view".into()) }));
                return;
            }
            Err(error) => {
                emit(Out::Msg(BrowserStreamMsg::Error {
                    message: error.message.as_str().to_owned(),
                    code: Some(error.code),
                }));
                return;
            }
        }
    }
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
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

async fn one_shot(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    req: Request,
) -> Result<Option<serde_json::Value>, AppErrorDto> {
    let dir = directory(app)?;
    let (ticket, id) = host_ticket(&dir, host_id)?;
    crate::retry::retry_once(|fresh| {
        let (dir, ticket, id, req) = (&dir, &ticket, &id, req.clone());
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            let data = request(&mut send, &mut recv, req, REPLY_TIMEOUT).await;
            let _ = send.finish();
            data
        }
    })
    .await
}

#[tauri::command]
pub async fn browser_status(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    Ok(one_shot(&app, &state, &host_id, Request::BrowserStatus).await?.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub async fn browser_close(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<(), AppErrorDto> {
    one_shot(&app, &state, &host_id, Request::BrowserClose).await.map(|_| ())
}

/// UNI-268: show (or hide) the PC browser's window; returns the new status.
#[tauri::command]
pub async fn browser_window(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    show: bool,
) -> Result<serde_json::Value, AppErrorDto> {
    Ok(one_shot(&app, &state, &host_id, Request::BrowserWindow { show }).await?.unwrap_or(serde_json::Value::Null))
}

/// UNI-296: the PC's browser profiles (`BrowserProfilesData`).
#[tauri::command]
pub async fn browser_profiles(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    Ok(one_shot(&app, &state, &host_id, Request::BrowserProfiles).await?.unwrap_or(serde_json::Value::Null))
}

/// Parses an optional profile name from the UI.
fn profile_arg(profile: Option<String>) -> Result<Option<unipi_proto::BrowserProfile>, AppErrorDto> {
    match profile.filter(|p| !p.trim().is_empty()) {
        Some(p) => Ok(Some(p.try_into().map_err(schema)?)),
        None => Ok(None),
    }
}

/// UNI-296: take the browser over from agents (`on`) or give it back; returns `BrowserAgent`.
#[tauri::command]
pub async fn browser_takeover(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    on: bool,
    profile: Option<String>,
) -> Result<serde_json::Value, AppErrorDto> {
    let profile = profile_arg(profile)?;
    Ok(one_shot(&app, &state, &host_id, Request::BrowserTakeover { on, profile }).await?.unwrap_or(serde_json::Value::Null))
}

struct StreamHandle {
    send: Arc<tokio::sync::Mutex<crate::op_stream::OpSend>>,
    reader: tokio::task::JoinHandle<()>,
}

/// Live browser views, keyed by stream id.
#[derive(Clone, Default)]
pub struct BrowserStreams {
    streams: Arc<Mutex<HashMap<String, StreamHandle>>>,
    next: Arc<AtomicU64>,
}

impl BrowserStreams {
    fn new_id(&self) -> String {
        format!("browser-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, StreamHandle>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOpenResult {
    stream_id: String,
    open: serde_json::Value,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn browser_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, BrowserStreams>,
    host_id: String,
    viewport: BrowserViewport,
    url: Option<String>,
    target: Option<String>,
    profile: Option<String>,
    on_event: Channel<BrowserStreamMsg>,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<BrowserOpenResult, AppErrorDto> {
    let url = match url.filter(|u| !u.trim().is_empty()) {
        Some(url) => Some(url.try_into().map_err(schema)?),
        None => None,
    };
    let dir = directory(&app)?;
    let (ticket, id) = host_ticket(&dir, &host_id)?;
    let profile = profile_arg(profile)?;
    let req = Request::BrowserOpen { viewport, url, target, profile };
    let (send, mut recv, open) = crate::retry::retry_once(|fresh| {
        let (state, dir, ticket, id, req) = (&state, &dir, &ticket, &id, req.clone());
        async move {
            let (mut send, mut recv) = open_stream(state, dir, ticket, id, fresh).await?;
            let open = request(&mut send, &mut recv, req, OPEN_TIMEOUT).await?;
            Ok((send, recv, open))
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
    Ok(BrowserOpenResult { stream_id, open: open.unwrap_or(serde_json::Value::Null) })
}

#[tauri::command]
pub async fn browser_cmd(
    streams: State<'_, BrowserStreams>,
    stream_id: String,
    cmd: BrowserCmd,
) -> Result<(), AppErrorDto> {
    let send = streams
        .lock()
        .get(&stream_id)
        .map(|h| h.send.clone())
        .ok_or_else(|| AppErrorDto::new(AppErrorCode::E_TRANSPORT, "The browser view is not connected", true))?;
    let frame = cmd_frame(cmd)?;
    let mut send = send.lock().await;
    send.write_all(&frame).await.map_err(transport)
}

#[tauri::command]
pub async fn browser_stop(streams: State<'_, BrowserStreams>, stream_id: String) -> Result<(), AppErrorDto> {
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
    use unipi_proto::BrowserFrameMeta;

    fn control(c: &Control) -> Vec<u8> {
        encode(FrameKind::Control, &serde_json::to_vec(c).unwrap())
    }

    #[tokio::test]
    async fn pump_joins_frames_and_ends_once() {
        let (mut host, mut phone) = tokio::io::duplex(1 << 16);
        let mut wire = Vec::new();
        wire.extend(control(&Control::BrowserEvent {
            event: BrowserEvent::Tabs { tabs: vec![], active: "T".into() },
        }));
        wire.extend(control(&Control::BrowserEvent {
            event: BrowserEvent::Frame(BrowserFrameMeta { seq: 5, css_width: 412.0, css_height: 780.0, quality: 70 }),
        }));
        wire.extend(encode(FrameKind::Output, &[0xFF, 0xD8, 0xFF, 1]));
        wire.extend(control(&Control::BrowserEvent { event: BrowserEvent::Closed { reason: "idle".into() } }));
        host.write_all(&wire).await.unwrap();
        drop(host);
        let mut out = Vec::new();
        pump(&mut phone, |o| {
            out.push(o);
            true
        })
        .await;
        assert_eq!(out.len(), 4, "{out:?}");
        assert!(matches!(&out[0], Out::Msg(BrowserStreamMsg::Event { event: BrowserEvent::Tabs { .. } })));
        let Out::Frame(bytes) = &out[1] else { panic!() };
        assert_eq!(&bytes[..8], &[0, 0, 0, 5, 1, 156, 3, 12]);
        assert_eq!(&bytes[8..], &[0xFF, 0xD8, 0xFF, 1]);
        assert!(matches!(&out[3], Out::Msg(BrowserStreamMsg::Closed { .. })));
    }

    #[tokio::test]
    async fn pump_reports_eof_and_errors() {
        let (host, mut phone) = tokio::io::duplex(64);
        drop(host);
        let mut out = Vec::new();
        pump(&mut phone, |o| {
            out.push(o);
            true
        })
        .await;
        assert!(matches!(&out[..], [Out::Msg(BrowserStreamMsg::Closed { message: Some(_) })]));

        let (mut host, mut phone) = tokio::io::duplex(1024);
        let err = AppErrorDto::new(AppErrorCode::E_BROWSER_UNAVAILABLE, "crashed", true);
        host.write_all(&control(&Control::Err { error: err })).await.unwrap();
        let mut out = Vec::new();
        pump(&mut phone, |o| {
            out.push(o);
            true
        })
        .await;
        assert!(matches!(&out[..], [Out::Msg(BrowserStreamMsg::Error { code: Some(AppErrorCode::E_BROWSER_UNAVAILABLE), .. })]));
    }

    #[tokio::test]
    async fn request_returns_ok_or_host_error() {
        let (mut phone_w, mut host_r) = tokio::io::duplex(4096);
        let (mut host_w, mut phone_r) = tokio::io::duplex(4096);
        let disabled = AppErrorDto::new(AppErrorCode::E_BROWSER_DISABLED, "off", false);
        host_w.write_all(&control(&Control::Err { error: disabled })).await.unwrap();
        let result = request(&mut phone_w, &mut phone_r, Request::BrowserStatus, REPLY_TIMEOUT).await;
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_BROWSER_DISABLED);
        // The hello went out.
        let mut buf = [0u8; 5];
        host_r.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn profile_arg_validates() {
        assert_eq!(profile_arg(None).unwrap(), None);
        assert_eq!(profile_arg(Some(" ".into())).unwrap(), None);
        assert_eq!(profile_arg(Some("work".into())).unwrap().unwrap().as_str(), "work");
        assert_eq!(profile_arg(Some("Bad Name".into())).unwrap_err().code, AppErrorCode::E_SCHEMA);
    }

    #[test]
    fn cmd_frame_validates() {
        let ok = cmd_frame(BrowserCmd::Ack { seq: 1 }).unwrap();
        assert_eq!(ok[0], 0);
        let bad = cmd_frame(BrowserCmd::Pinch { x: 0.0, y: 0.0, scale: 0.0 });
        assert_eq!(bad.unwrap_err().code, AppErrorCode::E_SCHEMA);
    }
}
