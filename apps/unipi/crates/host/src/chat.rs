//! Chat relay (DESIGN §3.4, docs/m5/PROTOCOL.md).
//!
//! Every pi with the unipi app bridge writes `~/.unipi/bridge/<pid>.json`
//! and listens on `<pid>.sock`. `chat_list` joins those records onto the
//! herdr tree (pane ↔ bridge by pi session file, then by pane id + herdr
//! socket); `chat_open` relays JSON lines between the phone and one socket.
//! The host never interprets the lines.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use unipi_proto::{
    frame::MAX_FRAME, AppErrorCode, AppErrorDto, ChatBridgeRef, ChatWaiting, Control, HerdrState,
};

use crate::protocol::write_control;

/// A bridge line bigger than this can't fit a control frame (the JSON
/// envelope adds a little); the bridge keeps lines under 900 KiB.
const MAX_LINE: usize = MAX_FRAME - 4096;

/// `~/.unipi/bridge/<pid>.json` as written by @pi-unipi/app-bridge.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRecord {
    pub pid: u32,
    pub socket: String,
    #[serde(default)]
    pub session_file: Option<String>,
    /// UNI-316: the pi session's id (`sessionId` on the wire), for cron
    /// `session_link` matching by id when a session file isn't given.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub herdr_pane_id: Option<String>,
    #[serde(default)]
    pub herdr_socket: Option<String>,
    /// Set while this pi is waiting on the user (UNI-161 "needs you").
    #[serde(default)]
    pub waiting: Option<BridgeWaiting>,
    /// pi's own run state (UNI-212); absent from older bridges.
    #[serde(default)]
    pub running: Option<bool>,
    /// UNI-221: wake-capable work still pending (null/absent = none).
    #[serde(default)]
    pub pending_work: Option<String>,
    /// UNI-269: the long-horizon working mode (`goal`/`ralph`/`swarm`/`graph`;
    /// null/absent = regular or no long-horizon).
    #[serde(default)]
    pub lh_mode: Option<String>,
    /// UNI-269: a parked long-horizon owner's mode.
    #[serde(default)]
    pub lh_paused: Option<String>,
}

/// `BridgeRecord.waiting` as @pi-unipi/app-bridge writes it (camelCase on the wire).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BridgeWaiting {
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    pub since: u64,
}

/// The bridge directory: `$UNIPI_BRIDGE_DIR`, else `~/.unipi/bridge`.
pub fn bridge_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("UNIPI_BRIDGE_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::fs::home_dir_opt().map(|home| home.join(".unipi").join("bridge"))
}

fn pid_alive(pid: u32) -> bool {
    // /proc is enough on Linux; on Windows/macOS ask the OS (a stale
    // record for a crashed pi would otherwise keep a dead chat listed).
    let proc = Path::new("/proc");
    if proc.is_dir() {
        return proc.join(pid.to_string()).exists();
    }
    crate::chat_ctl::pid_alive(pid)
}

/// Connects to a live bridge (UNI-3xx Windows parity): a unix socket on
/// Linux/macOS, a named pipe (`\\.\pipe\unipi-bridge-<pid>`) on Windows.
/// A Windows record that carries a path instead of a pipe is a bridge from
/// before the named-pipe change: it can't be reached, and saying so beats a
/// confusing connect error.
pub async fn connect_bridge(socket: &str) -> std::io::Result<crate::ipc::Stream> {
    #[cfg(unix)]
    {
        crate::ipc::connect_unix(socket).await
    }
    #[cfg(windows)]
    {
        if !crate::ipc::is_pipe(socket) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "this pi session runs an older UniPi bridge — restart pi after updating",
            ));
        }
        crate::ipc::connect_pipe(socket).await
    }
}

/// Every live bridge record in `dir` (bad files and dead pids skipped).
pub fn read_bridges(dir: &Path) -> Vec<BridgeRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if stem.parse::<u32>().is_err() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<BridgeRecord>(&text) else {
            continue;
        };
        if record.pid.to_string() != stem || !pid_alive(record.pid) {
            continue;
        }
        if !crate::ipc::is_pipe(&record.socket) && !Path::new(&record.socket).exists() {
            continue;
        }
        out.push(record);
    }
    out.sort_by_key(|r| r.pid);
    out
}

fn chat_ref(record: &BridgeRecord) -> ChatBridgeRef {
    ChatBridgeRef {
        pid: record.pid,
        session_name: record.session_name.clone(),
        cwd: record.cwd.clone(),
        waiting: record.waiting.as_ref().map(|w| ChatWaiting {
            kind: w.kind.clone(),
            title: w.title.clone(),
            since: w.since,
        }),
        running: record.running,
        pending_work: record.pending_work.clone().filter(|p| !p.is_empty()),
        lh_mode: lh_mode_id(record.lh_mode.as_deref()),
        lh_paused: lh_mode_id(record.lh_paused.as_deref()),
    }
}

/// A mode id fit for the list badge: a short lowercase word, else dropped.
fn lh_mode_id(mode: Option<&str>) -> Option<String> {
    let mode = mode?.trim();
    let ok = !mode.is_empty()
        && mode != "none"
        && mode.len() <= 32
        && mode.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    ok.then(|| mode.to_string())
}

/// Marks each herdr pane whose pi has a bridge (session file first, then
/// pane id within the same herdr socket). `sockets` maps a herdr session
/// name to its socket path.
pub fn join_bridges(
    state: &mut HerdrState,
    bridges: &[BridgeRecord],
    sockets: &std::collections::HashMap<String, String>,
) {
    let Some(sessions) = state.sessions.as_mut() else {
        return;
    };
    for session in sessions {
        let socket = sockets.get(session.name.as_str());
        for workspace in &mut session.workspaces {
            for tab in &mut workspace.tabs {
                for pane in &mut tab.panes {
                    let session_file = pane
                        .agent
                        .as_ref()
                        .and_then(|a| a.session.as_ref())
                        .map(|s| s.value.as_str())
                        .filter(|v| !v.is_empty());
                    let by_file = session_file.and_then(|file| {
                        bridges
                            .iter()
                            .find(|b| b.session_file.as_deref() == Some(file))
                    });
                    let by_pane = || {
                        bridges.iter().find(|b| {
                            b.herdr_pane_id.as_deref() == Some(pane.id.as_str())
                                && match (socket, b.herdr_socket.as_deref()) {
                                    (Some(a), Some(b)) => a == b,
                                    _ => false,
                                }
                        })
                    };
                    pane.chat = by_file.or_else(by_pane).map(chat_ref);
                }
            }
        }
    }
}

/// UNI-260: marks each tmux pane whose pi (a bridge pid descending from the
/// pane's shell pid) has a bridge. Bridges already placed in a herdr pane
/// are skipped, so a tmux running inside herdr doesn't list the same pi twice.
pub fn join_tmux_bridges(state: &mut unipi_proto::TmuxState, bridges: &[BridgeRecord], taken: &[u32]) {
    let Some(sessions) = state.sessions.as_mut() else {
        return;
    };
    for session in sessions {
        for window in &mut session.windows {
            for pane in &mut window.panes {
                if pane.pid == 0 {
                    continue;
                }
                pane.chat = bridges
                    .iter()
                    .filter(|b| !taken.contains(&b.pid))
                    .find(|b| crate::chat_ctl::descends_from(b.pid, pane.pid))
                    .map(chat_ref);
                if pane.chat.is_some() && pane.agent.is_none() {
                    pane.agent = Some(unipi_proto::PaneAgent {
                        name: "pi".into(),
                        status: unipi_proto::AgentStatus::Unknown,
                        session: None,
                    });
                }
            }
        }
    }
}

/// Pids of every bridge already shown in a herdr pane.
pub fn herdr_chat_pids(state: &HerdrState) -> Vec<u32> {
    let mut out = Vec::new();
    for session in state.sessions.iter().flatten() {
        for ws in &session.workspaces {
            for tab in &ws.tabs {
                for pane in &tab.panes {
                    if let Some(chat) = &pane.chat {
                        out.push(chat.pid);
                    }
                }
            }
        }
    }
    out
}

fn unavailable(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_CHAT_UNAVAILABLE, message, true)
}

/// The record for `pid`, when its bridge is live.
pub fn find_bridge(dir: &Path, pid: u32) -> Option<BridgeRecord> {
    read_bridges(dir).into_iter().find(|r| r.pid == pid)
}

/// Relays one chat: bridge lines → `chat{line}` controls; phone `chat`
/// controls → bridge lines. Ends when either side closes.
pub async fn relay<R, W, S>(read: &mut R, write: &mut W, bridge: S) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (bridge_read, mut bridge_write) = tokio::io::split(bridge);
    let mut lines = BufReader::new(bridge_read);
    write_control(write, &Control::Ok { data: None })
        .await
        .map_err(|e| unavailable(e.to_string()))?;
    let mut line = Vec::new();
    loop {
        tokio::select! {
            read_line = read_bounded_line(&mut lines, &mut line) => match read_line {
                Ok(true) => {
                    if line.len() > MAX_LINE {
                        tracing::warn!(len = line.len(), "dropping oversized bridge line");
                        continue;
                    }
                    let text = String::from_utf8_lossy(&line).into_owned();
                    if write_control(write, &Control::Chat { line: text }).await.is_err() {
                        return Ok(());
                    }
                }
                Ok(false) => {
                    // pi quit (or the bridge closed the socket).
                    let _ = write_control(write, &Control::Exit { code: None }).await;
                    return Ok(());
                }
                Err(error) => {
                    return Err(unavailable(format!("Lost the pi session: {error}")));
                }
            },
            frame = crate::protocol::read_frame(read) => match frame {
                Ok((0, bytes)) => {
                    if let Ok(Control::Chat { line }) = serde_json::from_slice::<Control>(&bytes) {
                        let mut out = line.into_bytes();
                        out.retain(|b| *b != b'\n');
                        out.push(b'\n');
                        if bridge_write.write_all(&out).await.is_err() {
                            let _ = write_control(write, &Control::Exit { code: None }).await;
                            return Ok(());
                        }
                    }
                }
                Ok(_) => {}
                // The phone closed its send half: unsubscribe.
                Err(_) => return Ok(()),
            },
        }
    }
}

/// Reads one `\n`-terminated line into `buf` (without the newline).
/// Over-long lines are read to their end and returned truncated past
/// `MAX_LINE + 1` bytes (the caller drops them). `Ok(false)` at EOF.
async fn read_bounded_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<bool> {
    buf.clear();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(!buf.is_empty());
        }
        if let Some(i) = available.iter().position(|b| *b == b'\n') {
            if buf.len() <= MAX_LINE {
                let take = i.min(MAX_LINE + 1 - buf.len());
                buf.extend_from_slice(&available[..take]);
            }
            reader.consume(i + 1);
            return Ok(true);
        }
        let n = available.len();
        if buf.len() <= MAX_LINE {
            let take = n.min(MAX_LINE + 1 - buf.len());
            buf.extend_from_slice(&available[..take]);
        }
        reader.consume(n);
    }
}

/// `chat_open`: connects to the pid's bridge socket and relays.
pub async fn open<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    read: &mut R,
    write: &mut W,
    pid: u32,
) -> Result<(), AppErrorDto> {
    let dir = bridge_dir().ok_or_else(|| unavailable("No home directory on the host"))?;
    let record = find_bridge(&dir, pid).ok_or_else(|| {
        AppErrorDto::new(
            AppErrorCode::E_NOT_FOUND,
            "That pi session is gone. Pull to refresh.",
            false,
        )
    })?;
    let stream = connect_bridge(&record.socket).await.map_err(|error| {
        // The "older bridge" case is already a full sentence for the user.
        if error.kind() == std::io::ErrorKind::InvalidInput {
            unavailable(error.to_string())
        } else {
            unavailable(format!("Can't reach that pi session: {error}"))
        }
    })?;
    relay(read, write, stream).await
}

/// Discards anything the phone sends (used by tests to mimic a closed half).
#[allow(dead_code)]
async fn drain<R: AsyncRead + Unpin>(read: &mut R) {
    let mut scratch = [0u8; 256];
    while matches!(read.read(&mut scratch).await, Ok(n) if n > 0) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use unipi_proto::{
        frame::{encode, FrameKind},
        AgentSessionRef, AgentStatus, HerdrId, HerdrPane, HerdrSession, HerdrTab, HerdrWorkspace,
        PaneAgent, SessionName,
    };

    fn pane(id: &str, session_file: Option<&str>) -> HerdrPane {
        HerdrPane {
            id: HerdrId::try_from(id.to_string()).unwrap(),
            focused: false,
            title: None,
            terminal_title: None,
            cwd: None,
            agent: Some(PaneAgent {
                name: "pi".into(),
                status: AgentStatus::Idle,
                session: session_file.map(|v| AgentSessionRef {
                    kind: "path".into(),
                    value: v.into(),
                }),
            }),
            chat: None,
        }
    }

    fn state(panes: Vec<HerdrPane>) -> HerdrState {
        HerdrState {
            available: true,
            reason: None,
            sessions: Some(vec![HerdrSession {
                name: SessionName::try_from("default".to_string()).unwrap(),
                running: true,
                default: true,
                error: None,
                workspaces: vec![HerdrWorkspace {
                    id: HerdrId::try_from("w1".to_string()).unwrap(),
                    label: "w".into(),
                    number: 1,
                    focused: true,
                    agent_status: AgentStatus::Idle,
                    tab_count: 1,
                    pane_count: panes.len() as u32,
                    tabs: vec![HerdrTab {
                        id: HerdrId::try_from("w1:t1".to_string()).unwrap(),
                        label: "t".into(),
                        number: 1,
                        focused: true,
                        agent_status: AgentStatus::Idle,
                        panes,
                    }],
                }],
            }]),
        }
    }

    fn record(pid: u32, file: Option<&str>, pane: Option<&str>, sock: Option<&str>) -> BridgeRecord {
        BridgeRecord {
            pid,
            socket: format!("/tmp/{pid}.sock"),
            session_file: file.map(Into::into),
            session_id: None,
            session_name: Some(format!("s{pid}")),
            cwd: Some("/p".into()),
            herdr_pane_id: pane.map(Into::into),
            herdr_socket: sock.map(Into::into),
            waiting: None,
            running: None,
            pending_work: None,
            lh_mode: None,
            lh_paused: None,
        }
    }

    #[test]
    fn joins_by_session_file_then_by_pane() {
        let mut tree = state(vec![pane("w1:p1", Some("/s/a.jsonl")), pane("w1:p2", None), pane("w1:p3", Some("/s/x.jsonl"))]);
        let bridges = vec![
            record(10, Some("/s/a.jsonl"), Some("w9:p9"), None),
            record(11, None, Some("w1:p2"), Some("/h.sock")),
            record(12, None, Some("w1:p3"), Some("/other.sock")),
        ];
        let sockets = HashMap::from([("default".to_string(), "/h.sock".to_string())]);
        join_bridges(&mut tree, &bridges, &sockets);
        let panes = &tree.sessions.as_ref().unwrap()[0].workspaces[0].tabs[0].panes;
        assert_eq!(panes[0].chat.as_ref().map(|c| c.pid), Some(10));
        assert_eq!(panes[1].chat.as_ref().map(|c| c.pid), Some(11));
        assert_eq!(panes[2].chat, None, "pane id in another herdr session does not match");
        assert_eq!(panes[0].chat.as_ref().unwrap().session_name.as_deref(), Some("s10"));
    }

    #[test]
    fn joins_the_waiting_state_through_to_the_pane_s_chat_ref() {
        let mut tree = state(vec![pane("w1:p1", Some("/s/a.jsonl"))]);
        let mut bridge = record(10, Some("/s/a.jsonl"), None, None);
        bridge.waiting = Some(BridgeWaiting { kind: "ask_user".into(), title: Some("Which plan?".into()), since: 1_700_000_000_000 });
        let bridges = vec![bridge];
        join_bridges(&mut tree, &bridges, &HashMap::new());
        let panes = &tree.sessions.as_ref().unwrap()[0].workspaces[0].tabs[0].panes;
        let waiting = panes[0].chat.as_ref().unwrap().waiting.as_ref().expect("waiting carried through");
        assert_eq!(waiting.kind, "ask_user");
        assert_eq!(waiting.title.as_deref(), Some("Which plan?"));
        assert_eq!(waiting.since, 1_700_000_000_000);
    }

    #[test]
    fn reads_a_waiting_record_from_disk_and_a_record_without_one_has_no_waiting_state() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let sock = dir.path().join(format!("{me}.sock"));
        std::fs::write(&sock, b"").unwrap();
        let waiting_json = serde_json::json!({
            "v":1,"pid":me,"socket":sock,"sessionFile":"/s.jsonl","cwd":"/p",
            "waiting": {"kind":"ask_user","title":"Pick one","since":1_700_000_000_000u64},
        });
        std::fs::write(dir.path().join(format!("{me}.json")), waiting_json.to_string()).unwrap();
        let records = read_bridges(dir.path());
        assert_eq!(records.len(), 1);
        let waiting = records[0].waiting.as_ref().expect("waiting parsed");
        assert_eq!(waiting.kind, "ask_user");
        assert_eq!(waiting.title.as_deref(), Some("Pick one"));
        assert_eq!(waiting.since, 1_700_000_000_000);
    }

    #[test]
    fn carries_the_bridge_s_running_flag_and_older_records_have_none() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let sock = dir.path().join(format!("{me}.sock"));
        std::fs::write(&sock, b"").unwrap();
        let json = serde_json::json!({"v":1,"pid":me,"socket":sock,"sessionFile":"/s/a.jsonl","running":true});
        std::fs::write(dir.path().join(format!("{me}.json")), json.to_string()).unwrap();
        let records = read_bridges(dir.path());
        assert_eq!(records[0].running, Some(true));
        let mut tree = state(vec![pane("w1:p1", Some("/s/a.jsonl"))]);
        join_bridges(&mut tree, &records, &HashMap::new());
        let panes = &tree.sessions.as_ref().unwrap()[0].workspaces[0].tabs[0].panes;
        assert_eq!(panes[0].chat.as_ref().unwrap().running, Some(true));
        let wire = serde_json::to_value(panes[0].chat.as_ref().unwrap()).unwrap();
        assert_eq!(wire["running"], true);
        assert!(record(1, None, None, None).running.is_none());
        let old = serde_json::to_value(chat_ref(&record(1, None, None, None))).unwrap();
        assert!(old.get("running").is_none(), "absent, not null, for older bridges");
    }

    #[test]
    fn carries_the_bridge_s_pending_work_into_chat_list() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let sock = dir.path().join(format!("{me}.sock"));
        std::fs::write(&sock, b"").unwrap();
        let json = serde_json::json!({"v":1,"pid":me,"socket":sock,"sessionFile":"/s/a.jsonl","running":false,"pendingWork":"bg: sleep 30 · subagent working"});
        std::fs::write(dir.path().join(format!("{me}.json")), json.to_string()).unwrap();
        let records = read_bridges(dir.path());
        assert_eq!(records[0].pending_work.as_deref(), Some("bg: sleep 30 · subagent working"));
        let mut tree = state(vec![pane("w1:p1", Some("/s/a.jsonl"))]);
        join_bridges(&mut tree, &records, &HashMap::new());
        let panes = &tree.sessions.as_ref().unwrap()[0].workspaces[0].tabs[0].panes;
        let wire = serde_json::to_value(panes[0].chat.as_ref().unwrap()).unwrap();
        assert_eq!(wire["pending_work"], "bg: sleep 30 · subagent working");
        // The bridge writes `null` when nothing is pending: absent on the wire.
        let none = serde_json::json!({"v":1,"pid":me,"socket":sock,"pendingWork":null});
        std::fs::write(dir.path().join(format!("{me}.json")), none.to_string()).unwrap();
        let records = read_bridges(dir.path());
        assert!(records[0].pending_work.is_none());
        let wire = serde_json::to_value(chat_ref(&records[0])).unwrap();
        assert!(wire.get("pending_work").is_none());
    }

    #[test]
    fn carries_the_long_horizon_mode_into_chat_list() {
        let mut r = record(1, None, None, None);
        let wire = serde_json::to_value(chat_ref(&r)).unwrap();
        assert!(wire.get("lh_mode").is_none() && wire.get("lh_paused").is_none(), "absent for older bridges / regular");
        r.lh_mode = Some("graph".into());
        r.lh_paused = Some("goal".into());
        let wire = serde_json::to_value(chat_ref(&r)).unwrap();
        assert_eq!(wire["lh_mode"], "graph");
        assert_eq!(wire["lh_paused"], "goal");
        // camelCase in the record; null = none; junk never reaches the phone.
        let parsed: BridgeRecord = serde_json::from_value(serde_json::json!({"pid":1,"socket":"/s","lhMode":"swarm","lhPaused":null})).unwrap();
        assert_eq!(parsed.lh_mode.as_deref(), Some("swarm"));
        assert!(parsed.lh_paused.is_none());
        for junk in ["none", "", "Graph Mode", "<b>x</b>", &"x".repeat(40)] {
            r.lh_mode = Some(junk.to_string());
            assert!(chat_ref(&r).lh_mode.is_none(), "{junk:?} dropped");
        }
    }

    #[test]
    fn reads_live_records_only() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        let sock = dir.path().join(format!("{me}.sock"));
        std::fs::write(&sock, b"").unwrap();
        let good = serde_json::json!({"v":1,"pid":me,"socket":sock,"sessionFile":"/s.jsonl","cwd":"/p"});
        std::fs::write(dir.path().join(format!("{me}.json")), good.to_string()).unwrap();
        // pid mismatch with file name, dead pid, garbage, missing socket
        std::fs::write(dir.path().join("1.json"), serde_json::json!({"pid":2,"socket":sock}).to_string()).unwrap();
        std::fs::write(dir.path().join("4000000.json"), serde_json::json!({"pid":4000000,"socket":sock}).to_string()).unwrap();
        std::fs::write(dir.path().join("77.json"), "{nope").unwrap();
        let records = read_bridges(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pid, me);
        assert_eq!(records[0].session_file.as_deref(), Some("/s.jsonl"));
        assert!(find_bridge(dir.path(), me).is_some());
        assert!(find_bridge(dir.path(), 1).is_none());
    }

    async fn read_control<R: AsyncRead + Unpin>(read: &mut R) -> Control {
        let (kind, bytes) = crate::protocol::read_frame(read).await.unwrap();
        assert_eq!(kind, 0);
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn relays_lines_both_ways_and_ends_on_bridge_close() {
        let (phone_side, host_side) = tokio::io::duplex(1 << 20);
        let (bridge_side, pi_side) = tokio::io::duplex(1 << 20);
        let (mut host_read, mut host_write) = tokio::io::split(host_side);
        let relay_task = tokio::spawn(async move { relay(&mut host_read, &mut host_write, bridge_side).await });
        let (mut phone_read, mut phone_write) = tokio::io::split(phone_side);
        let (pi_read, mut pi_write) = tokio::io::split(pi_side);
        let mut pi_lines = BufReader::new(pi_read).lines();

        assert!(matches!(read_control(&mut phone_read).await, Control::Ok { .. }));
        pi_write.write_all(b"{\"t\":\"hello\"}\n{\"t\":\"delta\",\"text\":\"a\"}\n").await.unwrap();
        match read_control(&mut phone_read).await {
            Control::Chat { line } => assert_eq!(line, "{\"t\":\"hello\"}"),
            other => panic!("{other:?}"),
        }
        match read_control(&mut phone_read).await {
            Control::Chat { line } => assert_eq!(line, "{\"t\":\"delta\",\"text\":\"a\"}"),
            other => panic!("{other:?}"),
        }
        // Phone → pi; embedded newlines can't split a line.
        let chat = Control::Chat { line: "{\"t\":\"prompt\",\n\"text\":\"hi\"}".into() };
        phone_write.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&chat).unwrap())).await.unwrap();
        assert_eq!(pi_lines.next_line().await.unwrap().unwrap(), "{\"t\":\"prompt\",\"text\":\"hi\"}");
        // pi quits → exit.
        drop(pi_write);
        drop(pi_lines);
        assert!(matches!(read_control(&mut phone_read).await, Control::Exit { code: None }));
        relay_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn oversized_bridge_lines_are_dropped() {
        let (phone_side, host_side) = tokio::io::duplex(4 << 20);
        let (bridge_side, pi_side) = tokio::io::duplex(4 << 20);
        let (mut host_read, mut host_write) = tokio::io::split(host_side);
        tokio::spawn(async move { relay(&mut host_read, &mut host_write, bridge_side).await });
        let (mut phone_read, _phone_write) = tokio::io::split(phone_side);
        let (_pi_read, mut pi_write) = tokio::io::split(pi_side);
        assert!(matches!(read_control(&mut phone_read).await, Control::Ok { .. }));
        let mut big = vec![b'x'; MAX_LINE + 10];
        big.push(b'\n');
        pi_write.write_all(&big).await.unwrap();
        pi_write.write_all(b"{\"t\":\"ok\"}\n").await.unwrap();
        match read_control(&mut phone_read).await {
            Control::Chat { line } => assert_eq!(line, "{\"t\":\"ok\"}"),
            other => panic!("{other:?}"),
        }
    }
}
