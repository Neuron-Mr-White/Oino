//! Big session stores never stall the host (perf-sessions): a real `unipi-host serve`
//! over a sandboxed store of generated sessions.
//!
//! 1. A slow `sessions_list` in flight (the debug-only `UNIPI_DEBUG_SESSIONS_LIST_DELAY_MS`
//!    hook sleeps where listings run — the blocking pool) does not hold up `host_info`,
//!    a second listing's neighbours or a `session_open`.
//! 2. `session_open` of an id that does not exist answers `E_NOT_FOUND` at once (file
//!    names + card index, never a content scan), so the app can forget it.
//! 3. The listing is served from the card index: warm listings stay fast.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "tests assert by unwrapping and print their timings"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use unipi_host::data_dir::HostPaths;
use unipi_host::ipc;
use unipi_proto::frame::{encode, FrameKind};
use unipi_proto::PROTO_VERSION;

/// Sessions in the generated store (a few of them big).
const SESSIONS: usize = 400;
/// How long the hooked listing sleeps.
const LIST_DELAY: Duration = Duration::from_secs(4);
/// Generous debug-build budget for a request that must not wait for the listing.
const FAST: Duration = Duration::from_millis(1_500);

struct Host {
    _dir: tempfile::TempDir,
    paths: HostPaths,
    child: Child,
    log: PathBuf,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_session(path: &Path, id: &str, cwd: &str, turns: usize, pad: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(
        out,
        r#"{{"type":"session","version":3,"id":"{id}","timestamp":"2026-08-04T08:36:35.282Z","cwd":"{cwd}"}}"#
    )
    .unwrap();
    writeln!(out, r#"{{"type":"session_info","id":"n0","parentId":null,"timestamp":"2026-08-04T08:36:36.000Z","name":"Chat {id}"}}"#).unwrap();
    let pad = "x".repeat(pad);
    for turn in 0..turns {
        writeln!(out, r#"{{"type":"message","id":"u{turn}","parentId":null,"timestamp":"2026-08-04T09:00:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"question {turn}"}}],"timestamp":1754298000000}}}}"#).unwrap();
        writeln!(out, r#"{{"type":"message","id":"a{turn}","parentId":"u{turn}","timestamp":"2026-08-04T09:00:01.000Z","message":{{"role":"assistant","content":[{{"type":"text","text":"{pad}"}}],"timestamp":1754298001000}}}}"#).unwrap();
    }
    out.flush().unwrap();
}

fn generate(sessions: &Path, work: &Path) {
    for n in 0..SESSIONS {
        let id = format!("019fcbea-df52-7491-b190-{n:012}");
        let cwd = work.join(format!("project-{}", n % 20));
        let path = oino_session::manager::cwd_session_dir(sessions, &cwd)
            .join(format!("2026-08-04T08-36-35-282Z_{id}.jsonl"));
        let cwd = cwd.display().to_string();
        if n < 2 {
            write_session(&path, &id, &cwd, 2_500, 4_000); // ~20 MB
        } else {
            write_session(&path, &id, &cwd, 2 + n % 40, 300 + (n * 997) % 3_000);
        }
    }
}

impl Host {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let agent_dir = root.join("agent");
        let work = root.join("work");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        generate(&agent_dir.join("sessions"), &work);
        let paths = HostPaths::from_dir(root.join("host"));
        std::fs::create_dir_all(paths.root.clone()).unwrap();
        let log = root.join("host.log");
        let out = std::fs::File::create(&log).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_unipi-host"));
        command
            .arg("--data-dir")
            .arg(&paths.root)
            .arg("serve")
            .arg("--relay")
            .arg("https://127.0.0.1:9")
            .arg("--relay-only")
            .arg("--no-extensions")
            .arg("--mode")
            .arg("manual")
            .env("OINO_AGENT_DIR", &agent_dir)
            .env("OINO_DIST_EXTENSIONS", root.join("dist"))
            .env(
                "UNIPI_DEBUG_SESSIONS_LIST_DELAY_MS",
                LIST_DELAY.as_millis().to_string(),
            )
            .env("RUST_LOG", "unipi_host=info,oino_client=info")
            .env_remove("OINO_HOST_DIR")
            .env_remove("UNIPI_HOST_DIR")
            .env_remove("UNIPI_UPDATE_FEED")
            .stdin(Stdio::null())
            .stdout(Stdio::from(out.try_clone().unwrap()))
            .stderr(Stdio::from(out));
        if cfg!(unix) {
            command.env("HOME", root.join("home"));
        }
        let child = command.spawn().expect("spawn unipi-host serve");
        Self {
            _dir: dir,
            paths,
            child,
            log,
        }
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(30)..].join("\n")
    }

    async fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("the host exited ({status}):\n{}", self.log_tail());
            }
            if let Ok(Ok(reply)) = tokio::time::timeout(
                Duration::from_secs(5),
                op(&self.paths, json!({"op": "host_info"})),
            )
            .await
            {
                if reply["type"] == "ok" {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the host did not become ready:\n{}",
                self.log_tail()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn hello(req: &Value) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&json!({"type": "hello", "proto": PROTO_VERSION, "req": req})).unwrap(),
    )
}

/// One op on its own connection: the first control frame's JSON.
async fn op(paths: &HostPaths, req: Value) -> Result<Value, String> {
    let mut stream = ipc::connect(&paths.local_addr())
        .await
        .map_err(|e| e.to_string())?;
    stream
        .write_all(&hello(&req))
        .await
        .map_err(|e| e.to_string())?;
    let mut head = [0u8; 5];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let mut payload = vec![0u8; len];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&payload).map_err(|e| e.to_string())
}

async fn timed(paths: &HostPaths, req: Value) -> (Value, Duration) {
    let start = Instant::now();
    let reply = tokio::time::timeout(Duration::from_secs(60), op(paths, req.clone()))
        .await
        .unwrap_or_else(|_| panic!("{req} timed out"))
        .unwrap_or_else(|e| panic!("{req}: {e}"));
    (reply, start.elapsed())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_listing_never_blocks_other_requests() {
    let mut host = Host::start();
    host.wait_ready().await;
    let paths = host.paths.clone();

    // Two slow listings in flight (live and archived) — more than enough to wedge a
    // small runtime if they ran on its workers.
    let list_paths = paths.clone();
    let list =
        tokio::spawn(async move { timed(&list_paths, json!({"op": "sessions_list"})).await });
    let archived_paths = paths.clone();
    let archived = tokio::spawn(async move {
        timed(
            &archived_paths,
            json!({"op": "sessions_list", "archived": true}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // host_info answers while both listings sleep.
    for _ in 0..3 {
        let (reply, took) = timed(&paths, json!({"op": "host_info"})).await;
        assert_eq!(reply["type"], "ok", "{reply}");
        println!("host_info during a slow listing: {took:?}");
        assert!(took < FAST, "host_info took {took:?}:\n{}", host.log_tail());
    }
    // An unknown session id: E_NOT_FOUND at once (no listing, no content scan).
    let (reply, took) = timed(
        &paths,
        json!({"op": "session_open", "session": "019fcbea-dead-7491-b190-00000000beef"}),
    )
    .await;
    println!("session_open of a missing id: {took:?} → {reply}");
    assert_eq!(reply["type"], "err", "{reply}");
    assert_eq!(reply["error"]["code"], "E_NOT_FOUND", "{reply}");
    assert!(took < FAST, "missing open took {took:?}");
    assert!(!list.is_finished(), "the listing was still in flight");

    let (reply, took) = list.await.unwrap();
    assert_eq!(reply["type"], "ok", "{reply}");
    let cards = reply["data"].as_array().unwrap();
    assert_eq!(cards.len(), SESSIONS);
    assert!(took >= LIST_DELAY, "the hook delayed the listing");
    println!("first (delayed, cold) sessions_list: {took:?}");
    let (reply, _) = archived.await.unwrap();
    assert_eq!(reply["type"], "ok", "{reply}");
    // The card carries what the sidebar shows.
    let first = cards
        .iter()
        .find(|card| card["session_id"] == "019fcbea-df52-7491-b190-000000000007")
        .unwrap();
    assert_eq!(first["name"], "Chat 019fcbea-df52-7491-b190-000000000007");
    assert_eq!(first["message_count"], 2 * (2 + 7));
}
