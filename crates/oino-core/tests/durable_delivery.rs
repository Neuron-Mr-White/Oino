//! Host delivery receipts share a physical JSONL line with their user message.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]
use oino_core::session::delivery::MESSAGE_ID;
use oino_core::session::durable_queue::DurableQueue;
use oino_core::session::{PromptOptions, QueueEdit, QueueKind};
use oino_core::testing::{FauxProvider, FauxResponse, TestSession};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn admitted_message_has_atomic_receipt_and_no_false_interrupted_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("done".into())]);
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(provider)
    .await;
    assert_eq!(
        session.restore_delivery_queue().unwrap(),
        [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
    );
    let command = json!({"type": "prompt", "message": "B"});
    assert!(session.admit_delivery("b", command.clone()).unwrap());
    assert!(!session.admit_delivery("b", command.clone()).unwrap());
    let path = session.session_file().unwrap();
    let queue = DurableQueue::new(&path);
    assert_eq!(
        queue
            .load(&std::collections::HashSet::default())
            .unwrap()
            .len(),
        1
    );
    MESSAGE_ID
        .scope(
            "b".into(),
            session.prompt("B".into(), PromptOptions::default()),
        )
        .await
        .unwrap();
    session.wait_for_idle().await;
    assert_eq!(
        queue.load(&std::collections::HashSet::default()).unwrap(),
        [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
    );
    let bytes = std::fs::read_to_string(&path).unwrap();
    let receipt: Vec<_> = bytes
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["id"] == "b")
        .collect();
    assert_eq!(receipt.len(), 1);
    assert_eq!(receipt[0]["message"]["role"], "user");
    assert!(!session.admit_delivery("b", command).unwrap());
    drop(session);
    assert_eq!(
        oino_session::SessionManager::recover_open_runs(dir.path()).unwrap(),
        [] as [std::path::PathBuf; 0]
    );
}

#[tokio::test]
async fn edits_removal_reorder_and_send_now_preserve_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("done".into())]);
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    session.restore_delivery_queue().unwrap();
    for id in ["b", "c", "d"] {
        session
            .admit_delivery(id, json!({"type": "follow_up", "message": id}))
            .unwrap();
        MESSAGE_ID
            .scope(
                id.to_owned(),
                session.follow_up(id.to_owned(), vec![], oino_core::session::InputSource::Rpc),
            )
            .await
            .unwrap();
    }
    session
        .edit_queue(QueueEdit::ById {
            message_id: "b".into(),
            edit: Box::new(QueueEdit::Edit {
                queue: QueueKind::FollowUp,
                index: 2, // Deliberately stale; durable identity selects b, never d.
                text: "edited".into(),
            }),
        })
        .await
        .unwrap();
    session
        .edit_queue(QueueEdit::Remove {
            queue: QueueKind::FollowUp,
            index: 2,
        })
        .await
        .unwrap();
    session
        .edit_queue(QueueEdit::Move {
            queue: QueueKind::FollowUp,
            from: 1,
            to: 0,
        })
        .await
        .unwrap();
    let queue = DurableQueue::new(&session.session_file().unwrap());
    let items = queue.load(&std::collections::HashSet::default()).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.message_id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "b"]
    );
    assert_eq!(items[1].command["message"], "edited");
    session
        .edit_queue(QueueEdit::SendNow {
            queue: QueueKind::FollowUp,
            index: 0,
        })
        .await
        .unwrap();
    session.wait_for_idle().await;
    assert_eq!(
        queue.load(&std::collections::HashSet::default()).unwrap(),
        [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
    );
    assert!(
        session
            .edit_queue(QueueEdit::ById {
                message_id: "c".into(),
                edit: Box::new(QueueEdit::Remove {
                    queue: QueueKind::FollowUp,
                    index: 0
                }),
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unsupported_snapshot_preserves_original_texts() {
    let dir = tempfile::tempdir().unwrap();
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![]))
    .await;
    session
        .with_manager(oino_session::SessionManager::materialize)
        .unwrap();
    let queue = DurableQueue::new(&session.session_file().unwrap());
    let bytes = br#"{"version":99,"items":[{"commandId":"b","command":{"type":"follow_up","message":"B"}}]}"#;
    std::fs::write(queue.path(), bytes).unwrap();
    assert!(session.restore_delivery_queue().is_err());
    assert_eq!(
        std::fs::read(queue.path().with_extension("json.failed")).unwrap(),
        bytes
    );
}

/// A slash-command's handled receipt must never identify the new extension input.
#[tokio::test]
async fn extension_prompt_does_not_inherit_command_receipt() {
    use oino_core::extensions::host::Delivery;
    use oino_core::session::{InputSource, PromptDisposition};
    for direct in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let provider = FauxProvider::new(vec![FauxResponse::Text("done".into())]);
        let session = TestSession {
            session_dir: Some(dir.path().to_owned()),
            ..TestSession::default()
        }
        .build(Arc::clone(&provider))
        .await;
        session.restore_delivery_queue().unwrap();
        session
            .admit_delivery(
                "outer-command",
                json!({"type":"prompt","message":"/swarm do X"}),
            )
            .unwrap();
        assert!(session.is_idle(), "command admission is not an active run");
        let disposition = MESSAGE_ID
            .scope("outer-command".into(), async {
                if direct {
                    session
                        .prompt(
                            "do X".into(),
                            PromptOptions {
                                source: InputSource::Extension,
                                ..PromptOptions::default()
                            },
                        )
                        .await
                } else {
                    session
                        .send_user_message("do X".into(), Some(Delivery::FollowUp), false)
                        .await
                }
            })
            .await
            .unwrap();
        assert_eq!(
            disposition,
            PromptDisposition::Started,
            "idle FollowUp starts a run"
        );
        session.finish_delivery("outer-command").unwrap();
        session.wait_for_idle().await;
        assert_eq!(provider.requests().len(), 1);
        let entries = session.entries();
        let users: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.message(), Some(oino_ai::AgentMessage::User(_))))
            .collect();
        assert_eq!(users.len(), 1);
        assert_ne!(users[0].id.as_str(), "outer-command");
        assert!(
            session
                .admit_delivery("outer-command", json!({"type":"prompt","message":"retry"}))
                .unwrap(),
            "the extension did not consume the command identity"
        );
    }
}

#[tokio::test]
async fn extension_queued_follow_up_does_not_inherit_command_receipt() {
    use oino_core::session::{InputSource, QueueKind};
    let dir = tempfile::tempdir().unwrap();
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![]))
    .await;
    session.restore_delivery_queue().unwrap();
    session
        .admit_delivery("outer", json!({"type":"prompt","message":"/swarm do X"}))
        .unwrap();
    MESSAGE_ID
        .scope(
            "outer".into(),
            session.follow_up("do X".into(), vec![], InputSource::Extension),
        )
        .await
        .unwrap();
    assert_eq!(session.delivery_queue_ids(QueueKind::FollowUp), vec![None]);
    session.finish_delivery("outer").unwrap();
    session
        .edit_queue(QueueEdit::SendNow {
            queue: QueueKind::FollowUp,
            index: 0,
        })
        .await
        .unwrap();
    session.wait_for_idle().await;
    assert_eq!(
        session
            .entries()
            .iter()
            .filter(|e| matches!(e.message(), Some(oino_ai::AgentMessage::User(_))))
            .count(),
        1
    );
}

#[tokio::test]
async fn extension_follow_up_during_active_run_drains_once_with_own_identity() {
    use oino_core::extensions::host::Delivery;
    use oino_core::session::PromptDisposition;
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first".into()),
        FauxResponse::Text("second".into()),
    ]);
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    session.restore_delivery_queue().unwrap();
    session
        .prompt("active".into(), PromptOptions::default())
        .await
        .unwrap();
    assert!(session.is_streaming());
    session
        .admit_delivery("command", json!({"type":"prompt","message":"/goal do X"}))
        .unwrap();
    let disposition = MESSAGE_ID
        .scope(
            "command".into(),
            session.send_user_message("do X".into(), Some(Delivery::FollowUp), false),
        )
        .await
        .unwrap();
    assert_eq!(disposition, PromptDisposition::Queued);
    assert_eq!(session.delivery_queue_ids(QueueKind::FollowUp), vec![None]);
    session.finish_delivery("command").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
    assert_eq!(provider.requests().len(), 2);
    let entries = session.entries();
    let users: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.message(), Some(oino_ai::AgentMessage::User(_))))
        .collect();
    assert_eq!(users.len(), 2);
    assert!(users.iter().all(|e| e.id.as_str() != "command"));
}

/// Exercise the actual host bridge with a command-local handoff identity. The
/// task cannot poll its spawned send before this test inspects the synced queue.
#[tokio::test]
async fn command_handoff_is_durable_before_return_and_replay_is_deduplicated() {
    use oino_core::extensions::{
        api::CallContext,
        host::{Delivery, HostOp},
    };
    use oino_core::session::delivery::COMMAND_HANDOFF;
    let dir = tempfile::tempdir().unwrap();
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![FauxResponse::Text("done".into())]))
    .await;
    session.restore_delivery_queue().unwrap();
    session
        .admit_delivery("outer", json!({"type":"prompt","message":"/swarm text"}))
        .unwrap();
    let call = || async {
        session
            .host_call(
                "test",
                &CallContext::default(),
                HostOp::SendUserMessage {
                    content: "text".into(),
                    deliver_as: Some(Delivery::FollowUp),
                },
            )
            .await
            .unwrap();
    };
    COMMAND_HANDOFF
        .scope(
            (
                "outer".into(),
                Arc::default(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ),
            call(),
        )
        .await;
    let queue = DurableQueue::new(&session.session_file().unwrap());
    let items = queue.load(&std::collections::HashSet::default()).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items[1].message_id.starts_with("outer:extension:"));
    let child_id = items[1].message_id.clone();
    assert_eq!(items[1].command["message"], "text");
    // Simulate replay before outer removal and before the child is persisted.
    COMMAND_HANDOFF
        .scope(
            (
                "outer".into(),
                Arc::default(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ),
            call(),
        )
        .await;
    assert_eq!(
        queue
            .load(&std::collections::HashSet::default())
            .unwrap()
            .len(),
        2
    );
    session.finish_delivery("outer").unwrap();
    assert_eq!(
        queue
            .load(&std::collections::HashSet::default())
            .unwrap()
            .len(),
        1
    );
    tokio::task::yield_now().await;
    session.wait_for_idle().await;
    // Simulate replay after child persistence but before outer completion.
    COMMAND_HANDOFF
        .scope(
            (
                "outer".into(),
                Arc::default(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ),
            call(),
        )
        .await;
    tokio::task::yield_now().await;
    session.wait_for_idle().await;
    assert_eq!(
        queue.load(&std::collections::HashSet::default()).unwrap(),
        [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
    );
    let users: Vec<_> = session
        .entries()
        .into_iter()
        .filter(|e| matches!(e.message(), Some(oino_ai::AgentMessage::User(_))))
        .collect();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id.as_str(), child_id);
}

/// Child process used by the boundary kill matrix below. Only the parent kills
/// its own child; a marker is written after each completed durability barrier.
#[tokio::test]
async fn handoff_kill_child() {
    use oino_core::extensions::{
        api::CallContext,
        host::{Delivery, HostOp},
    };
    use oino_core::session::delivery::COMMAND_HANDOFF;
    let Some(root) = std::env::var_os("OINO_HANDOFF_KILL_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let stop: usize = std::env::var("OINO_HANDOFF_KILL_POINT")
        .unwrap()
        .parse()
        .unwrap();
    let session = TestSession {
        session_dir: Some(root.clone()),
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![FauxResponse::Text("done".into())]))
    .await;
    session.restore_delivery_queue().unwrap();
    std::fs::write(
        root.join("session-path"),
        session.session_file().unwrap().to_string_lossy().as_bytes(),
    )
    .unwrap();
    let checkpoint = |point: usize| {
        if point == stop {
            std::fs::write(root.join("ready"), point.to_string()).unwrap();
            // Block this current-thread test executor so no spawned send can run.
            loop {
                std::thread::park();
            }
        }
    };
    session
        .admit_delivery("outer", json!({"type":"prompt","message":"/swarm text"}))
        .unwrap();
    checkpoint(0);
    COMMAND_HANDOFF
        .scope(
            ("outer".into(), Arc::default(), Arc::default()),
            session.host_call(
                "test",
                &CallContext::default(),
                HostOp::SendUserMessage {
                    content: "text".into(),
                    deliver_as: Some(Delivery::FollowUp),
                },
            ),
        )
        .await
        .unwrap();
    checkpoint(1);
    session.finish_delivery("outer").unwrap();
    checkpoint(2);
    tokio::task::yield_now().await;
    session.wait_for_idle().await;
    checkpoint(3);
}

#[tokio::test]
async fn handoff_kill_matrix_recovers_exactly_once() {
    use oino_core::extensions::{
        api::CallContext,
        host::{Delivery, HostOp},
    };
    use oino_core::session::delivery::COMMAND_HANDOFF;
    for point in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "handoff_kill_child", "--nocapture"])
            .env("OINO_HANDOFF_KILL_ROOT", dir.path())
            .env("OINO_HANDOFF_KILL_POINT", point.to_string())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !dir.path().join("ready").exists() {
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("checkpoint {point} not reached");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let path = std::path::PathBuf::from(
            std::fs::read_to_string(dir.path().join("session-path")).unwrap(),
        );
        let session = TestSession::default()
            .build(FauxProvider::new(vec![FauxResponse::Text("done".into())]))
            .await;
        session.with_manager(|manager| {
            *manager = oino_session::SessionManager::open(&path, dir.path(), None).unwrap();
        });
        let items = session.restore_delivery_queue().unwrap();
        if items.iter().any(|item| item.message_id == "outer") {
            COMMAND_HANDOFF
                .scope(
                    ("outer".into(), Arc::default(), Arc::default()),
                    session.host_call(
                        "test",
                        &CallContext::default(),
                        HostOp::SendUserMessage {
                            content: "text".into(),
                            deliver_as: Some(Delivery::FollowUp),
                        },
                    ),
                )
                .await
                .unwrap();
            session.finish_delivery("outer").unwrap();
        }
        // A fresh handoff above starts itself; a restored child is dispatched
        // using its independent identity, just like InProcess queue recovery.
        for item in items.into_iter().filter(|item| item.message_id != "outer") {
            MESSAGE_ID
                .scope(
                    item.message_id,
                    session.prompt(
                        item.command["message"].as_str().unwrap().to_owned(),
                        PromptOptions::default(),
                    ),
                )
                .await
                .unwrap();
        }
        tokio::task::yield_now().await;
        session.wait_for_idle().await;
        let entries = std::fs::read_to_string(&path).unwrap();
        let users: Vec<serde_json::Value> = entries
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|entry: &serde_json::Value| entry["message"]["role"] == "user")
            .collect();
        assert_eq!(users.len(), 1, "kill point {point}: {users:?}");
        assert!(
            users[0]["id"]
                .as_str()
                .unwrap()
                .starts_with("outer:extension:")
        );
        assert_eq!(
            DurableQueue::new(&path)
                .load(&std::collections::HashSet::default())
                .unwrap(),
            [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
        );
    }
}

#[tokio::test]
async fn reordered_handoffs_preserve_distinct_payloads_and_equal_occurrences() {
    use oino_core::extensions::{
        api::CallContext,
        host::{Delivery, HostOp},
    };
    use oino_core::session::delivery::COMMAND_HANDOFF;
    let dir = tempfile::tempdir().unwrap();
    let session = TestSession {
        session_dir: Some(dir.path().to_owned()),
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![FauxResponse::Text("done".into())]))
    .await;
    session.restore_delivery_queue().unwrap();
    for messages in [vec!["A", "A"], vec!["B", "A", "A"]] {
        COMMAND_HANDOFF
            .scope(("outer".into(), Arc::default(), Arc::default()), async {
                for message in messages {
                    session
                        .host_call(
                            "test",
                            &CallContext::default(),
                            HostOp::SendUserMessage {
                                content: message.into(),
                                deliver_as: Some(Delivery::FollowUp),
                            },
                        )
                        .await
                        .unwrap();
                }
            })
            .await;
    }
    let items = DurableQueue::new(&session.session_file().unwrap())
        .load(&std::collections::HashSet::default())
        .unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items
            .iter()
            .filter(|item| item.command["message"] == "A")
            .count(),
        2
    );
    assert_eq!(
        items
            .iter()
            .filter(|item| item.command["message"] == "B")
            .count(),
        1
    );
    assert_ne!(items[0].message_id, items[1].message_id);
    tokio::task::yield_now().await;
    session.wait_for_idle().await;
}
