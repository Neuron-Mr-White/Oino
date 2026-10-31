//! `_oino/hello` capabilities and the v2 registry wire (§9.2, §10) over the real RPC server.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;

use oino_ai::JsonValue;
use oino_client::SessionBackend;
use oino_client::fake::FakeBackend;
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::ui::UiView;
use oino_rpc::RpcServer;
use oino_ui::slot::{RegistryOp, Scope, Slot, UiCapabilities, UiEntry, UiPatch};
use oino_ui::widgets::paragraph::Paragraph;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn entry() -> UiEntry {
    UiEntry {
        id: "probe/panel-1".into(),
        extension_id: "probe".into(),
        scope: Scope::Session,
        slot: Slot::SidePane,
        order: 0,
        seq: 1,
        title: Some("Probe".into()),
        icon: None,
        badge: None,
        tone: None,
        view: UiView::Tree {
            root: Paragraph::new("hi").into(),
        },
        keys: Vec::new(),
        focus: None,
        close_on_escape: None,
        size: None,
        rev: 1,
    }
}

fn patch_event() -> ClientEvent {
    ClientEvent::Oino(OinoEvent::from(UiPatch {
        scope: Scope::Session,
        session_id: Some("s-1".into()),
        rev: 1,
        ops: vec![RegistryOp::Put { entry: entry() }],
    }))
}

#[tokio::test]
async fn v2_hello_receives_registry_patches() {
    let backend = FakeBackend::new("s-1");
    let caps = serde_json::to_value(UiCapabilities::full()).unwrap();
    let hello = json!({"type": "_oino/hello", "client": {"name": "app"}, "ui": caps});
    let backend_for_emit = backend.clone();
    let input = format!("{hello}\n");
    let backend: Arc<dyn SessionBackend> = Arc::new(backend.clone());
    let (client_in, server_in) = tokio::io::duplex(64 * 1024);
    let (server_out, client_out) = tokio::io::duplex(64 * 1024);
    let server = RpcServer::new(backend);
    let handle = tokio::spawn(async move { server.run(server_in, server_out).await });
    let mut writer = client_in;
    writer.write_all(input.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    backend_for_emit.emit(patch_event());
    let mut reader = BufReader::new(client_out);
    let mut lines = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let mut line = String::new();
        match tokio::time::timeout_at(deadline, reader.read_line(&mut line)).await {
            Ok(Ok(0) | Err(_)) | Err(_) => break,
            Ok(Ok(_)) => {
                let value: JsonValue = serde_json::from_str(line.trim()).unwrap();
                let done = value.get("type").and_then(JsonValue::as_str) == Some("oino_ui_patch");
                lines.push(value);
                if done {
                    break;
                }
            }
        }
    }
    let patch = lines
        .iter()
        .find(|line| line.get("type").and_then(JsonValue::as_str) == Some("oino_ui_patch"))
        .expect("v2 client did not receive oino_ui_patch");
    assert_eq!(patch["rev"], 1);
    assert_eq!(patch["ops"][0]["op"], "put");
    assert_eq!(patch["ops"][0]["entry"]["id"], "probe/panel-1");
    handle.abort();
}

#[tokio::test]
async fn plain_clients_never_see_registry_patches() {
    let backend = FakeBackend::new("s-1");
    let backend_for_emit = backend.clone();
    let backend: Arc<dyn SessionBackend> = Arc::new(backend);
    let (_client_in, server_in) = tokio::io::duplex(64 * 1024);
    let (server_out, client_out) = tokio::io::duplex(64 * 1024);
    let server = RpcServer::new(backend);
    let handle = tokio::spawn(async move { server.run(server_in, server_out).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    backend_for_emit.emit(patch_event());
    backend_for_emit.emit(ClientEvent::Ui(oino_core::ui::UiRequest {
        id: "u1".into(),
        method: "select".into(),
        params: serde_json::from_value(json!({"title": "t"})).unwrap(),
        extension_id: Some("probe".into()),
        session_id: Some("s-1".into()),
    }));
    let mut reader = BufReader::new(client_out);
    let mut lines = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let mut line = String::new();
        match tokio::time::timeout_at(deadline, reader.read_line(&mut line)).await {
            Ok(Ok(0) | Err(_)) | Err(_) => break,
            Ok(Ok(_)) => {
                let value: JsonValue = serde_json::from_str(line.trim()).unwrap();
                let dialog =
                    value.get("type").and_then(JsonValue::as_str) == Some("extension_ui_request");
                lines.push(value);
                if dialog {
                    break;
                }
            }
        }
    }
    assert!(
        lines
            .iter()
            .all(|line| line.get("type").and_then(JsonValue::as_str) != Some("oino_ui_patch")),
        "a pi client saw a registry patch: {lines:?}"
    );
    let dialog = lines
        .iter()
        .find(|line| line.get("type").and_then(JsonValue::as_str) == Some("extension_ui_request"))
        .expect("pi client lost its dialog record");
    assert_eq!(dialog["method"], "select");
    assert!(dialog.get("extensionId").is_none());
    handle.abort();
}
