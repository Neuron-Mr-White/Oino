//! Language servers (app spec §3.12, WP-A21): the servers live in the
//! **host**, this module keeps one stream per attached editor session —
//! same one-bistream-per-op seam as `term.rs`.
//!
//! - `lsp_list` is a one-shot `lsp_servers` request: `{root, servers[]}`
//!   for Settings › Language servers (and the editor's root check).
//! - `lsp_open` sends `lsp_open` and replies `ok{data:LspAttachData}`
//!   (`{server, attached, initialize_result}`). From then on server→app
//!   bytes pass through verbatim as kind-1 data on `on_data`, client→
//!   server bytes go via `lsp_write` (kind-2 input). `E_LSP_TAKEN`
//!   arrives as `taken`, a server exit as `exit`.
//! - `lsp_close` only *detaches*: the host keeps the server warm for
//!   `lsp.idle_minutes`; the next `lsp_open` attaches to it again.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, AppErrorDto, Control, FsPath, LspAttachData, LspLanguage, ProtoError, Request,
    PROTO_VERSION,
};

use crate::{net::NetState, retry::retry_once, term::WriterMsg};

/// Events pushed to the UI while an LSP stream is live. Exactly one of
/// `taken`/`exit`/`closed` is sent last.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LspEvent {
    /// Another device took the attach (`E_LSP_TAKEN`): the editor falls
    /// back to the LSP-off path; re-attaching takes it back.
    Taken { code: String, message: String },
    /// The language server process exited (`Exit{code}`).
    Exit { code: Option<i64> },
    /// The stream ended for any other reason (host gone, transport).
    Closed {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        error: Option<AppErrorDto>,
    },
}

/// The `lsp_open` reply the app's `LspStream.open()` returns (`lib/lsp`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspOpenResult {
    pub server: String,
    pub attached: bool,
    pub initialize_result: serde_json::Value,
}

struct LspStreams {
    send: crate::op_stream::OpSend,
    recv: crate::op_stream::OpRecv,
    data: LspAttachData,
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

fn host_ticket(dir: &std::path::Path, host_id: &str) -> Result<String, AppErrorDto> {
    let host = crate::local_host::hosts(dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    Ok(host.ticket)
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn schema(message: impl Into<String>) -> AppErrorDto {
    ProtoError::Internal(message.into()).to_app_error()
}

/// One `hello` carrying `lsp_open`, then the reply frame.
async fn open_lsp_streams(
    state: &NetState,
    dir: &std::path::Path,
    ticket: &str,
    relay: Option<&str>,
    request: Request,
) -> Result<LspStreams, AppErrorDto> {
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
                return Err(ProtoError::Internal("Invalid LSP reply frame".into()).to_app_error());
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
                    let data: LspAttachData = serde_json::from_value(payload)
                        .map_err(|e| schema(format!("bad lsp_open reply: {e}")))?;
                    Ok(LspStreams { send, recv, data })
                }
                Control::Err { error } => Err(error),
                _ => Err(ProtoError::Internal("Unexpected lsp_open reply".into()).to_app_error()),
            }
        }
    })
    .await
}

/// Lists the language servers the host knows for a path (`lsp_servers`).
#[tauri::command]
pub async fn lsp_list(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
    path: String,
    outside: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(&app)?;
    let ticket = host_ticket(&dir, &host_id)?;
    let (mut send, mut recv) = state
        .open_op(&dir, relay.as_deref().filter(|s| *s != "n0"), &ticket)
        .await?;
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
        req: Request::LspServers {
            path: FsPath::try_from(path).map_err(schema)?,
            outside,
        },
    };
    let bytes = serde_json::to_vec(&hello)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    send.write_all(&encode(FrameKind::Control, &bytes))
        .await
        .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
    let mut kind_buffer = [0u8; 1];
    let mut len_buffer = [0u8; 4];
    recv.read_exact(&mut kind_buffer).await.map_err(transport)?;
    recv.read_exact(&mut len_buffer).await.map_err(transport)?;
    let len = u32::from_be_bytes(len_buffer) as usize;
    if kind_buffer[0] != 0 || len > unipi_proto::MAX_FRAME {
        return Err(schema("invalid lsp_servers reply frame"));
    }
    let mut payload = vec![0u8; len];
    recv.read_exact(&mut payload).await.map_err(transport)?;
    match serde_json::from_slice::<Control>(&payload).map_err(|e| schema(e.to_string()))? {
        Control::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected lsp_servers reply")),
    }
}

/// Attaches this renderer's `lsp_id` to the host's `(root, language)`
/// server — spawning it on demand. The reply carries the cached
/// `initialize_result` when another client initialized it already
/// (§3.12: the editor can skip its own initialize round-trip then).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn lsp_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    streams: State<'_, LspRegistry>,
    host_id: String,
    relay: Option<String>,
    lsp_id: String,
    root: String,
    language: String,
    outside: Option<bool>,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<LspEvent>,
) -> Result<LspOpenResult, AppErrorDto> {
    let dir = directory(&app)?;
    let ticket = host_ticket(&dir, &host_id)?;
    let request = Request::LspOpen {
        root: FsPath::try_from(root).map_err(schema)?,
        language: LspLanguage::try_from(language).map_err(schema)?,
        outside,
        command: None,
    };
    let opened = open_lsp_streams(
        &state,
        &dir,
        &ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        request,
    )
    .await?;

    // Writer task: owns the send half; client LSP bytes go verbatim.
    let (writer_tx, mut writer_rx) = mpsc::channel::<WriterMsg>(64);
    let mut send = opened.send;
    tokio::spawn(async move {
        while let Some(message) = writer_rx.recv().await {
            let result = match message {
                WriterMsg::Input(bytes) => send.write_all(&encode(FrameKind::Input, &bytes)).await,
                WriterMsg::Resize { .. } => continue, // meaningless on LSP
                WriterMsg::Close => {
                    // Detach: the host keeps the server warm.
                    let _ = send.finish();
                    break;
                }
            };
            if result.is_err() {
                break;
            }
        }
    });

    // Reader task: kind-1 data to `on_data`, controls to `on_event`;
    // exactly one final event (taken / exit / closed) ends it.
    let mut recv = opened.recv;
    let registry = std::sync::Arc::clone(&streams.sessions);
    let reader_lsp_id = lsp_id.clone();
    tokio::spawn(async move {
        let mut ended = false;
        while !ended {
            let mut kind_buffer = [0u8; 1];
            let mut len_buffer = [0u8; 4];
            if recv.read_exact(&mut kind_buffer).await.is_err()
                || recv.read_exact(&mut len_buffer).await.is_err()
            {
                let _ = on_event.send(LspEvent::Closed { error: None });
                break;
            }
            let len = u32::from_be_bytes(len_buffer) as usize;
            if len > unipi_proto::MAX_FRAME {
                let _ = on_event.send(LspEvent::Closed {
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
                let _ = on_event.send(LspEvent::Closed { error: None });
                break;
            }
            match decode_frame(kind_buffer[0], &payload) {
                Ok(Decoded::Data(bytes)) => {
                    if on_data.send(InvokeResponseBody::Raw(bytes)).is_err() {
                        break;
                    }
                }
                Ok(Decoded::Event(event)) => {
                    let is_final = matches!(
                        event,
                        LspEvent::Taken { .. } | LspEvent::Exit { .. } | LspEvent::Closed { .. }
                    );
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
                    let _ = on_event.send(LspEvent::Closed { error: None });
                    ended = true;
                }
            }
        }
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&reader_lsp_id);
    });

    streams.insert(
        lsp_id,
        crate::term::TermHandle {
            writer: writer_tx,
            reader: None,
            // LSP streams keep the pre-generation behavior (their registry
            // removes entries unconditionally); 0 is never issued by
            // [`crate::term::TermRegistry::next_token`].
            token: 0,
        },
    );

    Ok(LspOpenResult {
        server: opened.data.server,
        attached: opened.data.attached,
        initialize_result: opened
            .data
            .initialize_result
            .unwrap_or(serde_json::Value::Null),
    })
}

/// Client→server bytes (whole LSP messages or fragments — the host's
/// parser reassembles; §3.12 bytes pass through untouched).
#[tauri::command]
pub async fn lsp_write(
    streams: State<'_, LspRegistry>,
    lsp_id: String,
    data: Vec<u8>,
) -> Result<(), AppErrorDto> {
    let writer = streams
        .lookup_writer(&lsp_id)
        .ok_or_else(|| AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "Unknown LSP stream", false))?;
    writer
        .send(WriterMsg::Input(data))
        .await
        .map_err(|_| ProtoError::Internal("LSP writer closed".into()).to_app_error())
}

/// Detaches this attach. The host keeps the server (idle timeout);
/// `lsp_open` on the same `(root, language)` takes it back.
#[tauri::command]
pub async fn lsp_close(streams: State<'_, LspRegistry>, lsp_id: String) -> Result<(), AppErrorDto> {
    if let Some(handle) = streams.remove(&lsp_id) {
        let _ = handle.writer.send(WriterMsg::Close).await;
        if let Some(reader) = handle.reader {
            reader.abort();
        }
    }
    Ok(())
}

/// What a single wire frame means for the UI (unit-tested like term's).
#[derive(Debug, Clone)]
pub enum Decoded {
    Data(Vec<u8>),
    Event(LspEvent),
    Ignore,
}

pub fn decode_frame(kind: u8, payload: &[u8]) -> Result<Decoded, Decoded> {
    match kind {
        1 => Ok(Decoded::Data(payload.to_vec())),
        0 => {
            let control: Control = serde_json::from_slice(payload).map_err(|_| closed(payload))?;
            match control {
                Control::Exit { code } => Ok(Decoded::Event(LspEvent::Exit { code })),
                Control::Err { error } if error.code == AppErrorCode::E_LSP_TAKEN => {
                    Ok(Decoded::Event(LspEvent::Taken {
                        code: serde_json::to_value(error.code)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_else(|| "E_LSP_TAKEN".into()),
                        message: error.message.as_str().to_owned(),
                    }))
                }
                Control::Err { error } => {
                    Ok(Decoded::Event(LspEvent::Closed { error: Some(error) }))
                }
                Control::Ok { .. } => Ok(Decoded::Ignore),
                _ => Err(closed(payload)),
            }
        }
        _ => Err(closed(payload)),
    }
}

fn closed(payload: &[u8]) -> Decoded {
    let _ = payload;
    Decoded::Event(LspEvent::Closed {
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

/// Managed registry of the app's live LSP streams — one per attached
/// `lsp_id` (the host owns the servers). Shared with each reader task so
/// it can remove its own entry when the stream ends.
#[derive(Clone, Default)]
pub struct LspRegistry {
    pub(crate) sessions: std::sync::Arc<Mutex<HashMap<String, crate::term::TermHandle>>>,
}

impl LspRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, id: String, handle: crate::term::TermHandle) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, handle);
    }

    fn lookup_writer(&self, id: &str) -> Option<mpsc::Sender<WriterMsg>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|handle| handle.writer.clone())
    }

    fn remove(&self, id: &str) -> Option<crate::term::TermHandle> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_event_serde_shape() {
        assert_eq!(
            serde_json::to_value(LspEvent::Taken {
                code: "E_LSP_TAKEN".into(),
                message: "moved".into()
            })
            .unwrap(),
            serde_json::json!({"type":"taken","code":"E_LSP_TAKEN","message":"moved"})
        );
        assert_eq!(
            serde_json::to_value(LspEvent::Exit { code: Some(1) }).unwrap(),
            serde_json::json!({"type":"exit","code":1})
        );
    }

    #[test]
    fn decode_frame_maps_stream_controls() {
        assert!(matches!(decode_frame(1, b"raw").unwrap(), Decoded::Data(_)));
        let taken = serde_json::to_vec(&Control::Err {
            error: AppErrorDto::new(AppErrorCode::E_LSP_TAKEN, "moved", false),
        })
        .unwrap();
        assert!(matches!(
            decode_frame(0, &taken).unwrap(),
            Decoded::Event(LspEvent::Taken { .. })
        ));
        let gone = serde_json::to_vec(&Control::Err {
            error: AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "gone", false),
        })
        .unwrap();
        assert!(matches!(
            decode_frame(0, &gone).unwrap(),
            Decoded::Event(LspEvent::Closed { .. })
        ));
        let exit = serde_json::to_vec(&Control::Exit { code: Some(0) }).unwrap();
        assert!(matches!(
            decode_frame(0, &exit).unwrap(),
            Decoded::Event(LspEvent::Exit { .. })
        ));
        assert!(decode_frame(7, b"x").is_err());
    }
}
