//! UNI-266: `unipi-host pair --json`, the real binary against an in-process
//! host (endpoint + handler + control socket) in a temp data dir.
//! Covers: serve not running → one error line + exit 1; the waiting line
//! (link/code/expires_at/host) → a phone pairs with the link's code →
//! `{"state":"paired","device":…}` + exit 0; `--ttl 1s` → expired; closing
//! stdin cancels and removes the code.
#![cfg(unix)]
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iroh::{
    endpoint::{presets, RelayMode},
    Endpoint,
};
use unipi_host::{
    data_dir::HostPaths,
    endpoint::{EndpointCache, ALPN},
    pairing::PairingStore,
    protocol::{read_frame, Handler},
    push::{ControlContext, PushHub},
};
use unipi_proto::Control;

fn hello(req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    )
}

struct Host {
    _temp: tempfile::TempDir,
    paths: HostPaths,
    endpoint: Endpoint,
    _router: iroh::protocol::Router,
    _control: tokio::task::JoinHandle<()>,
}

async fn host() -> Host {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    let endpoint = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    unipi_host::data_dir::write_json(
        &paths.endpoint(),
        &EndpointCache::from_addr(endpoint.addr()),
    )
    .unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: endpoint.id(),
        host_name: "pair-test-pc".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    let control = unipi_host::push::serve_control(ControlContext {
        paths: paths.clone(),
        hub: PushHub::shared(&paths),
        host_name: "pair-test-pc".into(),
        notify: false,
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    })
    .await
    .unwrap();
    let router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(ALPN, handler)
        .spawn();
    Host {
        _temp: temp,
        paths,
        endpoint,
        _router: router,
        _control: control,
    }
}

fn spawn(dir: &std::path::Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_unipi-host"))
        .arg("--data-dir")
        .arg(dir)
        .args(["pair", "--json"])
        .args(args)
        .env_remove("UNIPI_HOST_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Reads JSON lines on a thread (the child's stdout is blocking).
fn lines(child: &mut Child) -> tokio::sync::mpsc::UnboundedReceiver<serde_json::Value> {
    let out = child.stdout.take().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(
                serde_json::from_str::<serde_json::Value>(&line)
                    .unwrap_or(serde_json::json!({"raw": line})),
            );
        }
    });
    rx
}

async fn next(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("a line in time")
        .expect("a line")
}

async fn exit_code(mut child: Child) -> i32 {
    tokio::task::spawn_blocking(move || child.wait().unwrap().code().unwrap_or(-1))
        .await
        .unwrap()
}

#[tokio::test]
async fn error_line_when_serve_is_not_running() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = spawn(temp.path(), &[]);
    let mut rx = lines(&mut child);
    let line = next(&mut rx).await;
    assert_eq!(line["state"], "error");
    assert!(
        line["message"]
            .as_str()
            .unwrap()
            .contains("start unipi-host serve first"),
        "{line}"
    );
    assert!(
        line["hint"].as_str().unwrap().contains("unipi-host"),
        "{line}"
    );
    assert_eq!(exit_code(child).await, 1);
    // endpoint.json left behind by a serve that has stopped: still "not running".
    let paths = HostPaths::from_dir(temp.path());
    let ep = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    unipi_host::data_dir::write_json(&paths.endpoint(), &EndpointCache::from_addr(ep.addr()))
        .unwrap();
    let mut child = spawn(temp.path(), &[]);
    let mut rx = lines(&mut child);
    assert_eq!(next(&mut rx).await["state"], "error");
    assert_eq!(exit_code(child).await, 1);
    assert!(
        PairingStore::new(paths.pairing())
            .current()
            .unwrap()
            .is_none(),
        "no code was started"
    );
}

#[tokio::test]
async fn waiting_then_paired() {
    let host = host().await;
    let mut child = spawn(&host.paths.root, &["--ttl", "10m"]);
    let mut rx = lines(&mut child);
    let first = next(&mut rx).await;
    assert_eq!(first["state"], "waiting", "{first}");
    assert!(!first["host"].as_str().unwrap().is_empty());
    let link = first["link"].as_str().unwrap();
    assert!(link.starts_with("unipi://pair?"), "{link}");
    let code = first["code"].as_str().unwrap();
    let now = unipi_host::pairing::now_unix();
    let expires = first["expires_at"].as_u64().unwrap();
    assert!(
        expires > now + 590 && expires <= now + 600,
        "{expires} vs {now}"
    );
    // A phone scans the QR: the link's ticket + code.
    let url = url::Url::parse(link).unwrap();
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["c"], code);
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.endpoint.addr(), ALPN).await.unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(
        serde_json::json!({"op":"pair","code":q["c"],"name":"Pixel 9"}),
    ))
    .await
    .unwrap();
    send.finish().unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    assert!(matches!(
        serde_json::from_slice::<Control>(&bytes).unwrap(),
        Control::Ok { .. }
    ));
    let second = next(&mut rx).await;
    assert_eq!(
        second,
        serde_json::json!({"state":"paired","device":"Pixel 9"})
    );
    assert_eq!(exit_code(child).await, 0);
}

#[tokio::test]
async fn expires_and_cancels() {
    let host = host().await;
    let mut child = spawn(&host.paths.root, &["--ttl", "1s"]);
    let mut rx = lines(&mut child);
    assert_eq!(next(&mut rx).await["state"], "waiting");
    assert_eq!(
        next(&mut rx).await,
        serde_json::json!({"state":"expired","reason":"timeout"})
    );
    assert_eq!(exit_code(child).await, 0);

    // Closing stdin (the desktop app's Cancel) ends it and the code stops working.
    let mut child = spawn(&host.paths.root, &[]);
    let mut rx = lines(&mut child);
    assert_eq!(next(&mut rx).await["state"], "waiting");
    assert!(PairingStore::new(host.paths.pairing())
        .current()
        .unwrap()
        .is_some());
    drop(child.stdin.take());
    assert_eq!(
        next(&mut rx).await,
        serde_json::json!({"state":"expired","reason":"cancelled"})
    );
    assert_eq!(exit_code(child).await, 0);
    assert!(PairingStore::new(host.paths.pairing())
        .current()
        .unwrap()
        .is_none());

    // A second `pair` replaces the first one's code.
    let mut a = spawn(&host.paths.root, &[]);
    let mut ra = lines(&mut a);
    assert_eq!(next(&mut ra).await["state"], "waiting");
    let mut b = spawn(&host.paths.root, &[]);
    let mut rb = lines(&mut b);
    assert_eq!(next(&mut rb).await["state"], "waiting");
    assert_eq!(
        next(&mut ra).await,
        serde_json::json!({"state":"expired","reason":"replaced"})
    );
    assert_eq!(exit_code(a).await, 0);
    drop(b.stdin.take());
    assert_eq!(next(&mut rb).await["state"], "expired");
    assert_eq!(exit_code(b).await, 0);
}
