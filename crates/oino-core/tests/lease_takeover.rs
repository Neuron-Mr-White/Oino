//! D-LOCK / D-TAKEOVER (core spec §2.1, WP-C3): the runtime holds the single-writer lease
//! of its session, a second runtime takes an in-process holder over through the file
//! protocol (close there, reload here), and an unresponsive holder yields the exact
//! `SessionLocked` text.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use indexmap::IndexMap;
use oino_ai::JsonObject;
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use oino_session::{LeaseInfo, SessionManager, lock_metadata_path, read_takeover_request};
use oino_settings::SettingsManager;

/// Temp dirs under `CARGO_TARGET_TMPDIR` so the re-executed helper below sees the paths.
fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("lease-takeover-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// A session file with a dangling tool call `c2` (C2's `interrupted_file` shape).
fn interrupted_file(dir: &Path, sessions: &Path) -> PathBuf {
    let mut manager =
        SessionManager::create(dir, sessions, oino_session::CreateOptions::default()).unwrap();
    manager
        .append_message(oino_ai::AgentMessage::User(oino_ai::UserMessage {
            author: None,
            content: oino_ai::UserContentField::Text("run probe".into()),
            timestamp: 1,
        }))
        .unwrap();
    let mut assistant = oino_ai::AssistantMessage::empty("faux", "faux", "faux-1", 2);
    assistant.content = vec![
        oino_ai::AssistantContent::text("running"),
        oino_ai::AssistantContent::ToolCall(oino_ai::ToolCall {
            id: "c1".into(),
            name: "probe".into(),
            ..oino_ai::ToolCall::default()
        }),
        oino_ai::AssistantContent::ToolCall(oino_ai::ToolCall {
            id: "c2".into(),
            name: "probe".into(),
            ..oino_ai::ToolCall::default()
        }),
    ];
    assistant.stop_reason = oino_ai::StopReason::ToolUse;
    manager
        .append_message(oino_ai::AgentMessage::Assistant(assistant))
        .unwrap();
    manager
        .append_message(oino_ai::AgentMessage::ToolResult(
            oino_ai::ToolResultMessage {
                tool_call_id: "c1".into(),
                tool_name: "probe".into(),
                content: vec![oino_ai::UserContent::text("ok")],
                details: None,
                usage: None,
                nested_calls: None,
                is_error: false,
                timestamp: 3,
                duration_ms: None,
            },
        ))
        .unwrap();
    PathBuf::from(manager.session_file().unwrap())
}

async fn services(dir: &Path, provider: Arc<FauxProvider>) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let instances: Vec<Arc<dyn oino_core::extensions::api::ExtensionInstance>> =
        vec![Arc::new(FauxComponent(provider))];
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Print,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(instances)),
        approve: Some(false),
        models: Some(models),
        settings: Some(SettingsManager::in_memory(JsonObject::new())),
    })
    .await
    .unwrap()
}

fn options(source: SessionSource, sessions: &Path) -> SessionOptions {
    SessionOptions {
        source,
        session_dir: Some(sessions.to_path_buf()),
        ..SessionOptions::default()
    }
}

/// Every `session_taken_over` event seen on `stream` within `wait`.
async fn taken_over_events(
    mut stream: oino_core::hub::EventStream,
    wait: Duration,
) -> Vec<LeaseInfo> {
    let deadline = Instant::now() + wait;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let next =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), stream.next()).await;
        match next {
            Ok(Some(ClientEvent::Oino(OinoEvent::SessionTakenOver { by }))) => seen.push(by),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    seen
}

/// A runtime write open holds the lease and dispose releases it (D-LOCK ownership).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_open_holds_and_releases_lease() {
    let root = tempdir();
    let dir = root.path().join("project");
    std::fs::create_dir(&dir).unwrap();
    let sessions = root.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let file = interrupted_file(&dir, &sessions);
    let provider = Arc::new(FauxProvider::new(Vec::new()));
    let services = services(&dir, Arc::clone(&provider)).await;
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        options(SessionSource::Open(file.clone()), &sessions),
    )
    .await
    .unwrap();
    let held = lock_metadata_path(&file);
    let metadata = std::fs::read_to_string(&held).unwrap();
    assert!(
        metadata.contains(&format!("\"pid\":{}", std::process::id())),
        "our process holds the lease: {metadata}"
    );
    runtime.dispose().await;
    assert!(!held.exists(), "dispose releases the lease");
}

/// Two runtimes, one session file: the second open takes the first over through the file
/// protocol — the holder closes (releasing its lease) and reloads here within seconds;
/// repair stays idempotent (one resume-repair record for two write opens).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_runtime_takes_the_session_over() {
    let root = tempdir();
    let dir = root.path().join("project");
    std::fs::create_dir(&dir).unwrap();
    let sessions = root.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let file = interrupted_file(&dir, &sessions);
    let provider = Arc::new(FauxProvider::new(Vec::new()));
    let first = AgentSessionRuntime::create(
        services(&dir, Arc::clone(&provider)).await,
        options(SessionSource::Open(file.clone()), &sessions),
    )
    .await
    .unwrap();
    let first_id = first.session().session_id().clone();
    let first_events = first.subscribe();

    // The second open takes the first over (in-process holders go through the same
    // `.takeover` file protocol as other runners).
    let started = Instant::now();
    let second = AgentSessionRuntime::create(
        services(&dir, Arc::clone(&provider)).await,
        options(SessionSource::Open(file.clone()), &sessions),
    )
    .await
    .unwrap_or_else(|error| panic!("takeover must succeed: {error}"));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "reload here within the takeover deadline"
    );
    assert_eq!(
        second.session().session_id(),
        first_id,
        "same session reloaded"
    );

    // The holder saw the takeover and moved to a fresh session.
    let taken = taken_over_events(first_events, Duration::from_secs(5)).await;
    assert_eq!(taken.len(), 1, "exactly one session_taken_over: {taken:?}");
    assert_eq!(taken[0].pid, std::process::id());
    assert_ne!(
        first.session().session_id(),
        first_id,
        "holder continues on a fresh session"
    );

    // Exactly one repair for two write opens (idempotent, C2 risk 7).
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        text.matches("oino.resume-repair").count(),
        1,
        "repair idempotent across takeover"
    );
    second.dispose().await;
    first.dispose().await;
}

/// Helper half of [`unresponsive_holder_yields_session_locked`]: holds the lease like a
/// wedged runner (no takeover watcher) and parks.
#[test]
fn unresponsive_lease_holder() {
    const FILE_ENV: &str = "OINO_TAKEOVER_CHILD_FILE";
    let Some(file) = std::env::var_os(FILE_ENV) else {
        return; // normal test run: nothing to do
    };
    let holder = LeaseInfo::for_process("unresponsive-child", std::env::temp_dir().as_path());
    let file = PathBuf::from(file);
    let _lease =
        oino_session::SessionLease::acquire(&file, &holder).expect("child acquires the lease");
    let _ = std::fs::write(file.with_file_name("child.locked"), "locked");
    std::thread::sleep(Duration::from_secs(60));
}

/// A holder that never responds to the takeover request yields the precise
/// `SessionLocked` error after the 10 s deadline (D-TAKEOVER step 5), and the stale
/// request is cleaned up so the holder is not taken over when it resumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unresponsive_holder_yields_session_locked() {
    let root = tempdir();
    let dir = root.path().join("project");
    std::fs::create_dir(&dir).unwrap();
    let sessions = root.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let file = interrupted_file(&dir, &sessions);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unresponsive_lease_holder"])
        .env("OINO_TAKEOVER_CHILD_FILE", &file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let marker = file.with_file_name("child.locked");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "helper never took the lease");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let started = Instant::now();
    let provider = Arc::new(FauxProvider::new(Vec::new()));
    let error = AgentSessionRuntime::create(
        services(&dir, Arc::clone(&provider)).await,
        options(SessionSource::Open(file.clone()), &sessions),
    )
    .await
    .unwrap_err();
    let text = error.to_string();
    assert!(
        started.elapsed() >= Duration::from_secs(9),
        "the deadline (10 s) expired first: {text}"
    );
    assert!(
        text.contains("is open in unresponsive-child (pid ")
            && text.contains(
                "It was asked to close and did not respond; close it there and try again."
            ),
        "the exact SessionLocked text names the holder: {text}"
    );
    assert!(
        read_takeover_request(&file).is_none(),
        "a failed takeover cleans its request"
    );

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap()),
            nix::sys::signal::Signal::SIGKILL,
        )
        .unwrap();
        let status = child.wait().unwrap();
        assert!(status.signal().is_some(), "helper killed");
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}
