//! UNI-220 end to end over real iroh streams (own temp identity + data dir,
//! never `~/.unipi`): the host's extra safe roots (/tmp by default) show up
//! in `fs_roots`, open/read/write/upload without `outside`, are served to
//! `blob_get{path}` (file links), honour `fs deny` from `config.json` per
//! request, and system folders stay refused.
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
};
use unipi_proto::{AppErrorCode, Control};

#[allow(
    clippy::needless_pass_by_value,
    reason = "Owned JSON fixture input mirrors the request construction API."
)]
fn hello(req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    )
}

/// One request: the control reply plus any output bytes before `exit`.
async fn request(conn: &Connection, req: serde_json::Value) -> (Control, Vec<u8>) {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(req)).await.unwrap();
    send.finish().unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(20), read_frame(&mut recv))
        .await
        .unwrap()
        .unwrap();
    let reply: Control = serde_json::from_slice(&bytes).unwrap();
    let mut body = Vec::new();
    if matches!(reply, Control::Ok { .. }) {
        while let Ok(Ok((kind, bytes))) =
            tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv)).await
        {
            if kind == 1 {
                body.extend(bytes);
            } else {
                break;
            }
        }
    }
    (reply, body)
}

fn data(reply: &Control) -> serde_json::Value {
    match reply {
        Control::Ok { data } => data.clone().unwrap_or(serde_json::Value::Null),
        other => panic!("{other:?}"),
    }
}

fn err_code(reply: &Control) -> AppErrorCode {
    match reply {
        Control::Err { error } => error.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn extra_safe_roots_over_the_wire() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path().join("host"));
    paths.ensure_dir().unwrap();
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
        host_name: "fs-test-pc".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    let _router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"pair","code":state.code,"name":"Pixel"}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

    // A scratch folder under /tmp (the default extra root), outside $HOME.
    let tmp_root = std::path::Path::new("/tmp").canonicalize().unwrap();
    let scratch = tempfile::Builder::new()
        .prefix("uni220-")
        .tempdir_in(&tmp_root)
        .unwrap();
    let dir = scratch.path().canonicalize().unwrap();
    std::fs::write(dir.join("shot.txt"), "hello from /tmp").unwrap();
    let dir_s = dir.to_string_lossy().into_owned();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap())
        .canonicalize()
        .unwrap();
    assert!(!dir.starts_with(&home), "test needs /tmp outside $HOME");

    // fs_roots offers /tmp right after $HOME.
    let roots = data(&request(&conn, serde_json::json!({"op":"fs_roots"})).await.0);
    let roots: Vec<String> = serde_json::from_value(roots["roots"].clone()).unwrap();
    assert_eq!(roots[0], home.to_string_lossy());
    assert!(
        roots.contains(&tmp_root.to_string_lossy().into_owned()),
        "{roots:?}"
    );

    // List / read / write / mkdir / upload (fs_import) there, no `outside`.
    let listed = data(
        &request(&conn, serde_json::json!({"op":"fs_list","path":dir_s}))
            .await
            .0,
    );
    assert_eq!(listed["entries"][0]["name"], "shot.txt");
    let (reply, body) = request(
        &conn,
        serde_json::json!({"op":"fs_read","path":format!("{dir_s}/shot.txt")}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    assert_eq!(body, b"hello from /tmp");
    let (reply, _) = request(&conn, serde_json::json!({"op":"fs_write","path":format!("{dir_s}/new.md"),"text":"# hi","create":true})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    assert_eq!(std::fs::read_to_string(dir.join("new.md")).unwrap(), "# hi");
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"fs_mkdir","path":format!("{dir_s}/sub")}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    let bytes = b"uploaded from the phone";
    let sha: String = sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            use std::fmt::Write as _;
            write!(out, "{byte:02x}").unwrap();
            out
        });
    std::fs::create_dir_all(paths.blobs()).unwrap();
    std::fs::write(paths.blobs().join(&sha), bytes).unwrap();
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"fs_import","sha256":sha,"to":format!("{dir_s}/sub/up.bin")}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    assert_eq!(std::fs::read(dir.join("sub/up.bin")).unwrap(), bytes);

    // File links: blob_get{path} serves it too.
    let (reply, body) = request(
        &conn,
        serde_json::json!({"op":"blob_get","path":format!("{dir_s}/shot.txt")}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    assert_eq!(body, b"hello from /tmp");

    // A symlink in /tmp pointing at /etc doesn't escape.
    std::os::unix::fs::symlink("/etc", dir.join("etc")).unwrap();
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"fs_list","path":format!("{dir_s}/etc")}),
    )
    .await;
    assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE);
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"blob_get","path":format!("{dir_s}/etc/hostname")}),
    )
    .await;
    assert_eq!(err_code(&reply), AppErrorCode::E_UNAUTHORIZED);
    // System folders stay refused.
    for p in ["/etc", "/usr", "/"] {
        let (reply, _) = request(&conn, serde_json::json!({"op":"fs_list","path":p})).await;
        assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE, "{p}");
    }
    // The root itself can't be deleted.
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"fs_delete","path":tmp_root.to_string_lossy(),"confirm":true}),
    )
    .await;
    assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE);

    // `unipi-host fs deny /tmp` (config.json) applies on the next request.
    std::fs::write(
        paths.config(),
        serde_json::to_vec(&serde_json::json!({"fs":{"deny":[tmp_root]}})).unwrap(),
    )
    .unwrap();
    let (reply, _) = request(&conn, serde_json::json!({"op":"fs_list","path":dir_s})).await;
    assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE);
    // (fs_roots may still list /tmp here if a live tmux/herdr pane sits in it.)
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"blob_get","path":format!("{dir_s}/shot.txt")}),
    )
    .await;
    assert_eq!(err_code(&reply), AppErrorCode::E_UNAUTHORIZED);
}

/// G3-1: index discovery is not authorization for a paired phone. Exercise
/// the actual Iroh handler before and after installing the session service.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn indexed_folders_are_local_only_over_real_streams() {
    use oino_core::{
        extensions::api::ExtensionMode,
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
    };
    use std::{path::Path, sync::Arc};
    use unipi_host::sessions::{
        hub::{HostHubOptions, SessionHub},
        SessionService,
    };

    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path().join("host"));
    paths.ensure_dir().unwrap();
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
        host_name: "scope-regression".into(),
        lock: Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    let _router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler.clone())
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    let pairing = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    assert!(matches!(
        request(
            &conn,
            serde_json::json!({"op":"pair","code":pairing.code,"name":"scope-phone"})
        )
        .await
        .0,
        Control::Ok { .. }
    ));
    let read_hostname = serde_json::json!({"op":"fs_read","path":"/etc/hostname"});
    assert_eq!(
        err_code(&request(&conn, read_hostname.clone()).await.0),
        AppErrorCode::E_SCOPE
    );

    let sessions = temp.path().join("sessions");
    let agent = temp.path().join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    let fixture = sessions.join("fixture");
    std::fs::create_dir_all(&fixture).unwrap();
    let seed = |id: &str, cwd: &Path| {
        let path = fixture.join(format!("2026-11-18T00-00-00_{id}.jsonl"));
        std::fs::write(&path, format!("{}\n", serde_json::json!({"type":"session","version":3,"id":id,"timestamp":"2026-11-18T00:00:00.000Z","cwd":cwd}))).unwrap();
        path
    };
    seed("11111111-1111-4111-8111-111111111111", Path::new("/"));
    seed("22222222-2222-4222-8222-222222222222", Path::new("/etc"));
    // /tmp is already a default allowed root for the phone. Isolate a
    // valid outside-HOME project by using a HOME-only synthetic scope below;
    // wire-level isolation is additionally checked with a deny carve-out.
    let project = temp.path().join("project");
    let cwd = project.join("nested");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir(project.join(".git")).unwrap();
    seed("33333333-3333-4333-8333-333333333333", &cwd);
    // Existing /data is a valid root outside HOME/default roots on the QA
    // machine. Read-only fixture: never create or change anything there.
    let external = Path::new("/data");
    let external_present = external.is_dir();
    if external_present {
        seed("44444444-4444-4444-8444-444444444444", external);
    }
    let hub = SessionHub::create(HostHubOptions {
        cwd: temp.path().to_path_buf(),
        agent_dir: agent,
        docs_root: None,
        session_dir: Some(sessions),
        mode: ExtensionMode::Desktop,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        loader: Arc::new(StaticLoader(Vec::new())),
        models: None,
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: Some(temp.path().join("host")),
    })
    .unwrap();
    let service = SessionService::install(hub.clone());
    assert!(Arc::ptr_eq(service.hub(), &hub));
    assert_eq!(
        hub.list().await.unwrap().len(),
        3 + usize::from(external_present)
    );
    assert_eq!(
        err_code(&request(&conn, read_hostname).await.0),
        AppErrorCode::E_SCOPE
    );

    let roots = hub.local_session_roots().await.unwrap();
    assert!(roots.contains(&project.canonicalize().unwrap()));
    for forbidden in ["/", "/etc", "/usr", "/proc", "/root"] {
        assert!(
            !roots.contains(&Path::new(forbidden).to_path_buf()),
            "{roots:?}"
        );
    }
    let mut local = unipi_host::fs::FsScope::home_only(temp.path().join("home"));
    local.extra.extend(roots.iter().cloned());
    let remote = unipi_host::fs::FsScope::home_only(temp.path().join("home"));
    assert!(local.allows(&cwd));
    assert!(
        !remote.allows(&cwd),
        "unrelated indexed cwd must not authorize a phone"
    );

    // Cached requests do not list cards or discover/canonicalize folders again.
    let started = std::time::Instant::now();
    for _ in 0..10_000 {
        assert!(Arc::ptr_eq(
            &roots,
            &hub.local_session_roots().await.unwrap()
        ));
    }
    eprintln!(
        "G3-1 cached root lookup: 10000 iterations in {:?}",
        started.elapsed()
    );
    std::fs::remove_dir(project.join(".git")).unwrap();
    assert!(Arc::ptr_eq(
        &roots,
        &hub.local_session_roots().await.unwrap()
    ));

    // Real local transport, using the same authenticated Peer::Local handler.
    #[allow(
        clippy::items_after_statements,
        reason = "Scenario-local helpers stay next to their resource lifecycle and call sites."
    )]
    async fn local_request(handler: &Handler, req: serde_json::Value) -> Control {
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = tokio::io::duplex(65536);
        let handler = handler.clone();
        let task = tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(&mut server);
            handler
                .handle(&mut read, &mut write, unipi_host::protocol::Peer::Local)
                .await
                .unwrap();
        });
        client.write_all(&hello(req)).await.unwrap();
        client.shutdown().await.unwrap();
        let (_, bytes) = read_frame(&mut client).await.unwrap();
        let reply = serde_json::from_slice(&bytes).unwrap();
        drop(client);
        task.await.unwrap();
        reply
    }
    assert!(matches!(
        local_request(&handler, serde_json::json!({"op":"fs_list","path":cwd})).await,
        Control::Ok { .. }
    ));
    if external_present {
        let reply = request(&conn, serde_json::json!({"op":"fs_list","path":external}))
            .await
            .0;
        assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE);
        assert!(matches!(
            local_request(
                &handler,
                serde_json::json!({"op":"fs_list","path":external})
            )
            .await,
            Control::Ok { .. }
        ));
    }
    for forbidden in ["/", "/etc", "/usr", "/proc", "/root"] {
        let reply = local_request(
            &handler,
            serde_json::json!({"op":"fs_list","path":forbidden}),
        )
        .await;
        assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE, "{forbidden}");
    }
    // Live denies must carve out even cached session roots, for both peers.
    std::fs::write(
        paths.config(),
        serde_json::json!({"fs":{"deny":[project]}}).to_string(),
    )
    .unwrap();
    for reply in [
        request(&conn, serde_json::json!({"op":"fs_list","path":cwd}))
            .await
            .0,
        local_request(&handler, serde_json::json!({"op":"fs_list","path":cwd})).await,
    ] {
        assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE);
    }
    // Symlink access follows target scope, never the link's containing root.
    std::os::unix::fs::symlink("/etc", temp.path().join("escape")).unwrap();
    assert_eq!(
        err_code(
            &local_request(
                &handler,
                serde_json::json!({"op":"fs_read","path":temp.path().join("escape/hostname")})
            )
            .await
        ),
        AppErrorCode::E_SCOPE
    );
    std::os::unix::fs::symlink(&cwd, temp.path().join("denied-link")).unwrap();
    assert_eq!(
        err_code(
            &local_request(
                &handler,
                serde_json::json!({"op":"fs_list","path":temp.path().join("denied-link")})
            )
            .await
        ),
        AppErrorCode::E_SCOPE
    );
    std::fs::remove_file(
        fixture.join("2026-11-18T00-00-00_33333333-3333-4333-8333-333333333333.jsonl"),
    )
    .unwrap();
    hub.list().await.unwrap();
    assert!(!hub
        .local_session_roots()
        .await
        .unwrap()
        .contains(&project.canonicalize().unwrap()));
}

/// Against a separately started host:
/// `UNIPI_HOST_DIR=/tmp/qa-fs unipi-host serve` — pairs a throwaway phone
/// identity, then lists /tmp and uploads a file into `$FS_QA_DIR` (default
/// /tmp/qa-fs-files) with `fs_import`, no `outside`.
#[tokio::test]
#[ignore = "requires a separately started throwaway host; see scenario documentation"]
async fn external_serve_fs() {
    let dir = std::env::var("FS_HOST_DIR").unwrap_or_else(|_| "/tmp/qa-fs".into());
    let target = std::env::var("FS_QA_DIR").unwrap_or_else(|_| "/tmp/qa-fs-files".into());
    let paths = HostPaths::from_dir(&dir);
    let cache: unipi_host::endpoint::EndpointCache =
        unipi_host::data_dir::read_json(&paths.endpoint())
            .unwrap()
            .expect("serve running");
    let phone = Endpoint::builder(presets::N0).bind().await.unwrap();
    let conn = phone.connect(cache.addr.clone(), ALPN).await.unwrap();
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"pair","code":state.code,"name":"fs-qa-phone"}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    let roots = data(&request(&conn, serde_json::json!({"op":"fs_roots"})).await.0);
    println!("fs_roots: {roots}");
    let listed = data(
        &request(&conn, serde_json::json!({"op":"fs_list","path":"/tmp"}))
            .await
            .0,
    );
    println!(
        "/tmp: {} entries (more: {})",
        listed["entries"].as_array().unwrap().len(),
        listed["more"]
    );
    std::fs::create_dir_all(&target).unwrap();
    let bytes = b"uploaded by external_serve_fs";
    let sha: String = sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            use std::fmt::Write as _;
            write!(out, "{byte:02x}").unwrap();
            out
        });
    std::fs::create_dir_all(paths.blobs()).unwrap();
    std::fs::write(paths.blobs().join(&sha), bytes).unwrap();
    let to = format!("{target}/external-upload.txt");
    let _ = std::fs::remove_file(&to);
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"fs_import","sha256":sha,"to":to}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    assert_eq!(std::fs::read(&to).unwrap(), bytes);
    let (reply, _) = request(&conn, serde_json::json!({"op":"fs_list","path":"/etc"})).await;
    assert_eq!(err_code(&reply), AppErrorCode::E_SCOPE);
    println!("uploaded {to}; /etc refused");
}

/// QA helper (UNI-220): lets the app UI in a plain browser (vite) drive a
/// REAL separately started test host. Serves `POST /ipc {cmd,args}` on
/// `127.0.0.1:$FS_RELAY_PORT` (default 7931) and forwards the Files
/// commands (`fs_*`, `blob_put_bytes`) to `$FS_HOST_DIR`'s `serve` as a
/// paired throwaway phone; the page's `__TAURI_INTERNALS__.invoke` mock
/// fetches it. Runs for `$FS_RELAY_SECS` (default 900).
/// `FS_HOST_DIR=/tmp/qa-fs cargo test -p unipi-host --test fs_scope qa_ipc_relay -- --ignored --nocapture`
#[tokio::test]
#[ignore = "requires a separately started throwaway host; see scenario documentation"]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn qa_ipc_relay() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = std::env::var("FS_HOST_DIR").unwrap_or_else(|_| "/tmp/qa-fs".into());
    let port: u16 = std::env::var("FS_RELAY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(7931);
    let secs: u64 = std::env::var("FS_RELAY_SECS")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(900);
    let paths = HostPaths::from_dir(&dir);
    let cache: unipi_host::endpoint::EndpointCache =
        unipi_host::data_dir::read_json(&paths.endpoint())
            .unwrap()
            .expect("serve running");
    let phone = Endpoint::builder(presets::N0).bind().await.unwrap();
    let conn = phone.connect(cache.addr.clone(), ALPN).await.unwrap();
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    let (reply, _) = request(
        &conn,
        serde_json::json!({"op":"pair","code":state.code,"name":"fs-qa-relay"}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    println!("relay on http://127.0.0.1:{port}/ipc");

    #[allow(
        clippy::items_after_statements,
        reason = "Scenario-local helpers stay next to their resource lifecycle and call sites."
    )]
    async fn handle(
        conn: &Connection,
        cmd: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, serde_json::Value> {
        let s = |k: &str| args.get(k).cloned().unwrap_or(serde_json::Value::Null);
        let mut req = match cmd {
            "fs_roots" => serde_json::json!({"op":"fs_roots"}),
            "fs_list" => {
                serde_json::json!({"op":"fs_list","path":s("path"),"hidden":s("hidden"),"outside":s("outside")})
            }
            "fs_stat" => {
                serde_json::json!({"op":"fs_stat","path":s("path"),"outside":s("outside")})
            }
            "fs_read" => {
                serde_json::json!({"op":"fs_read","path":s("path"),"outside":s("outside"),"as_text":s("asText")})
            }
            "fs_write" => {
                serde_json::json!({"op":"fs_write","path":s("path"),"text":s("text"),"base_hash":s("baseHash"),"create":s("create"),"outside":s("outside")})
            }
            "fs_mkdir" => {
                serde_json::json!({"op":"fs_mkdir","path":s("path"),"outside":s("outside")})
            }
            "fs_rename" => {
                serde_json::json!({"op":"fs_rename","from":s("from"),"to":s("to"),"outside":s("outside")})
            }
            "fs_delete" => {
                serde_json::json!({"op":"fs_delete","path":s("path"),"confirm":s("confirm"),"outside":s("outside")})
            }
            "fs_search" => {
                serde_json::json!({"op":"fs_search","root":s("root"),"query":s("query"),"kind":s("kind"),"outside":s("outside")})
            }
            "fs_import" => {
                serde_json::json!({"op":"fs_import","sha256":s("sha256"),"to":s("to"),"overwrite":s("overwrite"),"outside":s("outside")})
            }
            // UNI-217: the Git view's one command.
            "git_op" => {
                serde_json::json!({"op":"git","path":s("path"),"outside":s("outside"),"cmd":s("cmd")})
            }
            "blob_put_bytes" => {
                let bytes: Vec<u8> = serde_json::from_value(s("bytes")).unwrap_or_default();
                let sha: String =
                    sha2::Sha256::digest(&bytes)
                        .iter()
                        .fold(String::new(), |mut out, byte| {
                            use std::fmt::Write as _;
                            write!(out, "{byte:02x}").unwrap();
                            out
                        });
                let (mut send, mut recv) = conn.open_bi().await.unwrap();
                let req = serde_json::json!({"op":"blob_put","sha256":sha,"size":bytes.len(),"mime":s("mime").as_str().unwrap_or("application/octet-stream"),"name":s("name")});
                send.write_all(&hello(req)).await.unwrap();
                let (_, first) = read_frame(&mut recv).await.unwrap();
                let first: Control = serde_json::from_slice(&first).unwrap();
                let have = match &first {
                    Control::Ok { data } => data
                        .as_ref()
                        .and_then(|d| d["have"].as_bool())
                        .unwrap_or(false),
                    Control::Err { error } => return Err(serde_json::to_value(error).unwrap()),
                    _ => false,
                };
                if !have {
                    for chunk in bytes.chunks(64 * 1024) {
                        send.write_all(&unipi_proto::frame::encode(
                            unipi_proto::FrameKind::Input,
                            chunk,
                        ))
                        .await
                        .unwrap();
                    }
                    let (_, end) = read_frame(&mut recv).await.unwrap();
                    if let Control::Err { error } = serde_json::from_slice::<Control>(&end).unwrap()
                    {
                        return Err(serde_json::to_value(error).unwrap());
                    }
                }
                let _ = send.finish();
                return Ok(
                    serde_json::json!({"sha256":sha,"size":bytes.len(),"deduped":have,"path":""}),
                );
            }
            other => {
                return Err(
                    serde_json::json!({"code":"E_UNSUPPORTED","message":format!("relay: {other}"),"retryable":false}),
                )
            }
        };
        if let Some(obj) = req.as_object_mut() {
            obj.retain(|_, v| !v.is_null());
        }
        let (reply, body) = request(conn, req).await;
        match reply {
            Control::Ok { data } => {
                let mut data = data.unwrap_or(serde_json::Value::Null);
                if cmd == "fs_read" {
                    data["text"] =
                        serde_json::Value::String(String::from_utf8_lossy(&body).into_owned());
                }
                Ok(data)
            }
            Control::Err { error } => Err(serde_json::to_value(error).unwrap()),
            other => Err(
                serde_json::json!({"code":"E_INTERNAL","message":format!("{other:?}"),"retryable":false}),
            ),
        }
    }

    let serve = async {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = vec![0u8; 65_536];
            let (head_end, len) = loop {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    break (i + 4, len);
                }
            };
            if head_end == 0 && len == 0 && buf.is_empty() {
                continue;
            }
            while buf.len() < head_end + len {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let cors = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\n";
            if buf.starts_with(b"OPTIONS") {
                let _ = sock
                    .write_all(
                        format!("HTTP/1.1 204 No Content\r\n{cors}Content-Length: 0\r\n\r\n")
                            .as_bytes(),
                    )
                    .await;
                continue;
            }
            let body: serde_json::Value =
                serde_json::from_slice(&buf[head_end..]).unwrap_or_default();
            let cmd = body["cmd"].as_str().unwrap_or("").to_string();
            let args = body["args"].clone();
            let out = match handle(&conn, &cmd, &args).await {
                Ok(v) => serde_json::json!({"ok":v}),
                Err(e) => serde_json::json!({"err":e}),
            };
            println!(
                "{cmd} {} -> {}",
                args.get("path")
                    .or(args.get("to"))
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default(),
                if out.get("ok").is_some() { "ok" } else { "err" }
            );
            let text = out.to_string();
            let _ = sock
                .write_all(format!("HTTP/1.1 200 OK\r\n{cors}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}", text.len()).as_bytes())
                .await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(secs), Box::pin(serve)).await;
}
