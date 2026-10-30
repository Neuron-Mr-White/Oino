//! WP-A21 (app spec §3.12): host language servers over the binary
//! frames. A scripted `examples/fake_lsp` process stands in for a real
//! server; the test drives `lsp::serve_request` on a duplex the way
//! `protocol.rs`'s handler does after the hello.
//!
//! Ruling: §3.12 says `lsp_open` is a binary-frame operation — kind-0
//! `open` reply with `ok{data:LspAttachData}`, then raw LSP bytes pass
//! as kind-1 chunks and client bytes arrive as kind-2.

use std::path::PathBuf;

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use unipi_host::{data_dir::HostPaths, lsp::LspRegistry, protocol::read_frame};
use unipi_proto::{
    AppErrorCode, Control, FsPath, LspLanguage, Request,
    frame::{FrameKind, encode},
};

fn fake_bin() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../..").join("target"));
    let exe = target.join(format!(
        "debug/examples/fake_lsp{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        exe.is_file(),
        "fake_lsp missing — `cargo build -p unipi-host --example fake_lsp` ({})",
        exe.display()
    );
    dunce::canonicalize(exe).unwrap()
}

struct Fixture {
    _dir: TempDir,
    paths: HostPaths,
    registry: LspRegistry,
    /// Canonical project root (has a `Cargo.toml` marker).
    root: PathBuf,
    /// The fake server's method log.
    log: PathBuf,
}

/// `idle_minutes` 0 kills a server as soon as its last attach detaches.
fn fixture(idle_minutes: u64) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("server.log");
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();
    let command = format!("{} {}", fake_bin().display(), log.display());
    let config = serde_json::json!({
        "fs": {"allow": [dunce::canonicalize(dir.path()).unwrap().to_string_lossy()]},
        "lsp": {
            "idle_minutes": idle_minutes,
            "servers": {
                "fakelang": {"command": command},
                "gone": {"command": "no-such-lsp-binary-wp-a21", "install": "install the fake"}
            }
        }
    });
    std::fs::write(
        dir.path().join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
    Fixture {
        paths: HostPaths::from_dir(dir.path().to_path_buf()),
        registry: LspRegistry::new(),
        root: dunce::canonicalize(&root).unwrap(),
        log,
        _dir: dir,
    }
}

fn fs_path(path: &std::path::Path) -> FsPath {
    FsPath::try_from(path.to_string_lossy().into_owned()).expect("fs path")
}

fn language(s: &str) -> LspLanguage {
    LspLanguage::try_from(s.to_string()).expect("language")
}

async fn read_control(read: &mut (impl AsyncReadExt + Unpin)) -> Control {
    let (kind, bytes) = read_frame(read).await.expect("control frame");
    assert_eq!(kind, 0, "expected a control frame");
    serde_json::from_slice(&bytes).expect("control json")
}

/// Writes one JSON-RPC LSP message as a kind-2 client stream frame.
async fn send_lsp(write: &mut (impl AsyncWriteExt + Unpin), body: &serde_json::Value) {
    let body = serde_json::to_vec(body).unwrap();
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend_from_slice(&body);
    write
        .write_all(&encode(FrameKind::Input, &framed))
        .await
        .unwrap();
}

/// Reads one complete LSP message off the kind-1 server stream.
async fn recv_lsp(read: &mut (impl AsyncReadExt + Unpin), buf: &mut Vec<u8>) -> serde_json::Value {
    loop {
        let (kind, bytes) = read_frame(read).await.expect("stream frame");
        assert_eq!(kind, 1, "expected a server stream frame");
        buf.extend_from_slice(&bytes);
        if let Some(sep) = find_subslice(buf, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..sep]);
            let mut len = None;
            for line in headers.split("\r\n") {
                if let Some(v) = line.strip_prefix("Content-Length:") {
                    len = v.trim().parse::<usize>().ok();
                }
            }
            let len = len.expect("content-length");
            if buf.len() >= sep + 4 + len {
                let body = &buf[sep + 4..sep + 4 + len];
                let message = serde_json::from_slice(body).expect("json");
                buf.drain(..sep + 4 + len);
                return message;
            }
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn log_lines(log: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn lsp_servers_lists_with_root_and_availability() {
    let fix = fixture(1);
    let fix_root_src = fix.root.join("src");
    let (mut client, server) = duplex(1 << 16);
    let (mut read, mut write) = tokio::io::split(server);
    let registry = fix.registry.clone();
    let paths = fix.paths.clone();
    let task = tokio::spawn(async move {
        unipi_host::lsp::serve_request(
            &registry,
            &paths,
            Request::LspServers {
                path: fs_path(&fix_root_src),
                outside: None,
            },
            &mut read,
            &mut write,
        )
        .await
    });
    let Control::Ok { data: Some(data) } = read_control(&mut client).await else {
        panic!("expected ok{{data}}");
    };
    assert_eq!(data["root"], fix.root.to_string_lossy().to_string());
    let servers = data["servers"].as_array().unwrap();
    let fake = servers
        .iter()
        .find(|s| s["language"] == "fakelang")
        .unwrap();
    assert_eq!(fake["name"], "fakelang");
    assert_eq!(fake["available"], true);
    let gone = servers.iter().find(|s| s["language"] == "gone").unwrap();
    assert_eq!(gone["available"], false);
    assert_eq!(gone["install_hint"], "install the fake");
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn lsp_open_streams_and_detaches_clean() {
    let fix = fixture(0);
    let (mut client, server) = duplex(1 << 16);
    let (mut read, mut write) = tokio::io::split(server);
    let registry = fix.registry.clone();
    let paths = fix.paths.clone();
    let root = fix.root.clone();
    let task = tokio::spawn(async move {
        unipi_host::lsp::serve_request(
            &registry,
            &paths,
            Request::LspOpen {
                root: fs_path(&root),
                language: language("fakelang"),
                outside: None,
                command: None,
            },
            &mut read,
            &mut write,
        )
        .await
    });
    // §3.12: the attach reply names the server and says this is a fresh attach.
    let Control::Ok { data: Some(data) } = read_control(&mut client).await else {
        panic!("expected ok{{data}}");
    };
    assert_eq!(data["server"], "fakelang");
    assert_eq!(data["attached"], false);
    assert_eq!(data["initialize_result"], serde_json::Value::Null);

    let mut stream = Vec::new();
    send_lsp(
        &mut client,
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri": format!("file://{}", fix.root.display())}}),
    )
    .await;
    let init = recv_lsp(&mut client, &mut stream).await;
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["capabilities"]["hoverProvider"], true);
    send_lsp(
        &mut client,
        &serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    )
    .await;
    let uri = format!("file://{}/src/a.fk", fix.root.display());
    send_lsp(
        &mut client,
        &serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"fakelang","version":1,"text":"fake"}}}),
    )
    .await;
    let diag = recv_lsp(&mut client, &mut stream).await;
    assert_eq!(diag["method"], "textDocument/publishDiagnostics");
    send_lsp(
        &mut client,
        &serde_json::json!({"jsonrpc":"2.0","id":7,"method":"textDocument/hover","params":{"textDocument":{"uri":uri},"position":{"line":0,"character":1}}}),
    )
    .await;
    let hover = recv_lsp(&mut client, &mut stream).await;
    assert_eq!(hover["id"], 7);
    assert_eq!(hover["result"]["contents"]["value"], "**fake** hover");

    // Detach: the host closes the open document on the server and —
    // idle_minutes 0 — kills it. (Drop the whole stream: split halves
    // keep a duplex alive and the peer never sees EOF.) The didClose
    // lands in the server's method log asynchronously — poll for it.
    drop(client);
    task.await.unwrap().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let lines = loop {
        let lines = log_lines(&fix.log);
        if lines
            .iter()
            .any(|l| l.starts_with("textDocument/didClose") && l.contains(&uri))
        {
            break lines;
        }
        assert!(std::time::Instant::now() < deadline, "detach must didClose the open document: {lines:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(
        lines.iter().any(|l| l.starts_with("initialize")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("textDocument/didOpen")),
        "{lines:?}"
    );
    // idle 0 kills the detached server; the exit-watcher reaps the
    // registry entry asynchronously.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while fix.registry.len() != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "idle 0 must kill the detached server"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn lsp_reattach_caches_initialize_and_takeover_errors_the_old() {
    let fix = fixture(60); // idle long enough to re-attach
    let open = |root: &PathBuf| Request::LspOpen {
        root: fs_path(root),
        language: language("fakelang"),
        outside: None,
        command: None,
    };

    // First attach: initialize for real.
    let (mut r1, s1) = duplex(1 << 16);
    let (mut sr1, mut sw1) = tokio::io::split(s1);
    let registry = fix.registry.clone();
    let paths = fix.paths.clone();
    let root = fix.root.clone();
    let t1 = tokio::spawn(async move {
        unipi_host::lsp::serve_request(&registry, &paths, open(&root), &mut sr1, &mut sw1).await
    });
    read_control(&mut r1).await;
    let mut stream = Vec::new();
    send_lsp(
        &mut r1,
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .await;
    let init = recv_lsp(&mut r1, &mut stream).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "fake_lsp");
    // Detach; idle 60 keeps the server warm.
    drop(r1);
    t1.await.unwrap().unwrap();
    assert_eq!(fix.registry.len(), 1);

    // Second attach: `attached:true`, the cached result rides the reply…
    let (mut r2, s2) = duplex(1 << 16);
    let (mut sr2, mut sw2) = tokio::io::split(s2);
    let registry = fix.registry.clone();
    let paths = fix.paths.clone();
    let root = fix.root.clone();
    let t2 = tokio::spawn(async move {
        unipi_host::lsp::serve_request(&registry, &paths, open(&root), &mut sr2, &mut sw2).await
    });
    let Control::Ok { data: Some(data) } = read_control(&mut r2).await else {
        panic!("expected ok{{data}}");
    };
    assert_eq!(data["attached"], true);
    assert_eq!(
        data["initialize_result"]["capabilities"]["hoverProvider"],
        true
    );
    // …and the host answers this client's initialize from the cache —
    // the server never sees a second one.
    send_lsp(
        &mut r2,
        &serde_json::json!({"jsonrpc":"2.0","id":9,"method":"initialize","params":{}}),
    )
    .await;
    let cached = recv_lsp(&mut r2, &mut stream).await;
    assert_eq!(cached["id"], 9);
    assert_eq!(cached["result"]["serverInfo"]["name"], "fake_lsp");
    send_lsp(
        &mut r2,
        &serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    )
    .await;
    // Poll until the first initialize is logged (it happened during
    // attach 1 — under load the file flush may lag).
    let init_count = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let count = log_lines(&fix.log)
                .iter()
                .filter(|l| l.starts_with("initialize"))
                .count();
            if count >= 1 || std::time::Instant::now() >= deadline {
                break count;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    assert_eq!(init_count, 1, "the server saw initialize exactly once");
    assert!(
        !log_lines(&fix.log)
            .iter()
            .any(|l| l.starts_with("initialized ")),
        "the swallowed initialized must not reach the server either"
    );

    // A third attach takes over: the second stream ends with E_LSP_TAKEN.
    // (Attach 3 first, then read attach 2's error — order the reads the
    // way the host writes them.)
    let (mut r3, s3) = duplex(1 << 16);
    let (mut sr3, mut sw3) = tokio::io::split(s3);
    let registry = fix.registry.clone();
    let paths = fix.paths.clone();
    let root = fix.root.clone();
    let t3 = tokio::spawn(async move {
        unipi_host::lsp::serve_request(&registry, &paths, open(&root), &mut sr3, &mut sw3).await
    });
    let Control::Err { error } = read_control(&mut r2).await else {
        panic!("expected err{{E_LSP_TAKEN}} on the evicted attach");
    };
    assert_eq!(error.code, AppErrorCode::E_LSP_TAKEN);
    t2.await.unwrap().unwrap();
    let Control::Ok { data: Some(data) } = read_control(&mut r3).await else {
        panic!("expected ok{{data}} on the new attach");
    };
    assert_eq!(data["attached"], true);
    // The take-over's didClose flush went through the new attach.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    drop(r3);
    t3.await.unwrap().unwrap();
    fix.registry.kill_all();
    assert_eq!(fix.registry.len(), 0, "kill_all removes every entry");
}

#[tokio::test]
async fn lsp_open_refusals() {
    let fix = fixture(0);
    let try_open = |req: Request| {
        let registry = fix.registry.clone();
        let paths = fix.paths.clone();
        async move {
            let (client, server) = duplex(1 << 16);
            drop(client);
            let (mut read, mut write) = tokio::io::split(server);
            unipi_host::lsp::serve_request(&registry, &paths, req, &mut read, &mut write).await
        }
    };
    // A request may never name a binary.
    let error = try_open(Request::LspOpen {
        root: fs_path(&fix.root),
        language: language("fakelang"),
        outside: None,
        command: Some("x".to_string()),
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::E_UNAUTHORIZED);
    // Unknown language.
    let error = try_open(Request::LspOpen {
        root: fs_path(&fix.root),
        language: language("cobol"),
        outside: None,
        command: None,
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::E_UNSUPPORTED);
    // Configured but not installed.
    let error = try_open(Request::LspOpen {
        root: fs_path(&fix.root),
        language: language("gone"),
        outside: None,
        command: None,
    })
    .await
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::E_NOT_FOUND);
    assert!(
        error.message.as_str().contains("install the fake"),
        "{}",
        error.message.as_str()
    );
}
