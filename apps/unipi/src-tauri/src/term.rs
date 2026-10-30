//! Live terminal sessions (DESIGN.md §3.3, §5.4): attach/shell streams
//! with kind-1 data frames, JSON control frames, and a per-session
//! writer task. One `TermHandle` per open terminal, keyed by a random
//! hex id.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, AppErrorDto, AttachTarget, Control, ProtoError, Request, PROTO_VERSION,
};

use crate::{net::NetState, path_kind::PathKind, retry::retry_once};

/// Events pushed to the UI while a terminal is live. Exactly one of
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

/// What a single wire frame means for the UI. Data goes to the
/// terminal buffer; events go to the event channel; `Ignore` is an
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

/// Messages for the per-session writer task that owns the send half.
pub(crate) enum WriterMsg {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

pub(crate) struct TermHandle {
    pub(crate) writer: mpsc::Sender<WriterMsg>,
    /// Aborted on close; not cloneable, so `lookup` returns writer only.
    pub(crate) reader: Option<tokio::task::JoinHandle<()>>,
}

/// Managed registry of live terminal sessions. Shared (Arc) with each
/// reader task so the task can remove its own entry when the stream
/// ends for any reason.
#[derive(Clone, Default)]
pub struct TermRegistry {
    pub(crate) sessions: std::sync::Arc<Mutex<HashMap<String, TermHandle>>>,
}

impl TermRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, id: String, handle: TermHandle) {
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

    /// Removes and returns a handle. Idempotent: a missing id (the
    /// reader already removed it after a natural end) returns `None`.
    fn remove(&self, id: &str) -> Option<TermHandle> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
    }
}

fn random_term_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    format!("{:08x}{:08x}", nanos.wrapping_mul(0x9e37_79b9), pid)
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

#[derive(Debug, Clone, Serialize)]
pub struct TermOpenResult {
    #[serde(rename = "termId")]
    pub term_id: String,
    pub effective_size_policy: unipi_proto::SizePolicy,
    pub path: PathKind,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn term_open(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    terms: State<'_, TermRegistry>,
    host_id: String,
    relay: Option<String>,
    target: Option<AttachTarget>,
    cols: u16,
    rows: u16,
    size_policy: unipi_proto::SizePolicy,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<TermEvent>,
) -> Result<TermOpenResult, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let request = match &target {
        Some(target) => Request::Attach {
            target: target.clone(),
            cols: unipi_proto::Dimension::try_from(cols as u32)
                .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
            rows: unipi_proto::Dimension::try_from(rows as u32)
                .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
            size_policy,
        },
        None => Request::Shell {
            cols: unipi_proto::Dimension::try_from(cols as u32)
                .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
            rows: unipi_proto::Dimension::try_from(rows as u32)
                .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
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

    let term_id = random_term_id();

    // Writer task: owns the send half.
    let (writer_tx, mut writer_rx) = mpsc::channel::<WriterMsg>(64);
    let mut send = streams.send;
    let writer_task = tokio::spawn(async move {
        while let Some(message) = writer_rx.recv().await {
            let result = match message {
                WriterMsg::Input(bytes) => send.write_all(&encode(FrameKind::Input, &bytes)).await,
                WriterMsg::Resize { cols, rows } => {
                    let control = serde_json::to_vec(&Control::Resize {
                        cols: unipi_proto::Dimension::try_from(cols as u32).unwrap_or_default(),
                        rows: unipi_proto::Dimension::try_from(rows as u32).unwrap_or_default(),
                    })
                    .unwrap_or_default();
                    send.write_all(&encode(FrameKind::Control, &control)).await
                }
                WriterMsg::Close => {
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
    // terminal (exit/closed) event is sent last. Removes its own
    // registry entry when it ends for any reason.
    let mut recv = streams.recv;
    let registry = std::sync::Arc::clone(&terms.sessions);
    // registry: Arc<Mutex<HashMap<..>>>
    let reader_term_id = term_id.clone();
    let reader_task = tokio::spawn(async move {
        let send_closed = |on_event: &Channel<TermEvent>, error: Option<AppErrorDto>| {
            let on_event = on_event.clone();
            async move {
                let _ = on_event.send(TermEvent::Closed { error });
            }
        };
        let mut ended = false;
        while !ended {
            let mut kind_buffer = [0u8; 1];
            let mut len_buffer = [0u8; 4];
            if recv.read_exact(&mut kind_buffer).await.is_err() {
                send_closed(&on_event, None).await;
                break;
            }
            if recv.read_exact(&mut len_buffer).await.is_err() {
                send_closed(&on_event, None).await;
                break;
            }
            let len = u32::from_be_bytes(len_buffer) as usize;
            if len > unipi_proto::MAX_FRAME {
                send_closed(
                    &on_event,
                    Some(AppErrorDto::new(
                        AppErrorCode::E_SCHEMA,
                        "oversize frame",
                        false,
                    )),
                )
                .await;
                break;
            }
            let mut payload = vec![0u8; len];
            if recv.read_exact(&mut payload).await.is_err() {
                send_closed(&on_event, None).await;
                break;
            }
            match decode_frame(kind_buffer[0], &payload) {
                Ok(Decoded::Data(bytes)) => {
                    if on_data.send(InvokeResponseBody::Raw(bytes)).is_err() {
                        break;
                    }
                }
                Ok(Decoded::Event(event)) => {
                    let is_final =
                        matches!(event, TermEvent::Exit { .. } | TermEvent::Closed { .. });
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
                    send_closed(&on_event, None).await;
                    ended = true;
                }
            }
        }
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&reader_term_id);
    });

    terms.insert(
        term_id.clone(),
        TermHandle {
            writer: writer_tx,
            reader: Some(reader_task),
        },
    );
    drop(writer_task);

    Ok(TermOpenResult {
        term_id,
        effective_size_policy: streams.effective_size_policy,
        path,
    })
}

struct TermStreams {
    send: crate::op_stream::OpSend,
    recv: crate::op_stream::OpRecv,
    effective_size_policy: unipi_proto::SizePolicy,
}

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
            send.write_all(&unipi_proto::frame::encode(FrameKind::Control, &bytes))
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
                    let effective = payload
                        .get("effective_size_policy")
                        .and_then(|v| v.as_str())
                        .and_then(|v| {
                            serde_json::from_value(serde_json::Value::String(v.to_string())).ok()
                        })
                        .unwrap_or(unipi_proto::SizePolicy::FitPhone);
                    Ok(TermStreams {
                        send,
                        recv,
                        effective_size_policy: effective,
                    })
                }
                Control::Err { error } => Err(error),
                _ => Err(ProtoError::Internal("Unexpected attach reply".into()).to_app_error()),
            }
        }
    })
    .await
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

#[tauri::command]
pub async fn term_close(
    terms: State<'_, TermRegistry>,
    term_id: String,
) -> Result<(), AppErrorDto> {
    // Idempotent: the reader task removes its own entry when the
    // stream ends naturally, so an already-removed id is fine.
    if let Some(handle) = terms.remove(&term_id) {
        let _ = handle.writer.send(WriterMsg::Close).await;
        if let Some(reader) = handle.reader {
            reader.abort();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unipi_proto::SessionId;

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
    fn registry_insert_lookup_remove() {
        let registry = TermRegistry::new();
        let (tx, _rx) = mpsc::channel::<WriterMsg>(4);
        registry.insert(
            "abc".into(),
            TermHandle {
                writer: tx.clone(),
                reader: None,
            },
        );
        assert!(registry.lookup_writer("abc").is_some());
        assert!(registry.remove("abc").is_some());
        assert!(registry.lookup_writer("abc").is_none());
        assert!(registry.remove("abc").is_none());
        drop(tx);
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
        // silence unused import when SessionId unused in non-test builds
        let _ = SessionId::try_from("s".to_string());
    }
}
