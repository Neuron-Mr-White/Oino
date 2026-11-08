//! Data-safety regressions for anchored compaction and immutable sent history.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use oino_ai::{AgentMessage, UserContentField, UserMessage};
use oino_core::session::PromptOptions;
use oino_core::testing::{FauxProvider, FauxResponse, TestSession};
use oino_session::{
    CompactionDraft, CreateOptions, EditableContent, EntryId, EntryKind, SessionEntry,
    SessionManager,
};
use serde_json::json;

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: UserContentField::Text(text.into()),
        timestamp: 1,
    })
}
fn manager() -> SessionManager {
    SessionManager::in_memory(&std::env::temp_dir(), CreateOptions::default(), vec![])
}
fn draft(anchor: &EntryId) -> CompactionDraft {
    CompactionDraft {
        summary: "Durable context".into(),
        first_kept_entry_id: Some(anchor.clone()),
        ..Default::default()
    }
}
async fn turn(session: &oino_core::session::AgentSession, text: &str) {
    session
        .prompt(text.into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

#[test]
fn crash_before_append() {
    let mut m = manager();
    let a = m.append_message(user("original")).unwrap();
    let before = m.entries().to_vec();
    // A generated summary is only a local value until the single record append.
    let completed_summary = draft(&a);
    drop(completed_summary);
    assert_eq!(m.entries(), before);
    assert!(m.append_compaction(draft(&"nonexistent".into())).is_err());
    assert_eq!(m.entries(), before);
    assert_eq!(
        m.build_session_projection().messages,
        vec![user("original")]
    );
}

#[test]
fn anchor_keeps_tool_pair() {
    let entries: Vec<SessionEntry> = serde_json::from_value(json!([
        {"type":"message","id":"u","parentId":null,"message":{"role":"user","content":"original","timestamp":1}},
        {"type":"message","id":"a","parentId":"u","message":{"role":"assistant","api":"faux","provider":"faux","model":"faux-1","content":[{"type":"toolCall","id":"call","name":"read","arguments":{}}],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":2}},
        {"type":"message","id":"r","parentId":"a","message":{"role":"toolResult","toolCallId":"call","toolName":"read","content":[{"type":"text","text":"result"}],"isError":false,"timestamp":3}},
        {"type":"message","id":"next","parentId":"r","message":{"role":"user","content":"next","timestamp":4}}
    ])).unwrap();
    let mut m = SessionManager::in_memory(&std::env::temp_dir(), CreateOptions::default(), entries);
    for bad in ["nonexistent", "a", "r"] {
        let before = m.entries().len();
        assert!(m.append_compaction(draft(&bad.into())).is_err(), "{bad}");
        assert_eq!(m.entries().len(), before);
    }
    let id = m.append_compaction(draft(&"next".into())).unwrap();
    assert!(matches!(
        m.entry(&id).unwrap().kind,
        EntryKind::CompactionState(_)
    ));
}

#[test]
fn epoch_reload_and_legacy() {
    let mut m = manager();
    let a = m.append_message(user("old")).unwrap();
    let b = m.append_message(user("kept")).unwrap();
    m.append_compaction(draft(&b)).unwrap();
    let expected = m.build_session_projection().messages;
    // A corrupt newer state must leave the preceding valid model view intact.
    let mut bad = serde_json::to_value(m.entries().last().unwrap()).unwrap();
    bad["id"] = json!("bad");
    bad["parentId"] = json!(m.entries().last().unwrap().id);
    bad["anchorMessage"]["id"] = json!("nonexistent");
    let mut entries = m.entries().to_vec();
    entries.push(serde_json::from_value(bad).unwrap());
    let restored =
        SessionManager::in_memory(&std::env::temp_dir(), CreateOptions::default(), entries);
    let projection = restored.build_session_projection();
    assert_eq!(projection.messages, expected);
    assert_eq!(projection.errors.len(), 1);
    assert!(!projection.messages.contains(&user("old")));
    let _ = a;
}

#[test]
fn deterministic_summary_bytes() {
    let mut m = manager();
    m.append_message(user("old")).unwrap();
    let anchor = m.append_message(user("keep")).unwrap();
    let id = m.append_compaction(draft(&anchor)).unwrap();
    let expected = serde_json::to_vec(&oino_session::convert_to_llm(
        &m.build_session_projection().messages,
    ))
    .unwrap();
    let next = m.append_message(user("tail")).unwrap();
    assert_eq!(
        m.entry(&next).unwrap().compaction_summary_id.as_ref(),
        Some(&id)
    );
    let entries: Vec<SessionEntry> =
        serde_json::from_slice(&serde_json::to_vec(&m.entries()[..m.entries().len() - 1]).unwrap())
            .unwrap();
    let restored =
        SessionManager::in_memory(&std::env::temp_dir(), CreateOptions::default(), entries);
    assert_eq!(
        expected,
        serde_json::to_vec(&oino_session::convert_to_llm(
            &restored.build_session_projection().messages
        ))
        .unwrap()
    );
    let text = String::from_utf8(expected).unwrap();
    assert!(text.contains("A previous instance of Oino"));
    assert!(text.contains("IMPORTANT: This summary was created"));
}

#[tokio::test]
async fn sent_context_edit_audit_exact_repro() {
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("ok".into()),
        FauxResponse::Text("ok".into()),
    ]);
    let session = TestSession::default().build(provider.clone()).await;
    turn(&session, "original").await;
    let target = session.with_manager(|m| {
        m.branch(None)
            .into_iter()
            .find(|e| {
                e.message().is_some_and(|msg| {
                    msg == &user("original")
                        || matches!(msg, AgentMessage::User(u) if u.content.text("") == "original")
                })
            })
            .unwrap()
            .id
            .clone()
    });
    let rejected = session.with_manager(|m| {
        m.append_context_edit(&target, Some(EditableContent::Text("rewritten".into())))
    });
    assert!(rejected.is_err());
    turn(&session, "next").await;
    let requests = provider.requests();
    for request in &requests[..2] {
        let text = request["context"]["messages"].to_string();
        assert!(text.contains("original"));
        assert!(!text.contains("rewritten"));
    }
}

#[test]
fn vcc_and_llm_triggers() {
    for method in ["vcc", "llm"] {
        let mut m = manager();
        m.append_message(user("old")).unwrap();
        let anchor = m.append_message(user("keep")).unwrap();
        let mut d = draft(&anchor);
        d.details = Some(json!({"method":method}));
        let id = m.append_compaction(d).unwrap();
        let state = serde_json::to_value(m.entry(&id).unwrap()).unwrap();
        assert_eq!(state["type"], "compaction_state");
        assert_eq!(state["summaryKind"], format!("{method}_summary"));
    }
}

#[tokio::test]
async fn overflow_retry_does_not_rerun_tools() {
    use oino_core::extensions::api::Registration;
    use oino_core::testing::FakeExtension;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let extension = Arc::new(FakeExtension::new("probe", Registration {
        tools: vec![serde_json::from_value(json!({"name":"effect","label":"Effect","description":"effect counter","parameters":{"type":"object","properties":{}},"annotations":{"readOnly":true}})).unwrap()],
        ..Default::default()
    }).on_tool(move |_, _| {
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(oino_agent::AgentToolResult::text("completed once"))
    }));
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("old answer".into()),
        FauxResponse::ToolCalls(vec![("call".into(), "effect".into(), Default::default())]),
        FauxResponse::Error("context length exceeded".into()),
        FauxResponse::Text("<summary>Old work preserved.</summary>".into()),
        FauxResponse::Text("recovered".into()),
    ]);
    let session = TestSession {
        extensions: vec![extension],
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":1}})).unwrap(),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session, "old user").await;
    turn(&session, "current user").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        session
            .entries()
            .iter()
            .any(|e| matches!(e.kind, EntryKind::CompactionState(_)))
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 5, "{requests:#?}");
    let retried = requests.last().unwrap()["context"]["messages"].to_string();
    assert!(retried.contains("completed once"));
    assert!(retried.contains("current user"));
}

#[tokio::test]
async fn extension_transform_cannot_rewrite_sent_prefix() {
    use oino_core::{extensions::api::Registration, testing::FakeExtension};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let turns = Arc::new(AtomicUsize::new(0));
    let observed = turns.clone();
    let extension = Arc::new(
        FakeExtension::new(
            "transform",
            Registration {
                events: vec!["context".into()],
                ..Default::default()
            },
        )
        .on_event(move |_, mut payload| {
            if observed.fetch_add(1, Ordering::SeqCst) > 0 {
                if let Some(messages) = payload["messages"].as_array_mut() {
                    for message in messages {
                        if message["role"] == "user" && message["content"] == "original" {
                            message["content"] = json!("rewritten");
                        }
                    }
                }
            }
            Ok(Some(payload))
        }),
    );
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("ok".into()),
        FauxResponse::Text("ok".into()),
    ]);
    let session = TestSession {
        extensions: vec![extension],
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session, "original").await;
    turn(&session, "next").await;
    assert_eq!(turns.load(Ordering::SeqCst), 2);
    for request in provider.requests() {
        let text = request["context"]["messages"].to_string();
        assert!(text.contains("original"));
        assert!(!text.contains("rewritten"));
    }
}

#[tokio::test]
async fn crash_before_commit_persists_no_state() {
    let dir = tempfile::tempdir().unwrap();
    let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = entered.clone();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("ok".into()),
        FauxResponse::Text("ok".into()),
    ]);
    let session = TestSession {
        session_dir: Some(dir.path().into()),
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":1}})).unwrap(),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session, "older user").await;
    turn(&session, "recent user").await;
    provider.route(move |request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            return Some(FauxResponse::Never);
        }
        None
    });
    let before = session.entries();
    let compacting = session.clone();
    let task = tokio::spawn(async move { compacting.compact(None).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !entered.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(session.entries(), before);
    assert!(
        !session.is_compacting(),
        "dropped compaction must release the session"
    );
    turn(&session, "message after cancelled compaction").await;
    let path = session.with_manager(|m| m.session_file().unwrap().to_path_buf());
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(!raw.contains("\"type\":\"compaction_state\""));
}

#[tokio::test]
async fn three_delta_compactions_replace_todos_and_keep_verbatim_prompts() {
    let provider = FauxProvider::new(vec![]);
    provider.route(|request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            Some(FauxResponse::Text(
                "<summary>Useful continuation state</summary>".into(),
            ))
        } else {
            Some(FauxResponse::Text("done".into()))
        }
    });
    let session = TestSession {
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":1}})).unwrap(),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    for epoch in 0..3 {
        turn(&session, &format!("older {epoch}")).await;
        turn(&session, &format!("recent {epoch}")).await;
        session
            .compact(Some("Preserve exact paths".into()))
            .await
            .unwrap();
    }
    let requests = provider.requests();
    let summaries = requests
        .iter()
        .filter(|request| {
            request["context"]["messages"][0]
                .to_string()
                .contains("Summarize this conversation or agent trajectory")
        })
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 3);
    let first = summaries[0]["context"]["messages"].to_string();
    assert!(first.contains("Please summarize the following conversation:"));
    assert!(first.contains("Preserve exact paths"));
    assert!(!first.contains("Chronological Play-by-Play"));
    for summary in &summaries[1..] {
        let text = summary["context"]["messages"].to_string();
        assert!(text.contains("Previous summary:"));
        assert!(text.contains("These messages were sent since the previous summary:"));
        assert!(!text.contains("A previous instance of Oino"));
    }
    assert_eq!(
        session
            .entries()
            .iter()
            .filter(|entry| matches!(entry.kind, EntryKind::CompactionState(_)))
            .count(),
        3
    );
}

#[tokio::test]
async fn threshold_failure_is_nonfatal_and_suppressed_until_usage_drops() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let summaries = Arc::new(AtomicUsize::new(0));
    let counted = summaries.clone();
    let provider = FauxProvider::new(vec![]);
    provider.route(move |request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            counted.fetch_add(1, Ordering::SeqCst);
            return Some(FauxResponse::Error("summary not available".into()));
        }
        Some(FauxResponse::Message {
            content: vec![oino_ai::AssistantContent::text("done")],
            stop_reason: oino_ai::StopReason::Stop,
            usage: oino_ai::Usage {
                input: 10,
                cache_read: 100,
                output: 1000,
                ..Default::default()
            },
            error_message: None,
        })
    });
    let session = TestSession {
        settings: serde_json::from_value(
            json!({"compaction":{"tokenLimit":50,"keepRecentTokens":1},"retry":{"enabled":false}}),
        )
        .unwrap(),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session, "older").await;
    // Initial attempt may have no removable turn. Change the limit to reset suppression.
    session.set_compaction_token_limit(Some(51));
    turn(&session, "newer").await;
    let attempted = summaries.load(Ordering::SeqCst);
    assert!(attempted > 0);
    turn(&session, "next").await;
    assert_eq!(summaries.load(Ordering::SeqCst), attempted);
    assert!(
        !session
            .entries()
            .iter()
            .any(|e| matches!(e.kind, EntryKind::CompactionState(_)))
    );
    assert!(
        session
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::User(u) if u.content.text("") == "older"))
    );
}

#[test]
fn sent_frontier_survives_reload_and_leaves_new_tail_editable() {
    let mut m = manager();
    let original = m.append_message(user("original")).unwrap();
    m.mark_sent(&m.build_session_projection().messages).unwrap();
    let mut reopened = SessionManager::in_memory(
        &std::env::temp_dir(),
        CreateOptions::default(),
        m.entries().to_vec(),
    );
    assert!(
        reopened
            .append_context_edit(&original, Some(EditableContent::Text("rewritten".into())))
            .is_err()
    );
    let tail = reopened.append_message(user("unsent")).unwrap();
    reopened
        .append_context_edit(&tail, Some(EditableContent::Text("allowed".into())))
        .unwrap();
    assert_eq!(
        reopened.build_session_projection().messages,
        vec![user("original"), user("allowed")]
    );
}

#[tokio::test]
async fn soft_cap_and_todo_replacement() {
    let provider = FauxProvider::new(vec![]);
    provider.route(|request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            Some(FauxResponse::Text(
                "<summary>Updated facts</summary>".into(),
            ))
        } else {
            Some(FauxResponse::Text("done".into()))
        }
    });
    let session = TestSession {
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":1}})).unwrap(),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session, "older").await;
    turn(&session, "keep").await;
    session.with_manager(|manager| {
        let anchor = manager
            .branch(None)
            .iter()
            .filter(|e| matches!(e.message(), Some(AgentMessage::User(_))))
            .last()
            .unwrap()
            .id
            .clone();
        let mut state = manager.prepare_compaction_state(draft(&anchor)).unwrap();
        state.summary_text = format!(
            "{}\nCurrent TODO List:\nobsolete item",
            "facts ".repeat(5000)
        );
        manager.append_compaction_state(state).unwrap();
    });
    turn(&session, "new delta").await;
    session.compact(None).await.unwrap();
    let request = provider
        .requests()
        .into_iter()
        .find(|r| {
            r["context"]["messages"][0]
                .to_string()
                .contains("Summarize this conversation or agent trajectory")
        })
        .unwrap();
    let text = request["context"]["messages"].to_string();
    assert!(text.contains(oino_core::compaction::prompts::SOFT_CAP_INSTRUCTION));
    assert!(!text.contains("obsolete item"));
}

#[test]
fn off_branch_anchor_rejected_without_write() {
    let mut m = manager();
    let root = m.append_message(user("root")).unwrap();
    let other = m.append_message(user("other branch")).unwrap();
    m.set_leaf(&root).unwrap();
    m.append_message(user("active branch")).unwrap();
    let before = m.entries().to_vec();
    assert!(m.append_compaction(draft(&other)).is_err());
    assert_eq!(m.entries(), before);
}

#[test]
fn retained_skills_images_and_current_todos_are_durable() {
    let mut m = manager();
    m.append_message(user("original")).unwrap();
    m.append_custom_entry("unipi:skills-revealed", Some(json!({"names":["rust"]})))
        .unwrap();
    let anchor = m.append_message(user("kept")).unwrap();
    let mut state = m.prepare_compaction_state(draft(&anchor)).unwrap();
    state.retained_images.push(oino_session::RetainedImage {
        path: "/tmp/diagram.png".into(),
        excerpt: "architecture".into(),
        image_index: 0,
        image_count: 1,
    });
    state.loaded_deferred_tool_names = vec!["mcp_search".into()];
    m.append_compaction_state(state).unwrap();
    m.append_message(user("new turn")).unwrap();
    let d = draft(&anchor);
    let state = m.prepare_compaction_state(d).unwrap();
    assert_eq!(state.retained_skills, ["rust"]);
    assert_eq!(state.loaded_deferred_tool_names, ["mcp_search"]);
    assert_eq!(state.retained_images[0].path, "/tmp/diagram.png");
    let bytes = serde_json::to_vec(&state).unwrap();
    let restored: oino_session::CompactionState = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(state.injected_text(), restored.injected_text());
}

#[test]
fn adaptive_prompt_matches_recovered_body_verbatim() {
    let reference = include_str!("../../../docs/dev/v3/ref-droid-prompts.md");
    let adaptive = reference
        .split("### 5.1 Adaptive body (newer)\n```text\n")
        .nth(1)
        .unwrap()
        .split("\n```")
        .next()
        .unwrap();
    let template = oino_core::compaction::prompts::SUMMARIZATION_SYSTEM_PROMPT;
    let before = adaptive.split("{IF previous summary:").next().unwrap();
    let after = adaptive
        .split("guidelines provided below.\"}")
        .nth(1)
        .unwrap();
    assert!(template.starts_with(before));
    assert!(template.ends_with(after));
    assert_eq!(template, format!("{before}{{continuation}}{after}"));
}

#[tokio::test]
async fn empty_summary_escalates_once_then_preserves_history() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    let provider = FauxProvider::new(vec![]);
    provider.route(move |request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(FauxResponse::Text("<summary></summary>".into()))
        } else {
            Some(FauxResponse::Text("ok".into()))
        }
    });
    let session = TestSession {
        settings: serde_json::from_value(
            json!({"compaction":{"keepRecentTokens":1},"retry":{"enabled":false}}),
        )
        .unwrap(),
        ..Default::default()
    }
    .build(provider)
    .await;
    turn(&session, "older").await;
    turn(&session, "recent").await;
    let before = session.entries();
    assert!(session.compact(None).await.is_err());
    assert_eq!(
        count.load(std::sync::atomic::Ordering::SeqCst),
        7,
        "two samples per shrink step, then one escalation request"
    );
    assert_eq!(session.entries(), before);
}

#[tokio::test]
async fn precompact_blocks_manual_only_and_invalid_extension_anchor_writes_nothing() {
    use oino_core::{extensions::api::Registration, testing::FakeExtension};
    use std::sync::Arc;
    let extension = Arc::new(
        FakeExtension::new(
            "block",
            Registration {
                events: vec!["session_before_compact".into()],
                ..Default::default()
            },
        )
        .on_event(|_, _| Ok(Some(json!({"cancel":true})))),
    );
    let provider = FauxProvider::new(vec![]);
    provider.route(|request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            Some(FauxResponse::Text(
                "<summary>retained work</summary>".into(),
            ))
        } else {
            Some(FauxResponse::Message {
                content: vec![oino_ai::AssistantContent::text("ok")],
                stop_reason: oino_ai::StopReason::Stop,
                usage: oino_ai::Usage {
                    input: 100,
                    ..Default::default()
                },
                error_message: None,
            })
        }
    });
    let session = TestSession {
        extensions: vec![extension],
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":1}})).unwrap(),
        ..Default::default()
    }
    .build(provider)
    .await;
    turn(&session, "old").await;
    turn(&session, "keep").await;
    let before = session.entries();
    assert!(session.compact(None).await.is_err());
    assert_eq!(session.entries(), before);
    session.set_compaction_token_limit(Some(50));
    turn(&session, "threshold").await;
    assert!(
        session
            .entries()
            .iter()
            .any(|e| matches!(e.kind, EntryKind::CompactionState(_)))
    );

    let invalid = Arc::new(FakeExtension::new("invalid", Registration { events:vec!["session_before_compact".into()], ..Default::default() })
        .on_event(|_, _| Ok(Some(json!({"compaction":{"summary":"bad","firstKeptEntryId":"nonexistent","tokensBefore":1}})))));
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("ok".into()),
        FauxResponse::Text("ok".into()),
    ]);
    let session = TestSession {
        extensions: vec![invalid],
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":1}})).unwrap(),
        ..Default::default()
    }
    .build(provider)
    .await;
    turn(&session, "old").await;
    turn(&session, "keep").await;
    let before = session.entries();
    assert!(session.compact(None).await.is_err());
    assert_eq!(session.entries(), before);
}

#[tokio::test]
async fn single_turn_overflow_compacts_and_recovers_without_second_user_turn() {
    use oino_core::extensions::api::Registration;
    use oino_core::testing::FakeExtension;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let extension = Arc::new(
        FakeExtension::new(
            "probe",
            Registration {
                tools: vec![serde_json::from_value(json!({
                    "name": "big_tool",
                    "label": "BigTool",
                    "description": "single turn tool",
                    "parameters": {"type": "object", "properties": {}},
                    "annotations": {"readOnly": true}
                }))
                .unwrap()],
                ..Default::default()
            },
        )
        .on_tool(move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(oino_agent::AgentToolResult::text("x".repeat(4096)))
        }),
    );
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![("call-1".into(), "big_tool".into(), Default::default())]),
        FauxResponse::Error("prompt is too long: 1013743 tokens > 1000000 maximum".into()),
        FauxResponse::Text("<summary>Single turn work preserved.</summary>".into()),
        FauxResponse::Text("recovered single turn".into()),
    ]);
    let session = TestSession {
        extensions: vec![extension],
        settings: serde_json::from_value(json!({"compaction":{"keepRecentTokens":20000}})).unwrap(),
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session, "run autonomous task").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        session
            .entries()
            .iter()
            .any(|e| matches!(e.kind, EntryKind::CompactionState(_)))
    );
    let final_messages = session.messages();
    assert!(
        final_messages.iter().any(|m| match m {
            AgentMessage::Assistant(a) => a
                .content
                .iter()
                .any(|c| matches!(c, oino_ai::AssistantContent::Text(t) if t.text == "recovered single turn")),
            _ => false,
        }),
        "{final_messages:#?}"
    );
}

#[test]
fn forward_snap_and_custom_message_anchor_when_first_turn_exceeds_keep_budget() {
    let big_result = "payload ".repeat(20_000);
    let entries: Vec<SessionEntry> = serde_json::from_value(json!([
        {"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"start lead","timestamp":1}},
        {"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","api":"faux","provider":"faux","model":"faux-1","content":[{"type":"toolCall","id":"c1","name":"recall","arguments":{}}],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":2}},
        {"type":"message","id":"r1","parentId":"a1","message":{"role":"toolResult","toolCallId":"c1","toolName":"recall","content":[{"type":"text","text":big_result}],"isError":false,"timestamp":3}},
        {"type":"custom_message","id":"cm1","parentId":"r1","customType":"subagent-completion","content":"Subagent finished OIN-36","display":true}
    ]))
    .unwrap();
    let refs: Vec<&SessionEntry> = entries.iter().collect();
    let settings = oino_core::compaction::CompactionSettings {
        enabled: true,
        reserve_tokens: 0,
        keep_recent_tokens: 20_000,
    };
    let prepared = oino_core::compaction::prepare_compaction(&refs, &settings)
        .expect("should snap forward onto custom_message boundary");
    assert_eq!(prepared.first_kept_entry_id, EntryId::new("cm1"));
    assert_eq!(prepared.messages_to_summarize.len(), 3);
}

#[tokio::test]
async fn threshold_suppression_resets_after_commit_and_rearms_on_further_growth() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let summaries = Arc::new(AtomicUsize::new(0));
    let counted = summaries.clone();
    let usage_step = Arc::new(AtomicUsize::new(0));
    let step = usage_step.clone();
    let provider = FauxProvider::new(vec![]);
    provider.route(move |request| {
        if request["context"]["messages"][0]
            .to_string()
            .contains("Summarize this conversation or agent trajectory")
        {
            counted.fetch_add(1, Ordering::SeqCst);
            return Some(FauxResponse::Text("<summary>Epoch summary</summary>".into()));
        }
        let n = step.fetch_add(1, Ordering::SeqCst);
        let input = match n {
            0 => 10,
            1 => 80, // crosses limit=50 -> triggers compaction #1
            2 => 70, // still > 50 in new epoch -> triggers compaction #2 because commit cleared latch
            _ => 10,
        };
        Some(FauxResponse::Message {
            content: vec![oino_ai::AssistantContent::text("ok")],
            stop_reason: oino_ai::StopReason::Stop,
            usage: oino_ai::Usage {
                input,
                ..Default::default()
            },
            error_message: None,
        })
    });
    let session = TestSession {
        settings: serde_json::from_value(
            json!({"compaction":{"tokenLimit":50,"keepRecentTokens":1},"retry":{"enabled":false}}),
        )
        .unwrap(),
        ..Default::default()
    }
    .build(provider)
    .await;
    turn(&session, "t1").await;
    turn(&session, "t2").await;
    assert_eq!(summaries.load(Ordering::SeqCst), 1);
    turn(&session, "t3").await;
    assert_eq!(
        summaries.load(Ordering::SeqCst),
        2,
        "post-compaction usage remaining above limit must not leave threshold compaction permanently latched"
    );
}
