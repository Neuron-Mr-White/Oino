//! UNI-260: chat_new / chat_close / chat_config against
//! - real tmux on a private socket (`UNIPI_TMUX_SOCKET=unipi-260-it`),
//! - a fake herdr: a `herdr` script on PATH (session list) + a fake API
//!   socket served here that opens a real `sh` per pane,
//! and a fake pi launcher (python) that writes an app-bridge record into a
//! temp `UNIPI_BRIDGE_DIR` and quits on SIGTERM like pi. Never touches the
//! user's herdr, tmux server or ~/.unipi.
#![cfg(unix)]
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use unipi_host::{
    board::{Board, BoardConfig},
    chat_ctl,
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, Handler},
};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, ChatClosed, ChatNewTarget, Control, HerdrId, MuxKind, SessionName, PROTO_VERSION,
};

/// Process-global env (PATH, UNIPI_BRIDGE_DIR, UNIPI_TMUX_SOCKET): one test at a time.
static ENV: Mutex<()> = Mutex::new(());
const TMUX_SOCKET: &str = "unipi-260-it";

const FAKE_PI: &str = r#"
import os, sys, socket, json, signal, time
d = os.environ["UNIPI_BRIDGE_DIR"]; pid = os.getpid()
sp = f"{d}/{pid}.sock"; rec = f"{d}/{pid}.json"
s = socket.socket(socket.AF_UNIX); s.bind(sp); s.listen()
def bye(*a):
    for p in (sp, rec):
        try: os.unlink(p)
        except OSError: pass
    sys.exit(0)
signal.signal(signal.SIGTERM, bye)
time.sleep(0.3)  # like pi: the record shows up a moment after start
json.dump({"v": 1, "pid": pid, "socket": sp, "cwd": os.getcwd(),
           "herdrPaneId": os.environ.get("HERDR_PANE_ID"), "herdrSocket": os.environ.get("HERDR_SOCKET_PATH")}, open(rec + ".tmp", "w"))
os.rename(rec + ".tmp", rec)
while True: time.sleep(1)
"#;

struct Fixture {
    dir: tempfile::TempDir,
    bridge: PathBuf,
    launcher: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let bridge = dir.path().join("bridge");
    std::fs::create_dir_all(&bridge).unwrap();
    let script = dir.path().join("fake-pi.py");
    std::fs::write(&script, FAKE_PI).unwrap();
    std::env::set_var("UNIPI_BRIDGE_DIR", &bridge);
    let launcher = format!("UNIPI_BRIDGE_DIR={} python3 {}", bridge.display(), script.display());
    Fixture { dir, bridge, launcher }
}

fn records(bridge: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(bridge)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    v.sort();
    v
}

// ── tmux (real, private socket) ─────────────────────────────────────────

struct TmuxGuard;
impl Drop for TmuxGuard {
    fn drop(&mut self) {
        let _ = Command::new("tmux").args(["-L", TMUX_SOCKET, "kill-server"]).stderr(Stdio::null()).status();
    }
}

fn tmux(args: &[&str]) -> String {
    let out = Command::new("tmux").arg("-L").arg(TMUX_SOCKET).args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn tmux_new_session_new_window_and_close() {
    if Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("tmux not installed: skipped");
        return;
    }
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let fx = fixture();
    std::env::set_var("UNIPI_TMUX_SOCKET", TMUX_SOCKET);
    let _guard = TmuxGuard;
    let cwd = fx.dir.path().canonicalize().unwrap();

    let result = tokio::time::timeout(Duration::from_secs(60), async {
        // A new session.
        let target = ChatNewTarget::TmuxNewSession { name: SessionName::try_from("pi-it".to_string()).unwrap() };
        let a = chat_ctl::new_chat(&target, &cwd, &fx.launcher, Duration::from_secs(15)).await.expect("chat_new session");
        assert_eq!(a.mux, MuxKind::Tmux);
        assert_eq!(a.session, "pi-it");
        assert!(a.pane_id.starts_with('%'));
        assert_eq!(a.cwd.as_deref(), Some(cwd.to_string_lossy().as_ref()));
        assert!(records(&fx.bridge).contains(&format!("{}.json", a.pid)));

        // A second window in it.
        let target = ChatNewTarget::TmuxWindow { session: SessionName::try_from("pi-it".to_string()).unwrap() };
        let b = chat_ctl::new_chat(&target, &cwd, &fx.launcher, Duration::from_secs(15)).await.expect("chat_new window");
        assert_ne!(a.pid, b.pid);
        assert_ne!(a.workspace_id, b.workspace_id);
        assert_eq!(tmux(&["list-windows", "-t", "pi-it", "-F", "#{window_id}"]).lines().count(), 2);

        // chat_list's tmux side marks both panes with their bridge.
        let mut tree = unipi_host::mux::tmux::tree().await.unwrap();
        let bridges = unipi_host::chat::read_bridges(&fx.bridge);
        unipi_host::chat::join_tmux_bridges(&mut tree, &bridges, &[]);
        let chats: Vec<u32> = tree.sessions.as_ref().unwrap().iter().flat_map(|s| &s.windows).flat_map(|w| &w.panes).filter_map(|p| p.chat.as_ref().map(|c| c.pid)).collect();
        assert!(chats.contains(&a.pid) && chats.contains(&b.pid), "{chats:?}");
        // …unless herdr already shows that pi.
        let mut tree = unipi_host::mux::tmux::tree().await.unwrap();
        unipi_host::chat::join_tmux_bridges(&mut tree, &bridges, &[a.pid]);
        let chats: Vec<u32> = tree.sessions.as_ref().unwrap().iter().flat_map(|s| &s.windows).flat_map(|w| &w.panes).filter_map(|p| p.chat.as_ref().map(|c| c.pid)).collect();
        assert!(!chats.contains(&a.pid));

        // Close b with its window: pi stops (record gone), window gone.
        let rec = unipi_host::chat::find_bridge(&fx.bridge, b.pid).expect("b bridge");
        // The test may itself run inside herdr: the fake pi then inherits an
        // outer HERDR_PANE_ID, which must NOT be taken as its place.
        let place = chat_ctl::place_of(&rec).await;
        assert!(matches!(place, chat_ctl::PiPlace::Tmux { ref session, .. } if session == "pi-it"), "{place:?}");
        let closed = chat_ctl::close_chat(b.pid, &place, true, Duration::from_secs(10)).await.unwrap();
        assert!(closed.stopped);
        assert_eq!(closed.closed, ChatClosed::Window);
        assert!(!records(&fx.bridge).contains(&format!("{}.json", b.pid)));
        assert_eq!(tmux(&["list-windows", "-t", "pi-it", "-F", "#{window_id}"]).lines().count(), 1);

        // Stop a but keep its window ("Stop pi" without closing).
        let rec = unipi_host::chat::find_bridge(&fx.bridge, a.pid).expect("a bridge");
        let place = chat_ctl::place_of(&rec).await;
        let closed = chat_ctl::close_chat(a.pid, &place, false, Duration::from_secs(10)).await.unwrap();
        assert_eq!((closed.stopped, closed.closed), (true, ChatClosed::None));
        assert_eq!(tmux(&["list-windows", "-t", "pi-it", "-F", "#{window_id}"]).lines().count(), 1);

        // A launcher that never starts pi: a plain error with where it ran.
        let target = ChatNewTarget::TmuxWindow { session: SessionName::try_from("pi-it".to_string()).unwrap() };
        let e = chat_ctl::new_chat(&target, &cwd, "true", Duration::from_secs(2)).await.unwrap_err();
        assert_eq!(e.code, AppErrorCode::E_CHAT_UNAVAILABLE);
        assert_eq!(e.detail.as_ref().unwrap()["mux"], "tmux");
    })
    .await;
    std::env::remove_var("UNIPI_TMUX_SOCKET");
    std::env::remove_var("UNIPI_BRIDGE_DIR");
    result.expect("tmux test timed out");
}

// ── herdr (fake CLI + fake API socket with real shells) ─────────────────

#[derive(Default)]
struct FakeHerdr {
    tabs: Vec<(String, String)>,             // (tab_id, workspace_id)
    panes: Vec<(String, String, Child)>,     // (pane_id, tab_id, shell)
    calls: Vec<String>,
    next: u32,
}

impl FakeHerdr {
    fn spawn_shell(&mut self, socket: &str, ws: &str, tab: &str, cwd: &str) -> String {
        self.next += 1;
        let pane = format!("{ws}:p{}", self.next);
        let child = Command::new("sh")
            .current_dir(cwd)
            .env("HERDR_PANE_ID", &pane)
            .env("HERDR_SOCKET_PATH", socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.panes.push((pane.clone(), tab.to_string(), child));
        pane
    }
    fn close_tab(&mut self, tab: &str) {
        self.tabs.retain(|t| t.0 != tab);
        for (_, t, child) in self.panes.iter_mut() {
            if t == tab {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.panes.retain(|p| p.1 != tab);
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        for (_, _, child) in self.panes.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn handle_herdr(state: &mut FakeHerdr, socket: &str, method: &str, p: &serde_json::Value) -> serde_json::Value {
    state.calls.push(method.to_string());
    match method {
        "workspace.create" => {
            state.next += 1;
            let ws = format!("w{}", state.next);
            let tab = format!("{ws}:t1");
            state.tabs.push((tab.clone(), ws.clone()));
            let pane = state.spawn_shell(socket, &ws, &tab, p["cwd"].as_str().unwrap());
            serde_json::json!({"type":"workspace_created","workspace":{"workspace_id":ws,"label":p["label"]},"tab":{"tab_id":tab},"root_pane":{"pane_id":pane,"tab_id":tab}})
        }
        "tab.create" => {
            let ws = p["workspace_id"].as_str().unwrap().to_string();
            state.next += 1;
            let tab = format!("{ws}:t{}", state.next);
            state.tabs.push((tab.clone(), ws.clone()));
            let pane = state.spawn_shell(socket, &ws, &tab, p["cwd"].as_str().unwrap());
            serde_json::json!({"type":"tab_created","tab":{"tab_id":tab,"workspace_id":ws},"root_pane":{"pane_id":pane,"tab_id":tab}})
        }
        "pane.process_info" => {
            let pane = p["pane_id"].as_str().unwrap();
            let pid = state.panes.iter().find(|x| x.0 == pane).map(|x| x.2.id()).unwrap_or(0);
            serde_json::json!({"type":"pane_process_info","process_info":{"pane_id":pane,"shell_pid":pid}})
        }
        "pane.send_input" => {
            let pane = p["pane_id"].as_str().unwrap();
            if let Some(x) = state.panes.iter_mut().find(|x| x.0 == pane) {
                let stdin = x.2.stdin.as_mut().unwrap();
                let _ = writeln!(stdin, "{}", p["text"].as_str().unwrap());
                let _ = stdin.flush();
            }
            serde_json::json!({"type":"ok"})
        }
        "session.snapshot" => {
            let panes: Vec<_> = state.panes.iter().map(|(id, tab, _)| serde_json::json!({"pane_id":id,"tab_id":tab,"focused":false})).collect();
            serde_json::json!({"type":"session_snapshot","snapshot":{"workspaces":[],"tabs":[],"panes":panes}})
        }
        "tab.close" => {
            state.close_tab(p["tab_id"].as_str().unwrap());
            serde_json::json!({"type":"ok"})
        }
        "pane.close" => serde_json::json!({"type":"ok"}),
        _ => serde_json::json!({"type":"ok"}),
    }
}

async fn serve_herdr(listener: tokio::net::UnixListener, state: Arc<Mutex<FakeHerdr>>, socket: String) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { return };
        let state = state.clone();
        let socket = socket.clone();
        tokio::spawn(async move {
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            if let Ok(Some(line)) = lines.next_line().await {
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let result = handle_herdr(&mut state.lock().unwrap(), &socket, req["method"].as_str().unwrap(), &req["params"]);
                let reply = serde_json::json!({"id": req["id"], "result": result});
                let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
            }
        });
    }
}

#[tokio::test]
async fn herdr_new_tab_new_workspace_and_close_through_the_handler() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let fx = fixture();
    let socket = fx.dir.path().join("herdr.sock");
    let socket_s = socket.to_string_lossy().into_owned();
    // `herdr session list --json` → our fake session.
    let bin = fx.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let list = serde_json::json!({"sessions":[
        {"name":"fake","running":true,"default":true,"socket_path":socket_s},
        {"name":"guarded","running":true,"default":false,"socket_path":socket_s}
    ]});
    std::fs::write(bin.join("herdr"), format!("#!/bin/sh\ncat <<'EOF'\n{list}\nEOF\n")).unwrap();
    std::fs::set_permissions(bin.join("herdr"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{old_path}", bin.display()));

    let state = Arc::new(Mutex::new(FakeHerdr::default()));
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(serve_herdr(listener, state.clone(), socket_s.clone()));

    // A host whose config sets the launcher; "guarded" is protected.
    let host_dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(host_dir.path());
    paths.ensure_dir().unwrap();
    let phone = iroh::SecretKey::generate().public();
    DeviceStore::new(paths.devices()).add(&phone, "Test phone".into()).unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: Arc::new(tokio::sync::Mutex::new(())),
        protected: Arc::new(vec!["guarded".into()]),
        board: Board::new(BoardConfig::from_env()),
            cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    let call = |req: serde_json::Value| {
        let handler = handler.clone();
        async move {
            let (client, server) = tokio::io::duplex(1 << 20);
            let (read, write) = tokio::io::split(server);
            let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
            let (mut c_read, mut c_write) = tokio::io::split(client);
            let hello = serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req});
            c_write.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&hello).unwrap())).await.unwrap();
            let (_, bytes) = tokio::time::timeout(Duration::from_secs(60), read_frame(&mut c_read)).await.unwrap().unwrap();
            task.await.unwrap().unwrap();
            serde_json::from_slice::<Control>(&bytes).unwrap()
        }
    };
    let ok = |c: Control| match c {
        Control::Ok { data } => data.unwrap(),
        other => panic!("{other:?}"),
    };
    let err = |c: Control| match c {
        Control::Err { error } => error,
        other => panic!("{other:?}"),
    };

    let result = tokio::time::timeout(Duration::from_secs(90), async {
        // Config: default, then set (persisted, other keys kept), then reset.
        let c = ok(call(serde_json::json!({"op":"chat_config"})).await);
        assert_eq!((c["launcher"].as_str(), c["default"].as_bool(), c["herdr"].as_bool()), (Some("pi"), Some(true), Some(true)));
        let c = ok(call(serde_json::json!({"op":"chat_config_set","launcher": fx.launcher})).await);
        assert_eq!((c["launcher"].as_str(), c["default"].as_bool()), (Some(fx.launcher.as_str()), Some(false)));

        let cwd = fx.dir.path().canonicalize().unwrap();
        let cwd_s = cwd.to_string_lossy().into_owned();
        // New workspace (label from the folder unless given).
        let a = ok(call(serde_json::json!({"op":"chat_new","target":{"kind":"herdr_new_workspace","session":"fake","label":"qa"},"cwd":cwd_s})).await);
        assert_eq!((a["mux"].as_str(), a["session"].as_str()), (Some("herdr"), Some("fake")));
        let ws = a["workspace_id"].as_str().unwrap().to_string();
        // New tab in that workspace.
        let b = ok(call(serde_json::json!({"op":"chat_new","target":{"kind":"herdr_workspace","session":"fake","workspace_id":ws},"cwd":cwd_s})).await);
        assert_eq!(b["workspace_id"].as_str(), Some(ws.as_str()));
        assert_ne!(a["pid"], b["pid"]);
        assert_ne!(a["tab_id"], b["tab_id"]);
        assert_eq!(records(&fx.bridge).len(), 2);
        assert_eq!(state.lock().unwrap().tabs.len(), 2);

        // Protected session / missing folder / unknown pid.
        let e = err(call(serde_json::json!({"op":"chat_new","target":{"kind":"herdr_new_workspace","session":"guarded"},"cwd":cwd_s})).await);
        assert_eq!(e.code, AppErrorCode::E_UNAUTHORIZED);
        let e = err(call(serde_json::json!({"op":"chat_new","target":{"kind":"herdr_new_workspace","session":"fake"},"cwd":format!("{cwd_s}/nope")})).await);
        assert_ne!(e.code, AppErrorCode::E_INTERNAL);
        let e = err(call(serde_json::json!({"op":"chat_close","pid":1,"close_tab":true})).await);
        assert_eq!(e.code, AppErrorCode::E_NOT_FOUND);

        // A pi that only *claims* a's pane (inherited HERDR_PANE_ID, e.g. tmux
        // inside herdr) is not placed there.
        let mut impostor = Command::new("sleep").arg("30").spawn().unwrap();
        let fake = unipi_host::chat::BridgeRecord {
            pid: impostor.id(),
            socket: "/nope".into(),
            session_file: None,
            session_id: None,
            session_name: None,
            cwd: None,
            herdr_pane_id: a["pane_id"].as_str().map(str::to_string),
            herdr_socket: Some(socket_s.clone()),
            waiting: None,
            running: None,
            pending_work: None,
            lh_mode: None,
            lh_paused: None,
        };
        let place = chat_ctl::place_of(&fake).await;
        assert!(!matches!(place, chat_ctl::PiPlace::Herdr { .. }), "{place:?}");
        let _ = impostor.kill();
        let _ = impostor.wait();
        let real = unipi_host::chat::find_bridge(&fx.bridge, a["pid"].as_u64().unwrap() as u32).unwrap();
        assert!(matches!(chat_ctl::place_of(&real).await, chat_ctl::PiPlace::Herdr { .. }));

        // Close b with its tab: pi stopped, tab closed, a untouched.
        let closed = ok(call(serde_json::json!({"op":"chat_close","pid":b["pid"],"close_tab":true})).await);
        assert_eq!((closed["stopped"].as_bool(), closed["closed"].as_str()), (Some(true), Some("tab")));
        assert_eq!(records(&fx.bridge).len(), 1);
        {
            let s = state.lock().unwrap();
            assert_eq!(s.tabs.len(), 1);
            assert!(s.calls.contains(&"tab.close".to_string()));
        }
        // Stop a only.
        let closed = ok(call(serde_json::json!({"op":"chat_close","pid":a["pid"]})).await);
        assert_eq!((closed["stopped"].as_bool(), closed["closed"].as_str()), (Some(true), Some("none")));
        assert!(records(&fx.bridge).is_empty());
        assert_eq!(state.lock().unwrap().tabs.len(), 1);

        // Reset the launcher.
        let c = ok(call(serde_json::json!({"op":"chat_config_set","launcher":""})).await);
        assert_eq!((c["launcher"].as_str(), c["default"].as_bool()), (Some("pi"), Some(true)));
        let _ = HerdrId::try_from("w1".to_string());
    })
    .await;
    server.abort();
    std::env::set_var("PATH", old_path);
    std::env::remove_var("UNIPI_BRIDGE_DIR");
    let _ = HashMap::<u8, u8>::new();
    result.expect("herdr test timed out");
}

/// Live check on the REAL herdr, only inside a throwaway workspace the caller
/// created (e.g. `herdr workspace create --label qa-260 --no-focus`):
/// `UNIPI_QA_WORKSPACE=<id> UNIPI_QA_LAUNCHER='pi' cargo test -p unipi-host --test chat_ctl -- --ignored live_`.
/// Starts pi in a new tab there, checks its bridge, stops it and closes that tab.
#[tokio::test]
#[ignore]
async fn live_herdr_new_tab_and_close() {
    let Ok(ws) = std::env::var("UNIPI_QA_WORKSPACE") else { return };
    let launcher = std::env::var("UNIPI_QA_LAUNCHER").unwrap_or_else(|_| "pi".into());
    let session = std::env::var("UNIPI_QA_SESSION").unwrap_or_else(|_| "default".into());
    let cwd = std::env::var("UNIPI_QA_CWD").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("UNIPI_BRIDGE_DIR");
    let target = ChatNewTarget::HerdrWorkspace {
        session: SessionName::try_from(session).unwrap(),
        workspace_id: HerdrId::try_from(ws.clone()).unwrap(),
    };
    let started = chat_ctl::new_chat(&target, &cwd, &launcher, Duration::from_secs(60)).await.expect("chat_new");
    eprintln!("started {started:?}");
    assert_eq!(started.workspace_id.as_deref(), Some(ws.as_str()));
    let dir = unipi_host::chat::bridge_dir().unwrap();
    let rec = unipi_host::chat::find_bridge(&dir, started.pid).expect("bridge");
    let place = chat_ctl::place_of(&rec).await;
    eprintln!("place {place:?}");
    assert!(matches!(&place, chat_ctl::PiPlace::Herdr { pane_id, .. } if *pane_id == started.pane_id), "{place:?}");
    let closed = chat_ctl::close_chat(started.pid, &place, true, Duration::from_secs(15)).await.expect("close");
    eprintln!("closed {closed:?}");
    assert!(closed.stopped);
    assert_eq!(closed.closed, ChatClosed::Tab);
    assert!(unipi_host::chat::find_bridge(&dir, started.pid).is_none());
}
