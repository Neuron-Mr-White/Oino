//! Release guard for persisted per-turn policy work, including a 72 MB-shaped history.
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    reason = "test fixture and timing output"
)]
use super::*;
use oino_ai::UserContentField;
use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

fn fixture(size: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    writeln!(
        file,
        "{}",
        serde_json::json!({"type":"session","version":3,"id":"perf",
        "timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"})
    )
    .unwrap();
    let content = "x".repeat(1650);
    for n in 0..size {
        writeln!(
            file,
            "{}",
            serde_json::json!({"type":"message","id":format!("e{n}"),
            "parentId": n.checked_sub(1).map(|n| format!("e{n}")),
            "timestamp":"2026-01-01T00:00:00.000Z",
            "message":{"role":"user","content":content,"timestamp":n}})
        )
        .unwrap();
    }
    drop(file);
    (dir, path)
}

#[tokio::test]
async fn policies_follow_branch_and_lazy_reopen() {
    let (dir, path) = fixture(100);
    let session = crate::testing::TestSession::default()
        .build(crate::testing::FauxProvider::new(vec![]))
        .await;
    session.with_manager(|m| {
        *m = SessionManager::open(&path, dir.path(), None).unwrap();
        m.append_custom_entry("oino:autonomy", Some(serde_json::json!({"level":"high"})))
            .unwrap();
        m.append_custom_entry("oino:superagent", None).unwrap();
    });
    assert_eq!(session.autonomy(), permission::Level::High);
    assert!(session.with_manager(|m| m.has_policy("oino:superagent")));
    session.with_manager(|m| m.set_leaf(&EntryId::new("e99")).unwrap());
    assert!(!session.with_manager(|m| m.has_policy("oino:superagent")));
    assert_ne!(session.autonomy(), permission::Level::High);
    let message = AgentMessage::User(oino_ai::UserMessage {
        author: None,
        content: UserContentField::Text("new".into()),
        timestamp: 1001,
    });
    session.with_manager(|m| m.append_message(message.clone()).unwrap());
    session
        .snapshot_image_policy(std::slice::from_ref(&message))
        .unwrap();
    let count = session.with_manager(|m| m.policy_data("oino.image-policy").count());
    session
        .snapshot_image_policy(std::slice::from_ref(&message))
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        session.with_manager(|m| m.policy_data("oino.image-policy").count()),
        count
    );
}

#[tokio::test]
async fn image_pins_and_autonomy_survive_checkpoint_reopen() {
    let (dir, path) = fixture(100);
    let session = crate::testing::TestSession::default()
        .build(crate::testing::FauxProvider::new(vec![]))
        .await;
    session.with_manager(|m| {
        *m = SessionManager::open(&path, dir.path(), None).unwrap();
        m.append_custom_entry("oino:autonomy", Some(serde_json::json!({"level":"high"})))
            .unwrap();
        m.append_custom_entry(
            "oino.image-policy",
            Some(serde_json::json!({"entryId":"e99","blocked":true})),
        )
        .unwrap();
        m.append_compaction(oino_session::CompactionDraft {
            summary: "checkpoint".into(),
            first_kept_entry_id: Some(EntryId::new("e99")),
            ..Default::default()
        })
        .unwrap();
        m.sync().unwrap();
        *m = SessionManager::open(&path, dir.path(), None).unwrap();
    });
    assert_eq!(session.autonomy(), permission::Level::High);
    let message = session.with_manager(|m| {
        m.policy_messages(99)
            .next()
            .unwrap()
            .message()
            .unwrap()
            .clone()
    });
    session
        .snapshot_image_policy(std::slice::from_ref(&message))
        .unwrap();
    assert_eq!(
        lock(&session.inner.image_policy).blocked(&message),
        Some(true)
    );
    assert_eq!(
        session.with_manager(|m| m.policy_data("oino.image-policy").count()),
        1
    );
}

#[tokio::test]
#[ignore = "release 72 MB persisted policy guard"]
async fn persisted_turn_policies_are_history_independent() {
    async fn measure(size: usize) -> (f64, f64) {
        let (dir, path) = fixture(size);
        let session = crate::testing::TestSession::default()
            .build(crate::testing::FauxProvider::new(vec![]))
            .await;
        session.with_manager(|m| {
            *m = SessionManager::open(&path, dir.path(), None).unwrap();
            m.append_custom_entry("oino:autonomy", Some(serde_json::json!({"level":"medium"})))
                .unwrap();
        });
        black_box(session.autonomy());
        session.snapshot_image_policy(&[]).unwrap();
        let start = Instant::now();
        for n in 0..500 {
            let message = AgentMessage::User(oino_ai::UserMessage {
                author: None,
                content: UserContentField::Text("next".into()),
                timestamp: 100_000 + n,
            });
            session.with_manager(|m| m.append_message(message.clone()).unwrap());
            black_box(session.autonomy());
            session
                .snapshot_image_policy(std::slice::from_ref(&message))
                .unwrap();
        }
        let turn = start.elapsed().as_secs_f64() * 1e6 / 500.;
        session.with_manager(|m| m.sync().unwrap());
        // Background writer may have refreshed during the turn loop. Stabilise once.
        oino_session::manager::OffsetIndex::load(&path, None).unwrap();
        let start = Instant::now();
        for _ in 0..50 {
            black_box(oino_session::manager::OffsetIndex::load(&path, None).unwrap());
        }
        let warm = start.elapsed().as_secs_f64() * 1e6 / 50.;
        eprintln!(
            "entries={size} bytes={} turn_us={turn:.3} warm_index_us={warm:.3}",
            std::fs::metadata(path).unwrap().len()
        );
        (turn, warm)
    }
    let mut small = Vec::new();
    let mut large = Vec::new();
    for _ in 0..5 {
        small.push(measure(100).await);
        large.push(measure(40_000).await);
    }
    small.sort_by(|a, b| a.0.total_cmp(&b.0));
    large.sort_by(|a, b| a.0.total_cmp(&b.0));
    let ratio = large[2].0 / small[2].0;
    eprintln!(
        "median turn_us 100={:.3} 40k={:.3} ratio={ratio:.3}",
        small[2].0, large[2].0
    );
    assert!(
        ratio <= 2.,
        "history-dependent persisted policies: {ratio:.3}"
    );
}

#[tokio::test]
async fn no_compaction_needed_is_bounded_on_large_history() {
    let session = crate::testing::TestSession::default()
        .build(crate::testing::FauxProvider::new(vec![]))
        .await;
    let assistant: AssistantMessage = serde_json::from_value(serde_json::json!({
        "api": "faux", "provider": "faux", "model": "faux-1", "stopReason": "stop"
    }))
    .unwrap();
    // A successful low-usage response cannot require recovery. Check repeatedly
    // even with a large retained branch; neither history nor projection is needed.
    let (dir, path) = fixture(40_000);
    session.with_manager(|m| {
        *m = SessionManager::open(&path, dir.path(), None).unwrap();
    });
    let start = Instant::now();
    for _ in 0..100 {
        assert!(!session.check_compaction(&assistant, false).await);
    }
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
#[ignore = "release transcript snapshot measurement"]
async fn last_assistant_snapshot_does_not_copy_history() {
    let session = crate::testing::TestSession::default()
        .build(crate::testing::FauxProvider::new(vec![]))
        .await;
    let mut messages = vec![
        AgentMessage::User(oino_ai::UserMessage {
            author: None,
            content: UserContentField::Text("x".repeat(1800)),
            timestamp: 0,
        });
        40_000
    ];
    messages.push(AgentMessage::Assistant(AssistantMessage::empty(
        "test", "test", "test", 0,
    )));
    session.inner.agent.set_messages(messages);
    let measure = |copy: bool| {
        let mut samples = Vec::new();
        for _ in 0..200 {
            let start = Instant::now();
            let last = if copy {
                session
                    .messages()
                    .into_iter()
                    .rev()
                    .find_map(|message| match message {
                        AgentMessage::Assistant(assistant) => Some(assistant),
                        _ => None,
                    })
            } else {
                session.last_assistant_message()
            };
            black_box(last);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        (samples[99], samples[189])
    };
    let before = measure(true);
    let after = measure(false);
    eprintln!("last assistant 40k/72MB n200 p50/p95 ms: clone={before:?}, borrowed={after:?}");
    assert!(after.1 < before.1, "borrow only the selected assistant");
}
