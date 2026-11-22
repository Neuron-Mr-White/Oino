//! Seeded append/branch/compact parity, including cold persisted-cache reopen.
#![allow(
    clippy::unwrap_used,
    reason = "tests assert successful fixture operations"
)]
use std::path::Path;

use oino_ai::{AgentMessage, UserContentField, UserMessage};
use oino_session::manager::{OffsetIndex, forget_offsets};
use oino_session::{CompactionDraft, CreateOptions, SessionManager};

#[test]
fn seeded_incremental_index_and_lazy_context_match_full() {
    for seed in 1..=8u64 {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            SessionManager::create(Path::new("/tmp"), dir.path(), CreateOptions::default())
                .unwrap();
        let mut random = seed;
        for step in 0..100 {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let choice = random >> 32;
            if step > 0 && choice % 11 == 0 {
                let entries = manager.entries();
                let id = entries[choice as usize % entries.len()].id.clone();
                manager.set_leaf(&id).unwrap();
            }
            let user = manager
                .append_message(AgentMessage::User(UserMessage {
                    author: None,
                    content: UserContentField::Text(format!("seed {seed} step {step}")),
                    timestamp: step,
                }))
                .unwrap();
            if choice % 5 == 0 {
                manager
                    .append_compaction(CompactionDraft {
                        summary: format!("checkpoint {step}"),
                        first_kept_entry_id: Some(user),
                        ..CompactionDraft::default()
                    })
                    .unwrap();
            }
            manager
                .append_custom_entry(
                    "ignored.large",
                    Some(serde_json::json!({"body": "x".repeat(100)})),
                )
                .unwrap();
            if choice % 9 == 0 {
                manager
                    .append_session_info(Some(&format!("title {step}")))
                    .unwrap();
            }
            manager
                .append_model_change("test", &format!("model-{step}"))
                .unwrap();
            manager
                .append_thinking_level_change(if choice % 2 == 0 { "high" } else { "off" })
                .unwrap();
            let path = manager.session_file().unwrap();
            let incremental = OffsetIndex::load(path, None).unwrap();
            forget_offsets(path);
            let _ = std::fs::remove_file(OffsetIndex::sidecar_path(path));
            let rebuilt = OffsetIndex::load(path, None).unwrap();
            assert_eq!(
                incremental.entries(),
                rebuilt.entries(),
                "seed {seed} step {step}"
            );
            forget_offsets(path);
            let lazy = SessionManager::open(path, dir.path(), None).unwrap();
            let full = SessionManager::in_memory(
                Path::new("/tmp"),
                CreateOptions::default(),
                manager.entries().to_vec(),
            );
            assert_eq!(lazy.leaf_id(), full.leaf_id());
            assert_eq!(
                lazy.build_session_context(),
                full.build_session_context(),
                "seed {seed} step {step}"
            );
            assert_eq!(lazy.branch(None), full.branch(None));
            assert_eq!(lazy.tree_flat(), full.tree_flat());
            assert_eq!(lazy.session_name(), full.session_name());
        }
    }
}

#[test]
fn append_after_lazy_wake_preserves_context_history_and_fork() {
    let dir = tempfile::tempdir().unwrap();
    let mut original =
        SessionManager::create(Path::new("/tmp"), dir.path(), CreateOptions::default()).unwrap();
    for i in 0..20 {
        let id = original
            .append_message(AgentMessage::User(UserMessage {
                author: None,
                content: UserContentField::Text(format!("turn {i}")),
                timestamp: i,
            }))
            .unwrap();
        if i == 10 {
            original
                .append_compaction(CompactionDraft {
                    summary: "summary".into(),
                    first_kept_entry_id: Some(id),
                    ..CompactionDraft::default()
                })
                .unwrap();
        }
    }
    let path = original.session_file().unwrap().to_path_buf();
    let mut lazy = SessionManager::open(&path, dir.path(), None).unwrap();
    let full_entries = original.entries().to_vec();
    let mut full =
        SessionManager::in_memory(Path::new("/tmp"), CreateOptions::default(), full_entries);
    // Prime both the full-history path and the checkpoint projection before append.
    assert_eq!(lazy.branch(None), full.branch(None));
    assert_eq!(lazy.session_projection(), full.session_projection());
    let msg = AgentMessage::User(UserMessage {
        author: None,
        content: UserContentField::Text("after wake".into()),
        timestamp: 21,
    });
    lazy.append_message(msg.clone()).unwrap();
    full.append_message(msg).unwrap();
    assert_eq!(lazy.build_session_context(), full.build_session_context());
    assert_eq!(lazy.entry_count(), full.entry_count());
    assert_eq!(
        lazy.branch(None)
            .iter()
            .map(|e| &e.kind)
            .collect::<Vec<_>>(),
        full.branch(None)
            .iter()
            .map(|e| &e.kind)
            .collect::<Vec<_>>()
    );
    let leaf = lazy.leaf_id().unwrap().clone();
    let fork = lazy.create_branched_session(&leaf).unwrap();
    let fork = SessionManager::open(&fork, dir.path(), None).unwrap();
    assert_eq!(fork.build_session_context(), lazy.build_session_context());
    assert_eq!(fork.entries(), lazy.entries());
}

#[test]
fn missing_lazy_history_is_reported_by_fallible_api() {
    let dir = tempfile::tempdir().unwrap();
    let mut original =
        SessionManager::create(Path::new("/tmp"), dir.path(), CreateOptions::default()).unwrap();
    original
        .append_message(AgentMessage::User(UserMessage {
            author: None,
            content: UserContentField::Text("hello".into()),
            timestamp: 1,
        }))
        .unwrap();
    let path = original.session_file().unwrap();
    let lazy = SessionManager::open(path, dir.path(), None).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(lazy.try_index().is_err());
}
