//! Exercises the real bundled subagents component's WIT reminder import.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, ComponentSource, EventKind, ExtensionRuntime, InitInput, LoadedPackage,
    RawJson,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_reminders_append_without_prompt_mutation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let package_root = root.join("extensions/dist/unipi.subagents");
    let manifest =
        Manifest::parse(&std::fs::read_to_string(package_root.join("extension.json")).unwrap())
            .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let runtime = WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
        dir.path(),
        dir.path(),
    )))
    .unwrap();
    let host = RecordingHostServices::new();
    let extension = runtime
        .load(
            LoadedPackage {
                manifest,
                component: ComponentSource::Path(package_root.join("component.wasm")),
                root: Some(package_root),
                bundled: true,
            },
            host.clone(),
        )
        .await
        .unwrap();
    let mut cx = CallContext {
        session_id: "reminders".into(),
        cwd: dir.path().into(),
        kind: CallKind::Init,
        ..Default::default()
    };
    extension
        .init(InitInput::default(), cx.clone())
        .await
        .unwrap();
    cx.kind = CallKind::Event;
    let input = json!({"type":"before_agent_start", "prompt":"hello",
        "systemPrompt":"immutable", "systemPromptOptions":{"sections":{}}});
    for _ in 0..2 {
        let result = extension
            .on_event(
                EventKind::BeforeAgentStart,
                RawJson::from(input.clone()),
                cx.clone(),
            )
            .await
            .unwrap();
        assert!(result.is_none());
    }
    let entries = host.entries();
    let reminders: Vec<_> = entries
        .iter()
        .filter(|entry| entry["customType"] == "oino.system-reminder")
        .collect();
    assert_eq!(reminders.len(), 2);
    assert_ne!(reminders[0]["id"], reminders[1]["id"]);
    assert_eq!(reminders[0]["details"]["key"], "unipi.subagents/catalog");
    assert!(
        reminders[0]["content"]
            .as_str()
            .unwrap()
            .contains("<system-reminder>")
    );
    assert_eq!(input["systemPrompt"], "immutable");
    drop(extension);
    drop(runtime);
}
