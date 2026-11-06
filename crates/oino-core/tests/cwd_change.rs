//! `AgentSession::change_cwd`: a session moves its working directory append-only — the
//! system prompt and every earlier message stay byte-identical (prompt cache), one
//! `oino.cwd_change` reminder is appended, tools run in the new folder, and a reopen keeps
//! working there.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonObject};
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::{AgentSession, PromptOptions, cwd_change_reminder};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_session::{CWD_CHANGE_CUSTOM_TYPE, EntryKind, SessionManager};
use oino_settings::SettingsManager;
use serde_json::{Value, json};

async fn services(cwd: &Path, agent: &Path, provider: Arc<FauxProvider>) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let instances: Vec<Arc<dyn ExtensionInstance>> = vec![Arc::new(FauxComponent(provider))];
    Services::create(ServicesOptions {
        cwd: cwd.to_path_buf(),
        agent_dir: agent.to_path_buf(),
        docs_root: agent.join("docs"),
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

async fn open_runtime(
    services: &Arc<Services>,
    source: SessionSource,
    sessions: &Path,
) -> (AgentSessionRuntime, AgentSession) {
    let context = Arc::new(SessionToolContext::default());
    let settings = services.settings.lock().unwrap().snapshot();
    let tools = builtin_tool_entries(
        &services.cwd,
        &services.agent_dir,
        &settings,
        Arc::clone(&context),
    );
    let runtime = AgentSessionRuntime::create(
        Arc::clone(services),
        SessionOptions {
            source,
            session_dir: Some(sessions.to_path_buf()),
            builtin_tools: tools,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let session = runtime.session();
    context.bind(&session);
    (runtime, session)
}

async fn ask(session: &AgentSession, text: &str) {
    session
        .prompt(text.into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

fn messages(request: &Value) -> Vec<String> {
    request["context"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn change_cwd_appends_a_reminder_and_keeps_the_prompt_prefix() {
    let root = tempfile::tempdir().unwrap();
    let scratch = root.path().join("scratch").join("s1");
    let workspace = root.path().join("project");
    let sessions = root.path().join("sessions");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("notes.txt"), "from the workspace\n").unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first".into()),
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "read".into(),
            serde_json::from_value(json!({"path": "notes.txt"})).unwrap(),
        )]),
        FauxResponse::Text("read it".into()),
        FauxResponse::Text("after reopen".into()),
    ]);
    let agent = root.path().join("agent");
    let services = services(&scratch, &agent, Arc::clone(&provider)).await;
    let (runtime, session) = open_runtime(&services, SessionSource::New, &sessions).await;

    ask(&session, "hello").await;
    let file = session.session_file().unwrap();
    let before = std::fs::read(&file).unwrap();
    assert_eq!(session.work_cwd(), scratch);

    session.change_cwd(workspace.clone()).await.unwrap();

    // Append-only: the file's earlier bytes are untouched; one custom message follows.
    let after = std::fs::read(&file).unwrap();
    assert!(after.starts_with(&before), "earlier entries were rewritten");
    let reopened = SessionManager::open(&file, &sessions, None).unwrap();
    assert_eq!(reopened.cwd(), scratch, "the header cwd stays");
    assert_eq!(reopened.moved_cwd().as_deref(), Some(workspace.as_path()));
    let last = reopened.entries().last().unwrap();
    match &last.kind {
        EntryKind::CustomMessage {
            custom_type,
            display,
            details,
            ..
        } => {
            assert_eq!(custom_type, CWD_CHANGE_CUSTOM_TYPE);
            assert!(display);
            assert_eq!(
                details.as_ref().unwrap(),
                &json!({"cwd": workspace.display().to_string(), "previousCwd": scratch.display().to_string()})
            );
        }
        other => panic!("expected the cwd_change message, got {other:?}"),
    }
    drop(reopened);
    assert_eq!(session.cwd(), scratch, "the prompt cwd never changes");
    assert_eq!(session.work_cwd(), workspace);
    // A second move to the same folder is refused; a missing folder too.
    assert!(session.change_cwd(workspace.clone()).await.is_err());
    assert!(
        session
            .change_cwd(root.path().join("missing"))
            .await
            .is_err()
    );

    // Next turn: tools run in the workspace, the prefix is byte-identical.
    ask(&session, "read the notes").await;
    let result = session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        result.content,
        vec![oino_ai::UserContent::text("from the workspace")]
    );
    session.set_autonomy(oino_core::session::permission::Level::High);
    let bash = session
        .execute_bash("pwd".into(), true, None)
        .await
        .unwrap();
    assert_eq!(
        Path::new(bash.output.trim()).canonicalize().unwrap(),
        workspace.canonicalize().unwrap()
    );

    let requests = provider.requests();
    let first = messages(&requests[0]);
    let second = messages(&requests[1]);
    // [system, user hello] then [system, user hello, assistant first, reminder, user read].
    assert_eq!(
        second[..first.len()],
        first[..],
        "system prompt and earlier messages must be byte-identical"
    );
    let reminder = &requests[1]["context"]["messages"][first.len() + 1];
    assert_eq!(reminder["role"], "user");
    assert!(
        serde_json::to_string(reminder)
            .unwrap()
            .contains("The workspace changed"),
        "{reminder}"
    );
    assert!(cwd_change_reminder(&scratch, &workspace).contains(&workspace.display().to_string()));

    // Reopen: same services (header cwd), the session keeps working in the workspace and the
    // system prompt is still the original one.
    runtime.dispose().await;
    let (runtime, session) = open_runtime(&services, SessionSource::Open(file), &sessions).await;
    assert_eq!(session.cwd(), scratch);
    assert_eq!(session.work_cwd(), workspace);
    ask(&session, "again").await;
    let requests = provider.requests();
    let third = messages(requests.last().unwrap());
    assert_eq!(
        third[0], first[0],
        "the reopened session's system prompt is unchanged"
    );
    runtime.dispose().await;
}
