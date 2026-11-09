//! D-DISPATCH / D-REPAIR (core spec §2.1, WP-C2): the tool-dispatch record lands before
//! the tool body runs, read-only tools record nothing, and a runtime write open repairs
//! an interrupted turn so the next request feeds a balanced transcript.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexmap::IndexMap;
use oino_ai::JsonObject;
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode, Registration, ToolSpec};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::{AgentSession, PromptOptions};
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse, TestSession};
use oino_models::ModelRegistry;
use oino_session::{EntryKind, SessionManager, TOOL_DISPATCH_TYPE};
use oino_settings::SettingsManager;
use serde_json::{Value, json};

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

/// Session file of a persisted `AgentSession` (exactly one in the sessions dir).
fn only_session_file(session: &AgentSession) -> PathBuf {
    session.session_file().expect("persisted session")
}

fn custom_entries(session: &AgentSession, custom_type: &str) -> Vec<Value> {
    session.with_manager(|manager| {
        manager
            .entries()
            .iter()
            .filter_map(|entry| match &entry.kind {
                EntryKind::Custom {
                    custom_type: t,
                    data,
                } if t == custom_type => Some(data.clone().unwrap_or(Value::Null)),
                _ => None,
            })
            .collect()
    })
}

async fn settle(session: &AgentSession) {
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

/// The dispatch record is on the log before the tool body runs (D-DISPATCH): the body
/// observes its own record on disk, and the record precedes the toolResult in the file.
#[tokio::test]
async fn dispatch_record_lands_before_the_tool_body() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let dispatch_seen_at_body = Arc::new(Mutex::new(Vec::<bool>::new()));
    let seen = Arc::clone(&dispatch_seen_at_body);
    let sessions_at_body = sessions.clone();
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![("c1".into(), "probe".into(), JsonObject::new())]),
        FauxResponse::Text("done".into()),
    ]);
    let extension = FakeExtension::new(
        "probe-ext",
        Registration {
            tools: vec![tool_spec("probe", false)],
            ..Registration::default()
        },
    )
    .on_tool(move |_name, _args| {
        // The body reads the raw session file: the dispatch record must already be there.
        let file = only_session_file_in(&sessions_at_body);
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        seen.lock()
            .unwrap()
            .push(text.contains(TOOL_DISPATCH_TYPE) && text.contains("\"toolCallId\":\"c1\""));
        Ok(oino_agent::AgentToolResult::text("probe ran"))
    });
    let session = TestSession {
        cwd: sessions.clone(),
        session_dir: Some(sessions.clone()),
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    session
        .prompt("run probe".into(), PromptOptions::default())
        .await
        .unwrap();
    settle(&session).await;

    assert_eq!(
        *dispatch_seen_at_body.lock().unwrap(),
        vec![true],
        "the body observed its own dispatch record on disk"
    );
    let records = custom_entries(&session, TOOL_DISPATCH_TYPE);
    assert_eq!(
        records,
        vec![json!({
            "toolCallId": "c1",
            "toolName": "probe",
            "input": {},
            "readOnly": false,
        })],
        "one dispatch record with the final input"
    );
    // File order: the record precedes the toolResult message.
    let text = std::fs::read_to_string(only_session_file(&session)).unwrap();
    let dispatch_at = text.find(TOOL_DISPATCH_TYPE).expect("record in file");
    let result_at = text
        .find("\"toolCallId\":\"c1\",\"toolName\"")
        .expect("result");
    assert!(dispatch_at < result_at, "record before result:\n{text}");
}

/// Read-only tools (`annotations.readOnly == true`) record no dispatch entry.
#[tokio::test]
async fn read_only_tool_records_no_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let sessions = root.path().join("sessions");
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![("c1".into(), "peek".into(), JsonObject::new())]),
        FauxResponse::Text("done".into()),
    ]);
    let extension = FakeExtension::new(
        "probe-ext",
        Registration {
            tools: vec![tool_spec("peek", true)],
            ..Registration::default()
        },
    );
    let session = TestSession {
        cwd: sessions.clone(),
        session_dir: Some(sessions),
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    session
        .prompt("peek".into(), PromptOptions::default())
        .await
        .unwrap();
    settle(&session).await;

    assert!(
        custom_entries(&session, TOOL_DISPATCH_TYPE).is_empty(),
        "read-only tools record nothing"
    );
}

/// The one `*.jsonl` file under `<sessions>/--<cwd>--/` for a session whose cwd is the
/// `sessions` root itself (the persisted-test layout of `TestSession`).
fn only_session_file_in(sessions: &Path) -> PathBuf {
    let dir = oino_session::manager::cwd_session_dir(sessions, sessions);
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            found.push(path);
        }
    }
    found.pop().expect("exactly one session file")
}

/// The crash-truncated session file at the sessions root a runtime will open: two calls
/// issued, `c1` answered, `c2` dispatched but never finished.
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
    let path = manager.session_file().unwrap().to_path_buf();
    drop(manager);
    path
}

async fn services(dir: &Path, provider: Arc<FauxProvider>) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let instances: Vec<Arc<dyn ExtensionInstance>> = vec![Arc::new(FauxComponent(provider))];
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

/// A runtime write open repairs the dangling tool call (D-REPAIR) and the next prompt
/// feeds a balanced transcript: the provider request carries the unknown-outcome
/// toolResult for `c2`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_open_repairs_and_feeds_a_balanced_request() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("project");
    std::fs::create_dir(&dir).unwrap();
    let sessions = root.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let file = interrupted_file(&dir, &sessions);
    let provider = FauxProvider::new(vec![FauxResponse::Text("resumed".into())]);
    let services = services(&dir, Arc::clone(&provider)).await;
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::Open(file),
            session_dir: Some(sessions),
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let session = runtime.session();

    // The repair closed `c2` on disk with the unknown-outcome toolResult + record.
    let text = std::fs::read_to_string(session.session_file().unwrap()).unwrap();
    assert!(text.contains("oino.resume-repair"), "{text}");
    assert!(text.contains("Tool execution was interrupted"), "{text}");
    let balanced = session.with_manager(|manager| {
        let messages = manager.build_session_projection().messages;
        let answered: Vec<String> = messages
            .iter()
            .filter_map(|m| match m {
                oino_ai::AgentMessage::ToolResult(r) => Some(r.tool_call_id.clone()),
                _ => None,
            })
            .collect();
        answered == ["c1".to_owned(), "c2".to_owned()]
    });
    assert!(balanced, "both calls answered after repair");

    // The next request feeds the balanced transcript, repaired result included.
    session
        .prompt("continue".into(), PromptOptions::default())
        .await
        .unwrap();
    settle(&session).await;
    let request = provider.requests().last().cloned().unwrap();
    let wire = request["context"]["messages"].to_string();
    assert!(
        wire.contains("\"toolCallId\":\"c2\"") && wire.contains("interrupted before completion"),
        "repaired result in the request: {wire}"
    );
    runtime.dispose().await;
}

/// Fork sources are read paths: forking a dangling session leaves the source untouched
/// (no repair record ever lands in it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_source_is_not_repaired() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("project");
    std::fs::create_dir(&dir).unwrap();
    let sessions = root.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    let file = interrupted_file(&dir, &sessions);
    let before = std::fs::read_to_string(&file).unwrap();
    let provider = FauxProvider::new(Vec::new());
    let services = services(&dir, Arc::clone(&provider)).await;
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::Fork(file.clone()),
            session_dir: Some(sessions),
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let after = std::fs::read_to_string(&file).unwrap();
    assert_eq!(before, after, "fork source untouched (read path)");
    runtime.dispose().await;
}
