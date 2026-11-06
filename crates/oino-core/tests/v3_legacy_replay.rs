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
    resume_with_base(path, &base).await
}

async fn resume_with_base(path: &Path, base: &AgentSession) -> AgentSession {
    let cwd = path.parent().unwrap().to_path_buf();
    let manager = SessionManager::open(path, &cwd, Some(&cwd)).unwrap();
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
        models: base.models().clone(),
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

#[tokio::test]
async fn actual_format_v2_preserves_source_bytes_on_open_append_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("format-v2.jsonl");
    let original = include_bytes!("fixtures/v2/format-v2-replay.jsonl");
    assert!(
        std::str::from_utf8(original)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .contains("\"version\":2")
    );
    std::fs::write(&path, original).unwrap();
    // Opening alone must not normalize the header, whitespace, or old records.
    let manager = SessionManager::open(&path, dir.path(), Some(dir.path())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(manager.entries().iter().any(|entry| matches!(&entry.kind,
        EntryKind::Compaction(c) if c.system_message.is_some())));
    drop(manager);
    let session = resume(&path).await;
    session
        .append_reminder("test", "test/v2", "new tail")
        .unwrap();
    let appended = std::fs::read(&path).unwrap();
    assert!(appended.starts_with(original));
    assert!(appended.len() > original.len());
    assert_eq!(
        session
            .entries()
            .iter()
            .filter(|entry| matches!(entry.kind, EntryKind::SystemSnapshot { version: 3, .. }))
            .count(),
        1
    );
    drop(session);
    let reopened = resume(&path).await;
    assert_eq!(std::fs::read(&path).unwrap(), appended);
    assert_eq!(
        reopened
            .entries()
            .iter()
            .filter(|entry| matches!(entry.kind, EntryKind::SystemSnapshot { version: 3, .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn image_policy_is_restored_per_entry_after_reopen() {
    use oino_core::session::PromptOptions;
    use oino_core::testing::FauxResponse;
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into()); 2]);
    let session = TestSession {
        session_dir: Some(dir.path().to_path_buf()),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    session
        .prompt(
            "stored image".into(),
            PromptOptions {
                images: vec![oino_ai::ImageContent {
                    data: "persisted-image".into(),
                    mime_type: "image/png".into(),
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    session.wait_for_idle().await;
    let first = provider.requests()[0].clone();
    let path = session.session_file().unwrap();
    // A fresh session object has no in-memory policy lookup.
    let reopened = resume_with_base(&path, &session).await;
    reopened
        .with_settings(|settings| settings.set_block_images(true))
        .unwrap();
    reopened
        .prompt("next turn".into(), PromptOptions::default())
        .await
        .unwrap();
    reopened.wait_for_idle().await;
    let second = provider.requests()[1].clone();
    let find = |request: &Value| {
        request["context"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["content"].to_string().contains("stored image"))
            .unwrap()
            .clone()
    };
    assert_eq!(find(&first), find(&second));
    assert!(find(&second).to_string().contains("persisted-image"));
    assert!(
        second
            .to_string()
            .contains("Image reading is now disabled for new content")
    );
    assert_eq!(
        reopened
            .entries()
            .iter()
            .filter(|entry| matches!(&entry.kind,
        EntryKind::Custom {custom_type, ..} if custom_type == "oino.image-policy"))
            .count(),
        session
            .entries()
            .iter()
            .filter(|entry| matches!(&entry.kind,
        EntryKind::Custom {custom_type, ..} if custom_type == "oino.image-policy"))
            .count()
            + 1
    );
}
