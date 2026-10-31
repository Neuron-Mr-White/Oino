//! WP-A15 (app.md §3.14): the git watcher. Real temp repos, real fsnotify:
//! `git_watch` on → a metadata change emits one `GitChanged` after the 1 s
//! debounce (rapid changes coalesce), `off` stops it, and the event rides an
//! `events_open` stream end to end. Never touches a real repo or `~/.unipi`.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use unipi_host::git_watch::{GitChangedEvent, GitWatchers};
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;

const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// The metadata debounce plus slack.
const SETTLE: Duration = Duration::from_secs(5);

fn isolate_env() {
    std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repo with an identity, one commit (a.txt), on `main`, in a tempdir
/// under /tmp (a default safe root).
fn repo(prefix: &str) -> PathBuf {
    let base = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in("/tmp")
        .unwrap();
    let dir = base.path().join("r");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test Person"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    // Keep the repo after the tempdir handle drops for the test's lifetime.
    let canonical = dir.canonicalize().unwrap();
    std::mem::forget(base);
    canonical
}

/// `GitChanged`s for `root` within `SETTLE` after `kick` runs.
async fn events_after(root: &Path, kick: impl FnOnce()) -> Vec<GitChangedEvent> {
    let mut rx = GitWatchers::shared().subscribe();
    kick();
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let Ok(Ok(event)) = tokio::time::timeout_at(deadline, rx.recv()).await else {
            break;
        };
        if event.root == root.to_string_lossy() {
            out.push(event);
        }
    }
    out
}

#[tokio::test]
async fn watch_on_emits_one_change_per_burst_off_stops() {
    isolate_env();
    let root = repo("a15-watch-");

    // On: the canonical root comes back (the events' root).
    let watched = GitWatchers::shared().set(root.clone(), true).await;
    assert_eq!(watched.as_deref(), Some(root.to_string_lossy().as_ref()));

    // Three rapid index writes inside one debounce window → one event.
    let index = root.join(".git").join("index");
    let events = events_after(&root, || {
        for _ in 0..3 {
            let bytes = std::fs::read(&index).unwrap();
            std::fs::write(&index, bytes.as_slice()).unwrap();
            std::thread::sleep(Duration::from_millis(120));
        }
    })
    .await;
    assert_eq!(events.len(), 1, "rapid metadata writes coalesce");

    // A later, separate change emits again.
    std::fs::write(root.join("b.txt"), "new\n").unwrap();
    git(&root, &["add", "b.txt"]);
    let events = events_after(&root, || {}).await;
    assert_eq!(events.len(), 1, "a later change notifies again");

    // Off: no more events.
    let off = GitWatchers::shared().set(root.clone(), false).await;
    assert_eq!(off.as_deref(), Some(root.to_string_lossy().as_ref()));
    std::fs::write(root.join("c.txt"), "gone\n").unwrap();
    git(&root, &["add", "c.txt"]);
    let events = events_after(&root, || {}).await;
    assert!(events.is_empty(), "off stops the watcher: {events:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn watch_refuses_plain_folders_and_double_off_is_fine() {
    isolate_env();
    let plain = tempfile::Builder::new()
        .prefix("a15-plain-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::write(plain.path().join("x.txt"), "x").unwrap();
    assert_eq!(
        GitWatchers::shared()
            .set(plain.path().to_path_buf(), true)
            .await,
        None
    );
    // `off` for something never watched: no panic, no watcher.
    assert_eq!(
        GitWatchers::shared()
            .set(plain.path().to_path_buf(), false)
            .await,
        None
    );
}

/// The session-stream fixture: a hub whose sessions would be answered by an
/// empty faux provider (none are opened here — `events_open` only).
async fn service() -> Arc<SessionService> {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let loader = Arc::new(oino_core::services::StaticLoader(Vec::new()));
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir,
        docs_root: None,
        session_dir: Some(sessions),
        mode: oino_core::extensions::api::ExtensionMode::Desktop,
        has_ui: false,
        approve: Some(false),
        flags: oino_core::services::ResourceFlags {
            no_context_files: true,
            ..Default::default()
        },
        loader,
        models: Some(models),
        tools: oino_core::runtime::ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: None,
    })
    .unwrap();
    SessionService::new(hub)
}

async fn read_frame(read: &mut (impl AsyncRead + Unpin)) -> Option<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    read.read_exact(&mut header).await.ok()?;
    let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    let mut body = vec![0u8; len];
    read.read_exact(&mut body).await.ok()?;
    Some((header[0], body))
}

/// End to end: a watched repo change lands on `events_open` as
/// `git_changed` (host.md §7.4 / app.md §3.14).
#[tokio::test]
async fn git_changed_rides_events_open() {
    isolate_env();
    let root = repo("a15-wire-");
    let service = service().await;
    let (client, server) = tokio::io::duplex(1 << 20);
    let service_for_task = Arc::clone(&service);
    let task = tokio::spawn(async move {
        let (mut read, mut write) = tokio::io::split(server);
        unipi_host::sessions::stream::serve_request(
            &service_for_task,
            unipi_proto::Request::EventsOpen,
            &mut read,
            &mut write,
        )
        .await
    });
    let (mut read, mut write) = tokio::io::split(client);

    // First frame: the stream's ok.
    let (kind, body) = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut read))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kind, 0);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        serde_json::json!({"type": "ok"})
    );

    // Watch on, then commit: the index and refs change → one debounced event.
    assert_eq!(
        GitWatchers::shared()
            .set(root.clone(), true)
            .await
            .as_deref(),
        Some(root.to_string_lossy().as_ref())
    );
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    git(&root, &["add", "b.txt"]);
    git(&root, &["commit", "-q", "-m", "second"]);

    let root_text = root.to_string_lossy().into_owned();
    let mut seen = Value::Null;
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let frame = tokio::time::timeout_at(deadline, read_frame(&mut read)).await;
        let Ok(Some((0, body))) = frame else { break };
        let value: Value = serde_json::from_slice(&body).unwrap();
        if value["kind"] == "git_changed" {
            seen = value;
            break;
        }
    }
    assert_eq!(seen["kind"], "git_changed", "the event arrived");
    assert_eq!(seen["root"], root_text.as_str());

    let _ = GitWatchers::shared().set(root.clone(), false).await;
    let _ = write.shutdown().await;
    let _ = tokio::time::timeout(READ_TIMEOUT, task).await;
    let _ = std::fs::remove_dir_all(&root);
}
