//! Interactive mode: `oino` on a pseudo-terminal starts the TUI, runs a prompt through the
//! faux provider and exits on `ctrl+d` (pi interactive mode, modes §7).
//!
//! Needs util-linux `setsid --ctty` so the child gets the pty as its controlling terminal
//! (crossterm reads `/dev/tty`); the test is skipped when it is missing.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    missing_docs,
    reason = "integration test"
)]

mod support;

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::pty::{Winsize, openpty};
use serde_json::json;
use support::{Sandbox, oino_bin};

fn has_setsid_ctty() -> bool {
    Command::new("setsid")
        .arg("--help")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--ctty"))
}

/// Visible text: escape sequences dropped, whitespace collapsed (cells are drawn with cursor
/// moves, so words of one line may not be adjacent in the byte stream).
fn visible(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() || c == '~' {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for c in chars.by_ref() {
                        if c == '\u{7}' {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<String>()
}

fn wait_for(output: &Mutex<Vec<u8>>, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if visible(&output.lock().unwrap()).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn interactive_mode_runs_a_prompt_and_exits() {
    if !has_setsid_ctty() {
        eprintln!("skipped: setsid --ctty not available");
        return;
    }
    let sandbox = Sandbox::new();
    let size = Winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = openpty(Some(&size), None).unwrap();
    let base = sandbox.command(&json!([{"text": "Hello from faux"}]), &[]);
    let mut command = Command::new("setsid");
    command
        .arg("--ctty")
        .arg(oino_bin())
        .args(base.get_args())
        .args(["--model", "faux/faux-1"])
        .current_dir(sandbox.cwd())
        .env_clear()
        .envs(base.get_envs().filter_map(|(k, v)| Some((k, v?))))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave));
    let mut child = command.spawn().unwrap();
    let mut master = std::fs::File::from(pty.master);
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut reader = master.try_clone().unwrap();
    let sink = Arc::clone(&output);
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });

    let timeout = Duration::from_secs(20);
    assert!(
        wait_for(&output, "faux-1", timeout),
        "footer with the model: {}",
        visible(&output.lock().unwrap())
    );
    master.write_all(b"hi there").unwrap();
    assert!(wait_for(&output, "hithere", timeout));
    master.write_all(b"\r").unwrap();
    assert!(
        wait_for(&output, "Hellofromfaux", timeout),
        "assistant reply: {}",
        visible(&output.lock().unwrap())
    );
    master.write_all(b"\x04").unwrap();
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "oino did not exit on ctrl+d");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "exit status {status}");
    let sessions = std::fs::read_dir(sandbox.agent().join("sessions"))
        .unwrap()
        .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
        .count();
    assert_eq!(sessions, 1, "the prompt was persisted to a session file");
}
