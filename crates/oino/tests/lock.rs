//! WP-C3 e2e: the single-writer lease and takeover between two real `oino` processes
//! (spec core.md §2.1 D-LOCK / D-TAKEOVER). A responsive runner (`--mode rpc`) closes its
//! session when taken over so the second open reloads it within the 10 s deadline; a
//! stopped (unresponsive) holder yields the `SessionLocked` text; a killed holder releases
//! the kernel lock immediately.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod support;

use support::Sandbox;

const TIMEOUT: Duration = Duration::from_secs(30);

/// The one `*.jsonl` session file under `root`.
fn only_session_file(root: &Path) -> PathBuf {
    let mut found = Vec::new();
    walk(root, &mut found);
    let file = found.into_iter().next().expect("exactly one session file");
    assert!(file.to_string_lossy().ends_with(".jsonl"));
    file
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

fn session_id(file: &Path) -> String {
    let header: Value = serde_json::from_str(
        std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    header["id"].as_str().unwrap().to_owned()
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Waits until the lock metadata of `file` exists (the holder opened the session).
fn wait_for_holder(file: &Path) -> Value {
    let metadata = file.with_file_name(format!(
        "{}.lock.json",
        file.file_name().unwrap().to_string_lossy()
    ));
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&metadata) {
            return serde_json::from_str(&text).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "no lease holder appeared for {}",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Creates one session in a fresh sandbox and returns (sandbox, sessions dir, file, id).
fn held_session(first_response: &str) -> (Sandbox, PathBuf, PathBuf, String) {
    let sandbox = Sandbox::new();
    let sessions = sandbox.dir.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let output = sandbox.run(
        &json!([{"text": first_response}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "-p",
            "first message",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let file = only_session_file(&sessions);
    let id = session_id(&file);
    (sandbox, sessions, file, id)
}

/// A second `oino` process takes the first's session over: the holder (an idle `--mode rpc`
/// runner) is asked to close through `<file>.takeover`, its watcher disposes the session and
/// releases the lease, and the second open reloads the session here within the deadline.
#[test]
fn second_open_closes_the_first_runner_and_reloads_here() {
    let (sandbox, sessions, file, id) = held_session("first reply");
    let mut holder = sandbox.rpc(
        &json!([{"text": "holder reply"}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--session",
            &id,
        ],
    );
    let holder_info = wait_for_holder(&file);
    assert!(holder_info["pid"].as_u64().unwrap() > 0);

    // The second runner opens the same session from another process.
    let started = Instant::now();
    let output = sandbox.run(
        &json!([{"text": "second reply"}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--session",
            &id,
            "-p",
            "second message",
        ],
        None,
    );
    let elapsed = started.elapsed();
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(
        elapsed < Duration::from_secs(10),
        "reload here within the takeover deadline (took {elapsed:?})"
    );
    assert_eq!(stdout_of(&output), "second reply\n");

    // The session continued here: the file now carries the second message.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("second message"), "{text}");
    assert!(text.contains("first message"), "{text}");
    // The holder observed the takeover (its runtime closed the session).
    let _ = holder.try_next(Duration::from_millis(200));
    drop(holder); // kill the idle rpc runner
}

/// A stopped (unresponsive) holder never answers the takeover request: the deadline expires
/// and the second open fails with the precise `SessionLocked` text naming the holder.
#[cfg(unix)]
#[test]
fn unresponsive_holder_yields_the_session_locked_text() {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let (sandbox, sessions, file, id) = held_session("first reply");
    let holder = sandbox.rpc(
        &json!([{"text": "holder reply"}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--session",
            &id,
        ],
    );
    let holder_info = wait_for_holder(&file);
    let pid = holder_info["pid"].as_u64().unwrap();
    let client = holder_info["client"].as_str().unwrap().to_owned();
    let host = holder_info["host"].as_str().unwrap().to_owned();

    // Freeze the holder: its takeover watcher cannot run.
    kill(Pid::from_raw(i32::try_from(pid).unwrap()), Signal::SIGSTOP).unwrap();
    let started = Instant::now();
    let output = sandbox.run(
        &json!([{"text": "second reply"}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--session",
            &id,
            "-p",
            "second message",
        ],
        None,
    );
    let elapsed = started.elapsed();
    assert_ne!(output.status.code(), Some(0), "the open must fail");
    assert!(
        elapsed >= Duration::from_secs(9),
        "the 10 s takeover deadline expired first ({elapsed:?})"
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains(&format!(
            "Session {id} is open in {client} (pid {pid} on {host}). It was asked to close \
             and did not respond; close it there and try again."
        )),
        "the exact SessionLocked text: {stderr}"
    );
    // Cleanup: resume and kill the frozen holder.
    kill(Pid::from_raw(i32::try_from(pid).unwrap()), Signal::SIGCONT).unwrap();
    drop(holder);
}

/// A holder killed hard releases the kernel lock: the next open proceeds (no lease expiry —
/// the kernel is the arbiter).
#[cfg(unix)]
#[test]
fn killed_holder_releases_the_kernel_lock() {
    let (sandbox, sessions, file, id) = held_session("first reply");
    let holder = sandbox.rpc(
        &json!([{"text": "holder reply"}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--session",
            &id,
        ],
    );
    let holder_info = wait_for_holder(&file);
    let pid = holder_info["pid"].as_u64().unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap()),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    drop(holder); // reaps the killed child

    let started = Instant::now();
    let output = sandbox.run(
        &json!([{"text": "second reply"}]),
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--session",
            &id,
            "-p",
            "second message",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a dead holder blocks nobody ({:?})",
        started.elapsed()
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("second message"), "{text}");
}
