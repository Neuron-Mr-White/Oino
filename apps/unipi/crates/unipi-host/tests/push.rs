//! UNI-214 end to end: the CLI side talks to a real control socket
//! (`<tmp host dir>/control.sock`), a phone endpoint talks to the host
//! `Handler` over real iroh streams — own temp identity + data dir, never
//! `~/.unipi`. Covers gating (off by default, per device), the pending
//! push lifecycle (queued → push_list long-poll → blob_get of the pushed
//! path → push_done → the CLI sees the outcome), the allow-list for
//! `blob_get`, and that a file changed after hashing is refused.
//!
//! `external_serve_push` (ignored) drives a separately started
//! `UNIPI_HOST_DIR=/tmp/pushhost unipi-host serve` (docs/m7/PUSH.md).
#![cfg(unix)]
use std::time::Duration;

use iroh::{
    endpoint::{presets, Connection, RelayMode},
    Endpoint,
};
use sha2::Digest;
use unipi_host::{
    data_dir::HostPaths,
    endpoint::ALPN,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, Handler},
    push::{ControlContext, ControlEvent, ControlRequest, PushHub},
};
use unipi_proto::{AppErrorCode, Control, PushListData, PushOutcome};

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
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(40), read_frame(&mut recv))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// `blob_get{path}`: the reply plus every output byte.
async fn blob_get(conn: &Connection, path: &str) -> (Control, Vec<u8>) {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(serde_json::json!({"op":"blob_get","path":path})))
        .await
        .unwrap();
    send.finish().unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    let reply: Control = serde_json::from_slice(&bytes).unwrap();
    let mut body = Vec::new();
    if matches!(reply, Control::Ok { .. }) {
        loop {
            let (kind, bytes) = read_frame(&mut recv).await.unwrap();
            if kind == 1 {
                body.extend(bytes);
            } else {
                break;
            }
        }
    }
    (reply, body)
}

fn list(reply: Control) -> PushListData {
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    serde_json::from_value(data).unwrap()
}

/// Also carves the temp dir out of the extra safe roots (UNI-220 makes
/// /tmp readable by default), so the "outside" file below is only reachable
/// as a push, like a file in a folder the owner never allowed.
fn write_config(paths: &HostPaths, mut value: serde_json::Value, deny: &std::path::Path) {
    value["fs"] = serde_json::json!({ "deny": [deny.to_string_lossy()] });
    std::fs::write(paths.config(), serde_json::to_vec(&value).unwrap()).unwrap();
}

fn sha(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The default `fs.deny` entry: the whole temp root, for tests whose files
/// only ever ride a push and make no scope assertions of their own.
fn deny_temp_root() -> std::path::PathBuf {
    std::env::temp_dir().canonicalize().unwrap()
}

/// The "outside" file's directory plus the `fs.deny` entry that keeps it out
/// of the phone's scope, wherever TMPDIR lives on this machine:
/// - TMPDIR outside `$HOME` (the usual `/tmp`): the temp dir itself, denied
///   wholesale — the carve-out applies because the temp root is an extra
///   safe root, not home;
/// - TMPDIR **inside** `$HOME` (e.g. `$HOME/.cache/…`): `fs.deny` cannot
///   hide anything there (`FsScope::allows` grants `$HOME` unconditionally,
///   deny carves extra roots only), so the file goes under `/var/tmp` (a
///   default extra safe root) with its own carve-out instead.
///
/// The push's whole premise — the file is unreadable before the push — is
/// re-verified against the real scope by the caller.
fn outside_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let home = unipi_host::fs::home_dir_opt().expect("the test host has a $HOME");
    let temp_root = std::env::temp_dir().canonicalize().unwrap();
    if !temp_root.starts_with(&home) {
        let dir = tempfile::tempdir().unwrap();
        return (dir, temp_root);
    }
    let dir = tempfile::Builder::new()
        .prefix("unipi-push-outside-")
        .tempdir_in("/var/tmp")
        .expect("/var/tmp exists and is writable (a default extra safe root)");
    let deny = dir.path().canonicalize().unwrap();
    (dir, deny)
}

struct Host {
    _temp: tempfile::TempDir,
    paths: HostPaths,
    _router: iroh::protocol::Router,
    _phone: Endpoint,
    conn: Connection,
    _control: tokio::task::JoinHandle<()>,
}

async fn setup() -> Host {
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
        host_name: "push-test-pc".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    let control = unipi_host::push::serve_control(ControlContext {
        paths: paths.clone(),
        hub: PushHub::shared(&paths),
        host_name: "push-test-pc".into(),
        notify: false,
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    })
    .await
    .unwrap();
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
    Host {
        _temp: temp,
        paths,
        _router: router,
        _phone: phone,
        conn,
        _control: control,
    }
}

async fn pair(conn: &Connection, paths: &HostPaths, name: &str) {
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    let reply = request(
        conn,
        serde_json::json!({"op":"pair","code":state.code,"name":name}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
}

/// The CLI side: sends one request, returns a reader of events.
async fn cli(paths: &HostPaths, request: ControlRequest) -> unipi_host::push::ControlLines {
    let (lines, write) = unipi_host::push::connect_control(paths, &request)
        .await
        .unwrap();
    // Keep the write half alive for the stream's lifetime (closing it = Ctrl+C).
    std::mem::forget(write);
    lines
}

async fn next_event(lines: &mut unipi_host::push::ControlLines) -> ControlEvent {
    let line = tokio::time::timeout(Duration::from_secs(20), lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .expect("event");
    serde_json::from_str(&line).unwrap()
}

fn push_request(path: &std::path::Path, bytes: &[u8], devices: Vec<String>) -> ControlRequest {
    ControlRequest::Push {
        path: path.to_string_lossy().into_owned(),
        name: None,
        note: Some("fresh build".into()),
        devices,
        sha256: sha(bytes),
        size: bytes.len() as u64,
        follow: true,
    }
}

#[tokio::test]
async fn push_lifecycle_cli_to_phone_and_back() {
    let host = setup().await;
    let conn = &host.conn;
    let paths = &host.paths;

    // Unpaired: refused like every op.
    let reply = request(conn, serde_json::json!({"op":"push_list"})).await;
    assert!(
        matches!(reply, Control::Err { ref error } if error.code == AppErrorCode::E_UNAUTHORIZED)
    );
    pair(conn, paths, "Pixel").await;

    // A file outside the phone's scope (wherever TMPDIR lives): only
    // reachable as a push.
    let (outside, deny) = outside_dir();
    let apk = outside.path().join("app.apk");
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&apk, &bytes).unwrap();
    let apk_str = apk.to_string_lossy().into_owned();

    // Off by default: the CLI is told how to enable it; the phone sees enabled:false (not an error).
    let mut lines = cli(paths, push_request(&apk, &bytes, vec![])).await;
    match next_event(&mut lines).await {
        ControlEvent::Error { message, hint } => {
            assert!(message.contains("off"), "{message}");
            assert!(hint.unwrap().contains("unipi-host push enable"));
        }
        other => panic!("{other:?}"),
    }
    let off = list(request(conn, serde_json::json!({"op":"push_list"})).await);
    assert!(!off.enabled && off.pushes.is_empty());
    assert_eq!(off.host_name, "push-test-pc");

    // Enable (read per request — no restart), keeping the file carved out.
    write_config(paths, serde_json::json!({"push":{"enabled":true}}), &deny);
    // Premise, verified against the real scope: the file is out of reach
    // except through a pending push.
    let scope = unipi_host::fs_scope::FsScope::load(&paths.config()).unwrap();
    assert!(
        !scope.allows(&apk.canonicalize().unwrap()),
        "the test premise needs a file outside the phone's fs scope \
         (home {}, file {})",
        scope.home.display(),
        apk.display()
    );

    // The phone long-polls before anything is pending…
    let poll = tokio::spawn({
        let conn = conn.clone();
        async move { request(&conn, serde_json::json!({"op":"push_list","wait":20})).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Before the push: the file is NOT readable by the phone.
    let (denied, _) = blob_get(conn, &apk_str).await;
    assert!(
        matches!(denied, Control::Err { ref error } if error.code == AppErrorCode::E_UNAUTHORIZED),
        "{denied:?}"
    );

    // …the CLI pushes; the phone is live, so no notification is attempted.
    let mut lines = cli(paths, push_request(&apk, &bytes, vec![])).await;
    let id = match next_event(&mut lines).await {
        ControlEvent::Queued {
            id,
            devices,
            notified,
        } => {
            assert_eq!(devices, vec!["Pixel".to_string()]);
            assert!(!notified);
            id
        }
        other => panic!("{other:?}"),
    };
    let woke = list(
        tokio::time::timeout(Duration::from_secs(5), poll)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(woke.enabled);
    assert_eq!(woke.pushes.len(), 1);
    let item = &woke.pushes[0];
    assert_eq!(item.id.as_str(), id);
    assert_eq!(
        (item.name.as_str(), item.size, item.mime.as_str()),
        (
            "app.apk",
            bytes.len() as u64,
            "application/vnd.android.package-archive"
        )
    );
    assert_eq!(item.sha256.as_str(), sha(&bytes));
    assert_eq!(item.note.as_deref(), Some("fresh build"));

    // The bytes ride blob_get{path}; the CLI hears "downloading".
    let (ok, body) = blob_get(conn, &item.path).await;
    assert!(matches!(ok, Control::Ok { .. }), "{ok:?}");
    assert_eq!(sha(&body), item.sha256.as_str());
    assert!(
        matches!(next_event(&mut lines).await, ControlEvent::Fetching { ref device, .. } if device == "Pixel")
    );

    // Disabling push mid-way closes the file again (gate re-read each time).
    write_config(paths, serde_json::json!({"push":{"enabled":false}}), &deny);
    let (closed, _) = blob_get(conn, &item.path).await;
    assert!(matches!(closed, Control::Err { .. }), "{closed:?}");
    let reply = request(
        conn,
        serde_json::json!({"op":"push_done","id":id,"outcome":"installed"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Err { ref error } if error.code == AppErrorCode::E_UNAUTHORIZED)
    );
    write_config(paths, serde_json::json!({"push":{"enabled":true}}), &deny);

    // The phone answers; the CLI prints the outcome and finishes.
    let reply = request(conn, serde_json::json!({"op":"push_done","id":id,"outcome":"installed","detail":"system installer opened"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    match next_event(&mut lines).await {
        ControlEvent::Done {
            outcome,
            device,
            detail,
            ..
        } => {
            assert_eq!(outcome, PushOutcome::Installed);
            assert_eq!(device, "Pixel");
            assert_eq!(detail.as_deref(), Some("system installer opened"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        next_event(&mut lines).await,
        ControlEvent::Finished { .. }
    ));

    // Done: no longer pending, the file is closed to the phone again.
    let after = list(request(conn, serde_json::json!({"op":"push_list"})).await);
    assert!(after.pushes.is_empty());
    let (closed, _) = blob_get(conn, &apk_str).await;
    assert!(matches!(closed, Control::Err { .. }));
    // A stale answer is harmless.
    let reply = request(
        conn,
        serde_json::json!({"op":"push_done","id":id,"outcome":"dismissed"}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }));
}

#[tokio::test]
async fn per_device_grant_device_filter_and_changed_file() {
    let host = setup().await;
    let paths = &host.paths;
    pair(&host.conn, paths, "Pixel").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.pdf");
    std::fs::write(&file, b"%PDF-1.7 tiny").unwrap();
    let bytes = std::fs::read(&file).unwrap();

    // Push is on, but only for a phone that isn't this one.
    write_config(
        paths,
        serde_json::json!({"push":{"enabled":true,"devices":["Redmi"]}}),
        &deny_temp_root(),
    );
    let off = list(request(&host.conn, serde_json::json!({"op":"push_list"})).await);
    assert!(!off.enabled, "not granted to Pixel");
    let mut lines = cli(paths, push_request(&file, &bytes, vec![])).await;
    assert!(
        matches!(next_event(&mut lines).await, ControlEvent::Error { ref message, .. } if message.contains("None of the paired phones"))
    );

    // Granted; --device that matches nobody is explained.
    write_config(
        paths,
        serde_json::json!({"push":{"enabled":true,"devices":["Pixel"]}}),
        &deny_temp_root(),
    );
    let mut lines = cli(paths, push_request(&file, &bytes, vec!["Galaxy".into()])).await;
    match next_event(&mut lines).await {
        ControlEvent::Error { message, hint } => {
            assert!(message.contains("Galaxy"));
            assert!(hint.unwrap().contains("Pixel"));
        }
        other => panic!("{other:?}"),
    }

    // The file changed after the CLI hashed it: refused.
    let mut stale = push_request(&file, &bytes, vec![]);
    if let ControlRequest::Push { sha256, .. } = &mut stale {
        *sha256 = "0".repeat(64);
    }
    let mut lines = cli(paths, stale).await;
    assert!(
        matches!(next_event(&mut lines).await, ControlEvent::Error { ref message, .. } if message.contains("changed"))
    );

    // --no-wait style (follow:false): queued, then the socket closes; list/cancel work.
    let mut quick = push_request(&file, &bytes, vec!["Pixel".into()]);
    if let ControlRequest::Push { follow, .. } = &mut quick {
        *follow = false;
    }
    let mut lines = cli(paths, quick).await;
    assert!(matches!(
        next_event(&mut lines).await,
        ControlEvent::Queued { .. }
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    let pending = list(request(&host.conn, serde_json::json!({"op":"push_list"})).await);
    assert_eq!(pending.pushes[0].mime, "application/pdf");
    let mut lines = cli(paths, ControlRequest::List).await;
    assert!(
        matches!(next_event(&mut lines).await, ControlEvent::Pending { ref pushes } if pushes.len() == 1)
    );
    let mut lines = cli(paths, ControlRequest::Cancel { id: None }).await;
    assert!(
        matches!(next_event(&mut lines).await, ControlEvent::Cancelled { ref ids } if ids.len() == 1)
    );
    assert!(
        list(request(&host.conn, serde_json::json!({"op":"push_list"})).await)
            .pushes
            .is_empty()
    );
}

#[tokio::test]
async fn second_serve_refuses_the_same_control_socket_and_stale_socket_is_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    let ctx = ControlContext {
        paths: paths.clone(),
        hub: PushHub::default(),
        host_name: "x".into(),
        notify: false,
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    // A stale socket file from a crashed serve is replaced.
    let stale =
        std::os::unix::net::UnixListener::bind(unipi_host::push::socket_path(&paths)).unwrap();
    drop(stale);
    let first = unipi_host::push::serve_control(ctx.clone()).await.unwrap();
    let second = unipi_host::push::serve_control(ctx).await;
    assert!(second.unwrap_err().to_string().contains("already"));
    first.abort();
    // CLI without a running serve: a clear message.
    let other = tempfile::tempdir().unwrap();
    let err = unipi_host::push::connect_control(
        &HostPaths::from_dir(other.path()),
        &ControlRequest::List,
    )
    .await
    .err()
    .expect("no serve: an error");
    assert!(err.to_string().contains("isn't running"), "{err}");
}

/// Against a separately started host (see docs/m7/PUSH.md):
/// `UNIPI_HOST_DIR=/tmp/pushhost unipi-host serve` with push enabled and
/// a pending `unipi-host push`, this pairs a throwaway phone identity and
/// drives list → fetch → done.
#[tokio::test]
#[ignore]
async fn external_serve_push() {
    let dir = std::env::var("PUSH_HOST_DIR").unwrap_or_else(|_| "/tmp/pushhost".into());
    let paths = HostPaths::from_dir(&dir);
    let cache: unipi_host::endpoint::EndpointCache =
        unipi_host::data_dir::read_json(&paths.endpoint())
            .unwrap()
            .expect("serve running");
    let phone = Endpoint::builder(presets::N0).bind().await.unwrap();
    let conn = phone.connect(cache.addr.clone(), ALPN).await.unwrap();
    pair(&conn, &paths, "external-test-phone").await;
    let data = list(request(&conn, serde_json::json!({"op":"push_list","wait":25})).await);
    assert!(
        data.enabled,
        "run `UNIPI_HOST_DIR={dir} unipi-host push enable`"
    );
    let item = data
        .pushes
        .first()
        .expect("run `unipi-host push <file>` within 25 s")
        .clone();
    let (ok, body) = blob_get(&conn, &item.path).await;
    assert!(matches!(ok, Control::Ok { .. }));
    assert_eq!(sha(&body), item.sha256.as_str());
    let reply = request(&conn, serde_json::json!({"op":"push_done","id":item.id.as_str(),"outcome":"saved","detail":"external test"})).await;
    assert!(matches!(reply, Control::Ok { .. }));
    println!("pushed {} ({} bytes) verified", item.name, body.len());
}
