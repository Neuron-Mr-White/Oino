//! WP-A22 (app.md §3.14; ext-abi §5.5): the board ops against the **real**
//! `unipi-kanboard` CLI built from `crates/unipi-kanboard`, over a temp board
//! home — `list`, `add`, `move`, the §5.5 error paths, the subcommand
//! allow-list and the `board_watch` file watcher. Never touches `~/.oino`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use unipi_host::board_cli::{self, BoardWatchers};
use unipi_proto::{AppErrorCode, BoardCliData, BoardErrorKind};

/// Env (UNIPI_KANBOARD_BIN/HOME) is process-global: the env-dependent tests
/// serialize behind this lock and restore what they changed.
fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn set_env(key: &str, value: &str) -> Option<String> {
    let previous = std::env::var(key).ok();
    std::env::set_var(key, value);
    previous
}

fn restore_env(key: &str, previous: Option<String>) {
    match previous {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

/// The real CLI binary, built on demand into the workspace target dir.
fn cli_binary() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf();
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let bin = target.join("debug").join("unipi-kanboard");
    if !bin.is_file() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let status = Command::new(cargo)
            .args(["build", "-p", "unipi-kanboard", "--bin", "unipi-kanboard"])
            .status()
            .expect("cargo build unipi-kanboard");
        assert!(status.success(), "building the real CLI failed");
    }
    bin
}

/// Points the host at the real CLI and a temp home; returns the guard to drop
/// (which restores the env) plus the home.
fn isolate(binary: &Path, home: &Path) -> (MutexGuard<'static, ()>, PathBuf) {
    let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(home).unwrap();
    std::env::set_var("UNIPI_KANBOARD_BIN", binary);
    std::env::set_var("UNIPI_KANBOARD_HOME", home);
    (guard, home.to_path_buf())
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|item| item.to_string()).collect()
}

async fn cli(list: &[&str], project: Option<&str>) -> BoardCliData {
    board_cli::run_cli(&args(list), project)
        .await
        .expect("the host layer itself must not fail")
}

/// Runs the real CLI outside the host's allow-list (test seeding, the way the
/// extension would): `--json`, actor user, project env pinned.
fn raw_cli(home: &Path, argv: &[&str]) -> serde_json::Value {
    let output = Command::new(cli_binary())
        .arg("--json")
        .arg("--actor")
        .arg("user")
        .args(argv)
        .env("UNIPI_KANBOARD_HOME", home)
        .output()
        .expect("run the CLI");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "raw_cli {argv:?} failed: {text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(&text).expect("JSON on stdout")
}

fn ok_payload(data: &BoardCliData) -> &serde_json::Value {
    assert!(data.ok, "expected success, got {data:?}");
    data.payload.as_ref().expect("payload")
}

/// A registered project with one todo task; returns the slug and task id.
/// Seeded through the raw CLI (registering projects is not an app op).
fn seeded_project(home: &Path, root: &Path, name: &str) -> (String, String) {
    let added = raw_cli(
        home,
        &[
            "project",
            "add",
            "--root",
            &root.to_string_lossy(),
            "--name",
            name,
        ],
    );
    let slug = added["slug"].as_str().expect("slug").to_owned();
    let task = raw_cli(
        home,
        &["--project", &slug, "add", "First task", "--status", "todo"],
    );
    let id = task["id"].as_str().expect("task id").to_owned();
    (slug, id)
}

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the env isolation lock is held across awaits on purpose"
)]
async fn list_add_move_round_trip_through_the_real_cli() {
    let bin = cli_binary();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("kb");
    let (_guard, _) = isolate(&bin, &home);

    let (slug, id) = seeded_project(&home, temp.path(), "Demo");

    // `list --all` carries the §5.5 task shape the Board reads.
    let listed = cli(&["list", "--all"], Some(&slug)).await;
    let tasks = ok_payload(&listed)["tasks"].as_array().expect("tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["title"], "First task");
    assert_eq!(tasks[0]["status"], "todo");
    assert!(tasks[0]["allowedMoves"]
        .as_array()
        .is_some_and(|m| !m.is_empty()));

    // `move` as actor user: todo → cancelled is in the user's table (todo →
    // done is not — the user reviews via in_review, the agent finishes).
    let moved = cli(&["move", &id, "cancelled"], Some(&slug)).await;
    assert_eq!(ok_payload(&moved)["status"], "cancelled");

    // `show` after the move.
    let shown = cli(&["show", &id], Some(&slug)).await;
    assert_eq!(ok_payload(&shown)["status"], "cancelled");
    let activity = ok_payload(&shown)["activity"].as_array().expect("activity");
    assert!(activity.iter().any(|entry| entry["text"]
        .as_str()
        .unwrap_or("")
        .starts_with("cancelled")));

    // `status` reports the home (the app's attachment path base).
    let status = cli(&["status"], None).await;
    assert_eq!(ok_payload(&status)["ok"], true);
    assert!(ok_payload(&status)["home"]
        .as_str()
        .is_some_and(|h| h.contains("kb")));
}

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the env isolation lock is held across awaits on purpose"
)]
async fn error_paths_carry_the_stderr_contract() {
    let bin = cli_binary();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("kb");
    let (_guard, _) = isolate(&bin, &home);

    // No project resolved: a not_found rule line on stderr, exit 1.
    let missing = cli(&["list"], None).await;
    assert!(!missing.ok);
    assert_eq!(missing.kind, Some(BoardErrorKind::NotFound));

    // A clap usage error (exit 2): the allow-list passes `list`; the flag does
    // not exist.
    let usage = cli(&["list", "--no-such-flag"], None).await;
    assert!(!usage.ok);
    assert_eq!(usage.kind, Some(BoardErrorKind::Usage));
    assert!(usage
        .error
        .unwrap()
        .contains("unexpected argument '--no-such-flag'"));

    // A rule refusal: moving a task to the lane it is already in.
    let (slug, id) = seeded_project(&home, temp.path(), "Rules");
    let same = cli(&["move", &id, "todo"], Some(&slug)).await;
    assert!(!same.ok);
    assert_eq!(same.kind, Some(BoardErrorKind::Rule));
    assert!(same.error.unwrap().contains("already in todo"));

    // An allow-list refusal never spawns a process.
    let refused = board_cli::run_cli(&args(&["start", &id]), Some(&slug)).await;
    assert_eq!(refused.unwrap_err().code, AppErrorCode::E_SCHEMA);
}

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the env isolation lock is held across awaits on purpose"
)]
async fn watcher_reports_cli_writes_per_project() {
    let bin = cli_binary();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("kb");
    let (_guard, _) = isolate(&bin, &home);
    let (slug, _id) = seeded_project(&home, temp.path(), "Watched");

    assert!(BoardWatchers::shared().set(true));
    let mut events = BoardWatchers::shared().subscribe();

    // A CLI write through the host must surface as one BoardChanged{project}.
    cli(&["add", "Second task", "--status", "backlog"], Some(&slug)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let received =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), events.recv()).await;
        let Ok(Ok(event)) = received else {
            panic!("no BoardChanged within 10 s of a CLI write");
        };
        if event.project == slug {
            break;
        }
    }

    // Refcount off: the last watcher goes away.
    assert!(!BoardWatchers::shared().set(false));
}

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the env isolation lock is held across awaits on purpose"
)]
async fn missing_binary_maps_to_not_installed() {
    let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("no-such-unipi-kanboard");
    let prev_bin = set_env("UNIPI_KANBOARD_BIN", &missing.to_string_lossy());
    let prev_home = set_env(
        "UNIPI_KANBOARD_HOME",
        &temp.path().join("kb").to_string_lossy(),
    );
    // Hide both other resolution roots (the agent dir's extension package and
    // PATH), so nothing real answers instead of the missing override.
    let prev_agent = set_env(
        "OINO_AGENT_DIR",
        &temp.path().join("agent").to_string_lossy(),
    );
    let prev_path = set_env("PATH", &temp.path().to_string_lossy());

    let error = board_cli::run_cli(&args(&["list"]), None)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::E_BOARD_UNAVAILABLE);
    let reason = error
        .detail
        .as_ref()
        .and_then(|detail| detail["reason"].as_str());
    assert_eq!(reason, Some("not_installed"));
    assert!(error.hint.is_some());

    restore_env("UNIPI_KANBOARD_BIN", prev_bin);
    restore_env("UNIPI_KANBOARD_HOME", prev_home);
    restore_env("OINO_AGENT_DIR", prev_agent);
    restore_env("PATH", prev_path);
    drop(guard);
}
