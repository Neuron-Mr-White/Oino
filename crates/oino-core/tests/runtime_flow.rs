//! Services + `AgentSessionRuntime`: startup, built-in tools, `!` commands, replacements.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonObject};
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::events::{ClientEvent, OinoEvent, ReplaceReason};
use oino_core::extensions::api::{
    ExtensionInstance, ExtensionMode, PackageResourceDirs, Registration,
};
use oino_core::runtime::{AgentSessionRuntime, ForkPosition, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::PromptOptions;
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;
use serde_json::json;

async fn services(
    dir: &std::path::Path,
    provider: Arc<FauxProvider>,
    extra: Vec<Arc<dyn ExtensionInstance>>,
) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let mut instances: Vec<Arc<dyn ExtensionInstance>> = vec![Arc::new(FauxComponent(provider))];
    instances.extend(extra);
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

fn tool_options(services: &Services) -> (SessionOptions, Arc<SessionToolContext>) {
    let context = Arc::new(SessionToolContext::default());
    let settings = services.settings.lock().unwrap().snapshot();
    let tools = builtin_tool_entries(
        &services.cwd,
        &services.agent_dir,
        &settings,
        Arc::clone(&context),
    );
    (
        SessionOptions {
            source: SessionSource::InMemory,
            builtin_tools: tools,
            ..SessionOptions::default()
        },
        context,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_runs_builtin_tools_and_bash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "line one\nline two\n").unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "read".into(),
            serde_json::from_value(json!({"path": "notes.txt"})).unwrap(),
        )]),
        FauxResponse::Text("read it".into()),
    ]);
    let services = services(dir.path(), Arc::clone(&provider), Vec::new()).await;
    let (options, context) = tool_options(&services);
    let runtime = AgentSessionRuntime::create(Arc::clone(&services), options)
        .await
        .unwrap();
    let session = runtime.session();
    context.bind(&session);
    assert_eq!(
        session.active_tool_names(),
        vec!["read", "bash", "edit", "write"]
    );
    session
        .prompt("read notes".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
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
        vec![oino_ai::UserContent::text("line one\nline two\n")]
    );
    let system = &provider.requests()[0]["context"]["messages"][0];
    assert!(
        system["sections"]["tools"]
            .as_str()
            .unwrap()
            .contains("- read:")
    );

    let bash = session
        .execute_bash("echo hello".into(), false, None)
        .await
        .unwrap();
    assert_eq!(bash.output.trim(), "hello");
    assert_eq!(bash.exit_code, Some(0));
    assert!(
        session
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::BashExecution(b) if b.command == "echo hello"))
    );
}

#[tokio::test]
async fn replacements_emit_session_events() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("one".into()),
        FauxResponse::Text("two".into()),
    ]);
    let watcher = Arc::new(FakeExtension::new(
        "watcher",
        Registration {
            events: vec![
                "session_start".into(),
                "session_shutdown".into(),
                "session_before_switch".into(),
            ],
            ..Registration::default()
        },
    ));
    let services = services(
        dir.path(),
        provider,
        vec![Arc::clone(&watcher) as Arc<dyn ExtensionInstance>],
    )
    .await;
    let runtime = AgentSessionRuntime::create(
        services,
        SessionOptions {
            source: SessionSource::InMemory,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let mut events = runtime.subscribe();
    let first = runtime.session();
    first
        .prompt("hello".into(), PromptOptions::default())
        .await
        .unwrap();
    first.wait_for_idle().await;
    let replaced = runtime.new_session(None).await.unwrap();
    assert!(!replaced.cancelled);
    assert_ne!(runtime.session().session_id(), first.session_id());
    assert!(
        runtime
            .session()
            .messages()
            .iter()
            .all(|m| !matches!(m, AgentMessage::User(_)))
    );
    let kinds: Vec<String> = watcher
        .received()
        .into_iter()
        .map(|(k, p)| format!("{k}:{}", p["reason"].as_str().unwrap_or("")))
        .collect();
    assert_eq!(
        kinds,
        vec![
            "session_start:startup",
            "session_before_switch:new",
            "session_shutdown:new",
            "session_start:new"
        ]
    );
    let replaced_event = loop {
        if let ClientEvent::Oino(OinoEvent::SessionReplaced { reason, .. }) =
            tokio::time::timeout(Duration::from_secs(5), events.next())
                .await
                .unwrap()
                .unwrap()
        {
            break reason;
        }
    };
    assert_eq!(replaced_event, ReplaceReason::New);
}

#[tokio::test]
async fn fork_before_user_message_returns_its_text() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("a".into()),
        FauxResponse::Text("b".into()),
    ]);
    let services = services(dir.path(), provider, Vec::new()).await;
    let runtime = AgentSessionRuntime::create(
        services,
        SessionOptions {
            source: SessionSource::New,
            session_dir: Some(dir.path().join("sessions")),
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let session = runtime.session();
    for text in ["first", "second"] {
        session
            .prompt(text.into(), PromptOptions::default())
            .await
            .unwrap();
        session.wait_for_idle().await;
    }
    let target = session.user_messages_for_forking()[1].entry_id.clone();
    let result = runtime
        .fork(&oino_session::EntryId::new(target), ForkPosition::Before)
        .await
        .unwrap();
    assert_eq!(result.selected_text.as_deref(), Some("second"));
    let forked = runtime.session();
    assert_ne!(forked.session_id(), session.session_id());
    let users: Vec<String> = forked
        .user_messages_for_forking()
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(users, vec!["first".to_owned()]);
}

/// Package skills (manifest `resources.skills`) load; an opt-in skill (`default-enabled:
/// false`) stays out of the prompt until `extensionSettings.skills.states.<name>.enabled` is
/// true, and `/skill:name` runs it either way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn package_skills_and_opt_in_skills() {
    let dir = tempfile::tempdir().unwrap();
    let skills = dir.path().join("pkg").join("skills");
    for (name, extra) in [
        ("plain-skill", ""),
        ("unipi-migrate", "default-enabled: false\n"),
    ] {
        std::fs::create_dir_all(skills.join(name)).unwrap();
        std::fs::write(
            skills.join(name).join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {name} description\n{extra}---\nBody of {name}.\n"
            ),
        )
        .unwrap();
    }
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("one".into()),
        FauxResponse::Text("two".into()),
        FauxResponse::Text("three".into()),
    ]);
    let package = Arc::new(
        FakeExtension::new("acme.pkg", Registration::default()).with_resources(
            PackageResourceDirs {
                skills: vec![skills.clone()],
                ..PackageResourceDirs::default()
            },
        ),
    );
    let services = services(
        dir.path(),
        Arc::clone(&provider),
        vec![package as Arc<dyn ExtensionInstance>],
    )
    .await;
    let (options, _context) = tool_options(&services);
    let runtime = AgentSessionRuntime::create(Arc::clone(&services), options)
        .await
        .unwrap();
    let session = runtime.session();
    let commands: Vec<String> = session.commands().into_iter().map(|c| c.name).collect();
    assert!(
        commands.contains(&"skill:plain-skill".to_owned()),
        "{commands:?}"
    );
    assert!(
        commands.contains(&"skill:unipi-migrate".to_owned()),
        "{commands:?}"
    );
    let prompt = session.system_prompt();
    assert!(prompt.contains("<name>plain-skill</name>"), "{prompt}");
    assert!(!prompt.contains("unipi-migrate"), "{prompt}");

    let run = |text: &str| {
        let session = session.clone();
        let text = text.to_owned();
        async move {
            session
                .prompt(text, PromptOptions::default())
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
                .await
                .unwrap();
        }
    };
    run("hello").await;
    let first = provider.requests()[0]["context"]["messages"][0]["sections"].to_string();
    assert!(
        first.contains("plain-skill") && !first.contains("unipi-migrate"),
        "{first}"
    );

    run("/skill:unipi-migrate go").await;
    let invoked = provider.requests()[1]["context"]["messages"].to_string();
    assert!(invoked.contains("Body of unipi-migrate."), "{invoked}");

    services
        .settings
        .lock()
        .unwrap()
        .set_extension_setting(
            oino_settings::SettingsScope::Global,
            "skills",
            "states",
            Some(json!({"unipi-migrate": {"enabled": true}})),
        )
        .unwrap();
    run("again").await;
    let enabled = provider.requests()[2]["context"]["messages"].to_string();
    assert!(enabled.contains("<name>unipi-migrate</name>"), "{enabled}");
}
