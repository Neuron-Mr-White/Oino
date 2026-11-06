//! C: mode changes and re-registration keep previously advertised declaration bytes
//! stable on the next request. Tools are declared at startup in registry order and
//! later catalog changes must not replace that prefix.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;

use indexmap::IndexMap;
use oino_ai::{JsonObject, Message};
use oino_core::extensions::api::CallContext;
use oino_core::extensions::api::{
    CallKind, ExtensionInstance, ExtensionMode, Registration, ToolExposure, ToolSpec,
};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::PromptOptions;
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;
use serde_json::{Value, json};

const GRAPH_TOOLS: [&str; 5] = [
    "update_agent_graph",
    "view_agent_graph",
    "yield_agent_graph",
    "agent_output",
    "agent_swarm_status",
];

fn tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        label: name.into(),
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        exposure: ToolExposure::Direct,
        default_active: Some(false),
        execution_mode: None,
        namespace: None,
        annotations: None,
        reaches_tools: None,
        output_schema: None,
        prepare_arguments: false,
    }
}

async fn runtime(dir: &std::path::Path, provider: Arc<FauxProvider>) -> AgentSessionRuntime {
    let lh = Arc::new(FakeExtension::new(
        "unipi.long-horizon",
        Registration {
            tools: GRAPH_TOOLS.iter().map(|t| tool(t)).collect(),
            ..Registration::default()
        },
    ));
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let instances: Vec<Arc<dyn ExtensionInstance>> = vec![
        Arc::new(FauxComponent(provider)),
        lh as Arc<dyn ExtensionInstance>,
    ];
    let services = Services::create(ServicesOptions {
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
    .unwrap();
    AgentSessionRuntime::create(
        services,
        SessionOptions {
            source: SessionSource::InMemory,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap()
}

/// The frozen tools a provider request declares.
fn declared(request: &Value) -> Vec<String> {
    let messages = request
        .pointer("/context/messages")
        .cloned()
        .expect("request context");
    let messages: Vec<Message> = serde_json::from_value(messages).unwrap();
    oino_ai::transcript::current_tools(&messages)
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mode_switch_mid_chat_reaches_the_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("planned".into()),
        FauxResponse::Text("graphing".into()),
        FauxResponse::Text("plain".into()),
    ]);
    let runtime = runtime(dir.path(), Arc::clone(&provider)).await;
    let session = runtime.session();
    let turn = |text: &'static str| {
        let session = session.clone();
        async move {
            session
                .prompt(text.into(), PromptOptions::default())
                .await
                .unwrap();
            session.wait_for_idle().await;
        }
    };
    turn("plan it").await;
    let initial = declared(&provider.requests()[0]);
    assert!(initial.iter().any(|t| t == "update_agent_graph"));
    let prefix = provider.requests()[0]["context"]["messages"][0].clone();
    // `/graph <task>` (a command call) switches the tools before its prompt runs.
    let cx = session.runner().call_context().with_kind(CallKind::Command);
    let names = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            HostOp::UpdateActiveTools {
                enable: names(&GRAPH_TOOLS),
                disable: Vec::new(),
            },
        )
        .await
        .unwrap();
    turn("go").await;
    let tools = declared(&provider.requests()[1]);
    for name in GRAPH_TOOLS {
        assert!(
            tools.iter().any(|t| t == name),
            "{name} missing from turn 2: {tools:?}"
        );
    }
    assert_eq!(provider.requests()[1]["context"]["messages"][0], prefix);
    // C: `/regular` must not remove the frozen graph declarations.
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            HostOp::UpdateActiveTools {
                enable: Vec::new(),
                disable: names(&GRAPH_TOOLS),
            },
        )
        .await
        .unwrap();
    turn("thanks").await;
    let tools = declared(&provider.requests()[2]);
    assert!(tools == initial, "{tools:?}");
}

/// The tool definitions (name → description) a provider request declares.
fn declared_descriptions(request: &Value) -> Vec<(String, String)> {
    let messages: Vec<Message> = serde_json::from_value(
        request
            .pointer("/context/messages")
            .cloned()
            .expect("request context"),
    )
    .unwrap();
    oino_ai::transcript::current_tools(&messages)
        .into_iter()
        .map(|t| (t.name, t.description))
        .collect()
}

/// C: re-registering a tool updates the registry without replacing its declaration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tool_re_registered_without_a_session_reaches_the_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("one".into()),
        FauxResponse::Text("two".into()),
    ]);
    let runtime = runtime(dir.path(), Arc::clone(&provider)).await;
    let session = runtime.session();
    let cx = session.runner().call_context().with_kind(CallKind::Command);
    session
        .host_call(
            "unipi.long-horizon",
            &cx,
            HostOp::UpdateActiveTools {
                enable: vec!["view_agent_graph".into()],
                disable: Vec::new(),
            },
        )
        .await
        .unwrap();
    session
        .prompt("one".into(), PromptOptions::default())
        .await
        .unwrap();
    session.wait_for_idle().await;
    let before = declared_descriptions(&provider.requests()[0]);
    assert!(
        before
            .iter()
            .any(|(name, description)| name == "view_agent_graph"
                && description.starts_with("view_agent_graph tool"))
    );
    let mut changed = tool("view_agent_graph");
    changed.description = "view the graph (profiles: a, b)".into();
    runtime
        .services()
        .host
        .call(
            "unipi.long-horizon",
            &CallContext::default(),
            HostOp::RegisterTool(changed),
        )
        .await
        .unwrap();
    session
        .prompt("two".into(), PromptOptions::default())
        .await
        .unwrap();
    session.wait_for_idle().await;
    let after = declared_descriptions(&provider.requests()[1]);
    assert_eq!(
        after, before,
        "C: previously declared schema bytes are immutable"
    );
}
