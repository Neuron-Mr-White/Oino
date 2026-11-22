//! Real bundled component: command and tool guards share the pending-work definition.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use oino_extension_api::manifest::Manifest;
use oino_extension_api::ops::{SessionsIndexQuery, SessionsScope};
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, ExtensionId, ExtensionRuntime,
    InitInput, LoadedPackage, RawJson, ToolInvocation,
};
use oino_wasm_host::{HostConfig, LocalServices, WasmRuntime, WasmRuntimeOptions, workspace_id};
use serde_json::json;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn all_sessions_index_returns_titles_without_full_history() {
    let agent = tempfile::tempdir().unwrap();
    let sessions = agent.path().join("sessions/fixture");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join("owner.jsonl"), concat!(
        "{\"type\":\"session\",\"version\":3,\"id\":\"owner\",\"cwd\":\"/fixture\",\"timestamp\":\"2026-11-17T00:00:00Z\"}\n",
        "{\"type\":\"session_info\",\"id\":\"title\",\"parentId\":null,\"timestamp\":\"2026-11-17T00:00:00Z\",\"name\":\"Owner title\"}\n"
    )).unwrap();
    let hub = Arc::new(LocalServices::new(agent.path()));
    let id = ExtensionId::new("unipi.long-horizon").unwrap();
    let host = hub.wrap_for_test(&id, RecordingHostServices::new());
    let list = host
        .sessions_index(&id, SessionsIndexQuery::List(SessionsScope::All))
        .await
        .unwrap()
        .to_value();
    assert_eq!(list[0]["id"], "owner");
    assert_eq!(list[0]["name"], "Owner title");
}

/// A real index outage must not turn a persisted graph into proof of liveness.
struct IndexUnavailable;
impl oino_extension_api::HostServices for IndexUnavailable {
    fn sessions_index(
        &self,
        _: &ExtensionId,
        _: SessionsIndexQuery,
    ) -> oino_extension_api::BoxFut<'_, Result<RawJson, oino_extension_api::HostError>> {
        Box::pin(async {
            Err(oino_extension_api::HostError::Io(
                "index unavailable".into(),
            ))
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
#[allow(
    clippy::too_many_lines,
    reason = "Keep this linear integration scenario and its assertions together."
)]
async fn bundled_update_graph_guard_accepts_idle_and_rejects_pending_work() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bundle = root.join("extensions/dist/unipi.long-horizon");
    for status in [
        "completed",
        "failed",
        "stopped",
        "superseded",
        "cancelled",
        "errored",
        "running",
        "queued",
        "waiting",
        "blocked",
        "index-failed",
    ] {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let sessions = agent.path().join("sessions/fixture");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("owner-session.jsonl"),
            json!({
                "type":"session", "version":3, "id":"owner-session",
                "cwd":project.path(), "timestamp":"2026-11-17T00:00:00Z"
            })
            .to_string()
                + "\n",
        )
        .unwrap();
        let state = agent
            .path()
            .join("state/unipi.long-horizon/workspace")
            .join(workspace_id(&project.path().canonicalize().unwrap()));
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("graph.json"),
            json!({"v":1,"current":{
                "v":1,"graphId":"seed","mode":"graph","task":"Seed task",
                "createdAt":1,"updatedAt":1,"revision":1,"ownerSession":"owner-session",
                "work":[{"workId":"work","instruction":"work","inputIds":[],"status":if status == "index-failed" { "running" } else { status },
                    "target":{"kind":"new_preset","subagentId":"worker"}}]
            }})
            .to_string(),
        )
        .unwrap();
        std::fs::write(state.join("state.json"), json!({"active":{
            "ownerId":"owner","kind":"graph","label":"Seed task","session":"owner-session",
            "status":"active","revision":1,"lease":{"ownerId":"owner","generation":1},"updatedAt":"now"
        }}).to_string()).unwrap();
        let runtime = WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
            agent.path(),
            project.path(),
        )))
        .unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(bundle.join("extension.json")).unwrap())
                .unwrap();
        let ext = runtime
            .load(
                LoadedPackage {
                    manifest,
                    component: ComponentSource::Path(bundle.join("component.wasm")),
                    root: Some(bundle.clone()),
                    bundled: true,
                },
                if status == "index-failed" {
                    Arc::new(IndexUnavailable)
                } else {
                    RecordingHostServices::new()
                },
            )
            .await
            .unwrap();
        let cx = CallContext {
            session_id: "other-session".into(),
            cwd: project.path().into(),
            kind: CallKind::Init,
            ..Default::default()
        };
        ext.init(InitInput::default(), cx.clone()).await.unwrap();
        // Invalid op deliberately stops immediately after the ownership guard,
        // avoiding subagent scheduling. This exercises update_graph itself.
        let mut tool_cx = cx;
        tool_cx.kind = CallKind::Tool;
        let result = ext
            .run_tool(
                ToolInvocation {
                    tool_call_id: "test".into(),
                    name: "update_agent_graph".into(),
                    args: RawJson::from(json!({})),
                    parent_tool_call_id: None,
                },
                CancellationToken::new(),
                tool_cx,
            )
            .await
            .unwrap()
            .to_value();
        let text = result["content"][0]["text"].as_str().unwrap();
        if matches!(status, "running" | "queued" | "waiting" | "blocked") {
            assert!(text.contains("graph_busy:"), "{status}: {result}");
            assert!(
                text.contains("owner-se") && text.contains("Seed task"),
                "{result}"
            );
        } else {
            assert!(text.contains("Set operation"), "{status}: {result}");
            if status == "index-failed" {
                let saved: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(state.join("graph.json")).unwrap())
                        .unwrap();
                assert_eq!(saved["current"]["abandoned"], "superseded(owner_archived)");
            }
        }
    }
}

/// Uses the production `CoreHost` + `HostRouter`, not `RecordingHostServices`. The
/// configured root deliberately differs from `LocalServices`' default root.
#[tokio::test(flavor = "multi_thread")]
#[allow(
    clippy::too_many_lines,
    reason = "Keep this linear integration scenario and its assertions together."
)]
async fn core_host_lists_live_sessions_and_releases_archived_graph_owner() {
    use oino_core::extensions::api::ExtensionMode;
    use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
    use oino_extension_api::HostServices;
    use oino_wasm_host::core_bridge::CoreHost;
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let index = agent.path().join("configured-sessions");
    let sessions = index.join("fixture");
    std::fs::create_dir_all(&sessions).unwrap();
    let owner = sessions.join("owner.jsonl");
    std::fs::write(
        &owner,
        json!({"type":"session", "version":3,
        "id":"owner-session", "cwd":project.path(),
        "timestamp":"2026-11-17T00:00:00Z"})
        .to_string()
            + "\n",
    )
    .unwrap();
    let services = Services::create(ServicesOptions {
        cwd: project.path().into(),
        agent_dir: agent.path().into(),
        docs_root: std::path::PathBuf::default(),
        mode: ExtensionMode::Desktop,
        has_ui: true,
        flags: ResourceFlags {
            no_context_files: true,
            ..Default::default()
        },
        extension_flag_values: indexmap::IndexMap::default(),
        loader: Arc::new(StaticLoader(vec![])),
        approve: Some(false),
        models: None,
        settings: Some(oino_settings::SettingsManager::in_memory(
            serde_json::Map::default(),
        )),
    })
    .await
    .unwrap();
    services.host.set_sessions_root(index);
    let core = Arc::new(CoreHost::new(services.host.clone()));
    let id = ExtensionId::new("unipi.long-horizon").unwrap();
    let list = core
        .sessions_index(&id, SessionsIndexQuery::List(SessionsScope::All))
        .await
        .unwrap()
        .to_value();
    assert_eq!(list[0]["id"], "owner-session");
    // Archiving removes the owner from the live index, not from the disk entirely.
    let trash = agent.path().join("trash");
    std::fs::create_dir(&trash).unwrap();
    std::fs::rename(&owner, trash.join("owner.jsonl")).unwrap();
    assert_eq!(
        core.sessions_index(&id, SessionsIndexQuery::List(SessionsScope::All))
            .await
            .unwrap()
            .to_value(),
        json!([])
    );
    let state = agent
        .path()
        .join("state/unipi.long-horizon/workspace")
        .join(workspace_id(&project.path().canonicalize().unwrap()));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("graph.json"),
        json!({"v":1,"current":{
            "v":1,"graphId":"seed","mode":"graph","task":"Archived graph",
            "createdAt":1,"updatedAt":1,"revision":1,"ownerSession":"owner-session",
            "work":[{"workId":"work","instruction":"work","inputIds":[],"status":"running",
                "target":{"kind":"new_preset","subagentId":"worker"}}]
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(state.join("state.json"), json!({"active":{
        "ownerId":"owner","kind":"graph","label":"Archived graph","session":"owner-session",
        "status":"active","revision":1,"lease":{"ownerId":"owner","generation":1},"updatedAt":"now"
    }}).to_string()).unwrap();
    let runtime = WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
        agent.path(),
        project.path(),
    )))
    .unwrap();
    let bundle = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/dist/unipi.long-horizon");
    let manifest =
        Manifest::parse(&std::fs::read_to_string(bundle.join("extension.json")).unwrap()).unwrap();
    assert!(manifest.permissions.sessions.read_all);
    let ext = runtime
        .load(
            LoadedPackage {
                manifest,
                component: ComponentSource::Path(bundle.join("component.wasm")),
                root: Some(bundle),
                bundled: true,
            },
            core,
        )
        .await
        .unwrap();
    let cx = CallContext {
        session_id: "other-session".into(),
        cwd: project.path().into(),
        kind: CallKind::Init,
        ..Default::default()
    };
    ext.init(InitInput::default(), cx.clone()).await.unwrap();
    let result = ext
        .run_tool(
            ToolInvocation {
                tool_call_id: "test".into(),
                name: "update_agent_graph".into(),
                args: RawJson::from(json!({})),
                parent_tool_call_id: None,
            },
            CancellationToken::new(),
            cx.with_kind(CallKind::Tool),
        )
        .await
        .unwrap()
        .to_value();
    assert!(!result.to_string().contains("graph_busy:"), "{result}");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("graph.json")).unwrap()).unwrap();
    assert_eq!(saved["current"]["abandoned"], "superseded(owner_archived)");
}
