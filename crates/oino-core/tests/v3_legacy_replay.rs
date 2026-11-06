//! Legacy records are replayed, never executed or rewritten by prompt migration.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use oino_ai::{AgentMessage, ThinkingLevel};
use oino_core::session::{AgentSession, SessionConfig};
use oino_core::testing::{FauxProvider, TestSession};
use oino_session::{EntryKind, SessionManager};
use serde_json::Value;
use std::path::Path;

async fn resume(path: &Path) -> AgentSession {
    let base = TestSession::default()
        .build(FauxProvider::new(vec![]))
        .await;
    let cwd = path.parent().unwrap().to_path_buf();
    let manager = SessionManager::open(path, &cwd, Some(&cwd)).unwrap();
    let models =
        oino_models::ModelRegistry::in_memory(std::sync::Arc::new(oino_models::MapEnv::default()));
    let settings = std::sync::Arc::new(std::sync::Mutex::new(
        oino_settings::SettingsManager::in_memory(Default::default()),
    ));
    let resources = std::sync::Arc::new(std::sync::RwLock::new(
        oino_resources::ResourceLoader::new(oino_resources::ResourceLoaderOptions {
            cwd: cwd.clone(),
            no_skills: true,
            no_context_files: true,
            ..Default::default()
        }),
    ));
    AgentSession::new(SessionConfig {
        cwd: cwd.clone(),
        agent_dir: cwd.clone(),
        docs_root: Default::default(),
        manager,
        settings,
        resources,
        models,
        extensions: base.runner().set().clone(),
        stream: std::sync::Arc::new(oino_core::provider::ProviderStream::new(
            base.runner().set().clone(),
        )),
        mode: oino_core::extensions::api::ExtensionMode::Print,
        has_ui: false,
        builtin_tools: vec![],
        custom_tools: vec![],
        initial_active_tools: vec![],
        allowed_tools: None,
        excluded_tools: vec![],
        model: Some(FauxProvider::model()),
        thinking_level: ThinkingLevel::Off,
        scoped_models: vec![],
        flag_values: Default::default(),
        child: None,
    })
}

async fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>, AgentSession) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.jsonl");
    let bytes = include_bytes!("fixtures/v2/cache-replay.jsonl").to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let session = resume(&path).await;
    (dir, path, bytes, session)
}

#[tokio::test]
async fn old_tool_arguments() {
    let (_dir, _path, _bytes, session) = fixture().await;
    let entries = session.entries();
    let calls = entries
        .iter()
        .filter_map(|e| e.message())
        .filter_map(|m| match m {
            AgentMessage::Assistant(a) => Some(&a.content),
            _ => None,
        })
        .flatten()
        .filter_map(|b| match b {
            oino_ai::AssistantContent::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect::<Vec<_>>();
    for name in ["find", "bg_run", "bg_logs", "read", "grep"] {
        assert!(calls.iter().any(|c| c.name == name));
    }
    let read = calls.iter().find(|c| c.name == "read").unwrap();
    assert_eq!(read.arguments["path"], "relative.rs");
    assert!(!read.arguments.contains_key("file_path"));
}

#[tokio::test]
async fn legacy_compaction_and_context_edit() {
    let (_dir, _path, _bytes, session) = fixture().await;
    assert!(
        session
            .entries()
            .iter()
            .any(|e| matches!(e.kind, EntryKind::ContextEdit { .. }))
    );
    assert!(
        session
            .entries()
            .iter()
            .any(|e| matches!(&e.kind, EntryKind::Compaction(c) if c.system_message.is_some()))
    );
    let messages = session.messages();
    let systems = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::System(s) => Some(s),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(systems.len(), 1);
    assert!(!oino_ai::transcript::system_message_text(systems[0]).contains("legacy prompt"));
}

#[tokio::test]
async fn workflow_and_reveal() {
    let (_dir, _path, _bytes, session) = fixture().await;
    let entries: Value = serde_json::to_value(session.entries()).unwrap();
    let text = entries.to_string();
    assert!(text.contains("workflow-mode"));
    assert!(text.contains("skill-registry.reveal"));
}

#[tokio::test]
async fn append_only_original_bytes() {
    let (_dir, path, original, session) = fixture().await;
    session
        .append_reminder("test", "test/replay", "tail")
        .unwrap();
    let after = std::fs::read(&path).unwrap();
    assert!(after.starts_with(&original));
    assert!(after.len() > original.len());
    drop(session);
    let reopened = resume(&path).await;
    assert_eq!(
        reopened
            .entries()
            .iter()
            .filter(|e| matches!(e.kind, EntryKind::SystemSnapshot { .. }))
            .count(),
        1
    );
    assert_eq!(std::fs::read(&path).unwrap(), after);
}
