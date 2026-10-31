//! UNI-228: one `serve` per data dir — racing serves leave exactly one
//! running, the loser exits 75 naming the owner; `status` reports
//! running / not-running; `--exit-with-parent` ends the host with its parent.
//! Temp data dirs only (never ~/.unipi/app-host), dead relay (no network).
#![cfg(unix)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_unipi-host")
}

fn serve(dir: &std::path::Path, extra: &[&str]) -> Child {
    Command::new(bin())
        .args([
            "--data-dir",
            dir.to_str().unwrap(),
            "serve",
            "--relay",
            "https://127.0.0.1:9",
            "--relay-only",
        ])
        .args(extra)
        .env("HOME", dir) // keep herdr/tmux/bridge lookups away from the real home
        .env_remove("INVOCATION_ID")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve")
}

fn status_json(dir: &std::path::Path) -> (i32, serde_json::Value) {
    let out = Command::new(bin())
        .args(["--data-dir", dir.to_str().unwrap(), "status", "--json"])
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

fn wait_running_for(dir: &std::path::Path, children: &mut [&mut Child]) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (code, v) = status_json(dir);
        if code == 0 {
            return v;
        }
        for c in children.iter_mut() {
            if let Some(st) = c.try_wait().unwrap() {
                let mut err = String::new();
                if let Some(e) = c.stderr.as_mut() {
                    let _ = std::io::Read::read_to_string(e, &mut err);
                }
                if st.code() != Some(75) {
                    panic!("serve exited early ({st}): {err}");
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "host never took the lock: {v} (code {code})"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[allow(dead_code)]
fn wait_running(dir: &std::path::Path) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (code, v) = status_json(dir);
        if code == 0 {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "host never took the lock: {v} (code {code})"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn kill(child: &mut Child) {
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn racing_serves_leave_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = serve(dir.path(), &["--mode", "app"]);
    let mut b = serve(dir.path(), &["--mode", "manual"]);
    let v = wait_running_for(dir.path(), &mut [&mut a, &mut b]);
    // Exactly one of them exits 75 with the owner's pid in its message.
    let deadline = Instant::now() + Duration::from_secs(15);
    let (loser, winner) = loop {
        if let Some(st) = a.try_wait().unwrap() {
            break ((a, st), b);
        }
        if let Some(st) = b.try_wait().unwrap() {
            break ((b, st), a);
        }
        assert!(Instant::now() < deadline, "neither serve gave up");
        std::thread::sleep(Duration::from_millis(50));
    };
    let (mut loser_child, loser_status) = loser;
    let mut winner = winner;
    assert_eq!(loser_status.code(), Some(75));
    let mut err = String::new();
    std::io::Read::read_to_string(loser_child.stderr.as_mut().unwrap(), &mut err).unwrap();
    assert!(
        err.contains("already running") && err.contains(&winner.id().to_string()),
        "{err}"
    );
    assert_eq!(v["state"], "running");
    assert_eq!(v["owner"]["pid"], winner.id());
    assert!(
        winner.try_wait().unwrap().is_none(),
        "the winner keeps running"
    );
    kill(&mut winner);
    let (code, after) = status_json(dir.path());
    assert_eq!(code, 3);
    assert_eq!(after["state"], "not-running");
}

#[test]
fn status_of_an_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let (code, v) = status_json(dir.path());
    assert_eq!(code, 3);
    assert_eq!(v, serde_json::json!({ "state": "not-running" }));
}

#[test]
fn exit_with_parent_follows_the_parent() {
    let dir = tempfile::tempdir().unwrap();
    // A stand-in parent that we can kill: `sleep`.
    let mut parent = Command::new("sleep").arg("60").spawn().unwrap();
    let mut host = serve(
        dir.path(),
        &[
            "--mode",
            "app",
            "--exit-with-parent",
            &parent.id().to_string(),
        ],
    );
    let v = wait_running_for(dir.path(), &mut [&mut host]);
    assert_eq!(v["owner"]["mode"], "app");
    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if host.try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host outlived its parent by > 3 s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(status_json(dir.path()).0, 3);
}
