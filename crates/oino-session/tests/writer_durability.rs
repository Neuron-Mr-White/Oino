//! D-WRITER / D-FLUSH durability (core spec §2.1, WP-C1): entries written and fsynced at
//! each barrier survive `kill -9` of the writer process and reopen without loss.
//!
//! The test re-executes its own binary as a child (env `OINO_WP_C1_CHILD`): the child
//! writes a session with a `sync()` at each of the three barrier shapes, drops readiness
//! marker files, then idles until the parent SIGKILLs it.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, StopReason, UserContentField, UserMessage,
};
use oino_session::entry::FileLine;
use oino_session::{CreateOptions, SessionManager};

const CHILD_ENV: &str = "OINO_WP_C1_CHILD_DIR";

const CWD: &str = "/work/wp-c1";

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        author: None,
        content: UserContentField::Text(text.into()),
        timestamp: 1,
    })
}

fn assistant(text: &str) -> AgentMessage {
    let mut message = AssistantMessage::empty("faux", "faux", "faux-1", 2);
    message.content = vec![AssistantContent::text(text)];
    message.stop_reason = StopReason::Stop;
    AgentMessage::Assistant(message)
}

/// The child writer: one barrier-shaped write per marker, then idle until killed.
fn child_writer(dir: &Path) {
    let mut sm = SessionManager::create(Path::new(CWD), dir, CreateOptions::default())
        .expect("child: create session");
    // Barrier 1 shape (before a model request): the user turn is durable first.
    sm.append_message(user("u1")).unwrap(); // first flush: write_new + sync + dir fsync
    sm.sync().unwrap();
    marker(dir, "request");
    // Barrier 2 shape (before a tool with side effects).
    sm.append_message(assistant("a1")).unwrap();
    sm.sync().unwrap();
    marker(dir, "tool");
    // Barrier 3 shape (at a step boundary).
    sm.append_message(user("u2")).unwrap();
    sm.sync().unwrap();
    marker(dir, "step");
    loop {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn marker(dir: &Path, name: &str) {
    std::fs::write(dir.join(format!("ready-{name}")), b"").unwrap();
}

fn wait_for_marker(dir: &Path, name: &str) {
    let marker = dir.join(format!("ready-{name}"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !marker.exists() {
        assert!(
            Instant::now() < deadline,
            "child never reached barrier {name:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn session_file(dir: &Path) -> PathBuf {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir.join("--work-wp-c1--")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            found.push(path);
        }
    }
    assert_eq!(found.len(), 1, "exactly one session file: {found:?}");
    found.pop().unwrap()
}

#[test]
fn synced_barriers_survive_kill9_and_reopen() {
    let root = common::tempdir();
    if let Ok(child_dir) = std::env::var(CHILD_ENV) {
        child_writer(Path::new(&child_dir));
        return;
    }
    let dir = root.path().join("s");
    std::fs::create_dir(&dir).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "synced_barriers_survive_kill9_and_reopen"])
        .env(CHILD_ENV, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_marker(&dir, "request");
    wait_for_marker(&dir, "tool");
    wait_for_marker(&dir, "step");
    // All three barriers are durable; kill the writer hard while it idles.
    child.kill().unwrap(); // SIGKILL
    let status = child.wait().unwrap();
    assert!(!status.success(), "child was killed, not exited");

    let file = session_file(&dir);
    let sm = SessionManager::open(&file, &dir, None).unwrap();
    let texts: Vec<String> = sm
        .entries()
        .iter()
        .filter_map(|entry| match entry.message() {
            Some(AgentMessage::User(user)) => Some(user.content.text("")),
            Some(AgentMessage::Assistant(assistant)) => {
                assistant.content.iter().find_map(|b| match b {
                    AssistantContent::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        texts
            .iter()
            .map(|t| t.trim().to_owned())
            .collect::<Vec<_>>(),
        vec!["u1", "a1", "u2"],
        "every barrier-synced entry survived kill -9 and reopens"
    );
    // The whole file is intact JSONL (no torn tail from the held append handle).
    for line in std::fs::read_to_string(&file).unwrap().lines() {
        assert!(FileLine::parse(line).is_ok(), "torn line: {line}");
    }
}
