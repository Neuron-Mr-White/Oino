//! agent-disable: `subagents.disabled` through the real subagents component in the wasm
//! host. A disabled profile leaves the system-prompt section, the `run_subagent` schema (the
//! tool is re-registered only when the offered set changes) and `subagents.profiles`; a call
//! naming it is refused naming the enabled ones; the graph's default falls back; a settings
//! change applies on the next turn (no reload), and a project list overrides the global one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::ops::SettingsScope;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
    ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// Builds the subagents component once (same target dir as the other harness tests).
fn wasm() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(extensions_dir()).args([
            "build",
            "-j",
            "2",
            "--release",
            "--target",
            "wasm32-wasip2",
            "-p",
            "unipi-subagents",
        ]);
        for var in [
            "CARGO_TARGET_DIR",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_MANIFEST_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("CARGO_TARGET_DIR", extensions_dir().join("target"));
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        extensions_dir().join("target/wasm32-wasip2/release/unipi_subagents.wasm")
    })
}

fn cx(project: &std::path::Path, kind: CallKind) -> CallContext {
    CallContext {
        session_id: "s1".into(),
        mode: Mode::Rpc,
        has_ui: true,
        cwd: project.to_path_buf(),
        kind,
        ..CallContext::default()
    }
}

fn text_of(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_profiles_leave_the_prompt_schema_and_graph_and_are_refused() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
    options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
    let runtime = WasmRuntime::new(options).unwrap();
    let host = RecordingHostServices::new();
    // Start with subagent_general disabled globally (what Settings writes).
    host.preset_extension_setting(
        SettingsScope::Global,
        "subagents",
        "disabled",
        json!(["subagent_general"]),
    );
    let manifest = Manifest::parse(
        &std::fs::read_to_string(extensions_dir().join("unipi/subagents/extension.json")).unwrap(),
    )
    .unwrap();
    let package = LoadedPackage {
        manifest,
        component: ComponentSource::Path(wasm().clone()),
        root: None,
        bundled: true,
    };
    let services: Arc<dyn HostServices> = Arc::clone(&host) as Arc<dyn HostServices>;
    let sa = runtime.load(package, services).await.unwrap();
    let init = InitInput {
        settings: RawJson::from(json!({"subagents": {"disabled": ["subagent_general"]}})),
        ..InitInput::default()
    };
    let reg = sa
        .init(init, cx(project.path(), CallKind::Init))
        .await
        .unwrap();
    let profile_desc = |params: &Value| {
        params["properties"]["profile"]["description"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let desc = profile_desc(&reg.tools[0].parameters.to_value());
    assert!(desc.contains("subagent_explore — Read-only"), "{desc}");
    assert!(!desc.contains("subagent_general"), "{desc}");

    let event = |kind: EventKind, payload: Value| {
        let sa = Arc::clone(&sa);
        let c = cx(project.path(), CallKind::Event);
        async move {
            sa.on_event(kind, RawJson::from(payload), c)
                .await
                .unwrap()
                .map(|r| r.to_value())
        }
    };
    let tool = |args: Value| {
        let sa = Arc::clone(&sa);
        let mut c = cx(project.path(), CallKind::Tool);
        async move {
            let inv = ToolInvocation {
                tool_call_id: "call-run".into(),
                name: "run_subagent".into(),
                args: RawJson::from(args),
                parent_tool_call_id: None,
            };
            c.tool_call_id = Some(inv.tool_call_id.clone());
            sa.run_tool(inv, CancellationToken::new(), c)
                .await
                .unwrap()
                .to_value()
        }
    };
    let section = || async {
        let result = event(
            EventKind::BeforeAgentStart,
            json!({"type": "before_agent_start", "prompt": "hi", "systemPrompt": "", "systemPromptOptions": {"sections": {}}}),
        ).await;
        assert!(result.is_none());
        host.entries().into_iter().rev()
            .find(|entry| entry["details"]["key"] == "unipi.subagents/catalog")
            .unwrap()["content"].as_str().unwrap().to_owned()
    };
    let profiles = || async {
        sa.handle_service(
            "subagents.profiles",
            RawJson::from(json!({})),
            &ExtensionId::new("unipi.long-horizon").unwrap(),
            cx(project.path(), CallKind::Service),
        )
        .await
        .unwrap()
        .to_value()
    };
    let graph_run = |profile: &str| {
        let sa = Arc::clone(&sa);
        let c = cx(project.path(), CallKind::Service);
        let payload = json!({"session": "s1", "title": "t", "task": "x", "profile": profile});
        async move {
            sa.handle_service(
                "subagents.graph-run",
                RawJson::from(payload),
                &ExtensionId::new("unipi.long-horizon").unwrap(),
                c,
            )
            .await
            .unwrap()
            .to_value()
        }
    };

    event(
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    // The schema registered at init already matches: no re-registration.
    assert!(host.calls_named("register_tool").is_empty());
    let s = section().await;
    assert!(s.contains("- subagent_explore: Read-only"), "{s}");
    assert!(!s.contains("subagent_general"), "{s}");
    assert_eq!(profiles().await, json!(["subagent_explore"]));
    let refused = tool(json!({"title": "t", "task": "x", "profile": "subagent_general"})).await;
    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(
        text_of(&refused),
        "Profile \"subagent_general\" is disabled in Settings › Subagents › Agent profiles. Enabled profiles: subagent_explore"
    );
    assert_eq!(
        refused["details"]["errorCode"], "subagents.profile.disabled",
        "{refused}"
    );
    // The graph's filled-in default runs the first enabled profile instead.
    let ran = graph_run("subagent_general").await;
    assert_eq!(ran["profile"], "subagent_explore", "{ran}");

    // Re-enabled in Settings: applies on the next turn, no reload; the tool is
    // re-registered once with the full list.
    host.preset_extension_setting(SettingsScope::Global, "subagents", "disabled", json!([]));
    event(
        EventKind::OinoSettingsChanged,
        json!({"namespace": "subagents", "keys": ["disabled"], "scope": "global"}),
    )
    .await;
    let s = section().await;
    assert!(s.contains("- subagent_general: General-purpose"), "{s}");
    assert_eq!(
        profiles().await,
        json!(["subagent_explore", "subagent_general"])
    );
    let reregs = host.calls_named("register_tool");
    assert_eq!(reregs.len(), 1, "{reregs:?}");
    assert!(profile_desc(&reregs[0].detail["parameters"]).contains("subagent_general — General"));
    // Another turn with the same set: nothing re-registered (stable tool schema).
    section().await;
    assert_eq!(host.calls_named("register_tool").len(), 1);

    // A project list overrides the global one; a list that disables everything keeps the
    // first profile.
    host.preset_extension_setting(
        SettingsScope::Project,
        "subagents",
        "disabled",
        json!(["subagent_explore", "subagent_general"]),
    );
    event(
        EventKind::OinoSettingsChanged,
        json!({"namespace": "subagents", "keys": ["disabled"], "scope": "project"}),
    )
    .await;
    assert_eq!(profiles().await, json!(["subagent_explore"]));
    let unknown = tool(json!({"title": "t", "task": "x", "profile": "nope"})).await;
    assert_eq!(
        text_of(&unknown),
        "Unknown profile \"nope\". Available: subagent_explore"
    );
}
