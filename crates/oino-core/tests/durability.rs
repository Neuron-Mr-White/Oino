//! D-FLUSH fsync barriers (core spec §2.1, WP-C1): fail-closed barriers around the
//! provider request and side-effect tools, and the read-only exemption.
//!
//! Failure injection uses an unwritable sessions directory: a session whose first flush
//! cannot succeed reports the I/O error at the next barrier, so the request never leaves
//! the process and a side-effect tool body never runs.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oino_ai::UserContentField;
use oino_core::extensions::api::{Registration, ToolSpec};
use oino_core::extensions::host::CustomMessageDraft;
use oino_core::session::AgentSession;
use oino_core::testing::{FakeExtension, FauxProvider, FauxResponse, TestSession};
use serde_json::{Value, json};

/// A sessions root made read-only for the test's duration (restored on drop).
struct ReadOnlyDir(PathBuf);

impl ReadOnlyDir {
    /// Makes `dir` read-only; `None` when permissions cannot block writes (running as
    /// root), in which case the injection is unavailable and the test is skipped.
    #[must_use]
    fn make(dir: &Path) -> Option<Self> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let canary = dir.join(".oino-canary");
        let blocked = std::fs::File::create(&canary).is_err();
        let _ = std::fs::remove_file(&canary);
        blocked.then(|| Self(dir.to_path_buf()))
    }
}

impl Drop for ReadOnlyDir {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

fn drained(events: &oino_core::hub::EventStream) -> Vec<Value> {
    events
        .drain_now()
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect()
}

async fn settle(session: &AgentSession) {
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

fn tool_spec(name: &str, read_only: bool) -> ToolSpec {
    let annotations = read_only.then(|| json!({"readOnly": true}));
    ToolSpec {
        name: name.into(),
        label: name.into(),
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        exposure: oino_core::extensions::api::ToolExposure::Direct,
        default_active: None,
        execution_mode: None,
        namespace: None,
        annotations,
        reaches_tools: None,
        output_schema: None,
        prepare_arguments: false,
    }
}

/// A persisted session in `sessions` whose `probe` extension tool records whether its
/// body ran.
async fn probe_session(
    sessions: &Path,
    read_only_tool: bool,
    responses: Vec<FauxResponse>,
) -> (AgentSession, Arc<FauxProvider>, Arc<AtomicBool>) {
    let provider = FauxProvider::new(responses);
    let ran = Arc::new(AtomicBool::new(false));
    let ran_tool = Arc::clone(&ran);
    let extension = FakeExtension::new(
        "probe-ext",
        Registration {
            tools: vec![tool_spec("probe", read_only_tool)],
            ..Registration::default()
        },
    )
    .on_tool(move |_name, _args| {
        ran_tool.store(true, Ordering::SeqCst);
        Ok(oino_agent::AgentToolResult::text("probe ran"))
    });
    let session = TestSession {
        cwd: sessions.to_path_buf(),
        session_dir: Some(sessions.to_path_buf()),
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    (session, provider, ran)
}

/// Barrier 1 (before each model request): with an unwritable sessions directory the
/// request fails inside the process — the provider is never called.
#[tokio::test]
async fn request_barrier_fails_closed_on_unwritable_session_dir() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let (session, provider, _ran) =
        probe_session(&sessions, false, vec![FauxResponse::Text("x".into())]).await;
    let Some(guard) = ReadOnlyDir::make(&sessions) else {
        // Running as root: permissions do not block writes, the injection is unavailable.
        return;
    };
    let events = session.subscribe();
    session
        .prompt("hi".into(), oino_core::session::PromptOptions::default())
        .await
        .unwrap();
    settle(&session).await;
    drop(guard);

    assert!(
        provider.requests().is_empty(),
        "the failed request never left the process"
    );
    let drained = drained(&events);
    let error = drained.iter().find_map(|event| {
        let message = &event["message"];
        (event["type"] == "message_end" && message["stopReason"] == "error").then(|| {
            message["errorMessage"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
    });
    let error = error.expect("an error assistant message event");
    assert!(
        error.contains("durability barrier"),
        "error names the barrier: {error}"
    );
}

/// Barrier 2 (before a side-effect tool): the tool call is converted into an error result
/// and the body never runs on top of an unpersisted log.
#[tokio::test]
async fn tool_barrier_fails_closed_and_never_runs_the_body() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let (session, _provider, ran) = probe_session(
        &sessions,
        false,
        vec![
            FauxResponse::ToolCalls(vec![(
                "c1".into(),
                "probe".into(),
                json!({}).as_object().unwrap().clone(),
            )]),
            FauxResponse::Text("done".into()),
        ],
    )
    .await;
    let guard = ReadOnlyDir::make(&sessions);
    let events = session.subscribe();
    // A custom-message trigger keeps the session unflushed (no user/assistant message
    // yet), so barrier 1 passes and the first barrier the run hits after the assistant
    // message is the tool barrier.
    session
        .send_custom_message(
            CustomMessageDraft {
                custom_type: "wp-c1".into(),
                content: UserContentField::Text("go".into()),
                display: false,
                details: None,
            },
            true,
            None,
        )
        .await
        .unwrap();
    settle(&session).await;
    drop(guard);

    assert!(!ran.load(Ordering::SeqCst), "tool body never ran");
    let drained = drained(&events);
    let result = drained
        .iter()
        .find_map(|event| {
            let message = &event["message"];
            (event["type"] == "message_end" && message["role"] == "toolResult")
                .then(|| message.clone())
        })
        .expect("a toolResult message");
    assert_eq!(result["isError"], json!(true));
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("durability barrier"),
        "tool result carries the barrier failure: {result}"
    );
}

/// Read-only tools (`annotations.readOnly == true`) skip the barrier: their body runs
/// even when the session log cannot be made durable.
#[tokio::test]
async fn read_only_tools_skip_the_barrier() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let (session, _provider, ran) = probe_session(
        &sessions,
        true,
        vec![
            FauxResponse::ToolCalls(vec![(
                "c1".into(),
                "probe".into(),
                json!({}).as_object().unwrap().clone(),
            )]),
            FauxResponse::Text("done".into()),
        ],
    )
    .await;
    let guard = ReadOnlyDir::make(&sessions);
    let events = session.subscribe();
    session
        .send_custom_message(
            CustomMessageDraft {
                custom_type: "wp-c1".into(),
                content: UserContentField::Text("go".into()),
                display: false,
                details: None,
            },
            true,
            None,
        )
        .await
        .unwrap();
    settle(&session).await;
    drop(guard);

    assert!(ran.load(Ordering::SeqCst), "read-only tool body ran");
    let drained = drained(&events);
    let result = drained
        .iter()
        .find_map(|event| {
            let message = &event["message"];
            (event["type"] == "message_end" && message["role"] == "toolResult")
                .then(|| message.clone())
        })
        .expect("a toolResult message");
    assert_eq!(result["isError"], json!(false), "tool succeeded: {result}");
    // The next model request then fails closed at barrier 1 (the assistant message made
    // the session flushable and the directory is still unwritable).
    let error = drained.iter().find_map(|event| {
        let message = &event["message"];
        (event["type"] == "message_end" && message["stopReason"] == "error").then(|| {
            message["errorMessage"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
    });
    assert!(
        error
            .as_deref()
            .unwrap_or_default()
            .contains("durability barrier"),
        "the following request fails closed: {error:?}"
    );
}
