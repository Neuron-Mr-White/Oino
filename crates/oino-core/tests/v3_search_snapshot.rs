//! Search settings are frozen in append-only session state.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use oino_core::builtin::search_snapshot;
use oino_session::{CreateOptions, SessionManager};

#[test]
fn snapshot_survives_reload_and_toggle_changes() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SessionManager::in_memory(root.path(), CreateOptions::default(), vec![]);
    assert!(search_snapshot(&mut manager, true).unwrap());
    let entries = manager.entries().to_vec();
    assert!(search_snapshot(&mut manager, false).unwrap());
    assert_eq!(entries, manager.entries());
    let mut reopened = SessionManager::in_memory(root.path(), CreateOptions::default(), entries);
    assert!(search_snapshot(&mut reopened, false).unwrap());
    let mut off = SessionManager::in_memory(root.path(), CreateOptions::default(), vec![]);
    assert!(!search_snapshot(&mut off, false).unwrap());
    assert!(!search_snapshot(&mut off, true).unwrap());
}

#[test]
fn legacy_history_remains_exact_and_bytes_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SessionManager::in_memory(root.path(), CreateOptions::default(), vec![]);
    manager
        .append_custom_message_entry(
            "legacy",
            oino_ai::UserContentField::Text("old conversation".into()),
            true,
            None,
        )
        .unwrap();
    let original = serde_json::to_vec(manager.entries()).unwrap();
    assert!(!search_snapshot(&mut manager, true).unwrap());
    assert_eq!(
        original,
        serde_json::to_vec(&manager.entries()[..manager.entries().len() - 1]).unwrap()
    );
}
