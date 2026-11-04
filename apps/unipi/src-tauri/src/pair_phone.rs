//! "Pair a phone" from the desktop app (UNI-266).
//!
//! - `pair_phone_start { onEvent }` runs this PC's own
//!   `unipi-host pair --json --ttl 10m` (found like sharing.rs does) and
//!   forwards each JSON line to the window:
//!   `{"state":"waiting","link","code","expires_at","host"}`, then
//!   `{"state":"paired","device"}` / `{"state":"expired","reason"}` /
//!   `{"state":"error","message","hint?"}`.
//! - `pair_phone_cancel` closes the child's stdin (unipi-host then removes
//!   the code, so the QR on screen stops working) and kills it if it lingers.
//!
//! One attempt at a time: a new start cancels the previous one.

use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::State;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use unipi_proto::{AppErrorCode, AppErrorDto};

/// The argv; fixed (nothing from the UI).
pub const ARGS: [&str; 4] = ["pair", "--json", "--ttl", "10m"];

#[derive(Default)]
pub struct PairPhone {
    current: Arc<Mutex<Option<Running>>>,
}

struct Running {
    generation: u64,
    stdin: Option<tokio::process::ChildStdin>,
    kill: tokio::sync::oneshot::Sender<()>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl PairPhone {
    fn stop(&self) {
        if let Some(mut running) = lock(&self.current).take() {
            // EOF on stdin = "cancelled" for unipi-host (it clears the code).
            drop(running.stdin.take());
            let kill = running.kill;
            // Runs from a sync command (main thread, no tokio context): Tauri's runtime.
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let _ = kill.send(());
            });
        }
    }
}

/// What a finished child left behind, as the UI's last event (when it
/// didn't print a final state itself).
pub fn exit_event(stderr: &str, saw_waiting: bool) -> Value {
    if stderr.contains("unexpected argument") || stderr.contains("unrecognized") {
        return json!({"state":"error","code":"E_UNSUPPORTED","message":"This PC's agent host is too old to pair from here","hint":"Update Unipi (Settings › About)"});
    }
    let message = stderr
        .lines()
        .map(|l| l.trim().trim_start_matches("Error: ").to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_else(|| {
            if saw_waiting {
                "unipi-host stopped".into()
            } else {
                "unipi-host didn't start pairing".into()
            }
        });
    json!({"state":"error","message":message})
}

/// Is this line a final state?
pub fn is_final(event: &Value) -> bool {
    matches!(
        event["state"].as_str(),
        Some("paired" | "expired" | "error")
    )
}

#[tauri::command]
pub async fn pair_phone_start(
    state: State<'_, PairPhone>,
    on_event: Channel<Value>,
) -> Result<(), AppErrorDto> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "Only the desktop app can pair a phone with this PC",
            false,
        ));
    }
    state.stop();
    let bin = crate::sharing::resolve_bin()?;
    let mut command = crate::no_window::tokio_command(&bin);
    command
        .args(ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!("couldn't run unipi-host: {e}"),
            false,
        )
    })?;
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel();
    *lock(&state.current) = Some(Running {
        generation,
        stdin: child.stdin.take(),
        kill: kill_tx,
    });
    let stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let current = state.current.clone();
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let (mut saw_waiting, mut finished) = (false, false);
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                        saw_waiting |= event["state"] == "waiting";
                        finished |= is_final(&event);
                        let _ = on_event.send(event);
                    }
                    _ => break,
                },
                _ = &mut kill_rx => {
                    let _ = child.start_kill();
                    break;
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        let _ = child.start_kill();
        if !finished {
            let mut err = String::new();
            let _ =
                tokio::time::timeout(Duration::from_secs(1), stderr.read_to_string(&mut err)).await;
            let _ = on_event.send(exit_event(&err, saw_waiting));
        }
        let mut current = lock(&current);
        if current.as_ref().is_some_and(|r| r.generation == generation) {
            *current = None;
        }
    });
    Ok(())
}

#[tauri::command]
pub fn pair_phone_cancel(state: State<'_, PairPhone>) {
    state.stop();
}

/// The fixed command that starts the installers' background service.
pub fn start_service_argv(os: &str) -> Option<Vec<String>> {
    let s = |v: &[&str]| Some(v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    match os {
        "linux" => s(&["systemctl", "--user", "start", "unipi-host"]),
        "windows" => s(&["schtasks", "/Run", "/TN", "UniPi Host"]),
        "macos" => {
            let uid = std::process::Command::new("id")
                .arg("-u")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;
            s(&[
                "launchctl",
                "kickstart",
                &format!("gui/{uid}/dev.unipi.host"),
            ])
        }
        _ => None,
    }
}

/// "Start it" in the dialog: starts the unipi-host service the installer set
/// up (systemd user unit / logon task / LaunchAgent), then waits until serve
/// has written a fresh endpoint.json (up to 10 s).
#[tauri::command]
pub async fn local_host_start() -> Result<(), AppErrorDto> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "Only the desktop app can start this PC's host",
            false,
        ));
    }
    let argv = start_service_argv(std::env::consts::OS).ok_or_else(|| {
        AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "Can't start unipi-host here",
            false,
        )
    })?;
    let started = std::time::SystemTime::now() - Duration::from_secs(1);
    let mut command = crate::no_window::tokio_command(&argv[0]);
    command.args(&argv[1..]).stdin(Stdio::null());
    let out = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .map_err(|_| {
            AppErrorDto::new(
                AppErrorCode::E_TIMEOUT,
                "Starting unipi-host took too long",
                true,
            )
        })?
        .map_err(|e| {
            AppErrorDto::new(
                AppErrorCode::E_INTERNAL,
                format!("couldn't run {}: {e}", argv[0]),
                false,
            )
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("failed")
            .to_string();
        return Err(AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!("`{}` failed: {first}", argv.join(" ")),
            false,
        )
        .with_hint("Run `unipi-host serve` in a terminal, or re-run the installer"));
    }
    // serve rewrites endpoint.json when it starts (a stale one may be there).
    let endpoint = crate::sharing::host_dir().join("endpoint.json");
    let fresh = || {
        endpoint
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= started)
    };
    for _ in 0..40 {
        if fresh() {
            // serve writes endpoint.json just before it opens the control channel.
            tokio::time::sleep(Duration::from_millis(500)).await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_argv_and_exit_events() {
        assert_eq!(ARGS.join(" "), "pair --json --ttl 10m");
        let e = exit_event("error: unexpected argument '--json' found\n", false);
        assert_eq!(e["code"], "E_UNSUPPORTED");
        let e = exit_event("Error: something broke\n", true);
        assert_eq!(e, json!({"state":"error","message":"something broke"}));
        assert_eq!(
            exit_event("", false)["message"],
            "unipi-host didn't start pairing"
        );
        assert!(is_final(&json!({"state":"paired","device":"x"})));
        assert!(!is_final(&json!({"state":"waiting"})));
        // Cancel comes from a sync command on the main thread: no tokio runtime there.
        let state = PairPhone::default();
        let (kill, _rx) = tokio::sync::oneshot::channel();
        *lock(&state.current) = Some(Running {
            generation: 1,
            stdin: None,
            kill,
        });
        state.stop();
        assert!(lock(&state.current).is_none());
        assert_eq!(
            start_service_argv("linux").unwrap().join(" "),
            "systemctl --user start unipi-host"
        );
        assert_eq!(
            start_service_argv("windows").unwrap().join(" "),
            "schtasks /Run /TN UniPi Host"
        );
        assert_eq!(start_service_argv("android"), None);
    }
}
