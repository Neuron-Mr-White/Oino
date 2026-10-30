//! Start and close pi chats from the app (UNI-260; docs/m5/PROTOCOL.md §10).
//!
//! `chat_new`: open a herdr tab (existing or new workspace) or a tmux
//! window/session in a folder, type the PC's launcher (`config.json`
//! `chat.launcher`, default `pi`) into its shell — so the user's login
//! environment (PATH, mise, …) applies — and wait for that pi's app-bridge
//! record. The new pi is recognised by process ancestry: its pid descends
//! from the pane's shell pid (herdr `pane.process_info`, tmux `#{pane_pid}`).
//!
//! `chat_close`: SIGTERM the pi (pi treats it as a normal quit: session
//! saved, extensions shut down), wait for it to exit, then close its herdr
//! tab / tmux window — only the pane when the tab/window has others.
//! Nothing is ever force-killed; a pi that won't stop is reported.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use unipi_proto::{
    AppErrorCode, AppErrorDto, ChatCloseData, ChatClosed, ChatConfigData, ChatNewData, ChatNewTarget, MuxKind,
    CHAT_LAUNCHER_DEFAULT,
};

use crate::chat::{bridge_dir, read_bridges, BridgeRecord};
use crate::mux::{herdr, tmux};

/// How long `chat_new` waits for the pi's bridge record.
pub const BRIDGE_WAIT: Duration = Duration::from_secs(45);
/// How long `chat_close` waits for pi to exit after SIGTERM.
pub const STOP_WAIT: Duration = Duration::from_secs(12);
const POLL: Duration = Duration::from_millis(250);

fn err(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}
fn internal(message: impl std::fmt::Display) -> AppErrorDto {
    err(AppErrorCode::E_INTERNAL, message.to_string())
}

// ── config.json `chat` block ────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSettings {
    /// The command new chats run; absent/empty = `pi`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    chat: ChatSettings,
}

impl ChatSettings {
    pub fn load(path: &Path) -> anyhow::Result<ChatSettings> {
        Ok(crate::data_dir::read_json::<ConfigFile>(path)?.unwrap_or_default().chat)
    }
    /// Writes the `chat` block back, keeping every other key of `config.json`.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut value: serde_json::Value = crate::data_dir::read_json(path)?.unwrap_or_else(|| serde_json::json!({}));
        if !value.is_object() {
            value = serde_json::json!({});
        }
        value["chat"] = serde_json::to_value(self)?;
        crate::data_dir::write_json(path, &value)
    }
    pub fn launcher(&self) -> &str {
        self.launcher.as_deref().map(str::trim).filter(|l| !l.is_empty()).unwrap_or(CHAT_LAUNCHER_DEFAULT)
    }
}

/// `chat_config`: the launcher plus which multiplexers can host a new chat.
pub async fn config(path: &Path) -> Result<ChatConfigData, AppErrorDto> {
    let settings = ChatSettings::load(path).map_err(internal)?;
    let (herdr_ok, tmux_ok) = tokio::join!(herdr_available(), tmux_available());
    Ok(ChatConfigData {
        launcher: settings.launcher().to_string(),
        default: settings.launcher.as_deref().map(str::trim).unwrap_or("").is_empty(),
        herdr: herdr_ok,
        tmux: tmux_ok,
    })
}

/// `chat_config_set`: stores the launcher (`""` = default) and answers the new config.
pub async fn set_launcher(path: &Path, launcher: &str) -> Result<ChatConfigData, AppErrorDto> {
    let mut settings = ChatSettings::load(path).map_err(internal)?;
    let trimmed = launcher.trim();
    settings.launcher = (!trimmed.is_empty()).then(|| trimmed.to_string());
    settings.save(path).map_err(internal)?;
    config(path).await
}

async fn herdr_available() -> bool {
    herdr::session_list().await.map(|s| s.iter().any(|s| s.running())).unwrap_or(false)
}

async fn tmux_available() -> bool {
    tmux::command()
        .arg("-V")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── process ancestry ────────────────────────────────────────────────────

/// The parent pid of `pid` (Linux /proc; a Toolhelp snapshot on Windows;
/// `ps` elsewhere).
pub fn parent_pid(pid: u32) -> Option<u32> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };
        // SAFETY: the snapshot handle is checked and closed; the entry is a
        // plain struct sized as the API requires.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                if entry.th32ProcessID == pid {
                    found = Some(entry.th32ParentProcessID);
                    break;
                }
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            return found;
        }
    }
    #[allow(unreachable_code)]
    let proc_stat = PathBuf::from(format!("/proc/{pid}/stat"));
    if let Ok(text) = std::fs::read_to_string(&proc_stat) {
        // "pid (comm with spaces) S ppid …": split after the last ')'.
        let rest = &text[text.rfind(')')? + 1..];
        return rest.split_whitespace().nth(1)?.parse().ok();
    }
    if Path::new("/proc/self").exists() {
        return None; // Linux, and the process is gone.
    }
    let out = std::process::Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Whether `ancestor` is `pid` or one of its ancestors (≤ 64 hops).
pub fn descends_from(pid: u32, ancestor: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        if current == ancestor {
            return true;
        }
        match parent_pid(current) {
            Some(parent) if parent > 1 && parent != current => current = parent,
            _ => return false,
        }
    }
    false
}

pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            && !is_zombie(pid)
    }
    #[cfg(windows)]
    {
        // OpenProcess + GetExitCodeProcess: the only liveness check that
        // works without a /proc or `kill`.
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: both handles are checked before use and closed below.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            ok && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|t| t.rfind(')').map(|i| t[i + 1..].trim_start().starts_with('Z')))
        .unwrap_or(false)
}

fn sigterm(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Waits for a bridge whose pid descends from `shell_pid` (and was not
/// there before). `None` after `limit`.
pub async fn wait_for_bridge(dir: &Path, shell_pid: u32, pane_id: Option<&str>, before: &[u32], limit: Duration) -> Option<BridgeRecord> {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        // A new bridge is ours when it runs in the pane we opened (herdr
        // records `herdrPaneId`) or descends from that pane's shell. The pane
        // match is the only one that works on Windows, which has no cheap
        // parent-pid lookup (no /proc, no `ps`).
        if let Some(found) = read_bridges(dir).into_iter().find(|r| {
            !before.contains(&r.pid)
                && (pane_id.is_some_and(|p| r.herdr_pane_id.as_deref() == Some(p)) || descends_from(r.pid, shell_pid))
        })
        {
            return Some(found);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(POLL).await;
    }
}

// ── chat_new ────────────────────────────────────────────────────────────

/// Where the shell for the new pi was opened.
#[derive(Debug, Clone)]
struct Opened {
    mux: MuxKind,
    session: String,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    pane_id: String,
    shell_pid: u32,
    /// herdr socket (to type into the pane / close it on failure).
    socket: Option<String>,
}

fn label_for(cwd: &Path) -> String {
    cwd.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "pi".into())
}

fn field(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut v = value;
    for key in path {
        v = v.get(key)?;
    }
    v.as_str().map(str::to_string)
}

async fn herdr_shell_pid(socket: &str, pane_id: &str) -> Result<u32, AppErrorDto> {
    // The shell may need a moment to start.
    for _ in 0..20 {
        let reply = herdr::socket_call(socket, "pane.process_info", serde_json::json!({ "pane_id": pane_id })).await?;
        if let Some(pid) = reply.pointer("/process_info/shell_pid").and_then(|v| v.as_u64()) {
            if pid > 0 {
                return Ok(pid as u32);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(internal("herdr didn't report the new pane's shell"))
}

async fn open_shell(target: &ChatNewTarget, cwd: &Path) -> Result<Opened, AppErrorDto> {
    let cwd_text = cwd.to_string_lossy().into_owned();
    match target {
        ChatNewTarget::HerdrWorkspace { session, workspace_id } => {
            let socket = herdr::socket_for(session.as_str()).await?;
            let reply = herdr::socket_call(
                &socket,
                "tab.create",
                serde_json::json!({ "workspace_id": workspace_id.as_str(), "cwd": cwd_text, "label": "pi", "focus": false }),
            )
            .await?;
            let pane_id = field(&reply, &["root_pane", "pane_id"]).ok_or_else(|| internal("herdr tab.create: no pane"))?;
            let shell_pid = herdr_shell_pid(&socket, &pane_id).await?;
            Ok(Opened {
                mux: MuxKind::Herdr,
                session: session.as_str().to_string(),
                workspace_id: Some(workspace_id.as_str().to_string()),
                tab_id: field(&reply, &["tab", "tab_id"]).or_else(|| field(&reply, &["root_pane", "tab_id"])),
                pane_id,
                shell_pid,
                socket: Some(socket),
            })
        }
        ChatNewTarget::HerdrNewWorkspace { session, label } => {
            let socket = herdr::socket_for(session.as_str()).await?;
            let label = label.as_ref().map(|l| l.as_str().to_string()).unwrap_or_else(|| label_for(cwd));
            let reply = herdr::socket_call(
                &socket,
                "workspace.create",
                serde_json::json!({ "cwd": cwd_text, "label": label, "focus": false }),
            )
            .await?;
            let pane_id = field(&reply, &["root_pane", "pane_id"]).ok_or_else(|| internal("herdr workspace.create: no pane"))?;
            let shell_pid = herdr_shell_pid(&socket, &pane_id).await?;
            Ok(Opened {
                mux: MuxKind::Herdr,
                session: session.as_str().to_string(),
                workspace_id: field(&reply, &["workspace", "workspace_id"]),
                tab_id: field(&reply, &["tab", "tab_id"]),
                pane_id,
                shell_pid,
                socket: Some(socket),
            })
        }
        ChatNewTarget::TmuxWindow { session } => {
            let out = tmux::run(&[
                "new-window", "-d", "-t", &format!("{}:", session.as_str()), "-c", &cwd_text, "-n", "pi", "-P", "-F",
                "#{window_id} #{pane_id} #{pane_pid}",
            ])
            .await?;
            tmux_opened(session.as_str(), &out)
        }
        ChatNewTarget::TmuxNewSession { name } => {
            let out = tmux::run(&[
                "new-session", "-d", "-s", name.as_str(), "-c", &cwd_text, "-n", "pi", "-P", "-F",
                "#{window_id} #{pane_id} #{pane_pid}",
            ])
            .await?;
            tmux_opened(name.as_str(), &out)
        }
    }
}

fn tmux_opened(session: &str, out: &[u8]) -> Result<Opened, AppErrorDto> {
    let text = String::from_utf8_lossy(out);
    let mut parts = text.split_whitespace();
    let (Some(window), Some(pane), Some(pid)) = (parts.next(), parts.next(), parts.next().and_then(|p| p.parse().ok())) else {
        return Err(internal(format!("tmux didn't report the new window: {text}")));
    };
    Ok(Opened {
        mux: MuxKind::Tmux,
        session: session.to_string(),
        workspace_id: Some(window.to_string()),
        tab_id: None,
        pane_id: pane.to_string(),
        shell_pid: pid,
        socket: None,
    })
}

async fn type_line(opened: &Opened, line: &str) -> Result<(), AppErrorDto> {
    match opened.mux {
        MuxKind::Herdr => {
            herdr::socket_call(
                opened.socket.as_deref().unwrap_or_default(),
                "pane.send_input",
                serde_json::json!({ "pane_id": opened.pane_id, "text": line, "keys": ["enter"] }),
            )
            .await?;
        }
        MuxKind::Tmux => {
            tmux::run(&["send-keys", "-t", &opened.pane_id, "-l", line]).await?;
            tmux::run(&["send-keys", "-t", &opened.pane_id, "Enter"]).await?;
        }
    }
    Ok(())
}

/// `chat_new`: see the module docs. `cwd` is already scope-checked.
pub async fn new_chat(target: &ChatNewTarget, cwd: &Path, launcher: &str, wait: Duration) -> Result<ChatNewData, AppErrorDto> {
    new_chat_line(target, cwd, launcher, launcher, wait).await
}

/// Single-quotes `value` for a POSIX shell line (`'` → `'\''`).
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// UNI-319: `<launcher> --session '<file>'` — pi on an existing session.
pub fn session_line(launcher: &str, session_file: &str) -> String {
    format!("{launcher} --session {}", shell_quote(session_file))
}

/// [`new_chat`] typing `line` (the launcher plus any arguments) instead
/// of the bare launcher; `launcher` names it in the error.
pub async fn new_chat_line(target: &ChatNewTarget, cwd: &Path, line: &str, launcher: &str, wait: Duration) -> Result<ChatNewData, AppErrorDto> {
    let dir = bridge_dir().ok_or_else(|| internal("No home directory on the host"))?;
    let before: Vec<u32> = read_bridges(&dir).iter().map(|r| r.pid).collect();
    let opened = open_shell(target, cwd).await?;
    type_line(&opened, line).await?;
    let where_ = serde_json::json!({
        "mux": opened.mux, "session": opened.session, "workspace_id": opened.workspace_id,
        "tab_id": opened.tab_id, "pane_id": opened.pane_id,
    });
    let herdr_pane = (opened.mux == MuxKind::Herdr).then_some(opened.pane_id.as_str());
    match wait_for_bridge(&dir, opened.shell_pid, herdr_pane, &before, wait).await {
        Some(record) => Ok(ChatNewData {
            pid: record.pid,
            mux: opened.mux,
            session: opened.session,
            workspace_id: opened.workspace_id,
            tab_id: opened.tab_id,
            pane_id: opened.pane_id,
            cwd: record.cwd.or_else(|| Some(cwd.to_string_lossy().into_owned())),
        }),
        None => Err(AppErrorDto::new(
            AppErrorCode::E_CHAT_UNAVAILABLE,
            format!("`{launcher}` started in a new {} but no pi chat appeared", if opened.mux == MuxKind::Herdr { "tab" } else { "window" }),
            true,
        )
        .with_hint("Open it in the terminal to see what happened. Check the launcher in Settings → PCs, and that UniPi is installed in pi.")
        .with_detail(where_)),
    }
}

// ── chat_close ──────────────────────────────────────────────────────────

/// Where a running pi lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PiPlace {
    Herdr { socket: String, session: String, pane_id: String },
    Tmux { pane_id: String, window_id: String, session: String },
    Unknown,
}

/// tmux pane (`%7`), window (`@3`), session and pane pid for every pane.
pub async fn tmux_panes() -> Vec<(String, String, String, u32)> {
    let Ok(out) = tmux::run(&["list-panes", "-a", "-F", "#{pane_id}\u{241f}#{window_id}\u{241f}#{session_name}\u{241f}#{pane_pid}"]).await else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\u{241f}').collect();
            (f.len() == 4).then(|| (f[0].to_string(), f[1].to_string(), f[2].to_string(), f[3].parse().unwrap_or(0)))
        })
        .collect()
}

pub async fn place_of(record: &BridgeRecord) -> PiPlace {
    if let (Some(pane), Some(socket)) = (&record.herdr_pane_id, &record.herdr_socket) {
        let sessions = herdr::session_list().await.unwrap_or_default();
        if let Some(session) = sessions.into_iter().find(|s| s.socket_path() == socket.as_str()) {
            // HERDR_PANE_ID is inherited: a pi in tmux inside a herdr pane
            // carries the outer pane's id. Only trust it when pi really runs
            // under that pane's shell — never close someone else's tab.
            let shell = herdr_shell_pid(socket, pane).await.ok();
            if shell.is_some_and(|shell| descends_from(record.pid, shell)) {
                return PiPlace::Herdr { socket: socket.clone(), session: session.name().to_string(), pane_id: pane.clone() };
            }
        }
    }
    for (pane_id, window_id, session, pid) in tmux_panes().await {
        if pid > 0 && descends_from(record.pid, pid) {
            return PiPlace::Tmux { pane_id, window_id, session };
        }
    }
    PiPlace::Unknown
}

impl PiPlace {
    pub fn session(&self) -> Option<&str> {
        match self {
            PiPlace::Herdr { session, .. } | PiPlace::Tmux { session, .. } => Some(session),
            PiPlace::Unknown => None,
        }
    }
}

/// Stops pi with SIGTERM and waits for it to go.
pub async fn stop_pi(pid: u32, limit: Duration) -> bool {
    if !pid_alive(pid) {
        return true;
    }
    sigterm(pid);
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    !pid_alive(pid)
}

async fn close_place(place: &PiPlace) -> Result<ChatClosed, AppErrorDto> {
    match place {
        PiPlace::Herdr { socket, pane_id, .. } => {
            let reply = herdr::socket_call(socket, "session.snapshot", serde_json::json!({})).await?;
            let snapshot: herdr::RawSnapshot = serde_json::from_value(reply.get("snapshot").cloned().unwrap_or_default())
                .map_err(|e| internal(format!("herdr snapshot: {e}")))?;
            let Some(pane) = snapshot.panes.iter().find(|p| &p.pane_id == pane_id) else {
                return Ok(ChatClosed::None);
            };
            let siblings = snapshot.panes.iter().filter(|p| p.tab_id == pane.tab_id).count();
            if siblings > 1 {
                herdr::socket_call(socket, "pane.close", serde_json::json!({ "pane_id": pane_id })).await?;
                Ok(ChatClosed::Pane)
            } else {
                herdr::socket_call(socket, "tab.close", serde_json::json!({ "tab_id": pane.tab_id })).await?;
                Ok(ChatClosed::Tab)
            }
        }
        PiPlace::Tmux { pane_id, window_id, .. } => {
            let panes = tmux_panes().await;
            if !panes.iter().any(|p| &p.0 == pane_id) {
                return Ok(ChatClosed::None);
            }
            let siblings = panes.iter().filter(|p| &p.1 == window_id).count();
            if siblings > 1 {
                tmux::run(&["kill-pane", "-t", pane_id]).await?;
                Ok(ChatClosed::Pane)
            } else {
                tmux::run(&["kill-window", "-t", window_id]).await?;
                Ok(ChatClosed::Window)
            }
        }
        PiPlace::Unknown => Ok(ChatClosed::None),
    }
}

/// `chat_close` after the place is known and allowed.
pub async fn close_chat(pid: u32, place: &PiPlace, close_tab: bool, limit: Duration) -> Result<ChatCloseData, AppErrorDto> {
    if !stop_pi(pid, limit).await {
        return Err(AppErrorDto::new(
            AppErrorCode::E_TIMEOUT,
            format!("pi didn't stop within {} s", limit.as_secs()),
            true,
        )
        .with_hint("It may be finishing a tool call. Try again, or stop it from the terminal."));
    }
    let closed = if close_tab { close_place(place).await? } else { ChatClosed::None };
    Ok(ChatCloseData { stopped: true, closed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_defaults_and_trims() {
        assert_eq!(ChatSettings::default().launcher(), "pi");
        assert_eq!(ChatSettings { launcher: Some("  ".into()) }.launcher(), "pi");
        assert_eq!(ChatSettings { launcher: Some(" unipi ".into()) }.launcher(), "unipi");
    }

    #[test]
    fn settings_save_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"forwarding":{"enabled":true}}"#).unwrap();
        ChatSettings { launcher: Some("unipi".into()) }.save(&path).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["forwarding"]["enabled"], true);
        assert_eq!(value["chat"]["launcher"], "unipi");
        assert_eq!(ChatSettings::load(&path).unwrap().launcher(), "unipi");
    }

    #[test]
    fn ancestry_finds_our_child() {
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        let me = std::process::id();
        assert_eq!(parent_pid(child.id()), Some(me));
        assert!(descends_from(child.id(), me));
        assert!(!descends_from(me, child.id()));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn tmux_output_parses() {
        let o = tmux_opened("work", b"@3 %7 4242\n").unwrap();
        assert_eq!((o.workspace_id.as_deref(), o.pane_id.as_str(), o.shell_pid), (Some("@3"), "%7", 4242));
        assert!(tmux_opened("work", b"garbage").is_err());
    }

    #[test]
    fn session_line_quotes_the_file() {
        assert_eq!(session_line("pi", "/home/u/.pi/agent/sessions/--tmp--/a.jsonl"), "pi --session '/home/u/.pi/agent/sessions/--tmp--/a.jsonl'");
        assert_eq!(session_line("unipi", "/x/it's.jsonl"), "unipi --session '/x/it'\\''s.jsonl'");
    }

    #[test]
    fn folder_label() {
        assert_eq!(label_for(Path::new("/home/u/proj")), "proj");
        assert_eq!(label_for(Path::new("/")), "pi");
    }
}
