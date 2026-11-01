//! Terminals (DESIGN.md §3.3, §5.4; host.md §9.3 v2): the PTYs live in the
//! **host**, this module keeps one stream per open tab.
//!
//! - `term_open` opens a new host PTY in a workspace (`cwd`) or, when
//!   `term_id` is given, re-attaches this tab to an existing one — the reply
//!   is `{termId, shell}` and then the stream carries output (kind 1), input
//!   (kind 2), `resize`, `title` and `exit` (host.md §9.3).
//! - Closing the tab (`term_close`) only *detaches*: the host keeps the shell
//!   running (ZCode parity), so a reopened tab or another device re-attaches
//!   it. `term_kill` really ends it.
//! - Every stream carries a generation token: a reader task removes its
//!   registry entry only while its token is still current
//!   (compare-and-remove), so a stream that ends late — e.g. the one a
//!   webview reload replaced — cannot remove the entry of the stream that
//!   took its place, and opening an already-tracked id replaces the previous
//!   stream cleanly (closing it first).
//! - The host keeps no scrollback: a re-attached tab sees new output only, so
//!   the app owns its xterm buffer per live tab.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, AppErrorDto, Control, Dimension, FsPath, ProtoError, Request, TermAttachData,
    TermId, TermInfo, TermShell, PROTO_VERSION,
};

use crate::{net::NetState, path_kind::PathKind, retry::retry_once};

/// Events pushed to the UI while a terminal stream is live. Exactly one of
/// `exit`/`closed` is sent last; `title` can appear any time before.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TermEvent {
    Title {
        title: String,
    },
    Exit {
        code: Option<i64>,
    },
    Closed {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        error: Option<AppErrorDto>,
    },
}

/// What a single wire frame means for the UI. Data goes to the terminal
/// buffer; events go to the event channel; `Ignore` is an
/// unexpected-but-harmless frame (e.g. a stray `ok`).
#[derive(Debug, Clone)]
pub enum Decoded {
    Data(Vec<u8>),
    Event(TermEvent),
    Ignore,
}

/// Pure frame → event mapping (unit-tested).
pub fn decode_frame(kind: u8, payload: &[u8]) -> Result<Decoded, Decoded> {
    match kind {
        1 => Ok(Decoded::Data(payload.to_vec())),
        0 => {
            let control: Control =
                serde_json::from_slice(payload).map_err(|_| schema_closed(payload))?;
            match control {
                Control::Title { title } => Ok(Decoded::Event(TermEvent::Title {
                    title: title.as_str().to_owned(),
                })),
                Control::Exit { code } => Ok(Decoded::Event(TermEvent::Exit { code })),
                Control::Err { error } => {
                    Ok(Decoded::Event(TermEvent::Closed { error: Some(error) }))
                }
                Control::Ok { .. } => Ok(Decoded::Ignore),
                _ => Err(schema_closed(payload)),
            }
        }
        _ => Err(schema_closed(payload)),
    }
}

fn schema_closed(payload: &[u8]) -> Decoded {
    let _ = payload;
    Decoded::Event(TermEvent::Closed {
        error: Some(
            ProtoError::Schema {
                message: "unexpected frame from host".into(),
                path: vec![],
                detail: None,
            }
            .to_app_error(),
        ),
    })
}

/// Messages for the per-tab writer task that owns the send half.
pub(crate) enum WriterMsg {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

pub(crate) struct TermHandle {
    pub(crate) writer: mpsc::Sender<WriterMsg>,
    /// The reader task's handle (aborted when the stream is replaced or
    /// closed; not cloneable, so `lookup` returns the writer only).
    pub(crate) reader: Option<tokio::task::JoinHandle<()>>,
    /// Generation token of *this* stream (fresh per `term_open`, unique even
    /// under one terminal id): the reader task removes the entry only while
    /// the token still matches, so streams can't remove each other.
    pub(crate) token: u64,
}

/// Detaches a stream for good: stop its reader task, then tell its writer to
/// half-close so the host drops the attach. Dropping the writer sender (the
/// fallback when the channel is full or gone) ends the host stream too.
fn close_handle(handle: TermHandle) {
    if let Some(reader) = handle.reader {
        reader.abort();
    }
    let _ = handle.writer.try_send(WriterMsg::Close);
}

/// Managed registry of the app's live terminal streams (the host owns the
/// PTYs; this is one stream per open tab). Cloned into each reader task so a
/// task can remove its own entry — by token — when the stream ends.
#[derive(Clone, Default)]
pub struct TermRegistry {
    pub(crate) sessions: std::sync::Arc<Mutex<HashMap<String, TermHandle>>>,
    /// Source of the per-stream generation tokens.
    next_token: std::sync::Arc<AtomicU64>,
}

impl TermRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh, never-repeated stream token (0 is never handed out, so a
    /// zero value can't be mistaken for a live stream's token).
    fn next_token(&self) -> u64 {
        1 + self.next_token.fetch_add(1, Ordering::Relaxed)
    }

    /// Registers a stream for `id`, cleanly replacing any stream an earlier
    /// open left under the same id: the old one is closed first (reader
    /// aborted, writer told to detach).
    fn register(&self, id: String, handle: TermHandle) {
        let replaced = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, handle);
        if let Some(old) = replaced {
            close_handle(old);
        }
    }

    fn lookup_writer(&self, id: &str) -> Option<mpsc::Sender<WriterMsg>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|handle| handle.writer.clone())
    }

    /// Removes and returns a handle, whatever stream it belongs to (the
    /// explicit close/kill of this tab's current stream). Idempotent: a
    /// missing id (the reader already removed it after a natural end)
    /// returns `None`.
    fn remove(&self, id: &str) -> Option<TermHandle> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
    }

    /// Compare-and-remove: takes the entry out only while it is still *this*
    /// stream's (`token` matches), so a stream that ends late cannot remove
    /// the entry of the stream that replaced it (the webview-reload race).
    fn remove_own(&self, id: &str, token: u64) -> Option<TermHandle> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(id) {
            Some(handle) if handle.token == token => sessions.remove(id),
            _ => None,
        }
    }
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

fn schema(message: String) -> AppErrorDto {
    ProtoError::Schema {
        message,
        path: vec![],
        detail: None,
    }
    .to_app_error()
}

fn dimension(value: u16) -> Result<Dimension, AppErrorDto> {
    Dimension::try_from(u32::from(value))
        .map_err(ProtoError::Internal)
        .map_err(|e| e.to_app_error())
}

/// The `term_open` reply the app's `openTerm()` uses (`lib/term.ts`).
#[derive(Debug, Clone, Serialize)]
pub struct TermOpenResult {
    #[serde(rename = "termId")]
    pub term_id: String,
    pub shell: String,
    pub path: PathKind,
}

struct TermStreams {
    send: crate::op_stream::OpSend,
    recv: crate::op_stream::OpRecv,
    data: TermAttachData,
}

/// One `hello` carrying `term_open` / `term_attach`, then the reply frame.
async fn open_term_streams(
    state: &NetState,
    dir: &std::path::Path,
    ticket: &str,
    relay: Option<&str>,
    request: Request,
) -> Result<TermStreams, AppErrorDto> {
    let id = crate::net::parse_ticket(ticket)?
        .endpoint_addr()
        .id
        .to_string();
    retry_once(|fresh| {
        let request = request.clone();
        let id = id.clone();
        let dir = dir.to_path_buf();
        let ticket = ticket.to_string();
        let relay = relay.map(str::to_string);
        async move {
            if fresh {
                state.drop_connection(&id);
            }
            let (mut send, mut recv) = state.open_op(&dir, relay.as_deref(), &ticket).await?;
            let hello = Control::Hello {
                proto: PROTO_VERSION
                    .to_string()
                    .try_into()
                    .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
                req: request.clone(),
            };
            let bytes = serde_json::to_vec(&hello)
                .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
            send.write_all(&encode(FrameKind::Control, &bytes))
                .await
                .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
            let kind = recv
                .read_u8()
                .await
                .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
            let len = recv
                .read_u32()
                .await
                .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?
                as usize;
            if kind != 0 || len > unipi_proto::MAX_FRAME {
                return Err(ProtoError::Internal("Invalid reply frame".into()).to_app_error());
            }
            let mut bytes = vec![0u8; len];
            recv.read_exact(&mut bytes)
                .await
                .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
            match serde_json::from_slice::<Control>(&bytes)
                .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?
            {
                Control::Ok { data } => {
                    let payload = data.unwrap_or(serde_json::Value::Null);
                    let data: TermAttachData = serde_json::from_value(payload).map_err(|e| {
                        ProtoError::Schema {
                            message: format!("bad terminal reply: {e}"),
                            path: vec![],
                            detail: None,
                        }
                        .to_app_error()
                    })?;
                    Ok(TermStreams { send, recv, data })
                }
                Control::Err { error } => Err(error),
                _ => Err(ProtoError::Internal("Unexpected terminal reply".into()).to_app_error()),
            }
        }
    })
    .await
}

/// Opens a terminal — a new host PTY in `cwd`, or (`term_id`) a new stream on
/// an existing one, which the host resizes to `cols`×`rows`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn term_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    terms: State<'_, TermRegistry>,
    host_id: String,
    relay: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    term_id: Option<String>,
    shell: Option<String>,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<TermEvent>,
) -> Result<TermOpenResult, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let request = match &term_id {
        Some(term_id) => Request::TermAttach {
            term_id: TermId::try_from(term_id.clone()).map_err(schema)?,
            cols: dimension(cols)?,
            rows: dimension(rows)?,
        },
        None => Request::TermOpen {
            cwd: FsPath::try_from(cwd.unwrap_or_default()).map_err(schema)?,
            cols: dimension(cols)?,
            rows: dimension(rows)?,
            shell: shell.map(TermShell::try_from).transpose().map_err(schema)?,
        },
    };
    let streams = open_term_streams(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        request,
    )
    .await?;
    let path = state.path_for(&host.ticket);
    let term_id = streams.data.term_id.as_str().to_owned();
    let shell = streams.data.shell.as_str().to_owned();

    // Writer task: owns the send half.
    let (writer_tx, mut writer_rx) = mpsc::channel::<WriterMsg>(64);
    let mut send = streams.send;
    tokio::spawn(async move {
        while let Some(message) = writer_rx.recv().await {
            let result = match message {
                WriterMsg::Input(bytes) => send.write_all(&encode(FrameKind::Input, &bytes)).await,
                WriterMsg::Resize { cols, rows } => {
                    let control = match (dimension(cols), dimension(rows)) {
                        (Ok(cols), Ok(rows)) => Control::Resize { cols, rows },
                        _ => continue,
                    };
                    let control = serde_json::to_vec(&control).unwrap_or_default();
                    send.write_all(&encode(FrameKind::Control, &control)).await
                }
                WriterMsg::Close => {
                    // Detach: the host keeps the PTY running.
                    let _ = send.finish();
                    break;
                }
            };
            if result.is_err() {
                break;
            }
        }
    });

    // Reader task: forwards frames to the UI channels; exactly one
    // terminal (exit/closed) event is sent last. Removes its own registry
    // entry — compare-and-remove on its token — when it ends for any
    // reason, so it can never take a newer stream's entry out.
    let token = terms.next_token();
    let reader = tokio::spawn(pump_reader(
        streams.recv,
        on_data,
        on_event,
        TermRegistry::clone(&terms),
        term_id.clone(),
        token,
    ));

    terms.register(
        term_id.clone(),
        TermHandle {
            writer: writer_tx,
            reader: Some(reader),
            token,
        },
    );

    Ok(TermOpenResult {
        term_id,
        shell,
        path,
    })
}

/// One reader task: forwards a terminal stream's frames to the UI channels
/// (data to `on_data`, events to `on_event`; exactly one terminal
/// exit/closed event goes out last), then removes its own registry entry —
/// compare-and-remove on `token`, so a late-ending stream (e.g. the one a
/// webview reload replaced) leaves the newer stream's entry alone.
async fn pump_reader(
    mut recv: crate::op_stream::OpRecv,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<TermEvent>,
    terms: TermRegistry,
    term_id: String,
    token: u64,
) {
    let mut ended = false;
    while !ended {
        let mut kind_buffer = [0u8; 1];
        let mut len_buffer = [0u8; 4];
        if recv.read_exact(&mut kind_buffer).await.is_err()
            || recv.read_exact(&mut len_buffer).await.is_err()
        {
            let _ = on_event.send(TermEvent::Closed { error: None });
            break;
        }
        let len = u32::from_be_bytes(len_buffer) as usize;
        if len > unipi_proto::MAX_FRAME {
            let _ = on_event.send(TermEvent::Closed {
                error: Some(AppErrorDto::new(
                    AppErrorCode::E_SCHEMA,
                    "oversize frame",
                    false,
                )),
            });
            break;
        }
        let mut payload = vec![0u8; len];
        if recv.read_exact(&mut payload).await.is_err() {
            let _ = on_event.send(TermEvent::Closed { error: None });
            break;
        }
        match decode_frame(kind_buffer[0], &payload) {
            Ok(Decoded::Data(bytes)) => {
                if on_data.send(InvokeResponseBody::Raw(bytes)).is_err() {
                    break;
                }
            }
            Ok(Decoded::Event(event)) => {
                let is_final = matches!(event, TermEvent::Exit { .. } | TermEvent::Closed { .. });
                if on_event.send(event).is_err() {
                    break;
                }
                if is_final {
                    ended = true;
                }
            }
            Ok(Decoded::Ignore) => {}
            Err(Decoded::Event(closed)) => {
                let _ = on_event.send(closed);
                ended = true;
            }
            Err(_) => {
                let _ = on_event.send(TermEvent::Closed { error: None });
                ended = true;
            }
        }
    }
    terms.remove_own(&term_id, token);
}

/// Every terminal the host holds (`cwd` = its workspace), live or exited —
/// what the tab bar restores from.
#[tauri::command]
pub async fn term_list(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
) -> Result<Vec<TermInfo>, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (mut send, mut recv) = state
        .open_op(&dir, relay.as_deref().filter(|s| *s != "n0"), &host.ticket)
        .await?;
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
        req: Request::TermList,
    };
    let bytes = serde_json::to_vec(&hello)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    send.write_all(&encode(FrameKind::Control, &bytes))
        .await
        .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
    let (kind, payload) = read_control(&mut recv).await?;
    if kind != 0 {
        return Err(ProtoError::Internal("Invalid term_list reply".into()).to_app_error());
    }
    match serde_json::from_slice::<Control>(&payload)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?
    {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or(serde_json::Value::Null))
            .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error()),
        Control::Err { error } => Err(error),
        _ => Err(ProtoError::Internal("Unexpected term_list reply".into()).to_app_error()),
    }
}

/// Closes a terminal on the host (its child is killed).
#[tauri::command]
pub async fn term_kill(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    terms: State<'_, TermRegistry>,
    host_id: String,
    relay: Option<String>,
    term_id: String,
) -> Result<(), AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (mut send, mut recv) = state
        .open_op(&dir, relay.as_deref().filter(|s| *s != "n0"), &host.ticket)
        .await?;
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
        req: Request::TermKill {
            term_id: TermId::try_from(term_id.clone()).map_err(schema)?,
        },
    };
    let bytes = serde_json::to_vec(&hello)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    send.write_all(&encode(FrameKind::Control, &bytes))
        .await
        .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
    let (kind, payload) = read_control(&mut recv).await?;
    if kind != 0 {
        return Err(ProtoError::Internal("Invalid term_kill reply".into()).to_app_error());
    }
    match serde_json::from_slice::<Control>(&payload)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?
    {
        Control::Ok { .. } => {
            // Drop this app's stream too (no-op when the tab was detached).
            if let Some(handle) = terms.remove(&term_id) {
                close_handle(handle);
            }
            Ok(())
        }
        Control::Err { error } => Err(error),
        _ => Err(ProtoError::Internal("Unexpected term_kill reply".into()).to_app_error()),
    }
}

async fn read_control(recv: &mut crate::op_stream::OpRecv) -> Result<(u8, Vec<u8>), AppErrorDto> {
    let kind = recv
        .read_u8()
        .await
        .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
    let len = recv
        .read_u32()
        .await
        .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())? as usize;
    if len > unipi_proto::MAX_FRAME {
        return Err(ProtoError::Internal("Oversize frame".into()).to_app_error());
    }
    let mut payload = vec![0u8; len];
    recv.read_exact(&mut payload)
        .await
        .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
    Ok((kind, payload))
}

#[tauri::command]
pub async fn term_input(
    terms: State<'_, TermRegistry>,
    term_id: String,
    data: String,
) -> Result<(), AppErrorDto> {
    let writer = terms
        .lookup_writer(&term_id)
        .ok_or_else(|| AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "Unknown terminal", false))?;
    writer
        .send(WriterMsg::Input(data.into_bytes()))
        .await
        .map_err(|_| ProtoError::Internal("terminal writer closed".into()).to_app_error())
}

#[tauri::command]
pub async fn term_resize(
    terms: State<'_, TermRegistry>,
    term_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), AppErrorDto> {
    let writer = terms
        .lookup_writer(&term_id)
        .ok_or_else(|| AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "Unknown terminal", false))?;
    writer
        .send(WriterMsg::Resize { cols, rows })
        .await
        .map_err(|_| ProtoError::Internal("terminal writer closed".into()).to_app_error())
}

/// Detaches this tab's stream. The host keeps the PTY (and its shell) alive;
/// `term_kill` is what ends it. Idempotent: the reader task removes its own
/// entry (by token) when the stream ends naturally.
#[tauri::command]
pub async fn term_close(
    terms: State<'_, TermRegistry>,
    term_id: String,
) -> Result<(), AppErrorDto> {
    if let Some(handle) = terms.remove(&term_id) {
        close_handle(handle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_event_serde_shape() {
        let title = TermEvent::Title {
            title: "uni".into(),
        };
        assert_eq!(
            serde_json::to_value(&title).unwrap(),
            serde_json::json!({"type":"title","title":"uni"})
        );
        let exit = TermEvent::Exit { code: Some(0) };
        assert_eq!(
            serde_json::to_value(&exit).unwrap(),
            serde_json::json!({"type":"exit","code":0})
        );
        let exit_null = TermEvent::Exit { code: None };
        assert_eq!(
            serde_json::to_value(&exit_null).unwrap(),
            serde_json::json!({"type":"exit","code":null})
        );
        let closed = TermEvent::Closed { error: None };
        assert_eq!(
            serde_json::to_value(&closed).unwrap(),
            serde_json::json!({"type":"closed"})
        );
        let closed_err = TermEvent::Closed {
            error: Some(AppErrorDto::new(AppErrorCode::E_TRANSPORT, "lost", true)),
        };
        let value = serde_json::to_value(&closed_err).unwrap();
        assert_eq!(value["type"], "closed");
        assert_eq!(value["error"]["code"], "E_TRANSPORT");
    }

    #[test]
    fn registry_register_lookup_remove() {
        let registry = TermRegistry::new();
        let (tx, _rx) = mpsc::channel::<WriterMsg>(4);
        registry.register(
            "abc".into(),
            TermHandle {
                writer: tx.clone(),
                reader: None,
                token: registry.next_token(),
            },
        );
        assert!(registry.lookup_writer("abc").is_some());
        assert!(registry.remove("abc").is_some());
        assert!(registry.lookup_writer("abc").is_none());
        assert!(registry.remove("abc").is_none(), "remove is idempotent");
        drop(tx);
    }

    #[test]
    fn register_replaces_the_previous_stream_and_closes_it() {
        let registry = TermRegistry::new();
        let (tx1, mut rx1) = mpsc::channel::<WriterMsg>(4);
        let (tx2, mut rx2) = mpsc::channel::<WriterMsg>(4);
        registry.register(
            "abc".into(),
            TermHandle {
                writer: tx1,
                reader: None,
                token: registry.next_token(),
            },
        );
        registry.register(
            "abc".into(),
            TermHandle {
                writer: tx2.clone(),
                reader: None,
                token: registry.next_token(),
            },
        );
        // The replaced stream is told to detach (graceful half-close)…
        assert!(
            matches!(rx1.try_recv(), Ok(WriterMsg::Close)),
            "the previous stream must be closed when its id is re-registered"
        );
        // …and the new one answers input while the old one is unreachable.
        registry
            .lookup_writer("abc")
            .expect("the new stream is registered")
            .try_send(WriterMsg::Input(b"x".into()))
            .expect("send");
        assert!(matches!(rx2.try_recv(), Ok(WriterMsg::Input(_))));
        drop(tx2);
    }

    #[test]
    fn remove_own_only_removes_the_stream_with_the_matching_token() {
        let registry = TermRegistry::new();
        let (tx1, _rx1) = mpsc::channel::<WriterMsg>(4);
        let (tx2, _rx2) = mpsc::channel::<WriterMsg>(4);
        let token1 = registry.next_token();
        registry.register(
            "t".into(),
            TermHandle {
                writer: tx1,
                reader: None,
                token: token1,
            },
        );
        let token2 = registry.next_token();
        registry.register(
            "t".into(),
            TermHandle {
                writer: tx2,
                reader: None,
                token: token2,
            },
        );
        // A stale generation ending late removes nothing.
        assert!(registry.remove_own("t", token1).is_none());
        assert!(
            registry.lookup_writer("t").is_some(),
            "a late-ending stream must not remove its replacement's entry"
        );
        // The current generation's end does remove it (exactly once).
        assert!(registry.remove_own("t", token2).is_some());
        assert!(registry.lookup_writer("t").is_none());
        assert!(registry.remove_own("t", token2).is_none(), "idempotent");
    }

    #[test]
    fn frame_to_event_mapping() {
        // kind 1 -> Data
        assert!(matches!(
            decode_frame(1, b"hi"),
            Ok(Decoded::Data(bytes)) if bytes == b"hi"
        ));
        // title control -> Title
        let title = serde_json::to_vec(&Control::Title {
            title: "t".to_owned().try_into().unwrap(),
        })
        .unwrap();
        assert!(matches!(
            decode_frame(0, &title),
            Ok(Decoded::Event(TermEvent::Title { .. }))
        ));
        // exit control -> Exit
        let exit = serde_json::to_vec(&Control::Exit { code: Some(3) }).unwrap();
        assert!(matches!(
            decode_frame(0, &exit),
            Ok(Decoded::Event(TermEvent::Exit { code: Some(3) }))
        ));
        // host err envelope -> closed with that error
        let err = serde_json::to_vec(&Control::Err {
            error: AppErrorDto::new(AppErrorCode::E_TRANSPORT, "lost", true),
        })
        .unwrap();
        match decode_frame(0, &err) {
            Ok(Decoded::Event(TermEvent::Closed { error: Some(error) })) => {
                assert_eq!(error.code, AppErrorCode::E_TRANSPORT);
            }
            other => panic!("expected closed, got {other:?}"),
        }
        // bad control payload -> closed with E_SCHEMA
        match decode_frame(0, b"junk") {
            Err(Decoded::Event(TermEvent::Closed { error: Some(error) })) => {
                assert_eq!(error.code, AppErrorCode::E_SCHEMA);
            }
            other => panic!("expected schema closed, got {other:?}"),
        }
        // bad kind -> closed with E_SCHEMA
        match decode_frame(7, b"{}") {
            Err(Decoded::Event(TermEvent::Closed { error: Some(error) })) => {
                assert_eq!(error.code, AppErrorCode::E_SCHEMA);
            }
            other => panic!("expected schema closed, got {other:?}"),
        }
        // stray ok is ignored
        let ok = serde_json::to_vec(&Control::Ok { data: None }).unwrap();
        assert!(matches!(decode_frame(0, &ok), Ok(Decoded::Ignore)));
    }

    #[cfg(test)]
    fn event_channel(sink: &std::sync::Arc<Mutex<Vec<TermEvent>>>) -> Channel<TermEvent> {
        let sink = std::sync::Arc::clone(sink);
        Channel::new(move |body: InvokeResponseBody| {
            if let InvokeResponseBody::Json(json) = body {
                if let Ok(event) = serde_json::from_str::<TermEvent>(&json) {
                    sink.lock().unwrap_or_else(|e| e.into_inner()).push(event);
                }
            }
            Ok(())
        })
    }

    /// The reported race, end to end: a webview reload re-opens the same
    /// terminal (a new stream under the same id) while the *old* stream's
    /// reader ends late. The old stream must not remove the new stream's
    /// registry entry — input keeps flowing — and only the new stream's own
    /// end removes the entry.
    #[cfg(all(unix, test))]
    #[tokio::test]
    async fn a_late_ending_stream_never_removes_its_replacement() {
        use tokio::io::AsyncWriteExt;

        let registry = TermRegistry::new();
        let events: std::sync::Arc<Mutex<Vec<TermEvent>>> = Default::default();

        // Stream 1 — the tab's stream from before the reload.
        let (client1, host1) = tokio::net::UnixStream::pair().expect("socket pair");
        let (recv1, _client1_write) = client1.into_split();
        let (tx1, mut rx1) = mpsc::channel::<WriterMsg>(4);
        let token1 = registry.next_token();
        let reader1 = tokio::spawn(pump_reader(
            crate::op_stream::OpRecv::Local(recv1),
            Channel::<InvokeResponseBody>::new(|_| Ok(())),
            event_channel(&events),
            registry.clone(),
            "t".into(),
            token1,
        ));
        registry.register(
            "t".into(),
            TermHandle {
                writer: tx1,
                reader: None,
                token: token1,
            },
        );

        // The reload: `term_open` again registers a new stream for the id…
        let (client2, mut host2) = tokio::net::UnixStream::pair().expect("socket pair");
        let (recv2, _client2_write) = client2.into_split();
        let (tx2, mut rx2) = mpsc::channel::<WriterMsg>(4);
        let token2 = registry.next_token();
        let reader2 = tokio::spawn(pump_reader(
            crate::op_stream::OpRecv::Local(recv2),
            Channel::<InvokeResponseBody>::new(|_| Ok(())),
            event_channel(&events),
            registry.clone(),
            "t".into(),
            token2,
        ));
        registry.register(
            "t".into(),
            TermHandle {
                writer: tx2,
                reader: None,
                token: token2,
            },
        );

        // …closing the old one first: its writer is told to detach.
        assert!(
            matches!(rx1.try_recv(), Ok(WriterMsg::Close)),
            "re-registering an id must close the stream it replaces"
        );

        // The old stream ends late (its host side drops: EOF). Its reader
        // runs to completion — and must leave the NEW stream's entry alone.
        drop(host1);
        reader1.await.expect("the old stream's reader ends");
        assert!(
            registry.lookup_writer("t").is_some(),
            "the old stream's late end must not remove the new stream's entry"
        );

        // Input still flows through the new stream.
        registry
            .lookup_writer("t")
            .expect("the new stream survived")
            .send(WriterMsg::Input(b"k".into()))
            .await
            .expect("send");
        assert!(matches!(rx2.recv().await, Some(WriterMsg::Input(_))));

        // The new stream ends naturally (the host sends `exit`): only now
        // does the entry go away, and each stream sent its terminal event.
        let exit = serde_json::to_vec(&Control::Exit { code: Some(0) }).expect("json");
        host2
            .write_all(&encode(FrameKind::Control, &exit))
            .await
            .expect("write exit");
        reader2.await.expect("the new stream's reader ends");
        assert!(registry.lookup_writer("t").is_none());
        let seen = events.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(seen.len(), 2, "one terminal event per stream: {seen:?}");
        assert!(matches!(seen[0], TermEvent::Closed { .. }));
        assert!(matches!(seen[1], TermEvent::Exit { code: Some(0) }));
    }
}
