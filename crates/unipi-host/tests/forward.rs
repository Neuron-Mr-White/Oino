//! UNI-213 end to end: a phone endpoint talks to a host `Handler` over
//! real iroh streams (own temp identity + data dir, never `~/.unipi`),
//! exercising `ports_list` and `tcp_forward` against real local servers:
//! a raw TCP echo, `python3 -m http.server` and a `websockets` echo.
//!
//! `external_serve_forward` (ignored) does the same against a separately
//! started `unipi-host serve` (see docs/m7/FORWARDING.md).
use std::time::Duration;

use iroh::{
    endpoint::{presets, Connection, RelayMode},
    Endpoint,
};
use tokio::io::AsyncReadExt;
use unipi_host::{
    data_dir::HostPaths,
    endpoint::ALPN,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, Handler},
};
use unipi_proto::{AppErrorCode, Control, PortsListData};

fn hello(req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    )
}

async fn request(conn: &Connection, req: serde_json::Value) -> Control {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(req)).await.unwrap();
    send.finish().unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn err_code(control: &Control) -> Option<AppErrorCode> {
    match control {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

/// Opens a `tcp_forward` stream; returns the reply and the raw stream halves.
async fn forward(
    conn: &Connection,
    port: u16,
    pipelined: &[u8],
) -> (
    Control,
    iroh::endpoint::SendStream,
    iroh::endpoint::RecvStream,
) {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(serde_json::json!({"op":"tcp_forward","port":port})))
        .await
        .unwrap();
    // Pipelined: raw bytes right behind the hello, before the ok arrives.
    if !pipelined.is_empty() {
        send.write_all(pipelined).await.unwrap();
    }
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut recv))
        .await
        .unwrap()
        .unwrap();
    (serde_json::from_slice(&bytes).unwrap(), send, recv)
}

fn write_config(paths: &HostPaths, value: serde_json::Value) {
    std::fs::write(paths.config(), serde_json::to_vec(&value).unwrap()).unwrap();
}

async fn pair(conn: &Connection, paths: &HostPaths) {
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    let reply = request(
        conn,
        serde_json::json!({"op":"pair","code":state.code,"name":"Test phone"}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn wait_listening(port: u16) -> bool {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Raw HTTP/1.0 GET over a forwarded stream (`Connection: close`).
async fn http_get(conn: &Connection, port: u16, path: &str) -> String {
    let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    let (reply, mut send, mut recv) = forward(conn, port, req.as_bytes()).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    send.finish().unwrap();
    let mut body = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        AsyncReadExt::read_to_end(&mut recv, &mut body),
    )
    .await
    .unwrap()
    .unwrap();
    String::from_utf8_lossy(&body).into_owned()
}

/// Minimal client-side websocket text frame (masked).
fn ws_text(payload: &[u8]) -> Vec<u8> {
    let mask = [1u8, 2, 3, 4];
    let mut frame = vec![0x81, 0x80 | payload.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    frame
}

async fn setup() -> (
    tempfile::TempDir,
    HostPaths,
    Endpoint,
    iroh::protocol::Router,
    Endpoint,
    Connection,
) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    let host = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: host.id(),
        host_name: "Forward test host".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    let router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    (temp, paths, host, router, phone, conn)
}

#[tokio::test]
async fn forwarding_gates_and_pipes_tcp_http_and_websocket() {
    let (_temp, paths, _host, _router, phone, conn) = setup().await;

    // Unpaired: refused before anything else.
    let reply = request(&conn, serde_json::json!({"op":"ports_list"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED));
    pair(&conn, &paths).await;

    // A raw echo server.
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = echo.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });

    // Off by default: both ops say so, with the enable hint.
    for req in [
        serde_json::json!({"op":"ports_list"}),
        serde_json::json!({"op":"tcp_forward","port":echo_port}),
    ] {
        let reply = request(&conn, req).await;
        assert_eq!(
            err_code(&reply),
            Some(AppErrorCode::E_FORWARD_DISABLED),
            "{reply:?}"
        );
        if let Control::Err { error } = reply {
            assert!(error.hint.unwrap().contains("unipi-host forward enable"));
        }
    }

    // Allow-list: only the echo port. Read fresh, no restart.
    write_config(
        &paths,
        serde_json::json!({"forwarding":{"enabled":true,"ports":[echo_port]}}),
    );
    let reply = request(&conn, serde_json::json!({"op":"ports_list"})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    let list: PortsListData = serde_json::from_value(data).unwrap();
    assert!(!list.any);
    assert_eq!(list.allowed, vec![echo_port]);
    let mine = list
        .ports
        .iter()
        .find(|p| p.port == echo_port)
        .expect("our listener is listed");
    assert!(mine.allowed);
    assert_eq!(mine.bind, "127.0.0.1");
    // Our own process owns it, so its name is readable.
    assert!(mine.pid.is_some() && mine.process.is_some(), "{mine:?}");

    // Not in the allow-list.
    let other = free_port();
    let (reply, _, _) = forward(&conn, other, b"").await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED));

    // Echo, pipelined bytes + more after ok, 1 MiB through, half-close.
    let (reply, mut send, mut recv) = forward(&conn, echo_port, b"hello ").await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    let big: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
    let writer = {
        let big = big.clone();
        tokio::spawn(async move {
            send.write_all(&big).await.unwrap();
            send.finish().unwrap();
        })
    };
    let mut back = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(20),
        AsyncReadExt::read_to_end(&mut recv, &mut back),
    )
    .await
    .unwrap()
    .unwrap();
    writer.await.unwrap();
    assert_eq!(&back[..6], b"hello ");
    assert_eq!(back.len(), 6 + big.len());
    assert!(back[6..] == big[..]);

    // Concurrent streams (a browser opens several connections).
    let mut tasks = Vec::new();
    for i in 0..8u8 {
        let conn = conn.clone();
        tasks.push(tokio::spawn(async move {
            let (reply, mut send, mut recv) = forward(&conn, echo_port, &[i; 3]).await;
            assert!(matches!(reply, Control::Ok { .. }));
            send.finish().unwrap();
            let mut got = Vec::new();
            AsyncReadExt::read_to_end(&mut recv, &mut got)
                .await
                .unwrap();
            assert_eq!(got, vec![i; 3]);
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    // "any loopback" + a closed port → E_NOT_FOUND.
    write_config(
        &paths,
        serde_json::json!({"forwarding":{"enabled":true,"ports":"any"}}),
    );
    let (reply, _, _) = forward(&conn, free_port(), b"").await;
    assert_eq!(
        err_code(&reply),
        Some(AppErrorCode::E_NOT_FOUND),
        "{reply:?}"
    );

    // Real HTTP: python3 -m http.server serving a temp dir.
    let www = tempfile::tempdir().unwrap();
    std::fs::write(www.path().join("index.html"), "<h1>forward-ok</h1>").unwrap();
    let http_port = free_port();
    let python = std::process::Command::new("python3")
        .args([
            "-m",
            "http.server",
            &http_port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(www.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(child) = python {
        let _guard = Child(child);
        assert!(wait_listening(http_port).await, "http.server did not start");
        let page = http_get(&conn, http_port, "/index.html").await;
        assert!(page.starts_with("HTTP/1.0 200"), "{page}");
        assert!(page.contains("<h1>forward-ok</h1>"), "{page}");
    } else {
        eprintln!("python3 missing: skipped http.server leg");
    }

    // Real WebSocket: python websockets echo (skipped if not installed).
    let ws_port = free_port();
    let script = format!(
        "import asyncio,websockets\nasync def e(ws):\n  async for m in ws: await ws.send('echo:'+m)\nasync def main():\n  async with websockets.serve(e,'127.0.0.1',{ws_port}): await asyncio.Future()\nasyncio.run(main())\n"
    );
    let has_ws = std::process::Command::new("python3")
        .args(["-c", "import websockets"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if has_ws {
        let _guard = Child(
            std::process::Command::new("python3")
                .args(["-c", &script])
                .spawn()
                .unwrap(),
        );
        assert!(wait_listening(ws_port).await, "ws echo did not start");
        let upgrade = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{ws_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        let (reply, mut send, mut recv) = forward(&conn, ws_port, upgrade.as_bytes()).await;
        assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            recv.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&head);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
        send.write_all(&ws_text(b"ping-1")).await.unwrap();
        let mut frame = [0u8; 2 + 11];
        tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut frame))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame[0], 0x81);
        assert_eq!(&frame[2..], b"echo:ping-1");
        // A second message on the same long-lived stream (HMR-style).
        send.write_all(&ws_text(b"ping-2")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut frame))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&frame[2..], b"echo:ping-2");
    } else {
        eprintln!("python websockets missing: skipped websocket leg");
    }

    // Per-device gate: a list that doesn't name this phone.
    write_config(
        &paths,
        serde_json::json!({"forwarding":{"enabled":true,"ports":"any","devices":["Someone else"]}}),
    );
    let reply = request(&conn, serde_json::json!({"op":"ports_list"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_FORWARD_DISABLED));
    write_config(
        &paths,
        serde_json::json!({"forwarding":{"enabled":true,"ports":"any","devices":["Test phone"]}}),
    );
    let reply = request(&conn, serde_json::json!({"op":"ports_list"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

    conn.close(0u32.into(), b"done");
    phone.close().await;
}

/// Against a separately started host (own identity/dir), e.g.:
/// `UNIPI_HOST_DIR=/tmp/fwdhost cargo run -p unipi-host -- serve`, then
/// `unipi-host --data-dir /tmp/fwdhost forward enable --any` and
/// `UNIPI_FWD_E2E_DIR=/tmp/fwdhost UNIPI_FWD_E2E_HTTP=8799 UNIPI_FWD_E2E_WS=8798
///  cargo test -p unipi-host --test forward -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn external_serve_forward() {
    let dir = std::env::var("UNIPI_FWD_E2E_DIR").expect("UNIPI_FWD_E2E_DIR");
    assert!(!dir.contains(".unipi/app-host"), "never the real host dir");
    let paths = HostPaths::from_dir(&dir);
    let cache: unipi_host::endpoint::EndpointCache =
        unipi_host::data_dir::read_json(&paths.endpoint())
            .unwrap()
            .expect("serve running?");
    let phone = Endpoint::builder(presets::N0).bind().await.unwrap();
    let conn = tokio::time::timeout(
        Duration::from_secs(20),
        phone.connect(cache.addr.clone(), ALPN),
    )
    .await
    .unwrap()
    .unwrap();
    pair(&conn, &paths).await;
    let reply = request(&conn, serde_json::json!({"op":"ports_list"})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    let list: PortsListData = serde_json::from_value(data).unwrap();
    eprintln!("ports_list: {}", serde_json::to_string(&list).unwrap());
    let http_port: u16 = std::env::var("UNIPI_FWD_E2E_HTTP")
        .unwrap_or("8799".into())
        .parse()
        .unwrap();
    assert!(
        list.ports.iter().any(|p| p.port == http_port && p.http),
        "http.server listed with http hint"
    );
    let page = http_get(&conn, http_port, "/").await;
    eprintln!("GET / → {}", page.lines().next().unwrap_or(""));
    assert!(page.starts_with("HTTP/1.0 200"));
    if let Ok(ws_port) = std::env::var("UNIPI_FWD_E2E_WS") {
        let ws_port: u16 = ws_port.parse().unwrap();
        let upgrade = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{ws_port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        let (reply, mut send, mut recv) = forward(&conn, ws_port, upgrade.as_bytes()).await;
        assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            recv.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"));
        send.write_all(&ws_text(b"hi")).await.unwrap();
        let mut frame = [0u8; 2 + 7];
        recv.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[2..], b"echo:hi");
        eprintln!("websocket echo ok");
    }
    conn.close(0u32.into(), b"done");
    phone.close().await;
}
