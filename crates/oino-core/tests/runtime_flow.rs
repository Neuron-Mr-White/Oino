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
        vec![
            "read",
            "bash",
            "edit",
            "write",
            "grep",
            "glob",
            "ls",
            "codemode",
            "tool_search"
        ]
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
        vec![oino_ai::UserContent::text("line one\nline two")]
    );
    let system = &provider.requests()[0]["context"]["messages"][0];
    assert!(system["content"].as_str().unwrap().contains("Oino"));
    assert!(system["sections"].is_null());
    session.set_autonomy(oino_core::session::permission::Level::High);

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
        ("acme-migrate", "default-enabled: false\n"),
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
        commands.contains(&"skill:acme-migrate".to_owned()),
        "{commands:?}"
    );
    let prompt = session.system_prompt();
    assert!(!prompt.contains("plain-skill"), "{prompt}");
    assert!(!prompt.contains("acme-migrate"), "{prompt}");

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
    let first = provider.requests()[0]["context"]["messages"][0]["content"].as_str().unwrap().to_owned();
    assert!(
        !first.contains("plain-skill") && !first.contains("acme-migrate"),
        "{first}"
    );

    run("/skill:acme-migrate go").await;
    let invoked = provider.requests()[1]["context"]["messages"].to_string();
    assert!(invoked.contains("Body of acme-migrate."), "{invoked}");

    services
        .settings
        .lock()
        .unwrap()
        .set_extension_setting(
            oino_settings::SettingsScope::Global,
            "skills",
            "states",
            Some(json!({"acme-migrate": {"enabled": true}})),
        )
        .unwrap();
    run("again").await;
    let enabled = provider.requests()[2]["context"]["messages"].to_string();
    assert!(
        !session.system_prompt().contains("acme-migrate"),
        "{enabled}"
    );
    assert!(
        session
            .commands()
            .iter()
            .any(|c| c.name == "skill:acme-migrate")
    );
}

/// An embedder that passes no built-ins (the desktop hub) still gets the default ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_defaults_builtin_tools_when_the_embedder_passes_none() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let services = services(dir.path(), Arc::clone(&provider), Vec::new()).await;
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::InMemory,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        runtime.session().active_tool_names(),
        vec![
            "read",
            "bash",
            "edit",
            "write",
            "grep",
            "glob",
            "ls",
            "codemode",
            "tool_search"
        ]
    );
}

/// A transcript whose recorded loadout lost every built-in (a pi session whose first Oino turn
/// ran without them) gets the default built-ins back on open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_restores_builtins_missing_from_the_recorded_loadout() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join("2026-11-02T00-00-00-000Z_s1.jsonl");
    let cwd = dir.path().display().to_string();
    let decl = |name: &str| json!({"name": name, "description": "", "parameters": {}});
    let lines = [
        json!({"type": "session", "version": 3, "id": "s1", "timestamp": "2026-11-02T00:00:00.000Z", "cwd": cwd}),
        json!({"type": "message", "id": "m1", "parentId": null, "timestamp": "2026-11-02T00:00:01.000Z",
            "message": {"role": "system", "content": "", "timestamp": 1,
                "toolsAdded": [decl("read"), decl("bash"), decl("codemode")]}}),
        json!({"type": "message", "id": "m2", "parentId": "m1", "timestamp": "2026-11-02T00:00:02.000Z",
            "message": {"role": "system", "content": "", "timestamp": 2,
                "toolsRemoved": [{"name": "read"}, {"name": "bash"}, {"name": "codemode"}],
                "toolsAdded": [decl("memory_store")]}}),
        json!({"type": "message", "id": "m3", "parentId": "m2", "timestamp": "2026-11-02T00:00:03.000Z",
            "message": {"role": "user", "content": "hi", "timestamp": 3}}),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&file, text).unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let services = services(dir.path(), Arc::clone(&provider), Vec::new()).await;
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
    let active = runtime.session().active_tool_names();
    for name in ["read", "bash", "edit", "write"] {
        assert!(
            active.iter().any(|a| a == name),
            "{name} missing from {active:?}"
        );
    }
}

fn ext_tool(name: &str) -> oino_core::extensions::api::ToolSpec {
    oino_core::extensions::api::ToolSpec {
        name: name.into(),
        label: name.into(),
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        exposure: oino_core::extensions::api::ToolExposure::Direct,
        default_active: Some(false),
        execution_mode: None,
        namespace: None,
        annotations: None,
        reaches_tools: None,
        output_schema: None,
        prepare_arguments: false,
    }
}

/// A runtime (no embedder built-ins, like the desktop hub) with a long-horizon-like extension
/// owning two inactive mode tools and one always-on tool.
async fn graph_runtime(dir: &std::path::Path) -> (Arc<Services>, AgentSessionRuntime) {
    graph_runtime_with(dir, SessionSource::InMemory, None).await
}

async fn graph_runtime_with(
    dir: &std::path::Path,
    source: SessionSource,
    session_dir: Option<std::path::PathBuf>,
) -> (Arc<Services>, AgentSessionRuntime) {
    let lh = Arc::new(FakeExtension::new(
        "unipi.long-horizon",
        Registration {
            tools: vec![
                ext_tool("view_agent_graph"),
                ext_tool("update_agent_graph"),
                oino_core::extensions::api::ToolSpec {
                    default_active: None,
                    ..ext_tool("memory_store")
                },
            ],
            ..Registration::default()
        },
    ));
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let services = services(dir, provider, vec![lh as Arc<dyn ExtensionInstance>]).await;
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source,
            session_dir,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    (services, runtime)
}

fn has_core_tools(active: &[String]) {
    for name in ["read", "bash", "edit", "write"] {
        assert!(
            active.iter().any(|a| a == name),
            "{name} missing from {active:?}"
        );
    }
}

/// P0 [fix-tools]: an extension that read the active tools while they came back empty
/// (failed/stale read during a wake or a trap replay) and wrote back "only my mode tools"
/// must not wipe the built-ins — the main agent kept `bash`/`read`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_whole_list_write_never_drops_builtin_tools() {
    use oino_core::extensions::api::CallKind;
    use oino_core::extensions::host::HostOp;
    let dir = tempfile::tempdir().unwrap();
    let (_services, runtime) = graph_runtime(dir.path()).await;
    let session = runtime.session();
    has_core_tools(&session.active_tool_names());
    let cx = session.runner().call_context().with_kind(CallKind::Event);
    // What long-horizon's `sync_mode_tools(&[], Mode::Graph)` produced on an empty read.
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            HostOp::SetActiveTools(vec!["view_agent_graph".into(), "update_agent_graph".into()]),
        )
        .await
        .unwrap();
    let active = session.active_tool_names();
    has_core_tools(&active);
    assert!(active.iter().any(|a| a == "view_agent_graph"), "{active:?}");
    // C: a whole-list write cannot remove already advertised declarations.
    let declared = session.active_tool_names();
    session.set_active_tools_by_name(&["read".to_owned()]);
    assert_eq!(
        session.active_tool_names(),
        declared,
        "C: declarations remain frozen"
    );
}

/// The atomic `update-active-tools`: adds/removes only the named tools relative to the
/// CURRENT registry; built-ins leave only when named; unknown names are ignored; an
/// unchanged set publishes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_active_tools_toggles_only_named_tools() {
    use oino_core::extensions::api::CallKind;
    use oino_core::extensions::host::HostOp;
    let dir = tempfile::tempdir().unwrap();
    let (_services, runtime) = graph_runtime(dir.path()).await;
    let session = runtime.session();
    let cx = session.runner().call_context().with_kind(CallKind::Event);
    let update = |enable: &[&str], disable: &[&str]| HostOp::UpdateActiveTools {
        enable: enable.iter().map(|s| (*s).to_owned()).collect(),
        disable: disable.iter().map(|s| (*s).to_owned()).collect(),
    };
    // Graph mode on.
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            update(
                &["view_agent_graph", "update_agent_graph", "no_such_tool"],
                &[],
            ),
        )
        .await
        .unwrap();
    let active = session.active_tool_names();
    has_core_tools(&active);
    assert!(active.iter().any(|a| a == "memory_store"), "{active:?}");
    assert!(
        active.iter().any(|a| a == "update_agent_graph"),
        "{active:?}"
    );
    assert!(!active.iter().any(|a| a == "no_such_tool"), "{active:?}");
    // Same request again: no change, no ToolsChanged churn.
    let mut events = session.subscribe();
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            update(&["view_agent_graph", "update_agent_graph"], &[]),
        )
        .await
        .unwrap();
    let mut changes = 0;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), events.next()).await
    {
        if matches!(event, ClientEvent::Oino(OinoEvent::ToolsChanged { .. })) {
            changes += 1;
        }
    }
    assert_eq!(changes, 0, "an unchanged set publishes nothing");
    // C: disabling a mode preserves the previously advertised declarations.
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            update(&[], &["view_agent_graph", "update_agent_graph"]),
        )
        .await
        .unwrap();
    let active = session.active_tool_names();
    has_core_tools(&active);
    assert!(active.iter().any(|a| a == "memory_store"), "{active:?}");
    assert!(
        active.iter().any(|a| a.contains("agent_graph")),
        "{active:?}"
    );
    // C: built-in declaration bytes also remain frozen until compaction.
    session
        .host_call("unipi.long-horizon", &cx, update(&[], &["bash"]))
        .await
        .unwrap();
    assert!(session.active_tool_names().iter().any(|a| a == "bash"));
}

/// Services evicted and re-created (hub maxServices / idle disposal) and the session woken
/// from its file while in graph mode: the built-ins come back with the extension's mode tools.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graph_mode_session_rewoken_on_fresh_services_keeps_builtins() {
    use oino_core::extensions::api::CallKind;
    use oino_core::extensions::host::HostOp;
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    let file = {
        let (_services, runtime) =
            graph_runtime_with(dir.path(), SessionSource::New, Some(sessions.clone())).await;
        let session = runtime.session();
        let cx = session.runner().call_context().with_kind(CallKind::Event);
        session
            .host_call(
                "unipi.long-horizon",
                &cx,
                HostOp::SetActiveTools(vec!["view_agent_graph".into()]),
            )
            .await
            .unwrap();
        has_core_tools(&session.active_tool_names());
        session
            .prompt("hi".into(), PromptOptions::default())
            .await
            .unwrap();
        session.wait_for_idle().await;
        session.session_file().expect("persisted")
    };
    // Fresh services (the evicted ones are gone) and the session woken from its file.
    let (_services, runtime) =
        graph_runtime_with(dir.path(), SessionSource::Open(file), Some(sessions)).await;
    let active = runtime.session().active_tool_names();
    has_core_tools(&active);
    assert!(active.iter().any(|a| a == "view_agent_graph"), "{active:?}");
}
